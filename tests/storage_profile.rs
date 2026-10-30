//! Coverage ported from the retired Python storage tests: the provisioning
//! state machine, the boot-wait decision table, swap-file safety and the full
//! `configure` flow against a fake root.
use robot_manager::storage::{
    configure_with_swap_size, finish_provisioning, network_wait_reason,
    prepare_swap_with_free, LOCAL_BOOT_CONF, SWAP_PATH, ZRAM_CONF,
};
use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::Path;

type Calls = RefCell<Vec<Vec<String>>>;

fn recording_runner(calls: &Calls) -> impl Fn(&[&str]) -> io::Result<String> + '_ {
    move |args: &[&str]| {
        calls.borrow_mut().push(args.iter().map(|a| a.to_string()).collect());
        Ok(String::new())
    }
}

fn write(root: &Path, relative: &str, text: &str) {
    let path = root.join(relative.trim_start_matches('/'));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

// --- cloud-init provisioning ------------------------------------------------

fn provisioning_fixture(root: &Path) {
    write(
        root,
        "etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg",
        "network: {config: disabled}\n",
    );
}

fn provisioning_with_status(status: &str) -> bool {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    provisioning_fixture(root);
    let runner = |_: &[&str]| Ok(status.to_string());
    finish_provisioning(root, &runner).unwrap();
    root.join("etc/cloud/cloud-init.disabled").is_file()
}

#[test]
fn only_a_cleanly_done_cloud_init_is_retired() {
    assert!(provisioning_with_status(
        r#"{"status":"done","extended_status":"done","errors":[],"recoverable_errors":[],"stage":null}"#
    ));
    for status in [
        r#"{"status":"running"}"#,
        r#"{"status":"error"}"#,
        r#"{"status":"done","extended_status":"degraded"}"#,
        r#"{"status":"done","extended_status":"done","errors":["boom"]}"#,
        r#"{"status":"done","extended_status":"done","recoverable_errors":["later"]}"#,
        r#"{"status":"done","extended_status":"done","stage":"init"}"#,
        "not json at all",
    ] {
        assert!(!provisioning_with_status(status), "must stay enabled: {status}");
    }
}

#[test]
fn provisioning_without_network_preparation_or_status_command_keeps_cloud_init() {
    // No network policy file yet.
    let temp = tempfile::tempdir().unwrap();
    let runner = |_: &[&str]| panic!("cloud-init must not be queried before networking is prepared");
    finish_provisioning(temp.path(), &runner).unwrap();
    assert!(!temp.path().join("etc/cloud/cloud-init.disabled").exists());

    // The status command is missing.
    let temp = tempfile::tempdir().unwrap();
    provisioning_fixture(temp.path());
    let runner = |_: &[&str]| Err(io::Error::other("cloud-init not found"));
    finish_provisioning(temp.path(), &runner).unwrap();
    assert!(!temp.path().join("etc/cloud/cloud-init.disabled").exists());
}

#[test]
fn an_existing_disable_marker_is_preserved_without_rerunning_cloud_init() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    provisioning_fixture(root);
    let marker = root.join("etc/cloud/cloud-init.disabled");
    fs::write(&marker, "# provisioned by an earlier release\n").unwrap();
    let runner = |_: &[&str]| panic!("the status command must not run again");
    finish_provisioning(root, &runner).unwrap();
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        "# provisioned by an earlier release\n"
    );
}

// --- boot wait decision table -----------------------------------------------

fn wait_reason(root: &Path, runner: &dyn Fn(&[&str]) -> io::Result<String>) -> Option<String> {
    network_wait_reason(root, runner).unwrap()
}

