use robot_manager::alerts::{self, AlertListener, ALERT_PREFIX, MAX_ALERT_LINE};
use robot_manager::output::LineSink;
use std::io;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Capture {
    lines: Mutex<Vec<String>>,
}

impl LineSink for Capture {
    fn publish(&self, line: &str) -> io::Result<()> {
        self.lines.lock().unwrap().push(line.to_string());
        Ok(())
    }
}

impl Capture {
    fn contains(&self, needle: &str) -> bool {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains(needle))
    }
}

fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn alert_lines_are_republished_with_the_prefix() {
    let capture = Arc::new(Capture::default());
    let listener = AlertListener::start(0, Some(capture.clone() as Arc<dyn LineSink>)).unwrap();
    let port = listener.address().port();

    alerts::send(port, "hwmon: under-voltage detected");

    assert!(
        wait_until(|| capture.contains("under-voltage")),
        "alert must reach the console: {:?}",
        capture.lines.lock().unwrap()
    );
    assert!(capture.contains(&format!("{ALERT_PREFIX}hwmon:")));
}

#[test]
fn malformed_alert_datagrams_are_dropped() {
    let capture = Arc::new(Capture::default());
    let listener = AlertListener::start(0, Some(capture.clone() as Arc<dyn LineSink>)).unwrap();
    let port = listener.address().port();
    let client = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
    let target = ("127.0.0.1", port);

    client.send_to(&[0xff, 0xfe, 0xfd], target).unwrap(); // invalid UTF-8
    client.send_to(b"   \n", target).unwrap(); // empty after trimming
    client
        .send_to(&vec![b'x'; MAX_ALERT_LINE + 1], target)
        .unwrap(); // oversized
    client.send_to(b"valid line", target).unwrap();

    assert!(wait_until(|| capture.contains("valid line")));
    let lines = capture.lines.lock().unwrap().clone();
    assert_eq!(lines.len(), 1, "only the valid line may be published: {lines:?}");
}

#[test]
fn a_send_without_a_listener_never_fails() {
    // Port 1 on loopback has no listener; the daemon must not care.
    alerts::send(1, "nobody is listening");
}
