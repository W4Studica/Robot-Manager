use robot_manager::hwmon::{describe_throttled, HardwareStats};

#[test]
fn describe_throttled_identifies_flags() {
    assert_eq!(describe_throttled(0), "OK - normal voltage and clocks");
    let desc = describe_throttled(0x50005);
    assert!(desc.contains("Under-voltage detected now"));
    assert!(desc.contains("Currently throttled"));
    assert!(desc.contains("Under-voltage occurred since boot"));
}

#[test]
fn collect_returns_populated_stats() {
    let stats = HardwareStats::collect().unwrap();
    assert_eq!(stats.throttled_hex.starts_with("0x"), true);
    assert!(stats.format_text().contains("CPU Temp:"));
    assert!(stats.format_text().contains("Memory:"));
    assert!(stats.format_text().contains("Swap:"));
}

#[test]
fn json_serialization_matches_schema() {
    let stats = HardwareStats::collect().unwrap();
    let json = serde_json::to_string(&stats).unwrap();
    assert!(json.contains("cpuTempC"));
    assert!(json.contains("throttledRaw"));
    assert!(json.contains("memTotalMb"));
}
