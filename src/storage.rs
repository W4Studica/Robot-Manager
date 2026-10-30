//! VMX storage profile, provisioning retirement, and boot wait configuration.
use std::fs::{self, symlink_metadata, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const GIB: u64 = 1024 * 1024 * 1024;
pub const SWAP_SIZE: u64 = 4 * GIB;
pub const SWAP_PATH: &str = "/var/lib/w4studica/swapfile";

pub const ZRAM_CONF: &str = "[zram0]\nzram-size = ram * 2\ncompression-algorithm = zstd\nswap-priority = 100\nfs-type = swap\n";
pub const JOURNALD_CONF: &str = "[Journal]\nStorage=volatile\nRuntimeMaxUse=64M\n";
pub const MEMORY_CONF: &str = "# Aggressive zram compressed swap settings for VMX robot background services\nvm.swappiness = 180\nvm.page-cluster = 0\nvm.vfs_cache_pressure = 50\nvm.watermark_boost_factor = 0\nvm.watermark_scale_factor = 125\n";
pub const NETWORK_WAIT_CONF: &str = "[Unit]\n# Installed by W4Studica storage profile: NetworkManager-wait-online should never wait for an access point or client Wi-Fi that may not connect.\n[Service]\nExecStart=\nExecStart=/usr/bin/nmcli connection show\n";
pub const LOCAL_BOOT_CONF: &str = "[Unit]\n# Installed only after provisioning, with no remote mounts, iSCSI or unknown\n# online consumers. Create this file to explicitly restore the online wait.\nConditionPathExists=/etc/robot_manager/require-network-online\n";

pub fn run_os(args: &[&str]) -> io::Result<String> {
    let output = Command::new(args[0])
        .args(&args[1..])
        .env("LC_ALL", "C")
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "Command failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn sync_directory(path: &Path) -> io::Result<()> {
    if let Ok(file) = File::open(path) {
        let _ = file.sync_all();
    }
    Ok(())
}

pub fn ensure_directory(path: &Path) -> io::Result<()> {
    if let Ok(meta) = symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(io::Error::other(format!(
                "Refusing symlink directory: {}",
                path.display()
            )));
        }
        if meta.is_dir() {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        ensure_directory(parent)?;
    }
    fs::create_dir_all(path)?;
    if let Some(parent) = path.parent() {
        let _ = sync_directory(parent);
    }
    Ok(())
}

pub fn atomic_write(path: &Path, text: &str, mode: u32) -> io::Result<()> {
    if let Ok(meta) = symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(io::Error::other(format!(
                "Refusing to replace symlink: {}",
                path.display()
            )));
        }
    }
    if path.is_file() {
        if let Ok(existing) = fs::read_to_string(path) {
            if existing == text {
                return Ok(());
            }
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("path must have a parent"))?;
    ensure_directory(parent)?;

    let temp_name = format!(".w4studica-{}", fastrand());
    let temp_path = parent.join(temp_name);
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp_path)?;
        file.write_all(text.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
    }
    fs::rename(&temp_path, path)?;
    let _ = sync_directory(parent);
    Ok(())
}

fn fastrand() -> u64 {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    ((nanos ^ (pid << 64)) & 0xFFFF_FFFF_FFFF_FFFF) as u64
}

