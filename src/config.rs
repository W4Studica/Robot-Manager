use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_CONFIG: &str = "/etc/robot_manager/config.json";
pub const DEFAULT_LSE_LIBRARY: &str = "/opt/robot_manager/lib/libvmx_lse_emu.so";

#[derive(Debug, Deserialize)]
pub struct Profile {
    pub name: Option<String>,
    pub kernel_regex: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub robot_home: PathBuf,
    #[serde(default)]
    pub robot_user: Option<String>,
    #[serde(default)]
    pub realtime: crate::realtime::Settings,
    pub robot_command: Option<PathBuf>,
    #[serde(default)]
    pub env: HashMap<String, Value>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default = "default_grace")]
    pub stop_grace_seconds: f64,
    #[serde(default = "default_signals")]
    pub stop_signals: Vec<String>,
    #[serde(default = "yes")]
    pub riolog_enabled: bool,
    #[serde(default = "default_port")]
    pub riolog_port: u16,
    #[serde(default = "default_history")]
    pub riolog_history: usize,
    /// Loopback port where daemons send alert lines that are republished into
    /// RioLog (`robots_manager` listens here; see alerts.rs).
    #[serde(default = "default_alert_port")]
    pub alert_port: u16,
    #[serde(default = "default_lse")]
    pub lse_emulation: String,
    #[serde(default = "default_library")]
    pub lse_emulation_library: PathBuf,
}
fn default_grace() -> f64 {
    5.0
}
fn default_signals() -> Vec<String> {
    vec!["SIGINT".into(), "SIGTERM".into()]
}
fn yes() -> bool {
    true
}
fn default_port() -> u16 {
    1741
}
fn default_alert_port() -> u16 {
    crate::alerts::DEFAULT_ALERT_PORT
}
fn default_history() -> usize {
    200
}
fn default_lse() -> String {
    "auto".into()
}
fn default_library() -> PathBuf {
    DEFAULT_LSE_LIBRARY.into()
}

pub fn signal_number(name: &str) -> Option<i32> {
    match name {
        "SIGINT" => Some(libc::SIGINT),
        "SIGTERM" => Some(libc::SIGTERM),
        "SIGHUP" => Some(libc::SIGHUP),
        "SIGQUIT" => Some(libc::SIGQUIT),
        "SIGUSR1" => Some(libc::SIGUSR1),
        "SIGUSR2" => Some(libc::SIGUSR2),
        _ => None,
    }
}
pub fn signal_name(number: i32) -> &'static str {
    match number {
        libc::SIGINT => "SIGINT",
        libc::SIGTERM => "SIGTERM",
        libc::SIGHUP => "SIGHUP",
        libc::SIGQUIT => "SIGQUIT",
        libc::SIGUSR1 => "SIGUSR1",
        libc::SIGUSR2 => "SIGUSR2",
        libc::SIGKILL => "SIGKILL",
        _ => "unknown signal",
    }
}
fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}
impl Config {
    pub fn load(path: &Path) -> io::Result<Self> {
        Self::parse(&fs::read_to_string(path)?)
    }
    pub fn parse(text: &str) -> io::Result<Self> {
        let config: Self = serde_json::from_str(text).map_err(invalid)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> io::Result<()> {
        if self.robot_home.as_os_str().is_empty() {
            return Err(invalid("robot_home must not be empty"));
        }
        self.stop_grace()?;
        self.realtime.prepare()?;
        if self.stop_signals.is_empty()
            || self.stop_signals.iter().any(|s| signal_number(s).is_none())
        {
            return Err(invalid("stop_signals must be a nonempty list of SIGINT, SIGTERM, SIGHUP, SIGQUIT, SIGUSR1 or SIGUSR2"));
        }
        if !matches!(
            self.lse_emulation.to_lowercase().as_str(),
            "auto" | "on" | "off"
        ) {
            return Err(invalid("lse_emulation must be auto, on or off"));
        }
        validate_env(&self.env)?;
        for profile in &self.profiles {
            if let Some(pattern) = &profile.kernel_regex {
                Regex::new(pattern).map_err(invalid)?;
            }
            validate_env(&profile.env)?;
        }
        Ok(())
    }
    pub fn stop_grace(&self) -> io::Result<Duration> {
        if !self.stop_grace_seconds.is_finite() || self.stop_grace_seconds <= 0.0 {
            return Err(invalid("stop_grace_seconds must be positive and finite"));
        }
        let duration = Duration::try_from_secs_f64(self.stop_grace_seconds).map_err(invalid)?;
        // Extremely large values cannot be represented by Instant deadlines.
        if std::time::Instant::now().checked_add(duration).is_none() {
            return Err(invalid("stop_grace_seconds is too large"));
        }
        Ok(duration)
    }
    pub fn command_file(&self) -> PathBuf {
        self.robot_command
            .as_ref()
            .filter(|p| !p.as_os_str().is_empty())
            .cloned()
            .unwrap_or_else(|| self.robot_home.join("robotCommand"))
    }
    pub fn signals(&self) -> Vec<i32> {
        self.stop_signals
            .iter()
            .filter_map(|s| signal_number(s))
            .collect()
    }
    pub fn environment(
        &self,
        kernel: &str,
        mut base: HashMap<String, String>,
        has_runtime_instructions: Option<bool>,
    ) -> io::Result<HashMap<String, String>> {
        extend_env(&mut base, &self.env);
        let mode = self.lse_emulation.to_lowercase();
        if mode != "off" && (mode == "on" || has_runtime_instructions == Some(false)) {
            if !self.lse_emulation_library.is_file() {
                crate::log("WARNING", format_args!("LSE emulation library {} is missing; programs using libMrcLib may fail with SIGILL", self.lse_emulation_library.display()));
            } else if base.get("LD_AUDIT").is_some_and(|s| !s.is_empty()) {
                crate::log(
                    "WARNING",
                    "LD_AUDIT is already set; not adding LSE emulation",
                );
            } else {
                base.insert(
                    "LD_AUDIT".into(),
                    self.lse_emulation_library.to_string_lossy().into_owned(),
                );
            }
        }
        for profile in &self.profiles {
            if let Some(pattern) = &profile.kernel_regex {
                if Regex::new(pattern).map_err(invalid)?.is_match(kernel) {
                    crate::log(
                        "INFO",
                        format_args!(
                            "platform profile '{}' matched kernel {kernel}",
                            profile.name.as_deref().unwrap_or(pattern)
                        ),
                    );
                    extend_env(&mut base, &profile.env);
                    break;
                }
            }
        }
        Ok(base)
    }
}
fn env_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Null => "None".into(),
        _ => value.to_string(),
    }
}
fn validate_env(env: &HashMap<String, Value>) -> io::Result<()> {
    for (key, value) in env {
        if key.is_empty() || key.contains(['=', '\0']) || env_value(value).contains('\0') {
            return Err(invalid(format!("invalid environment entry {key:?}")));
        }
    }
    Ok(())
}
fn extend_env(target: &mut HashMap<String, String>, values: &HashMap<String, Value>) {
    target.extend(values.iter().map(|(k, v)| (k.clone(), env_value(v))));
}
/// Every listed ARM core must support both LSE and RCpc to skip the compatibility library.
pub fn cpu_supports_runtime_instructions(text: &str) -> Option<bool> {
    text.lines()
        .filter_map(|line| {
            let (name, flags) = line.split_once(':')?;
            (name.trim() == "Features").then(|| {
                let has = |feature| flags.split_whitespace().any(|f| f == feature);
                has("atomics") && has("ilrcpc")
            })
        })
        .reduce(|all, supported| all && supported)
}
