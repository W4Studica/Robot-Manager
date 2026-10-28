use robot_manager::manager::{Manager, Policy};
use robot_manager::output::{Journal, LineSink};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tempfile::TempDir;

struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Running {
    home: TempDir,
    stopping: Arc<AtomicBool>,
    starts: Arc<AtomicUsize>,
    worker: Option<JoinHandle<io::Result<()>>>,
    output: Arc<Mutex<Vec<u8>>>,
}
fn policy() -> Policy {
    Policy {
        backoff_initial: Duration::from_millis(30),
        backoff_max: Duration::from_millis(100),
        min_uptime: Duration::from_millis(300),
        wait_poll: Duration::from_millis(20),
        stop_grace: Duration::from_millis(400),
        ..Policy::default()
    }
}
impl Running {
    fn start(script: Option<&str>, policy: Policy, console: Option<Arc<dyn LineSink>>) -> Self {
        Self::start_with_journal(script, policy, console, None)
    }
    fn start_with_journal(
        script: Option<&str>,
        policy: Policy,
        console: Option<Arc<dyn LineSink>>,
        journal: Option<Journal>,
    ) -> Self {
        let home = tempfile::tempdir().unwrap();
        let command_file = home.path().join("robotCommand");
        if let Some(script) = script {
            fs::write(&command_file, script).unwrap();
        }
        let stopping = Arc::new(AtomicBool::new(false));
        let starts = Arc::new(AtomicUsize::new(0));
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut env: HashMap<_, _> = std::env::vars().collect();
        env.insert("CUSTOM_ENV".into(), "forwarded".into());
        let manager = Manager {
            command_file,
            cwd: home.path().into(),
            env,
            policy,
            stopping: stopping.clone(),
            starts: starts.clone(),
            journal: journal.unwrap_or_else(|| {
                Journal::with_writer(Box::new(Capture(output.clone()))).unwrap()
            }),
            console,
        };
        let worker = thread::spawn(move || manager.run());
        Self {
            home,
            stopping,
            starts,
            worker: Some(worker),
            output,
        }
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        self.home.path().join(name)
    }
    fn stop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap().unwrap();
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < end, "condition did not become true");
        thread::sleep(Duration::from_millis(10));
    }
}
fn pid(path: &Path) -> Option<i32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}
fn alive(pid: i32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')
                .map(|(_, rest)| !rest.trim_start().starts_with('Z'))
        })
        .unwrap_or(false)
}