pub fn fstab_profile(text: &str) -> io::Result<(String, bool)> {
    let mut lines = Vec::new();
    let mut roots = 0;
    let mut network_mount = false;

    for line in text.lines() {
        let (body, separator, comment) = match line.find('#') {
            Some(pos) => (&line[..pos], true, &line[pos + 1..]),
            None => (line, false, ""),
        };
        let fields: Vec<&str> = body.split_whitespace().collect();
        if fields.is_empty() || fields[0].starts_with('#') {
            lines.push(line.to_string());
            continue;
        }
        if fields.len() < 4 {
            return Err(io::Error::other(format!("Invalid fstab entry: {line}")));
        }
        if ["nfs", "nfs4", "cifs", "smb3", "sshfs", "fuse.sshfs"].contains(&fields[2])
            || fields[3].split(',').any(|o| o == "_netdev")
        {
            network_mount = true;
        }
        if fields[2] == "swap" {
            if fields[0] != SWAP_PATH {
                lines.push(format!("# w4studica: superseded swap: {line}"));
            }
            continue;
        }
        if fields[1] == "/" {
            roots += 1;
            if fields[2] != "ext4" && fields[2] != "auto" {
                return Err(io::Error::other(
                    "This storage profile requires an ext4 root filesystem",
                ));
            }
            let replaced = [
                "atime", "relatime", "strictatime", "lazytime", "nolazytime",
                "noatime", "barrier", "nobarrier", "data", "commit", "errors",
                "noload", "norecovery", "auto_da_alloc", "noauto_da_alloc",
            ];
            let mut options: Vec<&str> = fields[3]
                .split(',')
                .filter(|o| {
                    let key = o.split('=').next().unwrap_or("");
                    !replaced.contains(&key)
                })
                .collect();
            options.extend(&[
                "noatime",
                "nolazytime",
                "data=ordered",
                "barrier=1",
                "auto_da_alloc",
                "commit=5",
                "errors=remount-ro",
            ]);
            let mut new_fields: Vec<String> = fields.iter().map(|s| s.to_string()).collect();
            new_fields[3] = options.join(",");
            while new_fields.len() < 6 {
                new_fields.push("0".into());
            }
            new_fields[5] = "1".into();
            let mut new_line = new_fields.join("\t");
            if separator {
                new_line.push_str(&format!(" #{}", comment));
            }
            lines.push(new_line);
            continue;
        }
        lines.push(line.to_string());
    }

    if roots != 1 {
        return Err(io::Error::other(
            "Expected exactly one root mount in /etc/fstab; no files changed",
        ));
    }
    lines.push(format!("{SWAP_PATH}\tnone\tswap\tsw,pri=10,nofail\t0\t0"));
    let mut result = lines.join("\n");
    result.push('\n');
    Ok((result, network_mount))
}

pub fn prepare_swap(
    path: &Path,
    runner: &dyn Fn(&[&str]) -> io::Result<String>,
    size: u64,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_directory(parent)?;
    }
    if let Ok(meta) = symlink_metadata(path) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(io::Error::other(format!(
                "Refusing non-regular swap file: {}",
                path.display()
            )));
        }
        if meta.len() != size
            || runner(&[
                "blkid",
                "-p",
                "-s",
                "TYPE",
                "-o",
                "value",
                path.to_str().unwrap(),
            ])? != "swap"
        {
            return Err(io::Error::other(format!(
                "Existing managed swap is not a valid {size}-byte swap file; refusing to overwrite it: {}",
                path.display()
            )));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        return Ok(());
    }

    let temporary = path.with_file_name(".swapfile.new");
    if let Ok(meta) = symlink_metadata(&temporary) {
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(io::Error::other(format!(
                "Refusing non-regular swap staging file: {}",
                temporary.display()
            )));
        }
    }
    let _ = fs::remove_file(&temporary);

    // Free disk space check using statvfs
    if let Some(parent) = path.parent() {
        let parent_str = std::ffi::CString::new(parent.to_str().unwrap())?;
        let mut stat = std::mem::MaybeUninit::uninit();
        let ret = unsafe { libc::statvfs(parent_str.as_ptr(), stat.as_mut_ptr()) };
        if ret == 0 {
            let stat = unsafe { stat.assume_init() };
            let free_bytes = stat.f_bavail as u64 * stat.f_frsize as u64;
            if free_bytes < size + 256 * 1024 * 1024 {
                return Err(io::Error::other(
                    "Not enough free disk space for 4 GiB swap plus 256 MiB reserve",
                ));
            }
        }
    }

    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        let chunk_size = 4 * 1024 * 1024;
        let chunk = vec![0u8; chunk_size];
        let mut remaining = size;
        while remaining > 0 {
            let to_write = std::cmp::min(remaining, chunk_size as u64) as usize;
            file.write_all(&chunk[..to_write])?;
            remaining -= to_write as u64;
        }
        file.flush()?;
        file.sync_all()?;
    }

    runner(&["mkswap", temporary.to_str().unwrap()])?;
    if let Ok(file) = File::open(&temporary) {
        let _ = file.sync_all();
    }
    fs::rename(&temporary, path)?;
    if let Some(parent) = path.parent() {
        let _ = sync_directory(parent);
    }
    Ok(())
}

