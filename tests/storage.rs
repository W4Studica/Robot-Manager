use robot_manager::storage::{
    finish_provisioning, fstab_profile, network_wait_reason, prepare_swap, SWAP_PATH,
};
use std::fs;
use std::io;

#[test]
fn fstab_profile_updates_ext4_root_and_retires_competing_swaps() {
    let before = "# robot\nUUID=root / ext4 defaults,data=writeback,nobarrier,noauto_da_alloc,lazytime 0 0 # root\nUUID=boot /boot/firmware vfat defaults 0 2\n/swap.img none swap sw 0 0\n";
    let (after, network) = fstab_profile(before).unwrap();
    assert!(!network);
    assert!(after.contains("UUID=boot /boot/firmware vfat defaults 0 2"));
    assert!(after.contains("# w4studica: superseded swap: /swap.img"));
    assert!(after.contains("data=ordered,barrier=1,auto_da_alloc,commit=5,errors=remount-ro\t0\t1 # root"));
    assert!(!after.contains("data=writeback"));
    assert!(!after.contains("nobarrier"));
    assert_eq!(fstab_profile(&after).unwrap().0, after);
    assert!(after.contains(&format!("{SWAP_PATH}\tnone\tswap\tsw,pri=10,nofail\t0\t0")));
}

#[test]
fn fstab_profile_rejects_missing_duplicate_or_non_ext4_root() {
    for text in ["", "UUID=x / btrfs defaults 0 1\n", "UUID=a / ext4 defaults 0 1\nUUID=b / ext4 defaults 0 1\n"] {
        assert!(fstab_profile(text).is_err(), "should fail on: {text}");
    }
}

#[test]
fn fstab_profile_detects_network_filesystems() {
    for mount in [
        "server:/data /data nfs defaults 0 0",
        "//server/data /data cifs defaults 0 0",
        "UUID=data /data ext4 defaults,_netdev 0 2",
    ] {
        let text = format!("UUID=root / ext4 defaults 0 1\n{mount}\n");
        let (_, network) = fstab_profile(&text).unwrap();
        assert!(network, "should detect network mount: {mount}");
    }
}

#[test]
fn prepare_swap_creates_valid_swap_file() {
    let temp = tempfile::tempdir().unwrap();
    let swap = temp.path().join("swapfile");

    let runner = |args: &[&str]| -> io::Result<String> {
        if args[0] == "mkswap" {
            Ok("Setting up swapspace".into())
        } else if args[0] == "blkid" {
            Ok("swap".into())
        } else {
            Ok(String::new())
        }
    };

    prepare_swap(&swap, &runner, 1024 * 1024).unwrap();
    assert!(swap.is_file());
    assert_eq!(fs::metadata(&swap).unwrap().len(), 1024 * 1024);
    assert!(!swap.with_file_name(".swapfile.new").exists());
}

#[test]
fn finish_provisioning_disables_completed_cloud_init() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let network_policy = root.join("etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg");
    fs::create_dir_all(network_policy.parent().unwrap()).unwrap();
    fs::write(&network_policy, "network: {config: disabled}\n").unwrap();

    let marker = root.join("etc/cloud/cloud-init.disabled");
    let runner = |args: &[&str]| -> io::Result<String> {
        if args == ["cloud-init", "status", "--format", "json"] {
            Ok(r#"{"status": "done", "extended_status": "done", "errors": [], "recoverable_errors": []}"#.into())
        } else {
            Ok(String::new())
        }
    };

    finish_provisioning(root, &runner).unwrap();
    assert!(marker.is_file());
}

#[test]
fn finish_provisioning_preserves_incomplete_cloud_init() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let network_policy = root.join("etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg");
    fs::create_dir_all(network_policy.parent().unwrap()).unwrap();
    fs::write(&network_policy, "network: {config: disabled}\n").unwrap();

    let marker = root.join("etc/cloud/cloud-init.disabled");
    let runner = |args: &[&str]| -> io::Result<String> {
        if args == ["cloud-init", "status", "--format", "json"] {
            Ok(r#"{"status": "running"}"#.into())
        } else {
            Ok(String::new())
        }
    };

    finish_provisioning(root, &runner).unwrap();
    assert!(!marker.exists());
}

#[test]
fn network_wait_reason_identifies_local_only_robot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("etc/cloud")).unwrap();
    fs::write(root.join("etc/cloud/cloud-init.disabled"), "# disabled\n").unwrap();
    fs::write(root.join("etc/fstab"), "UUID=root / ext4 defaults 0 1\n").unwrap();
    fs::create_dir_all(root.join("proc")).unwrap();
    fs::write(root.join("proc/cmdline"), "root=UUID=root rw\n").unwrap();

    let runner = |args: &[&str]| -> io::Result<String> {
        if args.contains(&"remote-fs.target") {
            Ok("remote-fs-pre.target".into())
        } else if args.contains(&"network-online.target") {
            Ok("cloud-config.service multi-user.target upki-fetch.service".into())
        } else {
            Ok(String::new())
        }
    };

    let reason = network_wait_reason(root, &runner).unwrap();
    assert!(reason.is_none(), "expected local-only, got: {reason:?}");
}