#[test]
fn command_without_shebang_receives_environment_and_cwd() {
    let mut running = Running::start(
        Some("printf '%s' \"$CUSTOM_ENV\" > result; pwd > cwd; sleep 60\n"),
        policy(),
        None,
    );
    wait(|| {
        fs::read_to_string(running.path("result")).ok().as_deref() == Some("forwarded")
            && fs::read_to_string(running.path("cwd"))
                .ok()
                .is_some_and(|text| text.trim() == running.home.path().to_str().unwrap())
    });
    assert_eq!(
        fs::read_to_string(running.path("result")).unwrap(),
        "forwarded"
    );
    assert_eq!(
        fs::read_to_string(running.path("cwd")).unwrap().trim(),
        running.home.path().to_str().unwrap()
    );
    running.stop();
}
#[test]
fn missing_command_waits_then_accepts_a_deploy() {
    let mut running = Running::start(None, policy(), None);
    thread::sleep(Duration::from_millis(80));
    assert_eq!(running.starts.load(Ordering::Relaxed), 0);
    fs::write(running.path("robotCommand"), "touch deployed; sleep 60").unwrap();
    wait(|| running.path("deployed").exists());
    running.stop();
}
#[test]
fn crash_loops_restart_and_stopping_interrupts_backoff() {
    let mut running = Running::start(Some("exit 7"), policy(), None);
    wait(|| running.starts.load(Ordering::Relaxed) >= 3);
    let started = Instant::now();
    running.stop();
    assert!(started.elapsed() < Duration::from_secs(2));
    let mut long = policy();
    long.backoff_initial = Duration::from_secs(60);
    long.backoff_max = long.backoff_initial;
    let mut running = Running::start(Some("touch ready; exit 0"), long, None);
    wait(|| running.path("ready").exists());
    let started = Instant::now();
    running.stop();
    assert!(started.elapsed() < Duration::from_secs(2));
}
#[test]
fn stop_cleans_the_entire_process_group() {
    let mut running = Running::start(Some("sleep 60 & echo $! > child.pid; wait"), policy(), None);
    wait(|| pid(&running.path("child.pid")).is_some());
    let child = pid(&running.path("child.pid")).unwrap();
    assert!(alive(child));
    running.stop();
    assert!(!alive(child));
}
#[test]
fn stubborn_descendants_cannot_outlive_the_supervisor() {
    let mut running = Running::start(Some("sh -c 'trap \"\" INT TERM; touch child.ready; sleep 60' &\necho $! > child.pid\ntrap 'exit 0' INT TERM\nwhile true; do sleep 1; done"), policy(), None);
    wait(|| running.path("child.ready").exists() && pid(&running.path("child.pid")).is_some());
    let child = pid(&running.path("child.pid")).unwrap();
    running.stop();
    assert!(!alive(child));
}
#[test]
fn natural_exit_cleans_orphans_before_restart() {
    let mut p = policy();
    p.backoff_initial = Duration::from_secs(5);
    p.backoff_max = p.backoff_initial;
    let mut running = Running::start(Some("sleep 60 & echo $! > orphan.pid; exit 0"), p, None);
    wait(|| pid(&running.path("orphan.pid")).is_some());
    let orphan = pid(&running.path("orphan.pid")).unwrap();
    wait(|| !alive(orphan));
    running.stop();
}
#[test]
fn signal_order_and_escalation_match_configuration() {
    let mut running = Running::start(Some("trap '' INT; trap 'echo term > stopped; exit 0' TERM; touch ready; while true; do sleep 0.02; done"), policy(), None);
    wait(|| running.path("ready").exists());
    running.stop();
    assert_eq!(
        fs::read_to_string(running.path("stopped")).unwrap().trim(),
        "term"
    );
    let mut p = policy();
    p.stop_signals = vec![libc::SIGTERM];
    let mut stubborn = Running::start(
        Some("trap '' INT TERM; echo $$ > leader.pid; touch ready; while true; do sleep 1; done"),
        p,
        None,
    );
    wait(|| stubborn.path("ready").exists());
    let child = pid(&stubborn.path("leader.pid")).unwrap();
    stubborn.stop();
    assert!(!alive(child));
}
#[test]
fn merged_stdout_stderr_and_invalid_utf8_are_drained() {
    let mut running = Running::start(
        Some("echo stdout; echo stderr >&2; printf '\\377invalid\\n'; touch ready; sleep 60"),
        policy(),
        None,
    );
    wait(|| running.path("ready").exists());
    wait(|| {
        running
            .output
            .lock()
            .unwrap()
            .windows(6)
            .any(|w| w == b"stderr")
    });
    running.stop();
    let output = running.output.lock().unwrap();
    assert!(output.windows(6).any(|w| w == b"stdout"));
    assert!(output.contains(&255));
}
#[test]
fn failed_console_cannot_break_robot_stdout() {
    struct Failing {
        calls: AtomicUsize,
    }
    impl LineSink for Failing {
        fn publish(&self, _: &str) -> io::Result<()> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err(io::Error::other("console broken"))
        }
    }
    let console = Arc::new(Failing {
        calls: AtomicUsize::new(0),
    });
    let mut running = Running::start(
        Some("i=0; while [ $i -lt 1000 ]; do echo line$i; i=$((i+1)); done; touch done; sleep 60"),
        policy(),
        Some(console.clone()),
    );
    wait(|| running.path("done").exists());
    running.stop();
    assert_eq!(console.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_journal_cannot_break_robot_stdout() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("journal broken"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let journal = Journal::with_writer(Box::new(Broken)).unwrap();
    let mut running = Running::start_with_journal(
        Some("i=0; while [ $i -lt 10000 ]; do echo line$i; i=$((i+1)); done; touch done; sleep 60"),
        policy(),
        None,
        Some(journal),
    );
    wait(|| running.path("done").exists());
    running.stop();
}
