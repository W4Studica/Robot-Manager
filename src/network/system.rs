//! Linux access is isolated here so network tests never touch the host network.
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub trait System {
    fn run(&mut self, args: &[String], check: bool) -> io::Result<String>;
    fn environ(&self, name: &str) -> Option<String> {
        parent_environment(name)
    }
    fn interfaces(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir("/sys/class/net")
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
    fn is_wireless(&self, name: &str) -> bool {
        Path::new("/sys/class/net")
            .join(name)
            .join("wireless")
            .is_dir()
    }
    fn is_root(&self) -> bool {
        unsafe { libc::geteuid() == 0 }
    }
    fn interactive(&self) -> bool {
        io::stdin().is_terminal()
    }
    fn ask(&mut self, prompt: &str) -> io::Result<String> {
        print!("{prompt}");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        Ok(answer)
    }
}

pub struct Linux;
impl System for Linux {
    fn run(&mut self, args: &[String], check: bool) -> io::Result<String> {
        let (program, arguments) = args
            .split_first()
            .ok_or_else(|| io::Error::other("empty command"))?;
        let mut child = Command::new(program)
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        thread::scope(|scope| {
            let out = scope.spawn(move || {
                let mut bytes = Vec::new();
                stdout.read_to_end(&mut bytes).map(|_| bytes)
            });
            let err = scope.spawn(move || {
                let mut bytes = Vec::new();
                stderr.read_to_end(&mut bytes).map(|_| bytes)
            });
            let start = Instant::now();
            let status = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) if start.elapsed() < Duration::from_secs(60) => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Ok(None) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("{program} timed out"),
                        ));
                    }
                    Err(error) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(error);
                    }
                }
            };
            let output = out
                .join()
                .map_err(|_| io::Error::other("command output reader failed"))??;
            let errors = err
                .join()
                .map_err(|_| io::Error::other("command error reader failed"))??;
            let status = status?;
            if check && !status.success() {
                let detail = if errors.is_empty() { &output } else { &errors };
                return Err(io::Error::other(format!(
                    "{} failed: {}",
                    args.iter().take(3).cloned().collect::<Vec<_>>().join(" "),
                    String::from_utf8_lossy(detail).trim()
                )));
            }
            Ok(String::from_utf8_lossy(&output).into_owned())
        })
    }
}

/// sudo can remove SSH_CONNECTION. Dropbear's parent still holds its original environment.
pub fn parent_environment(name: &str) -> Option<String> {
    if let Ok(value) = std::env::var(name) {
        if !value.is_empty() {
            return Some(value);
        }
    }
    let mut pid = unsafe { libc::getppid() };
    let key = format!("{name}=");
    for _ in 0..8 {
        let bytes = fs::read(format!("/proc/{pid}/environ")).ok()?;
        for item in bytes.split(|&b| b == 0) {
            if let Some(value) = item.strip_prefix(key.as_bytes()) {
                if !value.is_empty() {
                    return Some(String::from_utf8_lossy(value).into_owned());
                }
            }
        }
        let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        pid = status
            .lines()
            .find_map(|line| line.strip_prefix("PPid:")?.trim().parse().ok())?;
        if pid <= 1 {
            break;
        }
    }
    None
}

pub fn uuid() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}", bytes[0],bytes[1],bytes[2],bytes[3],bytes[4],bytes[5],bytes[6],bytes[7],bytes[8],bytes[9],bytes[10],bytes[11],bytes[12],bytes[13],bytes[14],bytes[15]))
}

/// Write privately and atomically; an interrupted update leaves the old profile usable.
pub fn write_secret(path: &Path, text: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("missing profile directory"))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".w4studica-{}", uuid()?));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()
    })();
    let _ = fs::remove_file(temp);
    result
}

pub fn read_uuid(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let mut connection = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            connection = line == "[connection]";
        }
        if connection {
            if let Some(id) = line.strip_prefix("uuid=") {
                if !id.is_empty() {
                    return Some(id.into());
                }
            }
        }
    }
    None
}
