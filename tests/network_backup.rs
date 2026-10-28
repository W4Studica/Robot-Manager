#[allow(dead_code)]
#[path = "support/network.rs"]
mod support;
use robot_manager::network::backup::{self, CLOUD, MANAGED, NETPLAN, NM, STATE};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use support::Fake;
fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
#[test]
fn reinstall_and_repeated_restore_preserve_original_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut system = Fake::default();
    let original = format!("{NETPLAN}/home.yaml");
    let wifi = format!("{NM}/home.nmconnection");
    write(
        root,
        &original,
        "network: {version: 2, renderer: NetworkManager}\n",
    );
    write(root, &wifi, "[wifi]\nssid=home\n");
    write(root, CLOUD, "original cloud configuration\n");
    let state = backup::snapshot(root, &mut system).unwrap();
    let initial = fs::read(state.join("state.json")).unwrap();
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o700
    );
    fs::remove_file(root.join(&original)).unwrap();
    fs::remove_file(root.join(&wifi)).unwrap();
    write(root, &format!("{NETPLAN}/{MANAGED}"), "installer config");
    write(root, &format!("{NM}/robot-ap.nmconnection"), "installer ap");
    write(root, CLOUD, "installer cloud config");
    system.calls.clear();
    backup::snapshot(root, &mut system).unwrap();
    assert!(system.calls.is_empty());
    assert_eq!(fs::read(state.join("state.json")).unwrap(), initial);
    for _ in 0..2 {
        backup::restore(root, &mut system).unwrap();
        assert!(fs::read_to_string(root.join(&original))
            .unwrap()
            .contains("NetworkManager"));
        assert_eq!(
            fs::read_to_string(root.join(&wifi)).unwrap(),
            "[wifi]\nssid=home\n"
        );
        assert_eq!(
            fs::metadata(root.join(&wifi)).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_to_string(root.join(CLOUD)).unwrap(),
            "original cloud configuration\n"
        );
        assert!(!root.join(format!("{NETPLAN}/{MANAGED}")).exists());
        assert_eq!(fs::read(state.join("state.json")).unwrap(), initial);
    }
    assert!(system.ran(&["systemctl", "start", "NetworkManager.service"]));
    assert!(!system.ran(&["systemctl", "start", "systemd-networkd.service"]));
    assert!(system.ran(&["nmcli", "connection", "reload"]));
}
#[test]
fn restore_networkd_and_remove_installer_cloud_config() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut system = Fake {
        manager: "systemd-networkd.service".into(),
        ..Fake::default()
    };
    write(
        root,
        &format!("{NETPLAN}/home.yaml"),
        "network: {version: 2, renderer: networkd}",
    );
    backup::snapshot(root, &mut system).unwrap();
    write(root, CLOUD, "installer cloud config");
    system.calls.clear();
    backup::restore(root, &mut system).unwrap();
    assert!(!root.join(CLOUD).exists());
    assert!(system.ran(&["systemctl", "start", "systemd-networkd.service"]));
    assert!(!system.ran(&["systemctl", "start", "NetworkManager.service"]));
    assert_eq!(
        system.calls[0],
        robot_manager::network::args(&["systemctl", "stop", "w4studica-network-switch.timer"])
    );
    assert_eq!(
        system.calls.last().unwrap(),
        &robot_manager::network::args(&["netplan", "apply"])
    );
}
#[test]
fn legacy_backup_selection_skips_installer_only_reinstalls() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut system = Fake::default();
    write(root, &format!("{NETPLAN}/{MANAGED}"), "installer");
    write(
        root,
        &format!("{NETPLAN}/w4studica-backup-0/{MANAGED}"),
        "installer only",
    );
    write(
        root,
        &format!("{NETPLAN}/w4studica-backup-1/home.yaml"),
        "original",
    );
    write(
        root,
        &format!("{NETPLAN}/w4studica-backup-2/{MANAGED}"),
        "installer only",
    );
    write(root, &format!("{NM}/robot-ap.nmconnection"), "installer AP");
    let state = backup::snapshot(root, &mut system).unwrap();
    assert_eq!(
        fs::read_to_string(state.join("netplan/home.yaml")).unwrap(),
        "original"
    );
    assert!(!state.join("netplan").join(MANAGED).exists());
    assert!(!state.join("nm/robot-ap.nmconnection").exists());
    assert!(backup::load(&state).unwrap().legacy);
    system.calls.clear();
    backup::restore(root, &mut system).unwrap();
    assert!(!root.join(NM).join("robot-ap.nmconnection").exists());
    assert!(system.ran(&["systemctl", "start", "systemd-networkd.service"]));
}
#[test]
fn no_original_legacy_backup_fails_before_commands_or_writes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut system = Fake::default();
    write(root, &format!("{NETPLAN}/{MANAGED}"), "installer");
    assert!(backup::snapshot(root, &mut system)
        .unwrap_err()
        .to_string()
        .contains("Original netplan"));
    assert!(system.calls.is_empty());
    assert!(!root.join(STATE).exists());
    assert_eq!(
        fs::read_to_string(root.join(NETPLAN).join(MANAGED)).unwrap(),
        "installer"
    );
}
#[test]
fn incomplete_backup_is_rejected_on_reinstall_and_restore_before_mutations() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut system = Fake::default();
    let state = backup::snapshot(root, &mut system).unwrap();
    fs::remove_dir(state.join("nm")).unwrap();
    system.calls.clear();
    assert!(backup::snapshot(root, &mut system)
        .unwrap_err()
        .to_string()
        .contains("Incomplete"));
    assert!(backup::restore(root, &mut system).is_err());
    assert!(system.calls.is_empty());
}
#[test]
fn file_inventory_missing_cloud_and_invalid_metadata_are_rejected() {
    for damage in ["nm", "cloud", "schema", "service", "inventory"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut system = Fake::default();
        write(root, &format!("{NM}/home.nmconnection"), "original");
        write(root, CLOUD, "original cloud");
        let state = backup::snapshot(root, &mut system).unwrap();
        match damage {
            "nm" => fs::remove_file(state.join("nm/home.nmconnection")).unwrap(),
            "cloud" => fs::remove_file(state.join("cloud.cfg")).unwrap(),
            _ => {
                let path = state.join("state.json");
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                match damage {
                    "schema" => value["schemaVersion"] = 2.into(),
                    "service" => {
                        value["services"]["NetworkManager.service"]["active"] = "yes".into()
                    }
                    _ => value["files"]["nm"] = serde_json::json!([]),
                };
                fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
        }
        system.calls.clear();
        assert!(backup::restore(root, &mut system).is_err(), "{damage}");
        assert!(system.calls.is_empty());
        assert_eq!(
            fs::read_to_string(root.join(NM).join("home.nmconnection")).unwrap(),
            "original"
        );
    }
}
#[test]
fn existing_python_schema_one_metadata_restores() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let state = root.join(STATE);
    fs::create_dir_all(state.join("netplan")).unwrap();
    fs::create_dir_all(state.join("nm")).unwrap();
    // Shape and service keys emitted by the retired Python implementation.
    fs::write(state.join("state.json"),r#"{"schemaVersion":1,"services":{"NetworkManager.service":{"enabled":"disabled","active":false},"systemd-networkd.service":{"enabled":"enabled","active":true},"systemd-networkd.socket":{"enabled":"enabled-runtime","active":true},"systemd-networkd-wait-online.service":{"enabled":"masked-runtime","active":false}},"legacy":false,"files":{"netplan":[],"nm":[],"cloud":false}}"#).unwrap();
    let mut system = Fake::default();
    backup::restore(root, &mut system).unwrap();
    assert!(system.ran(&[
        "systemctl",
        "enable",
        "--runtime",
        "systemd-networkd.socket"
    ]));
    assert!(system.ran(&[
        "systemctl",
        "mask",
        "--runtime",
        "systemd-networkd-wait-online.service"
    ]));
}
#[test]
fn backup_symlink_is_rejected_before_restore() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut system = Fake::default();
    write(root, &format!("{NM}/home.nmconnection"), "original");
    let state = backup::snapshot(root, &mut system).unwrap();
    let path = state.join("nm/home.nmconnection");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(root.join(NM).join("home.nmconnection"), &path).unwrap();
    system.calls.clear();
    assert!(backup::restore(root, &mut system).is_err());
    assert!(system.calls.is_empty());
}
