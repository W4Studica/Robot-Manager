//! VMX OS, SDK and deployed runtime inventory report generator.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Serialize, Deserialize)]
pub struct Fingerprint {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageInfo {
    pub id: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OsInfo {
    pub name: Option<String>,
    pub kernel: String,
    pub architecture: String,
    pub libc: Vec<String>,
    pub python: Option<String>,
    pub java: Option<String>,
    pub compiler: Option<String>,
    pub gpio_sysfs: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryRecord {
    pub schema_version: u32,
    pub captured_at: String,
    pub validation: String,
    pub image: ImageInfo,
    pub release: String,
    pub os: OsInfo,
    pub files: BTreeMap<String, Fingerprint>,
    pub missing_required: Vec<String>,
}

pub struct Sha256 {
    state: [u32; 8],
    count: u64,
    buffer: [u8; 64],
}

impl Default for Sha256 {
    fn default() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            count: 0,
            buffer: [0u8; 64],
        }
    }
}

impl Sha256 {
    pub fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let index = (self.count & 63) as usize;
            let to_fill = 64 - index;
            if data.len() >= to_fill {
                self.buffer[index..64].copy_from_slice(&data[..to_fill]);
                self.count += to_fill as u64;
                let block = self.buffer;
                self.transform(&block);
                data = &data[to_fill..];
            } else {
                self.buffer[index..index + data.len()].copy_from_slice(data);
                self.count += data.len() as u64;
                break;
            }
        }
    }

    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.count * 8;
        let mut index = (self.count & 63) as usize;
        self.buffer[index] = 0x80;
        index += 1;

        if index > 56 {
            self.buffer[index..64].fill(0);
            let block = self.buffer;
            self.transform(&block);
            self.buffer[..56].fill(0);
        } else {
            self.buffer[index..56].fill(0);
        }

        self.buffer[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let block = self.buffer;
        self.transform(&block);

        let mut out = [0u8; 32];
        for (i, val) in self.state.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&val.to_be_bytes());
        }
        out
    }

    fn transform(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

pub fn fingerprint(path: &Path) -> io::Result<Fingerprint> {
    let mut file = File::open(path)?;
    let meta = file.metadata()?;
    let mut hasher = Sha256::default();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hash_bytes = hasher.finalize();
    let hex = hash_bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();
    Ok(Fingerprint {
        bytes: meta.len(),
        sha256: hex,
    })
}

fn command_version(args: &[&str]) -> Option<String> {
    Command::new(args[0])
        .args(&args[1..])
        .output()
        .ok()
        .and_then(|output| {
            let text = if !output.stdout.is_empty() {
                String::from_utf8_lossy(&output.stdout)
            } else {
                String::from_utf8_lossy(&output.stderr)
            };
            text.lines().next().map(|l| l.trim().to_string())
        })
}

fn collect_headers(dir: &Path, paths: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                collect_headers(&p, paths);
            } else if p.extension().map_or(false, |ext| ext == "h") {
                paths.push(p);
            }
        }
    }
}

