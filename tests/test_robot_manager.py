import json
import os
import signal
import stat
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import robot_manager as rm  # noqa: E402


def write_script(path, text):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)


def read(path):
    with open(path, "r", encoding="utf-8") as f:
        return f.read()


def wait_for(predicate, timeout=10.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return True
        time.sleep(0.02)
    return False


class ProfileTest(unittest.TestCase):
    CONFIG = {
        "robot_home": "/home/x",
        "env": {"A": "base", "B": 1},
        "profiles": [
            {"name": "old", "kernel_regex": r"^5\.", "env": {"A": "old"}},
            {"name": "new", "kernel_regex": r"^6\.", "env": {"A": "new", "C": "c"}},
        ],
    }

    def test_first_matching_profile_wins(self):
        name, env = rm.select_profile(self.CONFIG, "6.8.0-generic")
        self.assertEqual(name, "new")
        self.assertEqual(env, {"A": "new", "C": "c"})

    def test_no_match(self):
        self.assertEqual(rm.select_profile(self.CONFIG, "4.19.0"), (None, {}))

    def test_environment_precedence_and_str_values(self):
        env = rm.build_environment(self.CONFIG, "6.8.0", base_env={"A": "os", "Z": "z"})
        self.assertEqual(env["A"], "new")   # profile > config > os
        self.assertEqual(env["B"], "1")     # converted to str
        self.assertEqual(env["C"], "c")
        self.assertEqual(env["Z"], "z")

    def test_environment_without_profile_keeps_config_env(self):
        env = rm.build_environment(self.CONFIG, "4.19.0", base_env={})
        self.assertEqual(env, {"A": "base", "B": "1"})

    def test_robot_home_is_required(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "config.json")
            with open(path, "w") as f:
                json.dump({"env": {}}, f)
            with self.assertRaises(ValueError):
                rm.load_config(path)


class ManagerTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.home = self.dir.name
        self.command = os.path.join(self.home, "robotCommand")
        self.out = os.path.join(self.home, "out.txt")
        self.threads = []

    def tearDown(self):
        self.dir.cleanup()

    def start(self, **kwargs):
        defaults = dict(backoff_initial=0.05, backoff_max=0.1, min_uptime=0.5, wait_poll=0.05, stop_grace=2.0)
        defaults.update(kwargs)
        manager = rm.Manager(self.command, self.home, dict(os.environ), **defaults)
        thread = threading.Thread(target=manager.run, daemon=True)
        thread.start()
        self.threads.append((manager, thread))
        return manager, thread

    def finish(self, manager, thread):
        manager.stop()
        thread.join(10)
        self.assertFalse(thread.is_alive(), "manager did not stop")

    def test_runs_a_command_line_without_shebang(self):
        # Like the real robotCommand: a command line, not an executable script.
        write_script(self.command, f'echo ran > "{self.out}"; sleep 30\n')
        manager, thread = self.start()
        self.assertTrue(wait_for(lambda: os.path.exists(self.out)))
        self.finish(manager, thread)

    def test_environment_reaches_the_program(self):
        write_script(self.command, f'echo "$HALSIMVMX_IMU" > "{self.out}"; sleep 30\n')
        env = dict(os.environ, HALSIMVMX_IMU="1")
        manager = rm.Manager(self.command, self.home, env, backoff_initial=0.05, stop_grace=2.0)
        thread = threading.Thread(target=manager.run, daemon=True)
        thread.start()
        self.assertTrue(wait_for(lambda: os.path.exists(self.out) and read(self.out).strip() == "1"))
        self.finish(manager, thread)

    def test_restarts_when_the_program_exits(self):
        counter = os.path.join(self.home, "count.txt")
        write_script(self.command, f'echo x >> "{counter}"; exit 1\n')
        manager, thread = self.start()
        self.assertTrue(wait_for(lambda: os.path.exists(counter) and len(read(counter).split()) >= 3))
        self.finish(manager, thread)
        self.assertGreaterEqual(manager.starts, 3)

    def test_waits_for_a_deploy_when_robotcommand_is_missing(self):
        manager, thread = self.start()
        time.sleep(0.3)
        self.assertEqual(manager.starts, 0)
        write_script(self.command, f'echo ok > "{self.out}"; sleep 30\n')
        self.assertTrue(wait_for(lambda: os.path.exists(self.out)))
        self.finish(manager, thread)

    def test_stop_terminates_the_whole_process_tree(self):
        pidfile = os.path.join(self.home, "child.pid")
        # The shell starts a background child; both must die on stop().
        write_script(self.command, f'sleep 60 & echo $! > "{pidfile}"; wait\n')
        manager, thread = self.start()
        self.assertTrue(wait_for(lambda: os.path.exists(pidfile) and read(pidfile).strip()))
        child = int(read(pidfile).strip())
        self.finish(manager, thread)
        self.assertTrue(wait_for(lambda: not _alive(child), timeout=5), "child process survived stop()")

    def test_stop_kills_a_program_that_ignores_sigterm(self):
        ready = os.path.join(self.home, "ready")
        write_script(self.command, f'trap "" INT TERM; touch "{ready}"; while true; do sleep 1; done\n')
        manager, thread = self.start(stop_grace=0.5)
        self.assertTrue(wait_for(lambda: os.path.exists(ready)))
        started = time.monotonic()
        self.finish(manager, thread)
        self.assertLess(time.monotonic() - started, 8)


class StopSequenceTest(ManagerTest):
    def test_a_child_that_ignores_sigterm_is_killed_when_the_leader_exits(self):
        # The leader exits on SIGTERM but leaves a child that ignores it. The leader being gone must not
        # make robot_manager think everything stopped.
        pidfile = os.path.join(self.home, "stubborn.pid")
        write_script(
            self.command,
            f"sh -c 'trap \"\" TERM; sleep 60' &\n"
            f"echo $! > \"{pidfile}\"\n"
            "trap 'exit 0' TERM\n"
            "while true; do sleep 1; done\n",
        )
        manager, thread = self.start(stop_grace=3.0)
        self.assertTrue(wait_for(lambda: os.path.exists(pidfile) and read(pidfile).strip()))
        stubborn = int(read(pidfile).strip())
        self.assertTrue(_alive(stubborn))
        self.finish(manager, thread)
        self.assertTrue(wait_for(lambda: not _alive(stubborn), timeout=5), "the stubborn child survived stop()")

    def test_a_leftover_child_is_cleaned_when_the_program_exits_on_its_own(self):
        pidfile = os.path.join(self.home, "orphan.pid")
        # The program starts a background child and exits at once; the child must not be left behind.
        write_script(self.command, f'sleep 60 &\necho $! > "{pidfile}"\nexit 0\n')
        manager, thread = self.start(min_uptime=0.0, backoff_initial=5.0, backoff_max=5.0)
        self.assertTrue(wait_for(lambda: os.path.exists(pidfile) and read(pidfile).strip()))
        orphan = int(read(pidfile).strip())
        self.assertTrue(wait_for(lambda: not _alive(orphan), timeout=5), "the orphaned child was not cleaned")
        self.finish(manager, thread)

    def test_stop_reports_a_clean_stop(self):
        write_script(self.command, "sleep 60\n")
        manager, thread = self.start()
        self.assertTrue(wait_for(lambda: manager.starts >= 1))
        with self.assertLogs("robot_manager", level="INFO") as logs:
            self.finish(manager, thread)
        text = "\n".join(logs.output)
        self.assertIn("robot program stopped by SIGINT after", text)
        self.assertNotIn("sending SIGKILL", text)

    def test_stop_reports_when_sigkill_was_needed(self):
        ready = os.path.join(self.home, "ready")
        write_script(self.command, f'trap "" INT TERM; touch "{ready}"; while true; do sleep 1; done\n')
        manager, thread = self.start(stop_grace=0.5)
        self.assertTrue(wait_for(lambda: os.path.exists(ready)))
        with self.assertLogs("robot_manager", level="INFO") as logs:
            self.finish(manager, thread)
        text = "\n".join(logs.output)
        self.assertIn("sending SIGKILL", text)
        self.assertIn("robot program killed after", text)


class SignalOrderTest(ManagerTest):
    def _stop_log(self, script, **kwargs):
        ready = os.path.join(self.home, "ready")
        write_script(self.command, f'touch "{ready}"\n' + script)
        manager, thread = self.start(**kwargs)
        self.assertTrue(wait_for(lambda: os.path.exists(ready)))
        started = time.monotonic()
        with self.assertLogs("robot_manager", level="INFO") as logs:
            self.finish(manager, thread)
        return "\n".join(logs.output), time.monotonic() - started

    def test_sigint_goes_first_and_ends_a_program_that_ignores_sigterm(self):
        # Like the VMX HAL: SIGINT is the real "stop", SIGTERM only cleans up and the program keeps running.
        text, elapsed = self._stop_log("trap 'exit 0' INT\ntrap '' TERM\nwhile true; do sleep 1; done\n",
                                       stop_grace=6.0)
        self.assertIn("stopped by SIGINT", text)
        self.assertNotIn("sending SIGKILL", text)
        self.assertLess(elapsed, 3.0)  # well inside the share of the grace period given to SIGINT

    def test_sigterm_is_the_fallback_when_sigint_does_not_end_the_program(self):
        text, _ = self._stop_log("trap '' INT\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n",
                                 stop_grace=2.0)
        self.assertIn("stopped by SIGTERM", text)
        self.assertNotIn("sending SIGKILL", text)

    def test_the_configured_order_is_used(self):
        # Only SIGTERM is configured: a program that exits on SIGINT but ignores SIGTERM needs SIGKILL.
        text, _ = self._stop_log("trap 'exit 0' INT\ntrap '' TERM\nwhile true; do sleep 1; done\n",
                                 stop_grace=1.0, stop_signals=[signal.SIGTERM])
        self.assertIn("sending SIGKILL", text)
        self.assertIn("robot program killed after", text)


class StopSignalsConfigTest(unittest.TestCase):
    def test_default_is_sigint_then_sigterm(self):
        self.assertEqual(rm.stop_signals_from_config({"robot_home": "/x"}), [signal.SIGINT, signal.SIGTERM])

    def test_configured_order(self):
        self.assertEqual(rm.stop_signals_from_config({"stop_signals": ["SIGTERM", "SIGINT"]}),
                         [signal.SIGTERM, signal.SIGINT])
        self.assertEqual(rm.stop_signals_from_config({"stop_signals": ["SIGHUP"]}), [signal.SIGHUP])

    def test_invalid_values_are_rejected(self):
        for bad in ([], "SIGINT", None, ["SIGKILL"], ["SIGINT", "NOPE"], [2], ["sigint"]):
            with self.assertRaises(ValueError, msg=repr(bad)):
                rm.stop_signals_from_config({"stop_signals": bad})


class StopGraceConfigTest(unittest.TestCase):
    def test_default(self):
        self.assertEqual(rm.stop_grace_from_config({"robot_home": "/x"}), 5.0)

    def test_configured_value(self):
        self.assertEqual(rm.stop_grace_from_config({"stop_grace_seconds": 12}), 12.0)
        self.assertEqual(rm.stop_grace_from_config({"stop_grace_seconds": 2.5}), 2.5)

    def test_invalid_values_are_rejected(self):
        for bad in (0, -1, "5", None, True):
            with self.assertRaises(ValueError, msg=repr(bad)):
                rm.stop_grace_from_config({"stop_grace_seconds": bad})


class CommandLineTest(unittest.TestCase):
    def test_sigterm_stops_the_service_and_its_program(self):
        import signal
        import subprocess
        with tempfile.TemporaryDirectory() as d:
            pidfile = os.path.join(d, "child.pid")
            write_script(os.path.join(d, "robotCommand"), f'sleep 60 & echo $! > "{pidfile}"; wait\n')
            cfg = os.path.join(d, "config.json")
            with open(cfg, "w") as f:
                json.dump({"robot_home": d}, f)
            script = os.path.join(os.path.dirname(__file__), "..", "robot_manager.py")
            proc = subprocess.Popen([sys.executable, script, "--config", cfg],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                self.assertTrue(wait_for(lambda: os.path.exists(pidfile) and read(pidfile).strip()))
                child = int(read(pidfile).strip())
                proc.send_signal(signal.SIGTERM)
                self.assertEqual(proc.wait(15), 0)
                self.assertTrue(wait_for(lambda: not _alive(child), timeout=5))
            finally:
                if proc.poll() is None:
                    proc.kill()

    def test_the_service_finishes_stopping_the_program_before_it_exits(self):
        # The leader exits on SIGTERM but leaves a child that ignores it. The service must not exit until
        # that child is gone (grace period, then SIGKILL): systemd would otherwise have to clean up after it.
        import signal
        import subprocess
        with tempfile.TemporaryDirectory() as d:
            pidfile = os.path.join(d, "stubborn.pid")
            write_script(
                os.path.join(d, "robotCommand"),
                f"sh -c 'trap \"\" TERM; sleep 60' &\n"
                f"echo $! > \"{pidfile}\"\n"
                "trap 'exit 0' TERM\n"
                "while true; do sleep 1; done\n",
            )
            cfg = os.path.join(d, "config.json")
            with open(cfg, "w") as f:
                json.dump({"robot_home": d, "stop_grace_seconds": 1}, f)
            script = os.path.join(os.path.dirname(__file__), "..", "robot_manager.py")
            proc = subprocess.Popen([sys.executable, script, "--config", cfg],
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            stubborn = None
            try:
                self.assertTrue(wait_for(lambda: os.path.exists(pidfile) and read(pidfile).strip()))
                stubborn = int(read(pidfile).strip())
                self.assertTrue(_alive(stubborn))
                proc.send_signal(signal.SIGTERM)
                self.assertEqual(proc.wait(15), 0)
                # The service has exited. The child must already be dead, not left for someone else to kill.
                self.assertFalse(_alive(stubborn), "the service exited while the program's child was still running")
            finally:
                if proc.poll() is None:
                    proc.kill()
                if stubborn is not None and _alive(stubborn):
                    os.kill(stubborn, signal.SIGKILL)

    def test_missing_config_is_an_error(self):
        import subprocess
        script = os.path.join(os.path.dirname(__file__), "..", "robot_manager.py")
        result = subprocess.run([sys.executable, script, "--config", "/nonexistent/config.json"],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)


def _alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    # A zombie is not alive for our purposes.
    try:
        with open(f"/proc/{pid}/stat") as f:
            return f.read().split(")")[-1].split()[0] != "Z"
    except FileNotFoundError:
        return False


if __name__ == "__main__":
    unittest.main()
