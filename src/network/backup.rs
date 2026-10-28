//! Immutable pre-install backup; metadata remains compatible with schema version 1.
use super::system::{uuid, System};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File, FileTimes};
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

pub const STATE: &str = "var/lib/robot_manager/network-original";
pub const NETPLAN: &str = "etc/netplan";
pub const NM: &str = "etc/NetworkManager/system-connections";
pub const CLOUD: &str = "etc/cloud/cloud.cfg.d/99-w4studica-disable-network-config.cfg";
pub const MANAGED: &str = "01-w4studica-network-manager.yaml";
pub const UNITS: [&str; 4] = [
    "NetworkManager.service",
    "systemd-networkd.service",
    "systemd-networkd.socket",
    "systemd-networkd-wait-online.service",
];
pub const MANAGED_PROFILES: [&str; 3] = [
    "robot-ap.nmconnection",
    "robot-eth.nmconnection",
    "robot-client.nmconnection",
];
#[derive(Serialize, Deserialize)]
pub struct Service {
    pub enabled: String,
    pub active: bool,
}
#[derive(Serialize, Deserialize)]
pub struct Inventory {
    pub netplan: Vec<String>,
    pub nm: Vec<String>,
    pub cloud: bool,
}
#[derive(Serialize, Deserialize)]
pub struct State {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub services: BTreeMap<String, Service>,
    pub legacy: bool,
    pub files: Inventory,
}
fn entries(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut result = match fs::read_dir(path) {
        Ok(entries) => entries
            .map(|e| e.map(|e| e.path()))
            .collect::<io::Result<Vec<_>>>()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => vec![],
        Err(e) => return Err(e),
    };
    result.sort();
    Ok(result)
}
fn names(path: &Path) -> io::Result<Vec<String>> {
    entries(path)?
        .iter()
        .map(|p| {
            p.file_name()
                .and_then(|v| v.to_str())
                .map(str::to_owned)
                .ok_or_else(|| io::Error::other("invalid backup filename"))
        })
        .collect()
}
fn copy(source: &Path, dest: &Path) -> io::Result<()> {
    fs::copy(source, dest)?;
    let metadata = fs::metadata(source)?;
    let times = FileTimes::new()
        .set_accessed(metadata.accessed()?)
        .set_modified(metadata.modified()?);
    File::open(dest)?.set_times(times)?;
    Ok(())
}
fn yaml(path: &Path) -> bool {
    path.is_file() && path.extension().is_some_and(|ext| ext == "yaml")
}
fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
fn run(system: &mut impl System, args: &[&str], check: bool) -> io::Result<String> {
    system
        .run(&super::args(args), check)
        .map(|v| v.trim().into())
}