pub fn collect(
    sdk: &Path,
    runtime: &Path,
    image_id: &str,
    image_sha256: Option<&str>,
    release: &str,
) -> io::Result<InventoryRecord> {
    let headers_dir = sdk.join("usr/local/include/vmxpi");
    let hal = sdk.join("usr/local/lib/vmxpi/libvmxpi_hal_cpp.so");

    let required_groups = vec![
        vec![headers_dir.join("VMXPi.h")],
        vec![hal.clone()],
        vec![runtime.join("libstudica_hardware.so")],
        vec![runtime.join("libstudica.so")],
        vec![
            runtime.join("libhalsim_studica.so"),
            runtime.join("libhalsim_studicad.so"),
        ],
        vec![
            runtime.join("libwpiHal.so"),
            runtime.join("libwpiHald.so"),
        ],
    ];

    let mut paths = Vec::new();
    collect_headers(&headers_dir, &mut paths);
    paths.push(hal);
    if let Ok(entries) = fs::read_dir(runtime) {
        for entry in entries.flatten() {
            let p = entry.path();
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.contains(".so") {
                    paths.push(p);
                }
            }
        }
    }

    let mut files = BTreeMap::new();
    for path in paths {
        if path.is_file() {
            if let Ok(fp) = fingerprint(&path) {
                files.insert(path.to_string_lossy().to_string(), fp);
            }
        }
    }

    let mut missing_required = Vec::new();
    for group in required_groups {
        if !group.iter().any(|p| p.is_file()) {
            let desc = group
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(" or ");
            missing_required.push(desc);
        }
    }

    let pretty_name = fs::read_to_string("/etc/os-release").ok().and_then(|text| {
        for line in text.lines() {
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                return Some(val.trim_matches('"').trim_matches('\'').to_string());
            }
        }
        None
    });

    let kernel = command_version(&["uname", "-r"]).unwrap_or_else(|| "unknown".into());
    let architecture = command_version(&["uname", "-m"]).unwrap_or_else(|| "unknown".into());

    let libc_ver = {
        let version_str = unsafe {
            let ptr = libc::gnu_get_libc_version();
            std::ffi::CStr::from_ptr(ptr).to_string_lossy().to_string()
        };
        vec!["glibc".into(), version_str]
    };

    let python = command_version(&["python3", "--version"])
        .map(|v| v.strip_prefix("Python ").unwrap_or(&v).to_string());
    let java = command_version(&["java", "-version"]);
    let compiler = command_version(&["g++", "--version"]);
    let gpio_sysfs = Path::new("/sys/class/gpio").exists();

    use std::time::SystemTime;
    let now = SystemTime::now();
    let dt = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Simple ISO 8601 UTC timestamp format: YYYY-MM-DDTHH:MM:SSZ
    let secs_per_day = 86400;
    let days = (dt / secs_per_day) as i64;
    let rem_secs = dt % secs_per_day;
    let hours = rem_secs / 3600;
    let mins = (rem_secs % 3600) / 60;
    let secs = rem_secs % 60;

    // Days to year-month-day (civil)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 142600) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let captured_at = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}+00:00",
        y, m, d, hours, mins, secs
    );

    Ok(InventoryRecord {
        schema_version: 1,
        captured_at,
        validation: "inventory-only; hardware acceptance must be recorded separately".into(),
        image: ImageInfo {
            id: image_id.to_string(),
            sha256: image_sha256.map(|s| s.to_string()),
        },
        release: release.to_string(),
        os: OsInfo {
            name: pretty_name,
            kernel,
            architecture,
            libc: libc_ver,
            python,
            java,
            compiler,
            gpio_sysfs,
        },
        files,
        missing_required,
    })
}

pub fn dispatch(args: &[String]) -> Option<io::Result<i32>> {
    if args.is_empty() || args[0] != "inventory" {
        return None;
    }
    let mut sdk = PathBuf::from("/");
    let mut runtime_dir: Option<PathBuf> = None;
    let mut image_id: Option<String> = None;
    let mut image_sha256: Option<String> = None;
    let mut release: Option<String> = None;
    let mut output_path: Option<PathBuf> = None;

    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--sdk" => sdk = PathBuf::from(iter.next().unwrap_or(&"".into())),
            "--runtime-dir" => runtime_dir = Some(PathBuf::from(iter.next().unwrap_or(&"".into()))),
            "--image-id" => image_id = Some(iter.next().unwrap_or(&"".into()).clone()),
            "--image-sha256" => image_sha256 = Some(iter.next().unwrap_or(&"".into()).clone()),
            "--release" => release = Some(iter.next().unwrap_or(&"".into()).clone()),
            "--output" => output_path = Some(PathBuf::from(iter.next().unwrap_or(&"".into()))),
            _ if arg.starts_with("--sdk=") => sdk = PathBuf::from(&arg[6..]),
            _ if arg.starts_with("--runtime-dir=") => runtime_dir = Some(PathBuf::from(&arg[14..])),
            _ if arg.starts_with("--image-id=") => image_id = Some(arg[11..].to_string()),
            _ if arg.starts_with("--image-sha256=") => image_sha256 = Some(arg[15..].to_string()),
            _ if arg.starts_with("--release=") => release = Some(arg[10..].to_string()),
            _ if arg.starts_with("--output=") => output_path = Some(PathBuf::from(&arg[9..])),
            _ => {
                return Some(Err(io::Error::other(format!(
                    "unknown inventory argument: {arg}"
                ))))
            }
        }
    }

    let Some(runtime) = runtime_dir else {
        return Some(Err(io::Error::other("--runtime-dir is required")));
    };
    let Some(id) = image_id else {
        return Some(Err(io::Error::other("--image-id is required")));
    };
    let Some(rel) = release else {
        return Some(Err(io::Error::other("--release is required")));
    };

    if let Some(hash) = &image_sha256 {
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(Err(io::Error::other(
                "--image-sha256 must contain 64 hexadecimal characters",
            )));
        }
    }

    let record = match collect(&sdk, &runtime, &id, image_sha256.as_deref(), &rel) {
        Ok(rec) => rec,
        Err(err) => return Some(Err(err)),
    };

    let has_missing = !record.missing_required.is_empty();
    let json_text = match serde_json::to_string_pretty(&record) {
        Ok(t) => t + "\n",
        Err(err) => return Some(Err(io::Error::other(err))),
    };

    if let Some(out) = output_path {
        if let Err(err) = fs::write(out, json_text) {
            return Some(Err(err));
        }
    } else {
        print!("{json_text}");
    }

    Some(Ok(if has_missing { 1 } else { 0 }))
}