#[test]
fn boot_wait_follows_the_provisioning_and_boot_configuration() {
    let base = |mutate: &dyn Fn(&Path)| {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "etc/cloud/cloud-init.disabled", "# done\n");
        write(root, "etc/fstab", "UUID=root / ext4 defaults 0 1\n");
        write(root, "proc/cmdline", "root=UUID=root rw\n");
        mutate(root);
        temp
    };
    let quiet = |_: &[&str]| Ok(String::new());

    // A clean local-only robot gets the skip.
    let temp = base(&|_| {});
    assert_eq!(wait_reason(temp.path(), &quiet), None);

    // cloud-init still enabled.
    let temp = base(&|root| {
        fs::remove_file(root.join("etc/cloud/cloud-init.disabled")).unwrap();
    });
    assert_eq!(wait_reason(temp.path(), &quiet).unwrap(), "cloud-init is still enabled");

    // Explicit online request.
    let temp = base(&|root| write(root, "etc/robot_manager/require-network-online", ""));
    assert_eq!(
        wait_reason(temp.path(), &quiet).unwrap(),
        "online waiting was explicitly requested"
    );

    // Network filesystems.
    let temp = base(&|root| {
        write(root, "etc/fstab", "UUID=root / ext4 defaults 0 1\nserver:/data /data nfs defaults 0 0\n")
    });
    assert_eq!(
        wait_reason(temp.path(), &quiet).unwrap(),
        "network filesystems are configured"
    );

    // Network boot parameters.
    let temp = base(&|root| write(root, "proc/cmdline", "root=UUID=root rw ip=dhcp\n"));
    assert_eq!(
        wait_reason(temp.path(), &quiet).unwrap(),
        "network boot parameters are configured"
    );

    // iSCSI nodes.
    let temp = base(&|root| write(root, "etc/iscsi/nodes/target/portal", "x"));
    assert_eq!(
        wait_reason(temp.path(), &quiet).unwrap(),
        "iSCSI nodes or sessions exist"
    );

    // systemd remote mounts.
    let temp = base(&|_| {});
    let runner = |args: &[&str]| {
        if args.contains(&"remote-fs.target") {
            Ok("/mnt/data.mount".into())
        } else {
            Ok(String::new())
        }
    };
    assert_eq!(
        wait_reason(temp.path(), &runner).unwrap(),
        "systemd remote mounts are configured"
    );

    // Unclassified online consumers keep the wait.
    let temp = base(&|_| {});
    let runner = |args: &[&str]| {
        if args.contains(&"network-online.target") {
            Ok("custom-network-client.service multi-user.target".into())
        } else {
            Ok(String::new())
        }
    };
    assert_eq!(
        wait_reason(temp.path(), &runner).unwrap(),
        "unclassified online consumers: custom-network-client.service"
    );

    // Only the known consumers: still local-only.
    let temp = base(&|_| {});
    let runner = |args: &[&str]| {
        if args.contains(&"network-online.target") {
            Ok("cloud-final.service multi-user.target upki-fetch.service".into())
        } else {
            Ok(String::new())
        }
    };
    assert_eq!(wait_reason(temp.path(), &runner), None);

    // An unreadable fstab keeps the wait instead of guessing: the decision
    // function reports the error and the installer then leaves the wait alone.
    let temp = base(&|root| {
        fs::remove_file(root.join("etc/fstab")).unwrap();
    });
    assert!(network_wait_reason(temp.path(), &quiet).is_err());
    robot_manager::storage::configure_network_wait(temp.path(), &quiet).unwrap();
    let dropin = temp
        .path()
        .join("etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica-local-boot.conf");
    assert!(!dropin.exists(), "the skip may only be installed when the robot is verified local");
}

// --- swap file safety -------------------------------------------------------

#[test]
fn a_failed_mkswap_never_publishes_the_staging_file() {
    let temp = tempfile::tempdir().unwrap();
    let swap = temp.path().join("swapfile");
    let runner = |args: &[&str]| {
        if args[0] == "mkswap" {
            Err(io::Error::other("format failed"))
        } else {
            Ok(String::new())
        }
    };
    assert!(prepare_swap_with_free(&swap, &runner, 65536, None).is_err());
    assert!(!swap.exists());
    assert!(!temp.path().join(".swapfile.new").exists());
}

