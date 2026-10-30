//! VMX hardware monitor (CPU temperature, throttling/undervoltage, RAM and zram swap).
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareStats {
    pub cpu_temp_c: Option<f32>,
    pub throttled_raw: u32,
    pub throttled_hex: String,
    pub throttled_description: String,
    pub under_voltage: bool,
    pub frequency_capped: bool,
    pub throttled_now: bool,
    pub mem_total_mb: u64,
    pub mem_available_mb: u64,
    pub mem_used_mb: u64,
    pub swap_total_mb: u64,
    pub swap_free_mb: u64,
    pub swap_used_mb: u64,
    pub load_avg: (f32, f32, f32),
}

impl HardwareStats {
    pub fn collect() -> io::Result<Self> {
        let cpu_temp_c = read_cpu_temp();
        let throttled_raw = read_throttled_bits();
        let throttled_hex = format!("0x{:x}", throttled_raw);
        let throttled_description = describe_throttled(throttled_raw);

        let under_voltage = (throttled_raw & 0x1) != 0;
        let frequency_capped = (throttled_raw & 0x2) != 0;
        let throttled_now = (throttled_raw & 0x4) != 0;

        let (mem_total, mem_available, swap_total, swap_free) = read_meminfo();
        let mem_used = mem_total.saturating_sub(mem_available);
        let swap_used = swap_total.saturating_sub(swap_free);

        let load_avg = read_loadavg();

        Ok(Self {
            cpu_temp_c,
            throttled_raw,
            throttled_hex,
            throttled_description,
            under_voltage,
            frequency_capped,
            throttled_now,
            mem_total_mb: mem_total / 1024,
            mem_available_mb: mem_available / 1024,
            mem_used_mb: mem_used / 1024,
            swap_total_mb: swap_total / 1024,
            swap_free_mb: swap_free / 1024,
            swap_used_mb: swap_used / 1024,
            load_avg,
        })
    }

    pub fn format_text(&self) -> String {
        let temp_str = match self.cpu_temp_c {
            Some(t) => format!("{t:.1}°C"),
            None => "unavailable".into(),
        };
        format!(
            "CPU Temp:    {}\n\
             Throttling:  {} ({})\n\
             Memory:      {} / {} MiB (Available: {} MiB)\n\
             Swap:        {} / {} MiB (Free: {} MiB)\n\
             Load Avg:    {:.2}, {:.2}, {:.2}\n",
            temp_str,
            self.throttled_hex,
            self.throttled_description,
            self.mem_used_mb,
            self.mem_total_mb,
            self.mem_available_mb,
            self.swap_used_mb,
            self.swap_total_mb,
            self.swap_free_mb,
            self.load_avg.0,
            self.load_avg.1,
            self.load_avg.2
        )
    }
}

fn read_cpu_temp() -> Option<f32> {
    for path in [
        "/sys/class/thermal/thermal_zone0/temp",
        "/sys/devices/virtual/thermal/thermal_zone0/temp",
    ] {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(raw) = text.trim().parse::<f32>() {
                return Some(raw / 1000.0);
            }
        }
    }
    None
}

fn read_throttled_bits() -> u32 {
    for path in [
        "/sys/devices/platform/soc/soc:firmware/get_throttled",
        "/sys/bus/platform/devices/soc:firmware/get_throttled",
    ] {
        if let Ok(text) = fs::read_to_string(path) {
            let trimmed = text.trim().trim_start_matches("0x");
            if let Ok(val) = u32::from_str_radix(trimmed, 16) {
                return val;
            }
        }
    }
    if let Ok(output) = Command::new("vcgencmd").arg("get_throttled").output() {
        if let Ok(text) = String::from_utf8(output.stdout) {
            if let Some(pos) = text.find("0x") {
                let trimmed = text[pos + 2..].trim();
                if let Ok(val) = u32::from_str_radix(trimmed, 16) {
                    return val;
                }
            }
        }
    }
    0
}

