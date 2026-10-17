#!/usr/bin/env python3
"""robot_manager: keeps the robot program running on a Studica VMX-pi.

Runs <robot_home>/robotCommand (the same contract as GradleRIO 2027) as the user the
service runs as (root, because the VMX HAL needs pigpio), restarts it when it exits,
and adds environment variables chosen from a platform profile (by kernel version).

It never opens the VMX hardware itself. Standard library only; works on Python 3.10.
"""

import argparse
from collections import deque
import json
import logging
import os
import platform
import queue
import select
import re
import socket
import struct
import signal
import subprocess
import sys
import threading
import time

log = logging.getLogger("robot_manager")

DEFAULT_CONFIG = "/etc/robot_manager/config.json"
ROBOT_COMMAND_FILE = "robotCommand"
DEFAULT_LSE_EMU_LIBRARY = "/opt/robot_manager/lib/libvmx_lse_emu.so"
RIOLOG_PORT = 1741


DEFAULT_STOP_GRACE = 5.0


def stop_grace_from_config(config):
    """Seconds to wait after SIGTERM before SIGKILL. 'stop_grace_seconds' in the config, default 5."""
    value = config.get("stop_grace_seconds", DEFAULT_STOP_GRACE)
    if isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0:
        raise ValueError("config: 'stop_grace_seconds' must be a positive number, got %r" % (value,))
    return float(value)


# What the VMX HAL does on each signal (measured on a VMX-pi with a program that only initialises the HAL and waits): SIGINT -> cleans up pigpio and exits
# in about 0.2 s; SIGTERM -> cleans up pigpio but returns and the process keeps running. So SIGINT goes first.
DEFAULT_STOP_SIGNALS = ("SIGINT", "SIGTERM")
ALLOWED_STOP_SIGNALS = ("SIGINT", "SIGTERM", "SIGHUP", "SIGQUIT", "SIGUSR1", "SIGUSR2")


def stop_signals_from_config(config):
    """Signals sent, in order, to ask the robot program to stop. 'stop_signals' in the config."""
    names = config.get("stop_signals", list(DEFAULT_STOP_SIGNALS))
    if not isinstance(names, list) or not names:
        raise ValueError("config: 'stop_signals' must be a non-empty list, got %r" % (names,))
    signals = []
    for name in names:
        if name not in ALLOWED_STOP_SIGNALS:
            raise ValueError("config: 'stop_signals' entries must be among %s, got %r"
                             % (", ".join(ALLOWED_STOP_SIGNALS), name))
        signals.append(getattr(signal, name))
    return signals


def load_config(path):
    with open(path, "r", encoding="utf-8") as f:
        config = json.load(f)
    if "robot_home" not in config:
        raise ValueError("config: 'robot_home' is required (for example \"/home/<user>\")")
    return config


def select_profile(config, kernel_release):
    """Returns (name, env) of the first profile whose kernel_regex matches, else (None, {}).

    A profile looks like {"name": "...", "kernel_regex": "^6\\.", "env": {"KEY": "value"}}.
    """
    for profile in config.get("profiles", []):
        regex = profile.get("kernel_regex")
        if regex is not None and re.search(regex, kernel_release):
            return profile.get("name", regex), dict(profile.get("env", {}))
    return None, {}


def cpu_has_lse(cpuinfo_path="/proc/cpuinfo"):
    """True if the CPU runs ARMv8.1 LSE atomics itself ("atomics" in the Features line of /proc/cpuinfo).

    Returns None when it cannot tell (not an ARM Linux, unreadable file), which callers treat as "do nothing".
    """
    try:
        with open(cpuinfo_path, "r", encoding="utf-8") as f:
            for line in f:
                if line.startswith("Features"):
                    return "atomics" in line.split(":", 1)[1].split()
    except OSError:
        return None
    return None


def lse_emulation_env(config, has_lse, env):
    """LD_AUDIT for the LSE emulation library, when the CPU lacks LSE, the library exists and nothing sets LD_AUDIT.

    libMrcLib.so (WPILib, binary only) uses LSE instructions, which a Raspberry Pi 4 does not have: without this the
    robot program dies with SIGILL, at the latest when it exits. See lse_emu/lse_emu.c. An audit library, not
    LD_PRELOAD, because it has to be initialised before the libraries the program needs.
    Config: "lse_emulation": "auto" (default) | "off" | "on", and "lse_emulation_library": "<path>".
    """
    mode = str(config.get("lse_emulation", "auto")).lower()
    library = config.get("lse_emulation_library", DEFAULT_LSE_EMU_LIBRARY)
    if mode == "off":
        return {}
    if mode == "auto" and has_lse is not False:
        return {}  # the CPU has LSE, or we cannot tell
    if not os.path.isfile(library):
        if mode == "on" or has_lse is False:
            log.warning("this CPU has no LSE atomics, but %s is not installed: programs using libMrcLib will "
                        "die with SIGILL (run install.sh)", library)
        return {}
    if env.get("LD_AUDIT"):
        log.warning("LD_AUDIT is already set (%s); not adding the LSE emulation", env["LD_AUDIT"])
        return {}
    log.info("CPU without LSE atomics: LD_AUDIT=%s", library)
    return {"LD_AUDIT": library}


