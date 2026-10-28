#[path = "support/network.rs"]
mod support;
use robot_manager::network::{self, cli, profiles, system, Hotspot, Network};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use support::Fake;

fn fixture() -> (tempfile::TempDir, Network) {
    let temp = tempfile::tempdir().unwrap();
    let net = Network {
        directory: temp.path().into(),
    };
    (temp, net)
}
fn hotspot<'a>(band: &'a str) -> Hotspot<'a> {
    Hotspot {
        prefix: "VMX",
        team: "1234",
        band,
        password: "password",
        channel: None,
        apply: true,
    }
}
fn text(net: &Network, name: &str) -> String {
    fs::read_to_string(net.directory.join(format!("{name}.nmconnection"))).unwrap()
}
fn cli(net: &Network, system: &mut Fake, args: &[&str]) -> std::io::Result<String> {
    cli::run(&network::args(args), net, system)
}

#[test]
fn address_validation() {
    for (team, address) in [
        ("1234", "10.12.34.2"),
        ("4444", "10.44.44.2"),
        ("254", "10.2.54.2"),
        ("1", "10.0.1.2"),
        ("9999", "10.99.99.2"),
    ] {
        assert_eq!(profiles::team_address(team).unwrap(), address);
    }
    for team in ["0", "10000", "-5", "abc", ""] {
        assert!(profiles::team_address(team).is_err());
    }
}
#[test]
fn bands_and_radio_channels() {
    assert_eq!(profiles::band("2.4GHz").unwrap(), "bg");
    assert_eq!(profiles::band("5g").unwrap(), "a");
    assert!(profiles::band("6").is_err());
    assert_eq!(profiles::parse_channels(support::IW), [34, 36, 40, 149]);
    assert_eq!(
        profiles::channels("a", &profiles::parse_channels(support::IW)),
        [36, 40, 149]
    );
    assert_eq!(profiles::channels("bg", &[]), (1..=11).collect::<Vec<_>>());
    assert_eq!(profiles::default_channel(&[1, 6, 11]), Some(6));
    assert_eq!(profiles::default_channel(&[36, 149]), Some(36));
    assert_eq!(profiles::default_channel(&[149, 153]), Some(149));
    assert_eq!(profiles::default_channel(&[]), None);
}
#[test]
fn default_profiles_and_private_passwords() {
    let (_temp, net) = fixture();
    let mut system = Fake::default();
    let info = net.hotspot(&mut system, &hotspot("2.4")).unwrap();
    assert_eq!(
        (info.ssid.as_str(), info.address.as_str(), info.channel),
        ("VMX-1234", "10.12.34.2", 6)
    );
    let file = text(&net, "robot-ap");
    for line in [
        "id=robot-ap",
        "type=wifi",
        "mode=ap",
        "interface-name=wlan0",
        "band=bg",
        "channel=6",
        "method=shared",
        "address1=10.12.34.2/24",
        "powersave=2",
        "autoconnect-priority=100",
        "psk=password",
    ] {
        assert!(file.lines().any(|l| l == line), "{line}");
    }
    assert_eq!(
        fs::metadata(net.directory.join("robot-ap.nmconnection"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(!system
        .calls
        .iter()
        .flatten()
        .any(|s| s.contains("password")));
    assert!(system.ran(&["nmcli", "connection", "reload"]));
}
#[test]
fn invalid_input_never_writes_profile() {
    for (team, password, channel, prefix) in [
        ("abc", "password", None, "VMX"),
        ("1234", "short", None, "VMX"),
        ("1234", "password", Some(12), "VMX"),
        (
            "1234",
            "password",
            None,
            "prefix-is-much-too-long-for-a-wifi-network",
        ),
    ] {
        let (_temp, net) = fixture();
        let mut system = Fake::default();
        let mut options = hotspot("2.4");
        options.team = team;
        options.password = password;
        options.channel = channel;
        options.prefix = prefix;
        assert!(net.hotspot(&mut system, &options).is_err());
        assert!(!net.directory.join("robot-ap.nmconnection").exists());
        assert!(!system.ran(&["nmcli", "connection", "up"]));
    }
}
#[test]
fn invalid_password_and_ssid_boundaries() {
    assert!(profiles::password("password").is_ok());
    assert!(profiles::password(&"x".repeat(64)).is_err());
    assert!(profiles::password("password\0").is_err());
    assert!(profiles::ssid(&"x".repeat(32)).is_ok());
    assert!(profiles::ssid(&"한".repeat(11)).is_err());
}
#[test]
fn keyfile_strings_cannot_inject_sections() {
    let (_temp, net) = fixture();
    let mut system = Fake::default();
    net.toggle(
        &mut system,
        "wifi",
        Some(" Home\\lab\n[ipv4]"),
        Some("pass\nword"),
    )
    .unwrap();
    let file = text(&net, "robot-client");
    assert!(file.contains("ssid=\\sHome\\\\lab\\n[ipv4]"));
    assert!(file.contains("psk=pass\\nword"));
    assert_eq!(file.lines().filter(|l| *l == "[ipv4]").count(), 1);
}
#[test]
fn setup_reuses_uuid_and_removes_duplicate_profiles() {
    let (_temp, net) = fixture();
    let mut system = Fake {
        profiles: vec!["robot-ap:old-uuid".into(), "robot-eth:eth-uuid".into()],
        ..Fake::default()
    };
    net.hotspot(&mut system, &hotspot("2.4")).unwrap();
    let path = net.directory.join("robot-ap.nmconnection");
    let first = system::read_uuid(&path).unwrap();
    let mut options = hotspot("2.4");
    options.channel = Some(11);
    net.hotspot(&mut system, &options).unwrap();
    assert_eq!(system::read_uuid(&path).unwrap(), first);
    assert!(system.ran(&["nmcli", "connection", "delete", "uuid", "old-uuid"]));
    assert!(!system.ran(&["nmcli", "connection", "delete", "uuid", "eth-uuid"]));
    assert!(system.ran(&["nmcli", "connection", "up", "uuid", &first]));
}
#[test]
fn interactive_channel_validation_is_an_error_not_a_panic() {
    let (_temp, net) = fixture();
    let mut system = Fake {
        tty: true,
        answer: "11".into(),
        ..Fake::default()
    };
    assert_eq!(
        net.hotspot(&mut system, &hotspot("2.4")).unwrap().channel,
        11
    );
    system.answer = "".into();
    assert_eq!(
        net.hotspot(&mut system, &hotspot("2.4")).unwrap().channel,
        6
    );
    system.answer = "not a number".into();
    assert!(net.hotspot(&mut system, &hotspot("2.4")).is_err());
}
#[test]
fn five_ghz_defaults_and_invalid_or_absent_channels() {
    let (_temp, net) = fixture();
    let mut system = Fake::default();
    assert_eq!(net.hotspot(&mut system, &hotspot("5")).unwrap().channel, 36);
    let mut options = hotspot("5");
    options.channel = Some(52);
    assert!(net.hotspot(&mut system, &options).is_err());
    system.iw = "* 5260.0 MHz [52] (no IR)".into();
    assert!(net.hotspot(&mut system, &hotspot("5")).is_err());
}
#[test]
fn wifi_ssh_defers_and_wired_ssh_applies_hotspot_now() {
    let (_temp, net) = fixture();
    let mut system = Fake::ssh("wlan0");
    assert!(net.hotspot(&mut system, &hotspot("5")).unwrap().deferred);
    assert!(!system.ran(&["nmcli", "connection", "up"]));
    let job = system.calls.iter().find(|c| c[0] == "systemd-run").unwrap();
    assert!(job.contains(&"--on-active=3".into()));
    let script = job.last().unwrap();
    assert!(script.starts_with("nmcli connection reload || exit $?; nmcli connection up uuid "));
    assert!(script.contains(" || { nmcli connection modify uuid "));
    assert!(script.contains("802-11-wireless.band bg 802-11-wireless.channel 6"));
    let mut wired = Fake::ssh("eth0");
    assert!(!net.hotspot(&mut wired, &hotspot("2.4")).unwrap().deferred);
    assert!(wired.ran(&["nmcli", "connection", "up"]));
    assert!(!wired.ran(&["systemd-run"]));
}
#[test]
fn immediate_failed_five_ghz_attempts_fallback_and_reports_error() {
    let (_temp, net) = fixture();
    let mut system = Fake {
        fail_up: true,
        ..Fake::default()
    };
    let error = net.hotspot(&mut system, &hotspot("5")).unwrap_err();
    assert!(error.to_string().contains("fallback"));
    assert!(system.ran(&["nmcli", "connection", "modify", "uuid"]));
}
#[test]
fn immediate_failed_two_ghz_reports_error() {
    let (_temp, net) = fixture();
    let mut system = Fake {
        fail_up: true,
        ..Fake::default()
    };
    assert!(net.hotspot(&mut system, &hotspot("2.4")).is_err());
}
#[test]
fn no_activate_works_before_networkmanager_installation() {
    let (_temp, net) = fixture();
    let mut system = Fake {
        active: false,
        missing_nmcli: true,
        ..Fake::ssh("wlan0")
    };
    let mut options = hotspot("2.4");
    options.apply = false;
    assert!(!net.hotspot(&mut system, &options).unwrap().deferred);
    net.ethernet(&mut system, None, false).unwrap();
    assert!(text(&net, "robot-eth").contains("address1=172.22.11.2/24"));
    assert!(!system.ran(&["nmcli", "connection"]));
    assert!(!system.ran(&["systemctl"]));
    assert!(!system.ran(&["ip"]));
    assert!(!system.ran(&["systemd-run"]));
}
#[test]
fn mutations_require_root_and_active_networkmanager() {
    let (_temp, net) = fixture();
    for mut system in [
        Fake {
            root: false,
            ..Fake::default()
        },
        Fake {
            active: false,
            ..Fake::default()
        },
    ] {
        assert!(net.hotspot(&mut system, &hotspot("2.4")).is_err());
        assert!(net.ethernet(&mut system, None, true).is_err());
        assert!(net.toggle(&mut system, "wifi", Some("Home"), None).is_err());
    }
}
#[test]
fn ethernet_selection_and_ssh_deferral() {
    let (_temp, net) = fixture();
    let mut wired = Fake::ssh("eth0");
    assert!(net.ethernet(&mut wired, None, true).unwrap().deferred);
    let mut wifi = Fake::ssh("wlan0");
    assert!(
        !net.ethernet(&mut wifi, Some("end0"), true)
            .unwrap()
            .deferred
    );
    assert!(text(&net, "robot-eth").contains("interface-name=end0"));
    assert!(net.ethernet(&mut wifi, Some("../bad"), false).is_err());
}
#[test]
fn ap_toggle_requires_profile_and_disables_saved_wifi() {
    let (_temp, net) = fixture();
    let mut system = Fake {
        wifi: vec!["robot-ap".into(), "HomeRouter".into()],
        ..Fake::default()
    };
    assert!(net.toggle(&mut system, "ap", None, None).is_err());
    net.hotspot(&mut system, &hotspot("2.4")).unwrap();
    net.toggle(&mut system, "ap", None, None).unwrap();
    assert!(system.ran(&[
        "nmcli",
        "connection",
        "modify",
        "HomeRouter",
        "connection.autoconnect",
        "no"
    ]));
    assert!(system.ran(&["nmcli", "connection", "up", "robot-ap"]));
}
#[test]
fn join_new_router_and_reuse_client_uuid() {
    let (_temp, net) = fixture();
    let mut system = Fake::default();
    net.hotspot(&mut system, &hotspot("2.4")).unwrap();
    net.toggle(&mut system, "wifi", Some("HomeRouter"), Some("routerpass"))
        .unwrap();
    let path = net.directory.join("robot-client.nmconnection");
    let uuid = system::read_uuid(&path).unwrap();
    net.toggle(&mut system, "wifi", Some("OtherRouter"), Some("routerpass"))
        .unwrap();
    assert_eq!(system::read_uuid(&path).unwrap(), uuid);
    let file = text(&net, "robot-client");
    assert!(file.contains("mode=infrastructure"));
    assert!(file.contains("powersave=2"));
    assert!(system.ran(&[
        "nmcli",
        "connection",
        "modify",
        "robot-ap",
        "connection.autoconnect",
        "no"
    ]));
    assert!(system.ran(&["nmcli", "connection", "up", "robot-client"]));
    assert!(!system
        .calls
        .iter()
        .flatten()
        .any(|s| s.contains("routerpass")));
}
#[test]
fn open_router_and_saved_router_selection() {
    let (_temp, net) = fixture();
    let mut system = Fake {
        wifi: vec!["robot-ap".into(), "Home:Router".into()],
        ..Fake::default()
    };
    net.toggle(&mut system, "wifi", None, None).unwrap();
    assert!(system.ran(&["nmcli", "connection", "up", "Home:Router"]));
    net.toggle(&mut system, "wifi", Some("OpenRouter"), None)
        .unwrap();
    assert!(!text(&net, "robot-client").contains("wifi-security"));
    assert!(net
        .toggle(&mut Fake::default(), "wifi", None, None)
        .unwrap_err()
        .to_string()
        .contains("SSID PASSWORD"));
    assert!(net.toggle(&mut system, "bridge", None, None).is_err());
}
#[test]
fn both_toggle_directions_defer_over_wifi_ssh_and_quote_names() {
    let (_temp, net) = fixture();
    net.hotspot(&mut Fake::default(), &hotspot("2.4")).unwrap();
    let mut system = Fake {
        wifi: vec!["Home'; echo INJECTION".into()],
        ..Fake::ssh("wlan0")
    };
    assert!(net.toggle(&mut system, "ap", None, None).unwrap());
    assert!(net.toggle(&mut system, "wifi", None, None).unwrap());
    assert!(!system.ran(&["nmcli", "connection", "up"]));
    let script = system.calls.last().unwrap().last().unwrap();
    assert!(script.contains("'Home'\"'\"'; echo INJECTION'"));
}
#[test]
fn ssh_device_and_no_session() {
    assert_eq!(
        network::ssh_device(&mut Fake::ssh("wlan0")).unwrap(),
        "wlan0"
    );
    assert_eq!(network::ssh_device(&mut Fake::ssh("eth0")).unwrap(), "eth0");
    assert_eq!(network::ssh_device(&mut Fake::default()).unwrap(), "");
}
#[test]
fn cli_legacy_flags_and_no_activate() {
    let (_temp, net) = fixture();
    let mut system = Fake {
        active: false,
        missing_nmcli: true,
        ..Fake::default()
    };
    assert!(cli(
        &net,
        &mut system,
        &[
            "setupHotspot",
            "--force",
            "--no-activate",
            "VMX",
            "1234",
            "2.4"
        ]
    )
    .unwrap()
    .contains("VMX-1234"));
    assert!(text(&net, "robot-ap").contains("psk=password"));
    cli(&net, &mut system, &["setupEthernet", "--no-activate"]).unwrap();
}
#[test]
fn cli_rejects_invalid_flags_and_arguments_before_side_effects() {
    let (_temp, net) = fixture();
    let mut system = Fake::default();
    for args in [
        vec![],
        vec!["setupEthernet", "--channel", "1"],
        vec!["status", "extra"],
        vec!["setupHotspot", "--channel", "bad", "VMX", "1234", "2.4"],
        vec!["toggleNetMode", "ap", "Extra"],
        vec!["unknown"],
    ] {
        assert!(cli(&net, &mut system, &args).is_err());
    }
    assert!(system.calls.is_empty());
}
#[test]
fn executable_alias_and_explicit_subcommand_dispatch() {
    let (_temp, net) = fixture();
    let mut system = Fake::default();
    let args = network::args(&["--no-activate", "VMX", "1234", "2.4"]);
    assert!(
        cli::dispatch("/usr/local/bin/setupHotspot", &args, &net, &mut system)
            .unwrap()
            .unwrap()
            .contains("VMX-1234")
    );
    assert!(cli::dispatch(
        "robot_manager",
        &network::args(&["network", "--help"]),
        &net,
        &mut system
    )
    .unwrap()
    .unwrap()
    .contains("setupEthernet"));
    assert!(cli::dispatch(
        "network_backup",
        &network::args(&["--help"]),
        &net,
        &mut system
    )
    .unwrap()
    .is_ok());
    assert!(cli::dispatch("robot_manager", &[], &net, &mut system).is_none());
}
#[test]
fn real_binary_parent_ssh_lookup_without_network_mutations() {
    // Fake ip in PATH only prints its arguments. The real lookup must recover
    // SSH_CONNECTION from the shell after it unsets the child's environment.
    let temp = tempfile::tempdir().unwrap();
    let ip = temp.path().join("ip");
    fs::write(
        &ip,
        "#!/bin/sh\nprintf '%s dev parent-wifi src x\\n' \"$5\"\n",
    )
    .unwrap();
    fs::set_permissions(&ip, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            "unset SSH_CONNECTION; \"$1\" network sshdevice; true",
            "sh",
            env!("CARGO_BIN_EXE_robot_manager"),
        ])
        .env("SSH_CONNECTION", "10.1.2.3 1 10.1.2.4 22")
        .env("PATH", temp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "parent-wifi"
    );
}
#[test]
fn secret_write_atomically_replaces_symlink_and_uuid_is_valid() {
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("original");
    fs::write(&target, "original").unwrap();
    let path = temp.path().join("profile");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    system::write_secret(&path, "new secret").unwrap();
    assert_eq!(fs::read_to_string(target).unwrap(), "original");
    assert_eq!(fs::read_to_string(&path).unwrap(), "new secret");
    assert!(!fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    assert!(regex::Regex::new(
        r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
    )
    .unwrap()
    .is_match(&system::uuid().unwrap()));
}

#[test]
fn deferred_shell_stops_on_reload_failure_and_falls_back_only_on_activation_failure() {
    let (_temp, net) = fixture();
    let mut system = Fake::ssh("wlan0");
    net.hotspot(&mut system, &hotspot("5")).unwrap();
    let script = system.calls.last().unwrap().last().unwrap();
    let commands = tempfile::tempdir().unwrap();
    let nmcli = commands.path().join("nmcli");
    fs::write(&nmcli, "#!/bin/sh\nprintf '%s %s\\n' \"$1\" \"$2\" >> \"$TRACE\"\ncase \"$1 $2:$FAIL\" in 'connection reload:reload'|'connection up:up') exit 5;; esac\n").unwrap();
    fs::set_permissions(nmcli, fs::Permissions::from_mode(0o755)).unwrap();
    for failure in ["reload", "up", "none"] {
        let trace = commands.path().join(format!("trace-{failure}"));
        let output = Command::new("/bin/sh")
            .args(["-c", script])
            .env("PATH", commands.path())
            .env("TRACE", &trace)
            .env("FAIL", failure)
            .output()
            .unwrap();
        let trace = fs::read_to_string(trace).unwrap();
        if failure == "reload" {
            assert!(!output.status.success());
            assert_eq!(trace, "connection reload\n");
        } else if failure == "up" {
            assert!(trace.contains("connection modify\n"));
            assert_eq!(trace.matches("connection up\n").count(), 2);
        } else {
            assert!(output.status.success());
            assert!(!trace.contains("connection modify\n"));
        }
    }
}

#[test]
fn network_dispatch_leaves_non_utf8_supervisor_paths_untouched() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let (_temp, net) = fixture();
    let mut system = Fake::default();
    let invalid = OsString::from_vec(b"config-\xff.json".to_vec());
    assert!(cli::dispatch_os(
        &["robot_manager".into(), "--config".into(), invalid.clone()],
        &net,
        &mut system
    )
    .is_none());
    assert!(cli::dispatch_os(
        &["robot_manager".into(), "network".into(), invalid],
        &net,
        &mut system
    )
    .unwrap()
    .is_err());
    assert!(system.calls.is_empty());
}
