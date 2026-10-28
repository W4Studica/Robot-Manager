use robot_manager::output::LineSink;
use robot_manager::riolog::{frame, RioLogServer};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn connect(server: &RioLogServer) -> TcpStream {
    let address = SocketAddr::from(([127, 0, 0, 1], server.address().port()));
    let client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client
}
fn wait_until(mut predicate: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(5));
    }
}
fn receive(stream: &mut TcpStream) -> (i16, String) {
    let mut header = [0; 2];
    stream.read_exact(&mut header).unwrap();
    let mut bytes = vec![0; u16::from_be_bytes(header) as usize];
    stream.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes[0], 12);
    assert!(f32::from_be_bytes(bytes[1..5].try_into().unwrap()).is_finite());
    (
        i16::from_be_bytes(bytes[5..7].try_into().unwrap()),
        String::from_utf8(bytes[7..].to_vec()).unwrap(),
    )
}
#[test]
fn history_is_ordered_before_live_frames() {
    let server = RioLogServer::with_limits(0, 3, 4, 2).unwrap();
    let publisher = server.publisher();
    publisher.publish("before").unwrap();
    publisher.publish("한글").unwrap();
    let mut socket = connect(&server);
    wait_until(|| server.client_count() == 1);
    publisher.publish("after").unwrap();
    assert_eq!(receive(&mut socket), (0, "before".into()));
    assert_eq!(receive(&mut socket), (1, "한글".into()));
    assert_eq!(receive(&mut socket), (2, "after".into()));
}
#[test]
fn frame_layout_and_sequence_support_signed_wrap() {
    let before = frame("x", i16::MAX);
    let after = frame("y", i16::MAX.wrapping_add(1));
    assert_eq!(i16::from_be_bytes(before[7..9].try_into().unwrap()), 32767);
    assert_eq!(i16::from_be_bytes(after[7..9].try_into().unwrap()), -32768);
    assert_eq!(
        u16::from_be_bytes(before[..2].try_into().unwrap()) as usize,
        before.len() - 2
    );
    assert_eq!(before[2], 12);
}
#[test]
fn frame_length_is_bounded_and_utf8_stays_valid() {
    let bytes = frame(&"한글".repeat(50000), 0);
    assert!(bytes.len() <= 65535);
    assert!(std::str::from_utf8(&bytes[9..]).is_ok());
}
#[test]
fn history_is_limited_by_the_client_queue() {
    let server = RioLogServer::with_limits(0, 20, 2, 2).unwrap();
    for i in 0..10 {
        server.publisher().publish(&i.to_string()).unwrap();
    }
    let mut socket = connect(&server);
    assert_eq!(receive(&mut socket).1, "8");
    assert_eq!(receive(&mut socket).1, "9");
}
#[test]
fn disconnected_clients_are_removed_without_new_output() {
    let server = RioLogServer::start(0, 0).unwrap();
    let socket = connect(&server);
    wait_until(|| server.client_count() == 1);
    drop(socket);
    wait_until(|| server.client_count() == 0);
}
#[test]
fn client_limit_and_shutdown_close_connections() {
    let mut server = RioLogServer::with_limits(0, 0, 2, 1).unwrap();
    let mut first = connect(&server);
    wait_until(|| server.client_count() == 1);
    let mut rejected = connect(&server);
    assert_eq!(rejected.read(&mut [0]).unwrap(), 0);
    server.stop();
    server.stop();
    assert_eq!(first.read(&mut [0]).unwrap(), 0);
}
#[test]
fn a_slow_client_does_not_block_publication() {
    let server = RioLogServer::with_limits(0, 0, 2, 2).unwrap();
    let _slow = connect(&server);
    wait_until(|| server.client_count() == 1);
    let mut healthy = connect(&server);
    wait_until(|| server.client_count() == 2);
    let began = Instant::now();
    let line = "x".repeat(65500);
    for _ in 0..200 {
        server.publisher().publish(&line).unwrap();
        assert_eq!(receive(&mut healthy).1, line);
    }
    assert!(began.elapsed() < Duration::from_secs(5));
    wait_until(|| server.client_count() == 1);
    server.publisher().publish("still running").unwrap();
    assert_eq!(receive(&mut healthy).1, "still running");
    // Incoming client bytes are discarded rather than mistaken for disconnect.
    healthy.write_all(b"ignored").unwrap();
}