pub fn load(backup: &Path) -> io::Result<State> {
    let state: State = serde_json::from_slice(&fs::read(backup.join("state.json"))?)
        .map_err(|e| io::Error::other(format!("Invalid network backup metadata: {e}")))?;
    if state.schema_version != 1
        || state.services.len() != UNITS.len()
        || UNITS.iter().any(|u| !state.services.contains_key(*u))
    {
        return Err(io::Error::other("Invalid network backup metadata"));
    }
    for (directory, expected) in [("netplan", &state.files.netplan), ("nm", &state.files.nm)] {
        let path = backup.join(directory);
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_dir())
            || names(&path)? != *expected
        {
            return Err(io::Error::other(format!(
                "Incomplete network backup: {directory}"
            )));
        }
        // Never let corrupt metadata/symlinks redirect a restore outside the backup.
        for file in entries(&path)? {
            if !fs::symlink_metadata(file)?.file_type().is_file() {
                return Err(io::Error::other("Invalid network backup entry"));
            }
        }
    }
    let cloud = backup.join("cloud.cfg");
    if cloud.exists() != state.files.cloud
        || (state.files.cloud && !fs::symlink_metadata(cloud)?.file_type().is_file())
    {
        return Err(io::Error::other(
            "Incomplete network backup: cloud configuration",
        ));
    }
    Ok(state)
}
pub fn snapshot(root: &Path, system: &mut impl System) -> io::Result<PathBuf> {
    let destination = root.join(STATE);
    if destination.join("state.json").is_file() {
        load(&destination)?;
        return Ok(destination);
    }
    if destination.exists() {
        return Err(io::Error::other(format!(
            "Incomplete network backup: {}",
            destination.display()
        )));
    }
    let mut source = root.join(NETPLAN);
    let legacy = source.join(MANAGED).exists();
    if legacy {
        source = entries(&source)?
            .into_iter()
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("w4studica-backup-")
            })
            .find(|p| {
                entries(p).is_ok_and(|files| {
                    files
                        .iter()
                        .any(|f| yaml(f) && f.file_name().unwrap() != MANAGED)
                })
            })
            .ok_or_else(|| {
                io::Error::other(
                    "Original netplan backup is missing; refusing to snapshot installer settings",
                )
            })?;
    }
    let mut services = BTreeMap::new();
    for unit in UNITS {
        let enabled = run(system, &["systemctl", "is-enabled", unit], false)?;
        let active = run(system, &["systemctl", "is-active", unit], false)? == "active";
        services.insert(
            unit.into(),
            if legacy {
                Service {
                    enabled: if unit.starts_with("systemd-networkd") {
                        "enabled"
                    } else {
                        "disabled"
                    }
                    .into(),
                    active: matches!(unit, "systemd-networkd.service" | "systemd-networkd.socket"),
                }
            } else {
                Service { enabled, active }
            },
        );
    }
    let parent = destination.parent().unwrap();
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".network-original-{}", uuid()?));
    DirBuilder::new().mode(0o700).create(&temporary)?;
    let result = (|| {
        fs::create_dir(temporary.join("netplan"))?;
        for file in entries(&source)?.iter().filter(|p| yaml(p)) {
            copy(
                file,
                &temporary.join("netplan").join(file.file_name().unwrap()),
            )?;
        }
        fs::create_dir(temporary.join("nm"))?;
        for file in entries(&root.join(NM))? {
            if file.is_file()
                && !(legacy
                    && MANAGED_PROFILES
                        .iter()
                        .any(|n| file.file_name().unwrap() == *n))
            {
                copy(&file, &temporary.join("nm").join(file.file_name().unwrap()))?;
            }
        }
        let cloud = root.join(CLOUD);
        if cloud.is_file() && !legacy {
            copy(&cloud, &temporary.join("cloud.cfg"))?;
        }
        let files = Inventory {
            netplan: names(&temporary.join("netplan"))?,
            nm: names(&temporary.join("nm"))?,
            cloud: temporary.join("cloud.cfg").is_file(),
        };
        let state = State {
            schema_version: 1,
            services,
            legacy,
            files,
        };
        let text = serde_json::to_string_pretty(&state).map_err(io::Error::other)? + "\n";
        fs::write(temporary.join("state.json"), text)?;
        load(&temporary)?;
        // A concurrent install must never replace the first snapshot.
        fs::rename(&temporary, &destination)?;
        Ok(destination.clone())
    })();
    let _ = fs::remove_dir_all(&temporary);
    result
}
pub fn restore(root: &Path, system: &mut impl System) -> io::Result<PathBuf> {
    let backup = root.join(STATE);
    let state = load(&backup)?;
    run(
        system,
        &["systemctl", "stop", "w4studica-network-switch.timer"],
        false,
    )?;
    run(
        system,
        &["systemctl", "stop", "w4studica-network-switch.service"],
        false,
    )?;
    let netplan = root.join(NETPLAN);
    fs::create_dir_all(&netplan)?;
    remove(&netplan.join(MANAGED))?;
    for file in entries(&backup.join("netplan"))?.iter().filter(|p| yaml(p)) {
        copy(file, &netplan.join(file.file_name().unwrap()))?;
    }
    let nm = root.join(NM);
    fs::create_dir_all(&nm)?;
    for name in MANAGED_PROFILES {
        remove(&nm.join(name))?;
    }
    for file in entries(&backup.join("nm"))? {
        copy(&file, &nm.join(file.file_name().unwrap()))?;
    }
    let cloud = root.join(CLOUD);
    if state.files.cloud {
        fs::create_dir_all(cloud.parent().unwrap())?;
        copy(&backup.join("cloud.cfg"), &cloud)?;
    } else {
        remove(&cloud)?;
    }
    run(system, &["netplan", "generate"], true)?;
    for unit in UNITS {
        if !state.services[unit].active {
            run(system, &["systemctl", "stop", unit], false)?;
        }
    }
    for unit in UNITS {
        let enabled = state.services[unit].enabled.as_str();
        if enabled.starts_with("masked") {
            let mut command = vec!["systemctl", "mask"];
            if enabled == "masked-runtime" {
                command.push("--runtime");
            }
            command.push(unit);
            run(system, &command, true)?;
        } else if matches!(
            enabled,
            "enabled" | "enabled-runtime" | "disabled" | "not-found" | ""
        ) {
            run(system, &["systemctl", "unmask", unit], false)?;
            let mut command = vec![
                "systemctl",
                if enabled.starts_with("enabled") {
                    "enable"
                } else {
                    "disable"
                },
            ];
            if enabled == "enabled-runtime" {
                command.push("--runtime");
            }
            command.push(unit);
            run(system, &command, !matches!(enabled, "not-found" | ""))?;
        }
    }
    for unit in UNITS {
        if state.services[unit].active {
            run(system, &["systemctl", "start", unit], true)?;
        }
    }
    if state.services["NetworkManager.service"].active {
        run(system, &["nmcli", "connection", "reload"], true)?;
    }
    run(system, &["netplan", "apply"], true)?;
    Ok(backup)
}