pub fn describe_throttled(bits: u32) -> String {
    if bits == 0 {
        return "OK - normal voltage and clocks".into();
    }
    let mut flags = Vec::new();
    if bits & 0x1 != 0 {
        flags.push("Under-voltage detected now");
    }
    if bits & 0x2 != 0 {
        flags.push("Arm frequency currently capped");
    }
    if bits & 0x4 != 0 {
        flags.push("Currently throttled");
    }
    if bits & 0x8 != 0 {
        flags.push("Soft temperature limit active");
    }
    if bits & (1 << 16) != 0 {
        flags.push("Under-voltage occurred since boot");
    }
    if bits & (1 << 17) != 0 {
        flags.push("Arm frequency cap occurred since boot");
    }
    if bits & (1 << 18) != 0 {
        flags.push("Throttling occurred since boot");
    }
    if bits & (1 << 19) != 0 {
        flags.push("Soft temp limit occurred since boot");
    }
    flags.join(", ")
}

fn read_meminfo() -> (u64, u64, u64, u64) {
    let mut mem_total = 0;
    let mut mem_available = 0;
    let mut swap_total = 0;
    let mut swap_free = 0;

    if let Ok(text) = fs::read_to_string("/proc/meminfo") {
        for line in text.lines() {
            let mut parts = line.split_whitespace();
            if let (Some(key), Some(val_str)) = (parts.next(), parts.next()) {
                if let Ok(val) = val_str.parse::<u64>() {
                    match key {
                        "MemTotal:" => mem_total = val,
                        "MemAvailable:" => mem_available = val,
                        "SwapTotal:" => swap_total = val,
                        "SwapFree:" => swap_free = val,
                        _ => {}
                    }
                }
            }
        }
    }
    (mem_total, mem_available, swap_total, swap_free)
}

fn read_loadavg() -> (f32, f32, f32) {
    if let Ok(text) = fs::read_to_string("/proc/loadavg") {
        let parts: Vec<&str> = text.split_whitespace().collect();
        if parts.len() >= 3 {
            let l1 = parts[0].parse().unwrap_or(0.0);
            let l2 = parts[1].parse().unwrap_or(0.0);
            let l3 = parts[2].parse().unwrap_or(0.0);
            return (l1, l2, l3);
        }
    }
    (0.0, 0.0, 0.0)
}

pub fn run_daemon(interval: Duration, alert_temp_c: f32) -> io::Result<()> {
    crate::log("INFO", "hwmon daemon started (monitoring temperature and voltage)");
    let mut last_throttled = 0;
    loop {
        if let Ok(stats) = HardwareStats::collect() {
            if stats.throttled_raw != last_throttled && stats.throttled_raw != 0 {
                crate::log(
                    "WARNING",
                    format_args!(
                        "Hardware status alert: {} ({})",
                        stats.throttled_hex, stats.throttled_description
                    ),
                );
                last_throttled = stats.throttled_raw;
            }
            if let Some(temp) = stats.cpu_temp_c {
                if temp >= alert_temp_c {
                    crate::log(
                        "WARNING",
                        format_args!("High CPU temperature alert: {temp:.1}°C (limit {alert_temp_c:.1}°C)"),
                    );
                }
            }
        }
        thread::sleep(interval);
    }
}

pub fn dispatch(args: &[String]) -> Option<io::Result<()>> {
    if args.is_empty() || args[0] != "hwmon" {
        return None;
    }
    let mut as_json = false;
    let mut daemon = false;
    let mut interval_secs = 5u64;
    let mut alert_temp = 80.0f32;

    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--json" => as_json = true,
            "--daemon" => daemon = true,
            "--interval" => {
                if let Some(val) = iter.next().and_then(|v| v.parse().ok()) {
                    interval_secs = val;
                }
            }
            "--alert-temp" => {
                if let Some(val) = iter.next().and_then(|v| v.parse().ok()) {
                    alert_temp = val;
                }
            }
            _ if arg.starts_with("--interval=") => {
                if let Ok(val) = arg[11..].parse() {
                    interval_secs = val;
                }
            }
            _ if arg.starts_with("--alert-temp=") => {
                if let Ok(val) = arg[13..].parse() {
                    alert_temp = val;
                }
            }
            _ => {
                return Some(Err(io::Error::other(format!(
                    "unknown hwmon argument: {arg}\nusage: robot_manager hwmon [--json] [--daemon] [--interval SECS]"
                ))))
            }
        }
    }

    if daemon {
        Some(run_daemon(Duration::from_secs(interval_secs), alert_temp))
    } else {
        Some((|| {
            let stats = HardwareStats::collect()?;
            if as_json {
                println!("{}", serde_json::to_string_pretty(&stats).unwrap());
            } else {
                print!("{}", stats.format_text());
            }
            Ok(())
        })())
    }
}
