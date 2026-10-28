use robot_manager::output::{Journal, OutputReader};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[test]
fn output_shutdown_is_bounded_even_when_an_escaped_writer_keeps_sending() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let pipe = File::from(OwnedFd::from(reader));
    let mut pump = OutputReader::start(
        pipe,
        Journal::with_writer(Box::new(io::sink())).unwrap(),
        None,
    )
    .unwrap();
    let stopping = Arc::new(AtomicBool::new(false));
    let stop = stopping.clone();
    let worker = thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            if writer.write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        }
    });
    thread::sleep(Duration::from_millis(20));
    let began = Instant::now();
    pump.stop();
    stopping.store(true, Ordering::Relaxed);
    worker.join().unwrap();
    assert!(began.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_blocked_journal_does_not_block_the_output_pipe() {
    struct Blocked(Arc<(Mutex<bool>, Condvar)>);
    impl Write for Blocked {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let (lock, changed) = &*self.0;
            let mut release = lock.lock().unwrap();
            while !*release {
                release = changed.wait(release).unwrap();
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let state = Arc::new((Mutex::new(false), Condvar::new()));
    let journal = Journal::with_writer(Box::new(Blocked(state.clone()))).unwrap();
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let mut pump = OutputReader::start(File::from(OwnedFd::from(reader)), journal, None).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let worker = thread::spawn(move || {
        for _ in 0..1024 {
            if writer.write_all(&[b'x'; 8192]).is_err() {
                break;
            }
        }
        finished.store(true, Ordering::Relaxed);
    });
    let end = Instant::now() + Duration::from_secs(5);
    while !done.load(Ordering::Relaxed) && Instant::now() < end {
        thread::sleep(Duration::from_millis(10));
    }
    let drained = done.load(Ordering::Relaxed);
    pump.stop();
    *state.0.lock().unwrap() = true;
    state.1.notify_all();
    worker.join().unwrap();
    assert!(drained, "journal stalled robot output");
}
