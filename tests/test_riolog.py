import os
import shutil
import socket
import struct
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import robot_manager as rm


def receive(sock):
    def exact(size):
        data = b""
        while len(data) < size:
            part = sock.recv(size - len(data))
            if not part:
                raise EOFError("RioLog disconnected")
            data += part
        return data
    size = struct.unpack(">H", exact(2))[0]
    frame = exact(size)
    kind = frame[0]
    _timestamp, sequence = struct.unpack(">fh", frame[1:7])
    if kind != 12:
        raise AssertionError("invalid NetConsole framing")
    return sequence, frame[7:].decode()


class BlockedSocket:
    """A connected client whose sender is deterministically stalled."""
    def __init__(self, sock):
        self.sock = sock
        self.sending = threading.Event()
        self.closed = threading.Event()

    def fileno(self):
        return self.sock.fileno()

    def settimeout(self, value):
        self.sock.settimeout(value)

    def recv(self, size):
        return self.sock.recv(size)

    def sendall(self, frame):
        self.sending.set()
        self.closed.wait(5)
        raise OSError("client stopped reading")

    def shutdown(self, how):
        self.closed.set()
        self.sock.shutdown(how)

    def close(self):
        self.closed.set()
        self.sock.close()


class RioLogTest(unittest.TestCase):
    def setUp(self):
        self.server = rm.RioLogServer(history=3, queue_limit=4, max_clients=2)
        self.addCleanup(self.server.stop)

    def connect(self, wrapper=lambda sock: sock):
        sender, receiver = socket.socketpair()
        receiver.settimeout(2)
        self.addCleanup(receiver.close)
        return self.server._add_client(wrapper(sender)), receiver

    def test_history_precedes_live_frames_and_sequence_wraps(self):
        self.server._sequence = 32767
        self.server.publish("before")
        self.server.publish("한글")
        _, receiver = self.connect()
        self.server.publish("after")
        self.assertEqual([receive(receiver) for _ in range(3)],
                         [(32767, "before"), (-32768, "한글"), (-32767, "after")])

    def test_slow_client_cannot_block_publication_or_a_healthy_client(self):
        client, _ = self.connect(BlockedSocket)
        self.server.publish("stall")
        self.assertTrue(client.socket.sending.wait(2))
        _, healthy = self.connect()
        self.assertEqual(receive(healthy)[1], "stall")
        # The first client's sender remains stalled while the healthy one receives.
        for index in range(6):
            self.server.publish(str(index))
            self.assertEqual(receive(healthy)[1], str(index))
        self.assertTrue(client.socket.closed.is_set())
        client.thread.join(2)
        self.assertFalse(client.thread.is_alive())

    def test_disconnect_is_cleaned_even_without_new_output(self):
        client, receiver = self.connect()
        receiver.close()
        client.thread.join(2)
        self.assertFalse(client.thread.is_alive())
        self.assertNotIn(client, self.server._clients)

    def test_connection_limit_and_stop_release_workers(self):
        first, _ = self.connect()
        second, _ = self.connect()
        rejected, receiver = self.connect()
        self.assertIsNone(rejected)
        self.assertEqual(receiver.recv(1), b"")
        self.server.stop()
        self.assertFalse(first.thread.is_alive())
        self.assertFalse(second.thread.is_alive())

    def test_history_is_limited_to_the_client_queue(self):
        self.server = rm.RioLogServer(history=20, queue_limit=2)
        self.addCleanup(self.server.stop)
        for index in range(10):
            self.server.publish(str(index))
        _, receiver = self.connect()
        self.assertEqual([receive(receiver)[1] for _ in range(2)], ["8", "9"])


class ForwardOutputTest(unittest.TestCase):
    """A failing console must never stop the robot program's output from being read (SIGPIPE would kill it)."""

    def run_program(self, riolog):
        directory = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, directory, True)
        command = os.path.join(directory, "robotCommand")
        with open(command, "w", encoding="utf-8") as f:
            f.write("i=0; while [ $i -lt 200 ]; do echo line$i; i=$((i+1)); done\n")
        manager = rm.Manager(command, directory, dict(os.environ), riolog=riolog)
        proc = manager._start()
        self.assertIsNotNone(proc)
        reader = proc._robot_output_reader
        reader.join(timeout=10)
        self.assertFalse(reader.is_alive())
        return proc.wait(timeout=10)

    def test_program_finishes_when_riolog_raises(self):
        class BrokenRioLog:
            calls = 0

            def publish(self, line):
                BrokenRioLog.calls += 1
                raise RuntimeError("broken")

        self.assertEqual(self.run_program(BrokenRioLog()), 0)
        self.assertEqual(BrokenRioLog.calls, 1)  # reported once, then skipped

    def test_program_finishes_when_the_journal_write_fails(self):
        class BrokenStdout:
            def write(self, text):
                raise OSError("journal gone")

            def flush(self):
                pass

        original = sys.stdout
        sys.stdout = BrokenStdout()
        try:
            code = self.run_program(None)
        finally:
            sys.stdout = original
        self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