pub fn finish_provisioning(
    root: &Path,
    runner: &dyn Fn(&[&str]) -> io::Result<String>,
) -> io::Result<()> {
    let cloud = root.join("etc/cloud");
    let marker = cloud.join("cloud-init.disabled");
    if marker.exists() {
        println!("cloud-init: already disabled; preserving the existing marker");
        return Ok(());
    }
    let network_policy = cloud.join("cloud.cfg.d/99-w4studica-disable-network-config.cfg");
    if !network_policy.is_file() {
        println!("cloud-init: keeping enabled until W4Studica networking has been prepared");
        return Ok(());
    }

    let status_output = runner(&["cloud-init", "status", "--format", "json"]);
    let status_json: serde_json::Value = match status_output {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(val) => val,
            Err(_) => {
                println!("cloud-init: initial provisioning is not cleanly complete; keeping enabled");
                return Ok(());
            }
        },
        Err(_) => {
            println!("cloud-init: completion could not be verified; keeping the existing policy");
            return Ok(());
        }
    };

    let is_done = status_json.is_object()
        && status_json.get("status").and_then(|s| s.as_str()) == Some("done")
        && status_json
            .get("extended_status")
            .map(|s| s.as_str() == Some("done"))
            .unwrap_or(true)
        && status_json
            .get("errors")
            .map(|v| v.is_null() || v.as_array().map_or(false, |a| a.is_empty()))
            .unwrap_or(true)
        && status_json
            .get("recoverable_errors")
            .map(|v| v.is_null() || v.as_array().map_or(false, |a| a.is_empty()))
            .unwrap_or(true)
        && status_json
            .get("stage")
            .map(|v| v.is_null() || v.as_str().map_or(false, |s| s.is_empty()))
            .unwrap_or(true);

    if !is_done {
        println!("cloud-init: initial provisioning is not cleanly complete; keeping enabled");
        return Ok(());
    }

    atomic_write(
        &marker,
        "# W4Studica robot provisioning is complete.\n# Remove this file to enable cloud-init on subsequent boots.\n",
        0o644,
    )?;
    println!("cloud-init: provisioning complete; disabled for subsequent boots");
    Ok(())
}

pub fn network_wait_reason(
    root: &Path,
    runner: &dyn Fn(&[&str]) -> io::Result<String>,
) -> io::Result<Option<String>> {
    if !root.join("etc/cloud/cloud-init.disabled").is_file() {
        return Ok(Some("cloud-init is still enabled".into()));
    }
    if root
        .join("etc/robot_manager/require-network-online")
        .exists()
    {
        return Ok(Some("online waiting was explicitly requested".into()));
    }
    let fstab_text = fs::read_to_string(root.join("etc/fstab"))?;
    let (_, remote) = fstab_profile(&fstab_text)?;
    if remote {
        return Ok(Some("network filesystems are configured".into()));
    }
    let cmdline = fs::read_to_string(root.join("proc/cmdline")).unwrap_or_default();
    let flags: Vec<&str> = cmdline.split_whitespace().collect();
    if flags.iter().any(|flag| {
        flag.starts_with("ip=")
            || flag.starts_with("nfsroot=")
            || flag.starts_with("netroot=")
            || flag.starts_with("root=/dev/nfs")
            || flag.starts_with("root=iscsi:")
            || *flag == "rd.neednet=1"
    }) {
        return Ok(Some("network boot parameters are configured".into()));
    }
    for dir in ["etc/iscsi/nodes", "sys/class/iscsi_session"] {
        let p = root.join(dir);
        if p.exists() {
            if let Ok(mut entries) = fs::read_dir(&p) {
                if entries.next().is_some() {
                    return Ok(Some("iSCSI nodes or sessions exist".into()));
                }
            }
        }
    }
    let remote_units = runner(&[
        "systemctl",
        "show",
        "remote-fs.target",
        "--property=Requires,Wants",
        "--value",
    ])
    .unwrap_or_default();
    if remote_units
        .split_whitespace()
        .any(|u| u.ends_with(".mount") || u.ends_with(".automount"))
    {
        return Ok(Some("systemd remote mounts are configured".into()));
    }
    let consumers_str = runner(&[
        "systemctl",
        "show",
        "network-online.target",
        "--property=WantedBy,RequiredBy",
        "--value",
    ])
    .unwrap_or_default();
    let consumers: std::collections::HashSet<&str> = consumers_str.split_whitespace().collect();
    let known: std::collections::HashSet<&str> = [
        "cloud-config.service",
        "cloud-final.service",
        "cloud-init-network.service",
        "cloud-init.service",
        "cloud-init.target",
        "cloud-config.target",
        "multi-user.target",
        "graphical.target",
        "remote-fs.target",
        "fwupd-refresh.service",
        "apt-daily.service",
        "apt-daily-upgrade.service",
        "upki-fetch.service",
        "open-iscsi.service",
        "iscsid.service",
    ]
    .into_iter()
    .collect();
    let mut unknown: Vec<&str> = consumers.difference(&known).copied().collect();
    if !unknown.is_empty() {
        unknown.sort();
        return Ok(Some(format!(
            "unclassified online consumers: {}",
            unknown.join(", ")
        )));
    }
    Ok(None)
}