def build_environment(config, kernel_release, base_env=None, has_lse=None):
    """os.environ < config 'env' < LSE emulation (only when LD_AUDIT is not already set) < matched profile 'env'.

    Values are converted to str."""
    env = dict(os.environ if base_env is None else base_env)
    env.update({k: str(v) for k, v in config.get("env", {}).items()})
    env.update(lse_emulation_env(config, has_lse, env))
    name, profile_env = select_profile(config, kernel_release)
    if name is not None:
        log.info("platform profile '%s' matched kernel %s", name, kernel_release)
    else:
        log.info("no platform profile matched kernel %s", kernel_release)
    env.update({k: str(v) for k, v in profile_env.items()})
    return env


class Manager:
    """Runs the robot command in a loop until stop() is called."""

    def __init__(self, command_file, cwd, env, backoff_initial=1.0, backoff_max=10.0,
                 min_uptime=10.0, wait_poll=2.0, stop_grace=5.0, stop_signals=None, riolog=None):
        self.command_file = command_file
        self.cwd = cwd
        self.env = env
        self.backoff_initial = backoff_initial
        self.backoff_max = backoff_max
        self.min_uptime = min_uptime
        self.wait_poll = wait_poll
        self.stop_grace = stop_grace
        self.stop_signals = list(stop_signals) if stop_signals else [signal.SIGINT, signal.SIGTERM]
        self.riolog = riolog
        self._stop = threading.Event()
        self._lock = threading.Lock()
        self._proc = None
        self._started_at = 0.0
        self.starts = 0

    def run(self):
        backoff = self.backoff_initial
        warned_missing = False
        while not self._stop.is_set():
            if not os.path.isfile(self.command_file):
                if not warned_missing:  # log once, not on every poll
                    log.warning("%s does not exist yet; waiting for a deploy", self.command_file)
                    warned_missing = True
                self._stop.wait(self.wait_poll)
                continue
            warned_missing = False

            started = self._start()
            if started is None:
                self._stop.wait(backoff)
                backoff = min(backoff * 2, self.backoff_max)
                continue

            code = started.wait()
            reader = getattr(started, "_robot_output_reader", None)
            if reader is not None:
                reader.join(timeout=1.0)
            uptime = time.monotonic() - self._started_at
            with self._lock:
                self._proc = None
            if not self._stop.is_set():  # while stopping, stop() cleans the group itself
                self._reap_group(started.pid, "robot program exited", send_term=True)
            if self._stop.is_set():
                break
            log.warning("robot program exited with code %s after %.1fs", code, uptime)
            # A program that ran for a while was healthy: restart quickly. A crash loop backs off.
            if uptime >= self.min_uptime:
                backoff = self.backoff_initial
                delay = backoff
            else:  # a crash loop: wait the current delay (the first one is backoff_initial), then double it for next time
                delay = backoff
                backoff = min(backoff * 2, self.backoff_max)
            self._stop.wait(delay)

    def _start(self):
        try:
            with self._lock:
                if self._stop.is_set():
                    return None
                # robotCommand holds a command line without a shebang, so run it through sh.
                self._proc = subprocess.Popen(
                    ["/bin/sh", self.command_file],
                    cwd=self.cwd,
                    env=self.env,
                    start_new_session=True,  # own process group so the whole tree can be stopped
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    bufsize=0,
                )
                reader = threading.Thread(target=self._forward_output, args=(self._proc,),
                                          name="robot-output", daemon=True)
                self._proc._robot_output_reader = reader
                reader.start()
                self._started_at = time.monotonic()
                self.starts += 1
                log.info("started robot program (pid %d)", self._proc.pid)
                return self._proc
        except OSError as e:
            log.error("could not start %s: %s", self.command_file, e)
            return None

    def _forward_output(self, proc):
        """Keep systemd journal output and forward robot prints to the RioLog TCP stream.

        The pipe is always read until the program closes it: if this thread stopped reading and closed its end, the
        robot program would get SIGPIPE on its next print and could die because of the console, not because of a bug.
        A failing journal or RioLog is reported once and then skipped.
        """
        journal_ok = True
        riolog_ok = self.riolog is not None
        try:
            for raw_line in iter(proc.stdout.readline, b""):
                line = raw_line.decode("utf-8", errors="replace")
                if journal_ok:
                    try:
                        sys.stdout.write(line)
                        sys.stdout.flush()
                    except (OSError, ValueError) as exc:
                        journal_ok = False
                        log.warning("could not write robot program output to the journal: %s", exc)
                if riolog_ok:
                    try:
                        self.riolog.publish(line.rstrip("\r\n"))
                    except Exception as exc:  # the console must never take the robot program down
                        riolog_ok = False
                        log.warning("RioLog forwarding stopped: %s", exc)
        except (OSError, ValueError) as exc:
            log.warning("could not read robot program output: %s", exc)
        finally:
            if proc.stdout is not None:
                proc.stdout.close()

    def stop(self):
        self._stop.set()
        with self._lock:
            proc = self._proc
        if proc is None or proc.poll() is not None:
            return
        started = time.monotonic()
        names = " then ".join(signal.Signals(sig).name for sig in self.stop_signals)
        log.info("stopping robot program (pid %d): %s, then SIGKILL; %.0fs in total", proc.pid, names,
                 self.stop_grace)

        # Each signal gets an equal share of the grace period. The first one that ends the leader wins.
        step = self.stop_grace / len(self.stop_signals)
        stopped_by = None
        for sig in self.stop_signals:
            _signal_group(proc, sig)
            try:
                proc.wait(step)
                stopped_by = signal.Signals(sig).name
                break
            except subprocess.TimeoutExpired:
                continue

        killed = stopped_by is None
        if killed:
            log.warning("robot program did not stop within %.0fs; sending SIGKILL (its cleanup, such as "
                        "gpioTerminate, did not run)", self.stop_grace)
            _signal_group(proc, signal.SIGKILL)
            proc.wait()
        # The signals already went to the whole group; give the rest of the group what is left of the grace period.
        self._reap_group(proc.pid, "robot program stopped", send_term=False, deadline=started + self.stop_grace)
        log.info("robot program %s after %.1fs", "killed" if killed else "stopped by " + stopped_by,
                 time.monotonic() - started)

    def _reap_group(self, pgid, why, send_term, deadline=None):
        """The leader is gone. Make sure nothing of its process group is left behind.

        Members get until `deadline` (default: one grace period from now) to leave, then SIGKILL. With
        send_term the members are asked to stop first (the program exited on its own, so nobody has told
        them); without it SIGTERM already went out (stop()) and this only waits.
        """
        if not _group_alive(pgid):
            return
        if send_term:
            log.warning("%s but processes of its group are still running; sending SIGTERM to the group", why)
            _killpg(pgid, signal.SIGTERM)
        if deadline is None:
            deadline = time.monotonic() + self.stop_grace
        while _group_alive(pgid) and time.monotonic() < deadline:
            time.sleep(0.02)
        if not _group_alive(pgid):
            return
        log.warning("processes of the robot program's group are still running after %.0fs; sending SIGKILL "
                    "to the group", self.stop_grace)
        _killpg(pgid, signal.SIGKILL)
        kill_deadline = time.monotonic() + 2.0
        while _group_alive(pgid) and time.monotonic() < kill_deadline:
            time.sleep(0.02)
        if _group_alive(pgid):
            log.error("processes of group %d survived SIGKILL", pgid)


