use robot_manager::output::Journal;
use robot_manager::process::ChildGroup;
use robot_manager::realtime::{self, Mode, Report, Settings, Snapshot};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

// These two filtered test entries are also subprocess probes. Their work is
// bounded and never busy-spins, even on a host allowed to grant RT priority.
#[test]
fn robot_probe() {
    let Some(path) = std::env::var_os("ROBOT_RT_PROBE_OUTPUT") else {
        return;
    };
    let main = Snapshot::current().unwrap();
    let worker = thread::spawn(|| Snapshot::current().unwrap())
        .join()
        .unwrap();
    let leaf = Path::new(&path).with_extension("leaf");
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "descendant_probe", "--nocapture"])
        .env("ROBOT_RT_LEAF", &leaf)
        .status()
        .unwrap();
    assert!(status.success());
    let descendant: serde_json::Value = serde_json::from_slice(&fs::read(leaf).unwrap()).unwrap();
    fs::write(path, json!({"main":[main.policy,main.priority],"worker":[worker.policy,worker.priority],"descendant":descendant}).to_string()).unwrap();
}
#[test]
fn descendant_probe() {
    if let Some(path) = std::env::var_os("ROBOT_RT_LEAF") {
        let state = Snapshot::current().unwrap();
        fs::write(path, json!([state.policy, state.priority]).to_string()).unwrap();
    }
}
fn launch(home: &Path, settings: Settings) -> io::Result<ChildGroup> {
    let command = home.join("robotCommand");
    fs::write(
        &command,
        "exec \"$ROBOT_RT_PROBE_BINARY\" --exact robot_probe --nocapture\n",
    )?;
    let mut env: HashMap<_, _> = std::env::vars().collect();
    env.insert(
        "ROBOT_RT_PROBE_BINARY".into(),
        std::env::current_exe()?.to_string_lossy().into_owned(),
    );
    env.insert(
        "ROBOT_RT_PROBE_OUTPUT".into(),
        home.join("scheduling.json").to_string_lossy().into_owned(),
    );
    ChildGroup::spawn(
        &command,
        home,
        &env,
        Journal::with_writer(Box::new(io::sink()))?,
        None,
        settings,
        None,
    )
}
fn finish(group: &mut ChildGroup) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = group.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "bounded scheduling probe did not exit"
        );
        thread::sleep(Duration::from_millis(5));
    }
    group.finish_after_exit(Duration::from_millis(300)).unwrap();
}
fn assert_inheritance(home: &Path, expected: Snapshot) {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join("scheduling.json")).unwrap()).unwrap();
    for field in ["main", "worker", "descendant"] {
        assert_eq!(
            value[field],
            json!([expected.policy, expected.priority]),
            "{field}"
        );
    }
}
#[test]
fn off_launches_normal_robot_and_keeps_the_supervisor_unchanged() {
    let home = tempfile::tempdir().unwrap();
    let before = Snapshot::current().unwrap();
    let mut group = launch(
        home.path(),
        Settings {
            mode: Mode::Off,
            ..Settings::default()
        },
    )
    .unwrap();
    assert_eq!(group.scheduling().actual, Snapshot::NORMAL);
    assert_eq!(group.scheduling().fallback_errno, 0);
    finish(&mut group);
    assert_inheritance(home.path(), Snapshot::NORMAL);
    assert_eq!(Snapshot::current().unwrap(), before);
}
#[test]
fn auto_reports_actual_policy_and_inheritance_even_when_rt_is_unavailable() {
    let home = tempfile::tempdir().unwrap();
    let before = Snapshot::current().unwrap();
    let mut group = launch(home.path(), Settings::default()).unwrap();
    let report = group.scheduling();
    if report.fallback_errno == 0 {
        assert_eq!(
            report.actual,
            Snapshot {
                policy: libc::SCHED_RR,
                priority: 10
            }
        );
    } else {
        assert_eq!(report.actual, Snapshot::NORMAL);
        eprintln!(
            "host denied RT scheduling: {}; fallback verified",
            io::Error::from_raw_os_error(report.fallback_errno)
        );
    }
    finish(&mut group);
    assert_inheritance(home.path(), report.actual);
    assert_eq!(Snapshot::current().unwrap(), before);
}
#[test]
fn required_mode_starts_with_rt_or_refuses_to_execute_the_robot() {
    let auto_home = tempfile::tempdir().unwrap();
    let mut probe = launch(auto_home.path(), Settings::default()).unwrap();
    let available = probe.scheduling().fallback_errno == 0;
    finish(&mut probe);
    let home = tempfile::tempdir().unwrap();
    let result = launch(
        home.path(),
        Settings {
            mode: Mode::On,
            ..Settings::default()
        },
    );
    if available {
        let mut group = result.unwrap();
        assert_eq!(
            group.scheduling().actual,
            Snapshot {
                policy: libc::SCHED_RR,
                priority: 10
            }
        );
        finish(&mut group);
        assert_inheritance(
            home.path(),
            Snapshot {
                policy: libc::SCHED_RR,
                priority: 10,
            },
        );
    } else {
        let error = match result {
            Ok(_) => panic!("required mode unexpectedly launched without RT"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("robot scheduling setup failed"));
        assert!(!home.path().join("scheduling.json").exists());
    }
}
#[test]
fn invalid_priority_and_missing_program_fail_before_execution() {
    let home = tempfile::tempdir().unwrap();
    let settings = Settings {
        priority: 100,
        ..Settings::default()
    };
    assert!(launch(home.path(), settings).is_err());
    assert!(!home.path().join("scheduling.json").exists());
    let env = HashMap::new();
    let mut group = ChildGroup::spawn(
        &home.path().join("missing"),
        home.path(),
        &env,
        Journal::with_writer(Box::new(io::sink())).unwrap(),
        None,
        Settings {
            mode: Mode::Off,
            ..Settings::default()
        },
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = group.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    group.finish_after_exit(Duration::from_millis(200)).unwrap();
}
#[test]
fn kernel_detection_checks_full_rt_instead_of_guessing_from_release_names() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(realtime::kernel_rt(home.path()), None);
    fs::create_dir_all(home.path().join("proc")).unwrap();
    fs::write(
        home.path().join("proc/version"),
        "Linux version 7.3.0-rt #1 SMP PREEMPT_DYNAMIC",
    )
    .unwrap();
    assert_eq!(realtime::kernel_rt(home.path()), Some(false));
    fs::write(
        home.path().join("proc/version"),
        "Linux version 7.3.0 #1 SMP PREEMPT_RT",
    )
    .unwrap();
    assert_eq!(realtime::kernel_rt(home.path()), Some(true));
    fs::create_dir_all(home.path().join("sys/kernel")).unwrap();
    fs::write(home.path().join("sys/kernel/realtime"), "0\n").unwrap();
    assert_eq!(realtime::kernel_rt(home.path()), Some(false));
    fs::write(home.path().join("sys/kernel/realtime"), "1\n").unwrap();
    assert_eq!(realtime::kernel_rt(home.path()), Some(true));
}
#[test]
fn truncated_scheduling_reports_are_errors() {
    assert!(Report::read(&mut &[][..]).is_err());
    assert!(Report::read(&mut &[0u8; 15][..]).is_err());
}

#[test]
fn child_process_affinity_is_reset_to_all_cores() {
    use robot_manager::output::Journal;
    use robot_manager::realtime::pin_to_core;
    use std::os::unix::fs::PermissionsExt;
    let _ = pin_to_core(0);
    let temp = tempfile::tempdir().unwrap();
    let command = temp.path().join("probe.sh");
    let out = temp.path().join("affinity.txt");
    fs::write(
        &command,
        format!(
            "#!/bin/sh\npython3 -c 'import os; print(len(os.sched_getaffinity(0)))' > {}\n",
            out.display()
        ),
    )
    .unwrap();
    let mut perm = fs::metadata(&command).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&command, perm).unwrap();

    let env = std::collections::HashMap::new();
    let mut group = ChildGroup::spawn(
        &command,
        temp.path(),
        &env,
        Journal::with_writer(Box::new(std::io::sink())).unwrap(),
        None,
        Settings {
            mode: Mode::Off,
            ..Settings::default()
        },
        None,
    )
    .unwrap();

    let _ = group.stop(&[libc::SIGTERM], Duration::from_secs(2));
    if out.exists() {
        let count: usize = fs::read_to_string(&out).unwrap().trim().parse().unwrap_or(0);
        let num_cpus = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) } as usize;
        let expected = if num_cpus > 1 { num_cpus - 1 } else { 1 };
        assert_eq!(
            count, expected,
            "child should have affinity to {expected} CPUs, excluding supervisor CPU 3"
        );
    }
}
