use serde_json::json;
use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Service {
    child: Child,
    home: tempfile::TempDir,
}
impl Service {
    fn start(script: &str) -> Self {
        let home = tempfile::tempdir().unwrap();
        fs::write(home.path().join("robotCommand"), script).unwrap();
        fs::write(
            home.path().join("config.json"),
            json!({"robot_home":home.path(),
            "stop_grace_seconds":0.5, "riolog_enabled":false, "realtime":{"mode":"off"}})
            .to_string(),
        )
        .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_robot_manager"))
            .args([
                "--config",
                home.path().join("config.json").to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self { child, home }
    }
    fn wait_file(&self, name: &str) {
        let end = Instant::now() + Duration::from_secs(5);
        while !self.home.path().join(name).is_file() {
            assert!(Instant::now() < end, "child did not create {name}");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn signal(&self, signal: i32) {
        // SAFETY: this PID belongs to the test's owned service process.
        assert_eq!(unsafe { libc::kill(self.child.id() as i32, signal) }, 0);
    }
    fn wait(&mut self) -> std::process::ExitStatus {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < end, "service did not finish shutdown");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGTERM);
            }
            let end = Instant::now() + Duration::from_secs(3);
            while Instant::now() < end {
                if self.child.try_wait().ok().flatten().is_some() {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
#[test]
fn service_sigterm_waits_for_stubborn_descendants_before_returning() {
    let mut service = Service::start("sh -c 'trap \"\" INT TERM; touch child.ready; sleep 60' &\necho $! > child.pid\ntrap 'exit 0' INT TERM\nwhile true; do sleep 1; done");
    service.wait_file("child.ready");
    service.wait_file("child.pid");
    let pid: i32 = fs::read_to_string(service.home.path().join("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    service.signal(libc::SIGTERM);
    assert!(service.wait().success());
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "descendant was not reaped"
    );
}
#[test]
fn repeated_service_signals_are_idempotent() {
    let mut service = Service::start(
        "trap '' INT; trap 'exit 0' TERM; touch ready; while true; do sleep 0.02; done",
    );
    service.wait_file("ready");
    service.signal(libc::SIGTERM);
    service.signal(libc::SIGINT);
    assert!(service.wait().success());
}
#[test]
fn invalid_configuration_fails_before_launch() {
    let result = Command::new(env!("CARGO_BIN_EXE_robot_manager"))
        .args(["--config", "/nonexistent/config.json"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("config.json");
    fs::write(
        &path,
        json!({"robot_home":home.path(), "stop_signals":[]}).to_string(),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_robot_manager"))
        .arg("--config")
        .arg(path)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("stop_signals"));
}
#[test]
fn configuration_check_does_not_start_the_program_or_riolog() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("robotCommand"), "touch must-not-exist").unwrap();
    let path = home.path().join("config.json");
    fs::write(&path, json!({"robot_home":home.path()}).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_robot_manager"))
        .arg("--config")
        .arg(path)
        .arg("--check-config")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(!home.path().join("must-not-exist").exists());
}
