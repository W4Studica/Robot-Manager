#!/usr/bin/env python3
"""robot_manager: keeps the robot program running on a Studica VMX-pi.

Runs <robot_home>/robotCommand (the same contract as GradleRIO 2027) as the user the
service runs as (root, because the VMX HAL needs pigpio), restarts it when it exits,
and adds environment variables chosen from a platform profile (by kernel version).

It never opens the VMX hardware itself. Standard library only; works on Python 3.10.
"""

import argparse
import json
import logging
import os
import platform
import re
import signal
import subprocess
import sys
import threading
import time

log = logging.getLogger("robot_manager")

DEFAULT_CONFIG = "/etc/robot_manager/config.json"
ROBOT_COMMAND_FILE = "robotCommand"


DEFAULT_STOP_GRACE = 5.0


def stop_grace_from_config(config):
    """Seconds to wait after SIGTERM before SIGKILL. 'stop_grace_seconds' in the config, default 5."""
    value = config.get("stop_grace_seconds", DEFAULT_STOP_GRACE)
    if isinstance(value, bool) or not isinstance(value, (int, float)) or value <= 0:
        raise ValueError("config: 'stop_grace_seconds' must be a positive number, got %r" % (value,))
    return float(value)


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


def build_environment(config, kernel_release, base_env=None):
    """os.environ < config 'env' < matched profile 'env'. Values are converted to str."""
    env = dict(os.environ if base_env is None else base_env)
    env.update({k: str(v) for k, v in config.get("env", {}).items()})
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
                 min_uptime=10.0, wait_poll=2.0, stop_grace=5.0):
        self.command_file = command_file
        self.cwd = cwd
        self.env = env
        self.backoff_initial = backoff_initial
        self.backoff_max = backoff_max
        self.min_uptime = min_uptime
        self.wait_poll = wait_poll
        self.stop_grace = stop_grace
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
            uptime = time.monotonic() - self._started_at
            with self._lock:
                self._proc = None
            if not self._stop.is_set():  # while stopping, stop() cleans the group itself
                self._reap_group(started.pid, "robot program exited", send_term=True)
            if self._stop.is_set():
                break
            log.warning("robot program exited with code %s after %.1fs", code, uptime)
            # A program that ran for a while was healthy: restart quickly. A crash loop backs off.
            backoff = self.backoff_initial if uptime >= self.min_uptime else min(backoff * 2, self.backoff_max)
            self._stop.wait(backoff)

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
                )
                self._started_at = time.monotonic()
                self.starts += 1
                log.info("started robot program (pid %d)", self._proc.pid)
                return self._proc
        except OSError as e:
            log.error("could not start %s: %s", self.command_file, e)
            return None

    def stop(self):
        self._stop.set()
        with self._lock:
            proc = self._proc
        if proc is None or proc.poll() is not None:
            return
        started = time.monotonic()
        log.info("stopping robot program (pid %d), SIGTERM then SIGKILL after %.0fs", proc.pid, self.stop_grace)
        _signal_group(proc, signal.SIGTERM)
        killed = False
        try:
            proc.wait(self.stop_grace)
        except subprocess.TimeoutExpired:
            killed = True
            log.warning("robot program did not stop within %.0fs; sending SIGKILL (its cleanup, such as "
                        "gpioTerminate, did not run)", self.stop_grace)
            _signal_group(proc, signal.SIGKILL)
            proc.wait()
        # SIGTERM already went to the whole group; give the rest of the group what is left of the grace period.
        self._reap_group(proc.pid, "robot program stopped", send_term=False, deadline=started + self.stop_grace)
        log.info("robot program %s after %.1fs", "killed" if killed else "stopped",
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


def main(argv=None):
    parser = argparse.ArgumentParser(description="Run and supervise the VMX robot program.")
    parser.add_argument("--config", default=DEFAULT_CONFIG, help="path to config.json (default: %(default)s)")
    args = parser.parse_args(argv)

    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s", stream=sys.stdout)

    config = load_config(args.config)
    env = build_environment(config, platform.release())
    command_file = config.get("robot_command") or os.path.join(config["robot_home"], ROBOT_COMMAND_FILE)

    manager = Manager(command_file, config["robot_home"], env, stop_grace=stop_grace_from_config(config))

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
    for thread in list(stop_threads):
        thread.join()
    log.info("robot_manager stopped")
    return 0


if __name__ == "__main__":
    sys.exit(main())
