use robot_manager::config::{cpu_supports_runtime_instructions, Config};
use serde_json::{json, Value};
use std::collections::HashMap;

#[test]
fn defaults_match_the_deployed_configuration() {
    let config = Config::parse(r#"{"robot_home":"/home/robot"}"#).unwrap();
    assert_eq!(
        config.command_file().to_str(),
        Some("/home/robot/robotCommand")
    );
    assert_eq!(config.signals(), [libc::SIGINT, libc::SIGTERM]);
    assert_eq!(config.stop_grace().unwrap().as_secs(), 5);
    assert_eq!(config.riolog_port, 1741);
    assert!(config.riolog_enabled);
}
#[test]
fn config_home_is_required_and_command_can_be_overridden() {
    assert!(Config::parse("{}").is_err());
    assert!(Config::parse(r#"{"robot_home":""}"#).is_err());
    let config =
        Config::parse(r#"{"robot_home":"/x", "robot_command":"/commands/start"}"#).unwrap();
    assert_eq!(config.command_file().to_str(), Some("/commands/start"));
}
#[test]
fn environment_preserves_precedence_and_first_matching_profile() {
    let config = Config::parse(
        r#"{"robot_home":"/x", "env":{"A":"base","B":1},
      "profiles":[{"name":"old","kernel_regex":"^5\\.","env":{"A":"old"}},
        {"name":"new","kernel_regex":"^6\\.","env":{"A":"new","C":"c"}},
        {"kernel_regex":"^6","env":{"A":"wrong"}}]}"#,
    )
    .unwrap();
    let base = HashMap::from([("A".into(), "os".into()), ("Z".into(), "z".into())]);
    let env = config.environment("6.8.0", base.clone(), None).unwrap();
    assert_eq!(env["A"], "new");
    assert_eq!(env["B"], "1");
    assert_eq!(env["C"], "c");
    assert_eq!(env["Z"], "z");
    assert_eq!(
        config.environment("4.19.0", base, None).unwrap()["A"],
        "base"
    );
}
#[test]
fn malformed_timeouts_and_signals_are_rejected() {
    for bad in [
        json!(0),
        json!(-1),
        json!("5"),
        Value::Null,
        json!(true),
        json!(1e300),
    ] {
        assert!(
            Config::parse(&json!({"robot_home":"/x", "stop_grace_seconds":bad}).to_string())
                .is_err()
        );
    }
    for bad in [
        json!([]),
        json!("SIGINT"),
        Value::Null,
        json!(["SIGKILL"]),
        json!([2]),
        json!(["sigint"]),
    ] {
        assert!(
            Config::parse(&json!({"robot_home":"/x", "stop_signals":bad}).to_string()).is_err()
        );
    }
    let config =
        Config::parse(r#"{"robot_home":"/x","stop_signals":["SIGTERM","SIGINT"]}"#).unwrap();
    assert_eq!(config.signals(), [libc::SIGTERM, libc::SIGINT]);
}
#[test]
fn invalid_env_and_regex_fail_before_starting_a_robot() {
    assert!(Config::parse(r#"{"robot_home":"/x","env":{"X=Y":"bad"}}"#).is_err());
    assert!(Config::parse(r#"{"robot_home":"/x","env":{"X":"bad\u0000value"}}"#).is_err());
    assert!(Config::parse(r#"{"robot_home":"/x","profiles":[{"kernel_regex":"["}]}"#).is_err());
}
#[test]
fn cpu_features_are_not_guessed_on_other_architectures() {
    assert_eq!(
        cpu_supports_runtime_instructions("Features : fp asimd atomics ilrcpc crc32\n"),
        Some(true)
    );
    assert_eq!(
        cpu_supports_runtime_instructions("Features : fp asimd crc32\n"),
        Some(false)
    );
    assert_eq!(
        cpu_supports_runtime_instructions("flags : sse sse2\n"),
        None
    );
    assert_eq!(
        cpu_supports_runtime_instructions("Features : fp asimd atomics\n"),
        Some(false)
    );
    assert_eq!(
        cpu_supports_runtime_instructions("Features : fp asimd ilrcpc\n"),
        Some(false)
    );
    assert_eq!(
        cpu_supports_runtime_instructions("Features : atomics ilrcpc\nFeatures : atomics\n"),
        Some(false)
    );
    assert_eq!(
        cpu_supports_runtime_instructions("Features : atomics ilrcpc\nFeatures : atomics ilrcpc\n"),
        Some(true)
    );
}
#[test]
fn lse_audit_only_loads_when_needed_and_never_overrides_an_existing_value() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let text = json!({"robot_home":"/x", "lse_emulation_library":file.path()}).to_string();
    let config = Config::parse(&text).unwrap();
    assert!(config
        .environment("6.0", HashMap::new(), Some(true))
        .unwrap()
        .is_empty());
    assert!(config
        .environment("6.0", HashMap::new(), None)
        .unwrap()
        .is_empty());
    let env = config
        .environment("6.0", HashMap::new(), Some(false))
        .unwrap();
    assert_eq!(env["LD_AUDIT"], file.path().to_str().unwrap());
    let existing = HashMap::from([("LD_AUDIT".into(), "existing".into())]);
    assert_eq!(
        config.environment("6.0", existing, Some(false)).unwrap()["LD_AUDIT"],
        "existing"
    );
}
#[test]
fn lse_off_and_missing_library_leave_the_environment_unchanged() {
    for mode in ["off", "auto", "on"] {
        let config = Config::parse(
            &json!({"robot_home":"/x", "lse_emulation":mode,
            "lse_emulation_library":"/does-not-exist.so"})
            .to_string(),
        )
        .unwrap();
        assert!(config
            .environment("6.0", HashMap::new(), Some(false))
            .unwrap()
            .is_empty());
    }
}

#[test]
fn realtime_defaults_enable_auto_rr_and_explicit_modes_are_validated() {
    use robot_manager::realtime::{Mode, Policy};
    let default = Config::parse(r#"{"robot_home":"/x"}"#).unwrap();
    assert_eq!(default.realtime.mode, Mode::Auto);
    assert_eq!(default.realtime.policy, Policy::Rr);
    assert_eq!(default.realtime.priority, 10);
    for mode in ["auto", "on", "off"] {
        for policy in ["rr", "fifo"] {
            for priority in [1, 10, 99] {
                Config::parse(&json!({"robot_home":"/x", "realtime":{"mode":mode,"policy":policy,"priority":priority}}).to_string()).unwrap();
            }
        }
    }
    for realtime in [
        json!({"mode":"maybe"}),
        json!({"policy":"other"}),
        json!({"priority":0}),
        json!({"priority":100}),
        json!({"priority":-1}),
        json!({"priority":"10"}),
        json!({"priority":2.5}),
        json!({"priority":null}),
        json!({"priorty":90}),
        json!(null),
        json!(true),
    ] {
        assert!(
            Config::parse(&json!({"robot_home":"/x", "realtime":realtime}).to_string()).is_err()
        );
    }
}