def _killpg(pgid, sig):
    try:
        os.killpg(pgid, sig)
    except ProcessLookupError:
        pass


def _group_alive(pgid):
    try:
        os.killpg(pgid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _signal_group(proc, sig):
    try:
        os.killpg(proc.pid, sig)
    except ProcessLookupError:
        pass


class _RioLogClient:
    """One bounded queue and sender per client; socket writes never run on the robot output thread."""

    def __init__(self, server, sock):
        self.server = server
        self.socket = sock
        self.pending = queue.Queue(maxsize=server.queue_limit)
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self._run, name="riolog-client", daemon=True)

    def close(self):
        if self.stopped.is_set():
            return
        self.stopped.set()
        try:
            self.socket.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.socket.close()

    def _run(self):
        try:
            while not self.stopped.is_set():
                readable, _, _ = select.select([self.socket], [], [], 0)
                if readable and not self.socket.recv(1024):
                    break
                try:
                    frame = self.pending.get(timeout=0.1)
                except queue.Empty:
                    continue
                self.socket.sendall(frame)
        except (OSError, ValueError):
            pass
        finally:
            self.close()
            with self.server._lock:
                self.server._clients.discard(self)


class RioLogServer:
    """Serve NetConsole TCP frames without letting slow clients stall robot stdout."""

    def __init__(self, port=RIOLOG_PORT, history=200, queue_limit=256, max_clients=8):
        self.port = int(port)
        self.history = deque(maxlen=max(0, int(history)))
        self.queue_limit = max(1, int(queue_limit))
        self.max_clients = max(1, int(max_clients))
        self._clients = set()
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._server = None
        self._sequence = 0
        self._thread = threading.Thread(target=self._serve, name="riolog-server", daemon=True)

    def start(self):
        self._thread.start()

    def stop(self):
        self._stop.set()
        if self._server is not None:
            self._server.close()
        with self._lock:
            clients = tuple(self._clients)
            self._clients.clear()
        for client in clients:
            client.close()
        if self._thread.is_alive():
            self._thread.join(timeout=1.0)
        for client in clients:
            client.thread.join(timeout=1.0)

    @staticmethod
    def _frame(line, sequence):
        text = line.encode("utf-8", errors="replace")[:0xFFFF - 3 - 6]
        payload = struct.pack(">fh", time.monotonic(), sequence) + text
        return struct.pack(">HB", len(payload) + 1, 12) + payload

    def publish(self, line):
        stale = []
        with self._lock:
            sequence = self._sequence
            self._sequence = (self._sequence + 1 + 0x8000) % 0x10000 - 0x8000
            frame = self._frame(line, sequence)
            if self.history.maxlen:
                self.history.append(frame)
            for client in tuple(self._clients):
                try:
                    client.pending.put_nowait(frame)
                except queue.Full:
                    self._clients.discard(client)
                    stale.append(client)
        # Drop lagging clients instead of dropping arbitrary bytes from a framed stream.
        for client in stale:
            client.close()

    def _add_client(self, sock):
        sock.settimeout(1.0)
        client = _RioLogClient(self, sock)
        with self._lock:
            if self._stop.is_set() or len(self._clients) >= self.max_clients:
                client.close()
                return None
            # Register history and live frames under the same lock to preserve order.
            for frame in tuple(self.history)[-self.queue_limit:]:
                client.pending.put_nowait(frame)
            self._clients.add(client)
            # Start before releasing the lock so stop() always sees a started thread.
            client.thread.start()
        return client

    def _serve(self):
        server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._server = server
        try:
            server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            server.bind(("0.0.0.0", self.port))
            server.listen(4)
            server.settimeout(0.5)
            log.info("RioLog console listening on TCP port %d", server.getsockname()[1])
            while not self._stop.is_set():
                try:
                    sock, address = server.accept()
                except socket.timeout:
                    continue
                except OSError:
                    if self._stop.is_set():
                        break
                    raise
                if self._add_client(sock) is not None:
                    log.info("RioLog client connected from %s:%d", *address)
        except OSError as exc:
            if not self._stop.is_set():
                log.error("RioLog console could not listen on TCP port %d: %s", self.port, exc)
        finally:
            server.close()


