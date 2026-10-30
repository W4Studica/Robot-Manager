//! In-place robot_manager configuration edits used by install.sh.
//! Replaces the previous sed-based JSON manipulation: unknown keys and every
//! other value are preserved, formatting is deterministic, and a write is
//! skipped entirely when nothing has to change.
use crate::config::DEFAULT_CONFIG;
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const REALTIME_POLICY: &str = r#"{"mode":"auto","policy":"rr","priority":10}"#;

fn parse(path: &Path) -> io::Result<Value> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))
}

fn object<'a>(path: &Path, value: &'a Value) -> io::Result<&'a Map<String, Value>> {
    value.as_object().ok_or_else(|| {
        io::Error::other(format!(
            "{}: configuration must be a JSON object",
            path.display()
        ))
    })
}

fn write(path: &Path, value: &Value) -> io::Result<()> {
    let mode = fs::metadata(path)
        .map(|m| {
            use std::os::unix::fs::PermissionsExt;
            m.permissions().mode() & 0o7777
        })
        .unwrap_or(0o644);
    let text = format!("{}\n", serde_json::to_string_pretty(value)?);
    crate::storage::atomic_write(path, &text, mode)
}

/// Add the default realtime policy when the configuration has none.
/// An existing policy is kept byte for byte; nothing else is touched.
pub fn ensure_realtime(path: &Path, dry_run: bool) -> io::Result<()> {
    let value = parse(path)?;
    let mut config = object(path, &value)?.clone();
    if config.contains_key("realtime") {
        println!("  keeping existing RT policy in {}", path.display());
        return Ok(());
    }
    if dry_run {
        println!("[dry-run] add realtime auto/rr/10 to {}", path.display());
        return Ok(());
    }
    let policy: Value = serde_json::from_str(REALTIME_POLICY).expect("static policy");
    config.insert("realtime".into(), policy);
    write(path, &Value::Object(config))?;
    println!("  added RT policy auto/rr/10 to {}", path.display());
    Ok(())
}

/// Append a library to env.LD_PRELOAD, preserving every other environment key
/// and any other preload entry. Idempotent.
pub fn add_preload(path: &Path, library: &str, dry_run: bool) -> io::Result<()> {
    let value = parse(path)?;
    let mut config = object(path, &value)?.clone();

    let env = config
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()));
    let env = env.as_object_mut().ok_or_else(|| {
        io::Error::other(format!("{}: env must be a JSON object", path.display()))
    })?;

    let current = match env.get("LD_PRELOAD") {
        None => "",
        Some(Value::String(text)) => text.as_str(),
        Some(_) => {
            return Err(io::Error::other(format!(
                "{}: env.LD_PRELOAD must be a string",
                path.display()
            )))
        }
    };
    let mut libraries: Vec<&str> = current
        .split(|c: char| c.is_whitespace() || c == ':')
        .filter(|entry| !entry.is_empty())
        .collect();
    if libraries.contains(&library) {
        println!(
            "  {} already in {} env.LD_PRELOAD",
            library,
            path.display()
        );
        return Ok(());
    }
    if dry_run {
        println!(
            "[dry-run] add {library} to {} env.LD_PRELOAD",
            path.display()
        );
        return Ok(());
    }
    libraries.push(library);
    env.insert(
        "LD_PRELOAD".into(),
        Value::String(libraries.join(" ")),
    );
    write(path, &Value::Object(config))?;
    println!("  added {library} to {} env.LD_PRELOAD", path.display());
    Ok(())
}

pub fn dispatch(args: &[String]) -> Option<io::Result<()>> {
    if args.is_empty() || args[0] != "config" {
        return None;
    }
    let mut dry_run = false;
    let mut positionals: Vec<&str> = Vec::new();
    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            value if value.starts_with("--") => {
                return Some(Err(io::Error::other(format!(
                    "unknown config argument: {value}\nusage: robot_manager config ensure-realtime [--dry-run] [PATH]\n       robot_manager config add-preload [--dry-run] [PATH] LIBRARY"
                ))))
            }
            value => positionals.push(value),
        }
    }
    let Some(command) = positionals.first().copied() else {
        return Some(Err(io::Error::other(
            "usage: robot_manager config ensure-realtime [--dry-run] [PATH]\n       robot_manager config add-preload [--dry-run] [PATH] LIBRARY",
        )));
    };
    let default = PathBuf::from(DEFAULT_CONFIG);
    let result = match (command, &positionals[1..]) {
        ("ensure-realtime", []) => ensure_realtime(&default, dry_run),
        ("ensure-realtime", [path]) => ensure_realtime(Path::new(path), dry_run),
        ("add-preload", [library]) => add_preload(&default, library, dry_run),
        ("add-preload", [path, library]) => add_preload(Path::new(path), library, dry_run),
        (command, _) => Err(io::Error::other(format!(
            "unexpected arguments for config {command}\nusage: robot_manager config ensure-realtime [--dry-run] [PATH]\n       robot_manager config add-preload [--dry-run] [PATH] LIBRARY"
        ))),
    };
    Some(result)
}