#[test]
fn an_existing_swap_is_never_reformatted_or_resized() {
    let temp = tempfile::tempdir().unwrap();
    let swap = temp.path().join("swapfile");
    let original = vec![7u8; 65536];
    fs::write(&swap, &original).unwrap();
    let calls: Calls = RefCell::new(Vec::new());
    let runner = recording_runner(&calls);

    // Wrong size is refused.
    assert!(prepare_swap_with_free(&swap, &runner, 1024, None).is_err());
    // Wrong filesystem type is refused.
    let wrong_type = |args: &[&str]| {
        if args[0] == "blkid" {
            Ok("ext4".to_string())
        } else {
            Ok(String::new())
        }
    };
    assert!(prepare_swap_with_free(&swap, &wrong_type, 65536, None).is_err());
    assert_eq!(fs::read(&swap).unwrap(), original);
    assert!(calls.borrow().is_empty(), "nothing may be formatted");

    // The matching file is accepted and only its permissions change.
    let correct = |args: &[&str]| {
        if args[0] == "blkid" {
            Ok("swap".to_string())
        } else {
            Ok(String::new())
        }
    };
    prepare_swap_with_free(&swap, &correct, 65536, None).unwrap();
    assert_eq!(fs::read(&swap).unwrap(), original);
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(fs::metadata(&swap).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn interrupted_staging_and_symlinks_are_handled_safely() {
    let temp = tempfile::tempdir().unwrap();
    let swap = temp.path().join("swapfile");
    // A leftover staging file from a power loss is replaced.
    fs::write(temp.path().join(".swapfile.new"), b"stale").unwrap();
    let runner = |_: &[&str]| Ok(String::new());
    prepare_swap_with_free(&swap, &runner, 65536, None).unwrap();
    assert!(swap.is_file());

    // A symlinked target is refused instead of followed.
    let other = temp.path().join("victim");
    fs::write(&other, b"keep me").unwrap();
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&other, &link).unwrap();
    assert!(prepare_swap_with_free(&link, &runner, 65536, None).is_err());
    assert_eq!(fs::read(&other).unwrap(), b"keep me");

    // A symlinked staging path is refused as well.
    let swap2 = temp.path().join("swap2");
    std::os::unix::fs::symlink(&other, temp.path().join(".swapfile.new")).unwrap();
    assert!(prepare_swap_with_free(&swap2, &runner, 65536, None).is_err());
    assert_eq!(fs::read(&other).unwrap(), b"keep me");
}

#[test]
fn low_disk_space_does_not_create_a_swap_file() {
    let temp = tempfile::tempdir().unwrap();
    let swap = temp.path().join("swapfile");
    let runner = |_: &[&str]| Ok(String::new());
    let error = prepare_swap_with_free(&swap, &runner, 4 * 1024 * 1024 * 1024, Some(1024)).unwrap_err();
    assert!(error.to_string().contains("Not enough free disk space"), "{error}");
    assert!(!swap.exists());
}

// --- the full profile -------------------------------------------------------

struct FakeRobot {
    temp: tempfile::TempDir,
    calls: Calls,
}

impl FakeRobot {
    /// Answers the findmnt/tune2fs probes of `configure` for a healthy ext4 root.
    fn new(fstab: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "etc/os-release", "ID=ubuntu\n");
        write(root, "etc/fstab", fstab);
        write(root, "proc/cmdline", "root=UUID=root rw\n");
        let calls = RefCell::new(Vec::new());
        Self { temp, calls }
    }

    fn runner(&self) -> impl Fn(&[&str]) -> io::Result<String> + '_ {
        move |args: &[&str]| {
            self.calls.borrow_mut().push(args.iter().map(|a| a.to_string()).collect());
            match args {
                ["findmnt", "-n", "-o", "FSTYPE", "--target", "/"] => Ok("ext4".into()),
                ["findmnt", "-n", "-o", "SOURCE", "--target", "/"] => Ok("/dev/sda1".into()),
                ["tune2fs", "-l", _] => Ok("Filesystem features: has_journal ext_attr\n".into()),
                ["findmnt", "-n", "-o", "OPTIONS", "--target", "/"] => Ok("rw,relatime".into()),
                ["findmnt", "-n", "-o", "FSTYPE", "--target", _] => Ok("ext4".into()),
                ["blkid", "-p", "-s", "TYPE", "-o", "value", _] => Ok("swap".into()),
                _ => Ok(String::new()),
            }
        }
    }
}

#[test]
fn full_profile_is_idempotent_and_keeps_the_original_backup() {
    let robot = FakeRobot::new("UUID=root / ext4 defaults 0 1\n/swap.img none swap sw 0 0\n");
    let runner = robot.runner();
    let root = robot.temp.path();

    configure_with_swap_size(root, &runner, 65536).unwrap();
    let fstab_after = fs::read_to_string(root.join("etc/fstab")).unwrap();
    assert!(fstab_after.contains(&format!("{SWAP_PATH}\tnone\tswap\tsw,pri=10,nofail\t0\t0")));
    assert!(fstab_after.contains("# w4studica: superseded swap: /swap.img"));
    assert_eq!(
        fs::read_to_string(root.join("etc/systemd/zram-generator.conf.d/99-w4studica.conf")).unwrap(),
        ZRAM_CONF
    );
    let backup = root.join("var/lib/w4studica/fstab.before-storage-profile");
    let original_backup = fs::read_to_string(&backup).unwrap();
    assert!(original_backup.contains("/swap.img"));

    // A second run changes nothing and never rewrites the backup.
    configure_with_swap_size(root, &runner, 65536).unwrap();
    assert_eq!(fs::read_to_string(root.join("etc/fstab")).unwrap(), fstab_after);
    assert_eq!(fs::read_to_string(&backup).unwrap(), original_backup);

    // No initramfs tooling, swapoff or mount is ever invoked.
    for call in robot.calls.borrow().iter() {
        let joined = call.join(" ");
        for forbidden in ["update-initramfs", "dracut", "swapoff", "swapon", "mount"] {
            assert!(!joined.contains(forbidden), "unexpected command: {joined}");
        }
    }
}

