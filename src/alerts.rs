//! Loopback alert lines: system daemons (hwmon, mrccomm) send one UTF-8 line
//! per datagram to 127.0.0.1:1743 and the supervisor republishes it into the
//! journal and the RioLog console, so hardware and Driver Station events show
//! up in VS Code instead of only in the robot's journal.
use crate::output::LineSink;
use std::io;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Default loopback port for alert lines.
pub const DEFAULT_ALERT_PORT: u16 = 1743;
/// One datagram is one line; longer payloads are dropped.
pub const MAX_ALERT_LINE: usize = 512;
/// Prefix added to every republished line.
pub const ALERT_PREFIX: &str = "[alert] ";

pub struct AlertListener {
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    address: std::net::SocketAddr,
}

impl AlertListener {
    pub fn start(port: u16, console: Option<Arc<dyn LineSink>>) -> io::Result<Self> {
        let socket = UdpSocket::bind(("127.0.0.1", port))?;
        socket.set_read_timeout(Some(Duration::from_millis(100)))?;
        let address = socket.local_addr()?;
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let worker = thread::Builder::new()
            .name("alert-listener".into())
            .spawn(move || {
                let mut buffer = [0u8; MAX_ALERT_LINE + 1];
                while !stop.load(Ordering::Relaxed) {
                    match socket.recv_from(&mut buffer) {
                        Ok((size, peer)) => {
                            if size > MAX_ALERT_LINE || !peer.ip().is_loopback() {
                                continue;
                            }
                            if let Ok(text) = std::str::from_utf8(&buffer[..size]) {
                                let line = text.trim();
                                if !line.is_empty() {
                                    publish(&console, line);
                                }
                            }
                        }
                        Err(error)
                            if error.kind() == io::ErrorKind::WouldBlock
                                || error.kind() == io::ErrorKind::TimedOut => {}
                        Err(_) => break,
                    }
                }
            })?;
        crate::log(
            "INFO",
            format_args!("alert listener on {}", address),
        );
        Ok(Self {
            stopping,
            worker: Some(worker),
            address,
        })
    }

    pub fn address(&self) -> std::net::SocketAddr {
        self.address
    }

    pub fn stop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for AlertListener {
    fn drop(&mut self) {
        self.stop();
    }
}

fn publish(console: &Option<Arc<dyn LineSink>>, line: &str) {
    let text = format!("{ALERT_PREFIX}{line}");
    crate::log("WARNING", &text);
    if let Some(console) = console {
        let _ = console.publish(&text);
    }
}

/// Best-effort sender used by the daemons; a missing supervisor must never
/// disturb the caller.
pub fn send(port: u16, line: &str) {
    if line.len() > MAX_ALERT_LINE {
        return;
    }
    if let Ok(socket) = UdpSocket::bind(("127.0.0.1", 0)) {
        let _ = socket.send_to(line.as_bytes(), ("127.0.0.1", port));
    }
}
