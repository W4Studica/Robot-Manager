//! NetworkManager keyfile generation and radio/channel validation.
use super::{AP_NAME, CLIENT_NAME, ETH_NAME};
use std::io;

pub fn team_address(team: &str) -> io::Result<String> {
    let number: u16 = team
        .parse()
        .map_err(|_| io::Error::other("the team number must be 1-9999"))?;
    if !(1..=9999).contains(&number) {
        return Err(io::Error::other("the team number must be 1-9999"));
    }
    Ok(format!("10.{}.{}.2", number / 100, number % 100))
}
pub fn band(band: &str) -> io::Result<&'static str> {
    match band
        .trim()
        .to_lowercase()
        .replace("ghz", "")
        .replace('g', "")
        .as_str()
    {
        "2.4" | "2" => Ok("bg"),
        "5" => Ok("a"),
        _ => Err(io::Error::other("the band must be 2.4 or 5")),
    }
}
pub fn password(password: &str) -> io::Result<()> {
    if !(8..=63).contains(&password.len()) || password.contains('\0') {
        return Err(io::Error::other("a WPA2 password has 8 to 63 bytes"));
    }
    Ok(())
}
pub fn ssid(ssid: &str) -> io::Result<()> {
    if ssid.is_empty() || ssid.len() > 32 || ssid.contains('\0') {
        return Err(io::Error::other("an SSID has 1 to 32 bytes"));
    }
    Ok(())
}
pub fn channels(band: &str, allowed: &[u16]) -> Vec<u16> {
    const STANDARD: &[u16] = &[
        36, 40, 44, 48, 52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140, 144,
        149, 153, 157, 161, 165,
    ];
    if band == "bg" {
        return (1..=11).collect();
    }
    let mut result: Vec<_> = allowed
        .iter()
        .copied()
        .filter(|c| STANDARD.contains(c))
        .collect();
    result.sort_unstable();
    result.dedup();
    result
}
pub fn default_channel(channels: &[u16]) -> Option<u16> {
    [6, 36, 149]
        .into_iter()
        .find(|c| channels.contains(c))
        .or_else(|| channels.first().copied())
}
pub fn parse_channels(text: &str) -> Vec<u16> {
    let re = regex::Regex::new(r"^\s*\*\s*(\d+)(?:\.\d+)?\s*MHz\s*\[(\d+)\](.*)").unwrap();
    text.lines()
        .filter_map(|line| {
            let captures = re.captures(line)?;
            if captures[1].parse::<u32>().ok()? < 5000
                || ["disabled", "radar detection", "no IR"]
                    .iter()
                    .any(|flag| captures[3].contains(flag))
            {
                return None;
            }
            captures[2].parse().ok()
        })
        .collect()
}
// GLib keyfile string escaping prevents SSID/password text from creating new keys.
fn escape(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '\\' => "\\\\".into(),
            '\n' => "\\n".into(),
            '\r' => "\\r".into(),
            '\t' => "\\t".into(),
            ' ' => "\\s".into(),
            _ => c.to_string(),
        })
        .collect()
}
fn section(text: &mut String, name: &str, values: &[(&str, &str)]) {
    text.push_str(&format!("[{name}]\n"));
    for (key, value) in values {
        text.push_str(&format!("{key}={}\n", escape(value)));
    }
    text.push('\n');
}
fn connection(name: &str, kind: &str, interface: &str, uuid: &str, priority: bool) -> String {
    let mut text = String::new();
    section(
        &mut text,
        "connection",
        &[
            ("id", name),
            ("uuid", uuid),
            ("type", kind),
            ("interface-name", interface),
            ("autoconnect", "true"),
            ("autoconnect-retries", "0"),
        ],
    );
    if priority {
        text.push_str("autoconnect-priority=100\n\n");
    }
    text
}
pub struct AccessPoint<'a> {
    pub ssid: &'a str,
    pub address: &'a str,
    pub band: &'a str,
    pub channel: u16,
    pub password: &'a str,
    pub interface: &'a str,
    pub uuid: &'a str,
}
pub fn access_point(ap: &AccessPoint<'_>) -> String {
    let mut text = connection(AP_NAME, "wifi", ap.interface, ap.uuid, true);
    section(
        &mut text,
        "wifi",
        &[
            ("mode", "ap"),
            ("ssid", ap.ssid),
            ("band", ap.band),
            ("channel", &ap.channel.to_string()),
            ("powersave", "2"),
        ],
    );
    section(
        &mut text,
        "wifi-security",
        &[("key-mgmt", "wpa-psk"), ("psk", ap.password)],
    );
    section(
        &mut text,
        "ipv4",
        &[
            ("method", "shared"),
            ("address1", &format!("{}/24", ap.address)),
        ],
    );
    section(&mut text, "ipv6", &[("method", "ignore")]);
    text
}
pub fn ethernet(interface: &str, uuid: &str) -> String {
    let mut text = connection(ETH_NAME, "ethernet", interface, uuid, true);
    section(
        &mut text,
        "ipv4",
        &[("method", "shared"), ("address1", "172.22.11.2/24")],
    );
    section(&mut text, "ipv6", &[("method", "ignore")]);
    text
}
pub fn client(ssid: &str, password: Option<&str>, interface: &str, uuid: &str) -> String {
    let mut text = connection(CLIENT_NAME, "wifi", interface, uuid, false);
    section(
        &mut text,
        "wifi",
        &[
            ("mode", "infrastructure"),
            ("ssid", ssid),
            ("powersave", "2"),
        ],
    );
    if let Some(password) = password.filter(|p| !p.is_empty()) {
        section(
            &mut text,
            "wifi-security",
            &[("key-mgmt", "wpa-psk"), ("psk", password)],
        );
    }
    section(&mut text, "ipv4", &[("method", "auto")]);
    section(&mut text, "ipv6", &[("method", "auto")]);
    text
}
