use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub trait LineSink: Send + Sync {
    fn publish(&self, line: &str) -> io::Result<()>;
}

// The output reader never blocks on a broken journal. Its bounded queue is
// shared across robot restarts, so at most one writer can be blocked in stdout.
#[derive(Clone)]
pub struct Journal {
    sender: mpsc::SyncSender<Vec<u8>>,
}
impl Journal {
    pub fn stdout() -> io::Result<Self> {
        Self::with_writer(Box::new(io::stdout()))
    }
    pub fn with_writer(mut writer: Box<dyn Write + Send>) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(256);
        thread::Builder::new()
            .name("robot-journal".into())
            .spawn(move || {
                while let Ok(bytes) = receiver.recv() {
                    if let Err(error) = writer.write_all(&bytes).and_then(|()| writer.flush()) {
                        crate::log(
                            "WARNING",
                            format_args!("could not write robot output to journal: {error}"),
                        );
                        break;
                    }
                }
            })?;
        Ok(Self { sender })
    }
    fn write(&self, bytes: &[u8]) {
        let _ = self.sender.try_send(bytes.to_vec());
    }
}
pub struct OutputReader {
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl OutputReader {
    pub fn start(
        mut pipe: File,
        journal: Journal,
        console: Option<Arc<dyn LineSink>>,
    ) -> io::Result<Self> {
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let worker = thread::Builder::new()
            .name("robot-output".into())
            .spawn(move || {
                let mut pending = Vec::new();
                let mut buffer = [0; 8192];
                let mut console = console;
                let mut drain_deadline = None;
                loop {
                    if stop.load(Ordering::Relaxed) {
                        let deadline = drain_deadline
                            .get_or_insert_with(|| Instant::now() + Duration::from_millis(200));
                        if Instant::now() >= *deadline {
                            break;
                        }
                    }
                    let mut entry = libc::pollfd {
                        fd: pipe.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    // SAFETY: entry is a live pollfd and the owned File keeps fd open.
                    let result = unsafe { libc::poll(&mut entry, 1, 50) };
                    if result < 0 {
                        if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                            continue;
                        }
                        break;
                    }
                    if result == 0 {
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        continue;
                    }
                    match pipe.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(size) => {
                            journal.write(&buffer[..size]);
                            for &byte in &buffer[..size] {
                                pending.push(byte);
                                if byte == b'\n' || pending.len() >= 65526 {
                                    publish(&mut console, &pending);
                                    pending.clear();
                                }
                            }
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                if !pending.is_empty() {
                    publish(&mut console, &pending);
                }
            })?;
        Ok(Self {
            stopping,
            worker: Some(worker),
        })
    }
    pub fn stop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for OutputReader {
    fn drop(&mut self) {
        self.stop();
    }
}
fn publish(console: &mut Option<Arc<dyn LineSink>>, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    if let Some(sink) = console {
        if let Err(error) = sink.publish(text.trim_end_matches(['\r', '\n'])) {
            crate::log(
                "WARNING",
                format_args!("RioLog forwarding stopped: {error}"),
            );
            *console = None;
        }
    }
}
