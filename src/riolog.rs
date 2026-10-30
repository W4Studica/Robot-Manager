use crate::output::LineSink;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

type Frame = Arc<Vec<u8>>;
struct Client {
    pending: mpsc::SyncSender<Frame>,
    socket: TcpStream,
    worker: JoinHandle<()>,
}
struct State {
    sequence: i16,
    history: VecDeque<Frame>,
    history_limit: usize,
    queue_limit: usize,
    max_clients: usize,
    next_client: u64,
    clients: HashMap<u64, Client>,
    stopped: bool,
}
#[derive(Clone)]
pub struct Publisher {
    state: Arc<Mutex<State>>,
}
impl LineSink for Publisher {
    fn publish(&self, line: &str) -> io::Result<()> {
        let mut removed = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            if state.stopped {
                return Ok(());
            }
            let frame = Arc::new(frame(line, state.sequence));
            state.sequence = state.sequence.wrapping_add(1);
            if state.history_limit > 0 {
                if state.history.len() == state.history_limit {
                    state.history.pop_front();
                }
                state.history.push_back(frame.clone());
            }
            let stale: Vec<_> = state
                .clients
                .iter()
                .filter_map(|(id, c)| c.pending.try_send(frame.clone()).err().map(|_| *id))
                .collect();
            for id in stale {
                if let Some(client) = state.clients.remove(&id) {
                    removed.push(client);
                }
            }
        }
        // Socket shutdown happens outside the publication lock; no blocking IO
        // or joins run on the robot's output reader.
        for client in removed {
            let _ = client.socket.shutdown(Shutdown::Both);
        }
        Ok(())
    }
}
pub struct RioLogServer {
    publisher: Publisher,
    stopping: Arc<AtomicBool>,
    acceptor: Option<JoinHandle<()>>,
    address: SocketAddr,
}
impl RioLogServer {
    pub fn start(port: u16, history: usize) -> io::Result<Self> {
        Self::with_limits(port, history, 1024, 8)
    }
    pub fn with_limits(
        port: u16,
        history: usize,
        queue_limit: usize,
        max_clients: usize,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let publisher = Publisher {
            state: Arc::new(Mutex::new(State {
                sequence: 0,
                history: VecDeque::new(),
                history_limit: history,
                queue_limit: queue_limit.max(1),
                max_clients: max_clients.max(1),
                next_client: 0,
                clients: HashMap::new(),
                stopped: false,
            })),
        };
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let state = publisher.state.clone();
        let acceptor = thread::Builder::new()
            .name("riolog-listener".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((socket, peer)) => {
                            if let Err(error) = add_client(&state, socket) {
                                crate::log(
                                    "WARNING",
                                    format_args!("RioLog client {peer}: {error}"),
                                );
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(error) => {
                            crate::log("ERROR", format_args!("RioLog accept failed: {error}"));
                            break;
                        }
                    }
                }
            })?;
        crate::log(
            "INFO",
            format_args!("RioLog console listening on TCP port {}", address.port()),
        );
        Ok(Self {
            publisher,
            stopping,
            acceptor: Some(acceptor),
            address,
        })
    }
    pub fn publisher(&self) -> Publisher {
        self.publisher.clone()
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn client_count(&self) -> usize {
        self.publisher.state.lock().unwrap().clients.len()
    }
    pub fn stop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        let clients: Vec<_> = {
            let mut state = self.publisher.state.lock().unwrap();
            state.stopped = true;
            state.clients.drain().map(|(_, client)| client).collect()
        };
        for client in &clients {
            let _ = client.socket.shutdown(Shutdown::Both);
        }
        if let Some(worker) = self.acceptor.take() {
            let _ = worker.join();
        }
        for client in clients {
            let _ = client.worker.join();
        }
    }
}
impl Drop for RioLogServer {
    fn drop(&mut self) {
        self.stop();
    }
}
fn add_client(state: &Arc<Mutex<State>>, mut socket: TcpStream) -> io::Result<()> {
    socket.set_nodelay(true)?;
    socket.set_write_timeout(Some(Duration::from_secs(1)))?;
    socket.set_read_timeout(Some(Duration::from_millis(50)))?;
    let control = socket.try_clone()?;
    let mut guard = state.lock().unwrap();
    if guard.stopped || guard.clients.len() >= guard.max_clients {
        let _ = socket.shutdown(Shutdown::Both);
        return Ok(());
    }
    let (sender, receiver) = mpsc::sync_channel(guard.queue_limit);
    // History registration and live publication share one lock, preserving order.
    for entry in guard
        .history
        .iter()
        .skip(guard.history.len().saturating_sub(guard.queue_limit))
    {
        sender
            .try_send(entry.clone())
            .map_err(|_| io::Error::other("history queue full"))?;
    }
    let id = guard.next_client;
    guard.next_client = guard.next_client.wrapping_add(1);
    let shared = Arc::downgrade(state);
    let worker = thread::Builder::new()
        .name("riolog-client".into())
        .spawn(move || {
            loop {
                match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(frame) => {
                        let mut batch = Vec::with_capacity(4096);
                        batch.extend_from_slice(&frame);
                        while batch.len() < 4096 {
                            match receiver.try_recv() {
                                Ok(next) => batch.extend_from_slice(&next),
                                Err(_) => break,
                            }
                        }
                        if socket.write_all(&batch).is_err() {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if disconnected(&mut socket) {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            let _ = socket.shutdown(Shutdown::Both);
            if let Some(state) = shared.upgrade() {
                state.lock().unwrap().clients.remove(&id);
            }
        })?;
    guard.clients.insert(
        id,
        Client {
            pending: sender,
            socket: control,
            worker,
        },
    );
    Ok(())
}
fn disconnected(socket: &mut TcpStream) -> bool {
    let mut poll = libc::pollfd {
        fd: socket.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll receives one initialized entry and does not retain its pointer.
    let ready = unsafe { libc::poll(&mut poll, 1, 0) };
    if ready < 0 {
        return io::Error::last_os_error().kind() != io::ErrorKind::Interrupted;
    }
    if poll.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
        return true;
    }
    if poll.revents & libc::POLLIN != 0 {
        let mut discard = [0; 1024];
        return match socket.read(&mut discard) {
            Ok(0) => true,
            Ok(_) => false,
            Err(error) => !matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
            ),
        };
    }
    false
}
pub fn frame(line: &str, sequence: i16) -> Vec<u8> {
    let mut size = line.len().min(0xffff - 3 - 6);
    while !line.is_char_boundary(size) {
        size -= 1;
    }
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: now is writable and CLOCK_MONOTONIC has no process-global side effects.
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now);
    }
    let timestamp = (now.tv_sec as f64 + now.tv_nsec as f64 * 1e-9) as f32;
    let mut bytes = Vec::with_capacity(size + 9);
    bytes.extend_from_slice(&((size + 7) as u16).to_be_bytes());
    bytes.push(12);
    bytes.extend_from_slice(&timestamp.to_be_bytes());
    bytes.extend_from_slice(&sequence.to_be_bytes());
    bytes.extend_from_slice(&line.as_bytes()[..size]);
    bytes
}