#[test]
fn profile_refuses_to_weaken_journaling_or_ignore_the_kernel_command_line() {
    // Root without a journal.
    let robot = FakeRobot::new("UUID=root / ext4 defaults 0 1\n");
    let runner = |args: &[&str]| match args {
        ["findmnt", "-n", "-o", "FSTYPE", "--target", "/"] => Ok("ext4".into()),
        ["findmnt", "-n", "-o", "SOURCE", "--target", "/"] => Ok("/dev/sda1".into()),
        ["tune2fs", "-l", _] => Ok("Filesystem features: ext_attr\n".into()),
        _ => Ok(String::new()),
    };
    assert!(configure_with_swap_size(robot.temp.path(), &runner, 65536).is_err());

    // Mounted with data=writeback already: refuse instead of remounting.
    let robot = FakeRobot::new("UUID=root / ext4 defaults 0 1\n");
    let runner = |args: &[&str]| match args {
        ["findmnt", "-n", "-o", "FSTYPE", "--target", "/"] => Ok("ext4".into()),
        ["findmnt", "-n", "-o", "SOURCE", "--target", "/"] => Ok("/dev/sda1".into()),
        ["tune2fs", "-l", _] => Ok("Filesystem features: has_journal\n".into()),
        ["findmnt", "-n", "-o", "OPTIONS", "--target", "/"] => Ok("rw,data=writeback".into()),
        _ => Ok(String::new()),
    };
    assert!(configure_with_swap_size(robot.temp.path(), &runner, 65536).is_err());

    // Conflicting rootflags in the kernel command line.
    let robot = FakeRobot::new("UUID=root / ext4 defaults 0 1\n");
    write(robot.temp.path(), "proc/cmdline", "root=UUID=root rw rootflags=noload\n");
    let runner = |args: &[&str]| match args {
        ["findmnt", "-n", "-o", "FSTYPE", "--target", "/"] => Ok("ext4".into()),
        ["findmnt", "-n", "-o", "SOURCE", "--target", "/"] => Ok("/dev/sda1".into()),
        ["tune2fs", "-l", _] => Ok("Filesystem features: has_journal\n".into()),
        ["findmnt", "-n", "-o", "OPTIONS", "--target", "/"] => Ok("rw,relatime".into()),
        _ => Ok(String::new()),
    };
    assert!(configure_with_swap_size(robot.temp.path(), &runner, 65536).is_err());

    // zram disabled on the kernel command line.
    let robot = FakeRobot::new("UUID=root / ext4 defaults 0 1\n");
    write(robot.temp.path(), "proc/cmdline", "root=UUID=root rw systemd.zram=0\n");
    assert!(configure_with_swap_size(robot.temp.path(), &runner, 65536).is_err());
}

#[test]
fn network_mounts_keep_their_wait_policy_and_remove_only_our_override() {
    let robot = FakeRobot::new("UUID=root / ext4 defaults 0 1\nserver:/data /data nfs defaults 0 0\n");
    let runner = robot.runner();
    let root = robot.temp.path();
    let dropin = root.join("etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica.conf");
    fs::create_dir_all(dropin.parent().unwrap()).unwrap();
    fs::write(&dropin, "previous override").unwrap();

    configure_with_swap_size(root, &runner, 65536).unwrap();
    assert!(!dropin.exists(), "our override must go when network mounts need the wait");
    // The local-boot decision file is untouched by the profile.
    assert!(!root
        .join("etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica-local-boot.conf")
        .exists());
}

#[test]
fn local_boot_dropin_uses_the_shipped_condition() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "etc/cloud/cloud-init.disabled", "# done\n");
    write(root, "etc/fstab", "UUID=root / ext4 defaults 0 1\n");
    write(root, "proc/cmdline", "root=UUID=root rw\n");
    let runner = |_: &[&str]| Ok(String::new());
    robot_manager::storage::configure_network_wait(root, &runner).unwrap();
    let dropin = root.join("etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica-local-boot.conf");
    assert_eq!(fs::read_to_string(&dropin).unwrap(), LOCAL_BOOT_CONF);
}
