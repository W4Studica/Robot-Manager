use robot_manager::config_tool::{add_preload, ensure_realtime};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn write(path: &std::path::Path, text: &str, mode: u32) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn read_value(path: &std::path::Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn realtime_policy_is_added_and_every_other_setting_survives() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    write(&path, r#"{"robot_home":"/home/lvuser","env":{"LD_PRELOAD":"/custom.so"},"profiles":[{"name":"custom","kernel_regex":"^6\\."}]}"#, 0o640);

    ensure_realtime(&path, false).unwrap();

    let value = read_value(&path);
    assert_eq!(value["realtime"], json!({"mode":"auto","policy":"rr","priority":10}));
    assert_eq!(value["robot_home"], "/home/lvuser");
    assert_eq!(value["env"]["LD_PRELOAD"], "/custom.so");
    assert_eq!(value["profiles"][0]["name"], "custom");
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
}

#[test]
fn existing_realtime_policy_is_preserved_byte_for_byte() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    for policy in [r#"{"mode":"off"}"#, r#"{"mode":"on","policy":"fifo","priority":25}"#] {
        let before = format!("{{\"robot_home\":\"/home/lvuser\",\"realtime\":{policy}}}\n");
        write(&path, &before, 0o644);
        ensure_realtime(&path, false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "policy {policy} must stay untouched");
    }
}

#[test]
fn non_object_configuration_fails_without_rewriting() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    write(&path, "[]\n", 0o644);
    let error = ensure_realtime(&path, false).unwrap_err();
    assert!(error.to_string().contains("must be a JSON object"), "{error}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "[]\n");
}

#[test]
fn dry_run_reports_the_edit_without_touching_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    let before = "{\"robot_home\":\"/home/lvuser\"}\n";
    write(&path, before, 0o644);
    ensure_realtime(&path, true).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn preload_is_appended_to_a_non_empty_environment() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    // The regression case of the old sed edit: a non-empty env object and a
    // multi-line layout must both keep every existing entry.
    write(
        &path,
        "{\n  \"env\": {\n    \"LD_PRELOAD\": \"/custom.so /other.so\",\n    \"KEEP\": \"yes\"\n  },\n  \"robot_home\": \"/home/lvuser\"\n}\n",
        0o640,
    );

    add_preload(&path, "/opt/robot_manager/lib/libvmx_gpio_isr_shim.so", false).unwrap();

    let value = read_value(&path);
    assert_eq!(
        value["env"]["LD_PRELOAD"],
        "/custom.so /other.so /opt/robot_manager/lib/libvmx_gpio_isr_shim.so"
    );
    assert_eq!(value["env"]["KEEP"], "yes");
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
}

#[test]
fn preload_is_created_when_env_or_the_key_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");

    write(&path, "{\"robot_home\":\"/x\"}\n", 0o644);
    add_preload(&path, "/lib/a.so", false).unwrap();
    assert_eq!(read_value(&path)["env"]["LD_PRELOAD"], "/lib/a.so");

    write(&path, "{\"robot_home\":\"/x\",\"env\":{}}\n", 0o644);
    add_preload(&path, "/lib/b.so", false).unwrap();
    assert_eq!(read_value(&path)["env"]["LD_PRELOAD"], "/lib/b.so");
}

#[test]
fn preload_is_idempotent_and_understands_colon_lists() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    write(&path, "{\"env\":{\"LD_PRELOAD\":\"/lib/a.so:/lib/b.so\"}}\n", 0o644);
    let before = fs::read_to_string(&path).unwrap();

    add_preload(&path, "/lib/b.so", false).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), before, "already present: no rewrite");

    add_preload(&path, "/lib/c.so", false).unwrap();
    assert_eq!(read_value(&path)["env"]["LD_PRELOAD"], "/lib/a.so /lib/b.so /lib/c.so");
}

#[test]
fn preload_rejects_malformed_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");

    write(&path, "{\"env\":[]}\n", 0o644);
    assert!(add_preload(&path, "/lib/a.so", false).unwrap_err().to_string().contains("env must be a JSON object"));

    write(&path, "{\"env\":{\"LD_PRELOAD\":7}}\n", 0o644);
    assert!(add_preload(&path, "/lib/a.so", false).unwrap_err().to_string().contains("env.LD_PRELOAD must be a string"));
}

#[test]
fn preload_dry_run_does_not_write() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    let before = "{\"env\":{\"LD_PRELOAD\":\"/x.so\"}}\n";
    write(&path, before, 0o644);
    add_preload(&path, "/lib/a.so", true).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}