def main(argv=None):
    parser = argparse.ArgumentParser(description="Run and supervise the VMX robot program.")
    parser.add_argument("--config", default=DEFAULT_CONFIG, help="path to config.json (default: %(default)s)")
    args = parser.parse_args(argv)

    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s", stream=sys.stdout)

    config = load_config(args.config)
    env = build_environment(config, platform.release(), has_lse=cpu_has_lse())
    command_file = config.get("robot_command") or os.path.join(config["robot_home"], ROBOT_COMMAND_FILE)

    riolog = None
    if config.get("riolog_enabled", True):
        riolog = RioLogServer(port=config.get("riolog_port", RIOLOG_PORT),
                              history=config.get("riolog_history", 200))
        riolog.start()

    manager = Manager(command_file, config["robot_home"], env, stop_grace=stop_grace_from_config(config),
                      stop_signals=stop_signals_from_config(config), riolog=riolog)

    stop_threads = []

    def shutdown(signum, frame):
        log.info("received signal %d", signum)
        # Not manager.stop() directly: this handler runs on the main thread, which is inside
        # Popen.wait() holding its lock, so waiting on the child again here would deadlock.
        # The thread is not a daemon and main() joins it below: run() returns as soon as the leader
        # is gone, and exiting then would cut stop() off before it cleaned up the process group.
        thread = threading.Thread(target=manager.stop)
        stop_threads.append(thread)
        thread.start()

    signal.signal(signal.SIGTERM, shutdown)
    signal.signal(signal.SIGINT, shutdown)

    log.info("robot_manager starting; command: %s", command_file)
    manager.run()
    if riolog is not None:
        riolog.stop()
    for thread in list(stop_threads):
        thread.join()
    log.info("robot_manager stopped")
    return 0


if __name__ == "__main__":
    sys.exit(main())