pub fn configure_network_wait(
    root: &Path,
    runner: &dyn Fn(&[&str]) -> io::Result<String>,
) -> io::Result<()> {
    let dropin = root.join(
        "etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica-local-boot.conf",
    );
    let reason = network_wait_reason(root, runner).unwrap_or_else(|_| {
        Some("network requirements could not be verified".into())
    });
    if let Some(r) = reason {
        if let Ok(meta) = symlink_metadata(&dropin) {
            if meta.file_type().is_symlink() {
                return Err(io::Error::other(format!(
                    "Refusing symlink boot configuration: {}",
                    dropin.display()
                )));
            }
        }
        if dropin.exists() {
            fs::remove_file(&dropin)?;
            if let Some(parent) = dropin.parent() {
                let _ = sync_directory(parent);
            }
        }
        println!("Network online wait retained: {r}");
    } else {
        atomic_write(&dropin, LOCAL_BOOT_CONF, 0o644)?;
        println!("Local-only robot: NetworkManager online wait skipped at next boot");
    }
    Ok(())
}

pub fn configure(
    root: &Path,
    runner: &dyn Fn(&[&str]) -> io::Result<String>,
) -> io::Result<()> {
    let target = |rel: &str| root.join(rel.trim_start_matches('/'));

    let release = fs::read_to_string(target("/etc/os-release"))?;
    if !release.lines().any(|l| l.trim() == "ID=ubuntu" || l.trim() == "ID=\"ubuntu\"" || l.trim() == "ID='ubuntu'") {
        return Err(io::Error::other("This profile supports Ubuntu only"));
    }
    let fstab_path = target("/etc/fstab");
    if let Ok(meta) = symlink_metadata(&fstab_path) {
        if meta.file_type().is_symlink() {
            return Err(io::Error::other("Refusing symlink /etc/fstab"));
        }
    }
    let before = fs::read_to_string(&fstab_path)?;
    let (after, network_mount) = fstab_profile(&before)?;
    ensure_directory(&target("/var/lib/w4studica"))?;

    if runner(&["findmnt", "-n", "-o", "FSTYPE", "--target", "/"])? != "ext4" {
        return Err(io::Error::other(
            "Root is not ext4; refusing to change mount options",
        ));
    }
    let device = runner(&["findmnt", "-n", "-o", "SOURCE", "--target", "/"])?;
    let features = runner(&["tune2fs", "-l", &device])?;
    if !features
        .lines()
        .any(|l| l.starts_with("Filesystem features:") && l.contains("has_journal"))
    {
        return Err(io::Error::other(
            "Root ext4 has no journal; enable it offline before using this profile",
        ));
    }
    let mounted_options = runner(&["findmnt", "-n", "-o", "OPTIONS", "--target", "/"])?;
    if mounted_options
        .split(',')
        .any(|o| o == "data=writeback" || o == "data=journal")
    {
        return Err(io::Error::other(
            "Root journal data mode cannot be changed by remount; configure it offline before installation",
        ));
    }
    let boot_options_str = fs::read_to_string(target("/proc/cmdline")).unwrap_or_default();
    let boot_options: Vec<&str> = boot_options_str.split_whitespace().collect();
    if boot_options
        .iter()
        .any(|v| ["systemd.zram=0", "systemd.zram=no", "systemd.zram=false", "systemd.zram=off"].contains(v))
    {
        return Err(io::Error::other(
            "Kernel command line disables zram; remove systemd.zram=0 before installation",
        ));
    }
    for option in &boot_options {
        if let Some(flags) = option.strip_prefix("rootflags=") {
            if flags
                .split(',')
                .any(|f| ["noload", "norecovery", "data=writeback", "data=journal"].contains(&f))
            {
                return Err(io::Error::other(
                    "Conflicting rootflags in kernel command line; fix them before changing ext4 mount policy",
                ));
            }
        }
    }
    let var_lib_fstype = runner(&[
        "findmnt",
        "-n",
        "-o",
        "FSTYPE",
        "--target",
        target("/var/lib").to_str().unwrap(),
    ])?;
    if !["ext4", "xfs"].contains(&var_lib_fstype.as_str()) {
        return Err(io::Error::other("Swap directory must be on ext4 or XFS"));
    }

    let backup = target("/var/lib/w4studica/fstab.before-storage-profile");
    if !backup.exists() {
        atomic_write(&backup, &before, 0o600)?;
    }
    prepare_swap(&target(SWAP_PATH), runner, SWAP_SIZE)?;

    for (conf, dest) in [
        (ZRAM_CONF, "/etc/systemd/zram-generator.conf.d/99-w4studica.conf"),
        (JOURNALD_CONF, "/etc/systemd/journald.conf.d/99-w4studica.conf"),
        (MEMORY_CONF, "/etc/sysctl.d/99-w4studica-memory.conf"),
    ] {
        atomic_write(&target(dest), conf, 0o644)?;
    }

    let wait = target("/etc/systemd/system/NetworkManager-wait-online.service.d/99-w4studica.conf");
    if network_mount {
        if wait.exists() && !symlink_metadata(&wait).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            let _ = fs::remove_file(&wait);
            if let Some(parent) = wait.parent() {
                let _ = sync_directory(parent);
            }
        }
        println!("Network mounts present: keeping the existing network wait policy");
    } else {
        atomic_write(&wait, NETWORK_WAIT_CONF, 0o644)?;
    }

    let mode = fs::metadata(&fstab_path).map(|m| m.permissions().mode()).unwrap_or(0o644);
    atomic_write(&fstab_path, &after, mode)?;

    println!("Prepared: zram 8 GiB priority 100; disk swap 4 GiB priority 10");
    println!("Existing swap files are preserved; old fstab swap entries are disabled at next boot");
    println!("Prepared: volatile journal (64 MiB); ext4 journal/barriers retained; no OverlayFS");
    println!("Reboot required; running swap and root mount were not changed");
    Ok(())
}

pub fn dispatch(args: &[String]) -> Option<io::Result<()>> {
    if args.is_empty() {
        return None;
    }
    if args[0] != "storage" {
        return None;
    }
    let mut root = PathBuf::from("/");
    let action = args.get(1).map(|s| s.as_str());
    let mut iter = args.iter().skip(2);
    while let Some(arg) = iter.next() {
        if arg == "--root" {
            if let Some(r) = iter.next() {
                root = PathBuf::from(r);
            }
        } else if let Some(r) = arg.strip_prefix("--root=") {
            root = PathBuf::from(r);
        }
    }

    let runner = &run_os;
    match action {
        Some("configure") => Some(configure(&root, runner)),
        Some("finish-provisioning") => Some((|| {
            finish_provisioning(&root, runner)?;
            configure_network_wait(&root, runner)
        })()),
        Some("network-wait") => Some(configure_network_wait(&root, runner)),
        _ => Some(Err(io::Error::other(
            "usage: robot_manager storage configure|finish-provisioning|network-wait [--root PATH]",
        ))),
    }
}
