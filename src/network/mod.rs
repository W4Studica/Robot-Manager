//! VMX networking commands. The supervisor and CLI share one installed binary.
pub mod backup;
pub mod cli;
pub mod profiles;
pub mod system;

use std::io;
use std::path::PathBuf;
use system::{read_uuid, uuid, write_secret, System};

pub const AP_NAME: &str = "robot-ap";
pub const CLIENT_NAME: &str = "robot-client";
pub const ETH_NAME: &str = "robot-eth";
pub const NM_DIR: &str = "/etc/NetworkManager/system-connections";
pub const DEFER_SECONDS: u32 = 3;

pub fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|s| (*s).into()).collect()
}
fn run(system: &mut impl System, values: &[&str], check: bool) -> io::Result<String> {
    system.run(&args(values), check)
}
fn require_network(system: &mut impl System, apply: bool) -> io::Result<()> {
    if !system.is_root() {
        return Err(io::Error::other("run it with sudo"));
    }
    if apply
        && run(system, &["systemctl", "is-active", "NetworkManager"], false)?.trim() != "active"
    {
        return Err(io::Error::other(
            "NetworkManager is not running; install.sh configures it automatically",
        ));
    }
    Ok(())
}
pub fn interface(system: &mut impl System, wifi: bool) -> String {
    let kind = if wifi { "wifi" } else { "ethernet" };
    if let Ok(text) = run(
        system,
        &["nmcli", "-t", "-f", "DEVICE,TYPE", "device"],
        false,
    ) {
        for line in text.lines() {
            if let Some((device, ty)) = line.split_once(':') {
                if ty == kind && !device.is_empty() {
                    return device.into();
                }
            }
        }
    }
    for name in system.interfaces() {
        if (wifi && system.is_wireless(&name))
            || (!wifi
                && !system.is_wireless(&name)
                && (name.starts_with("eth") || name.starts_with("en")))
        {
            return name;
        }
    }
    if wifi {
        "wlan0".into()
    } else {
        "eth0".into()
    }
}
pub fn ssh_device(system: &mut impl System) -> io::Result<String> {
    let Some(connection) = system.environ("SSH_CONNECTION") else {
        return Ok(String::new());
    };
    let Some(client) = connection.split_whitespace().next() else {
        return Ok(String::new());
    };
    let route = run(system, &["ip", "-o", "route", "get", client], false)?;
    let mut words = route.split_whitespace();
    while let Some(word) = words.next() {
        if word == "dev" {
            return Ok(words.next().unwrap_or("").into());
        }
    }
    Ok(String::new())
}
#[derive(Clone, Debug)]
pub struct Step {
    pub args: Vec<String>,
    pub check: bool,
}
impl Step {
    fn new(values: &[&str], check: bool) -> Self {
        Self {
            args: args(values),
            check,
        }
    }
}
fn quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}
fn shell(steps: &[Step]) -> String {
    steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let command = step
                .args
                .iter()
                .map(|value| quote(value))
                .collect::<Vec<_>>()
                .join(" ");
            // Mandatory preparation failures must stop a deferred job just as they
            // stop immediate execution. Only activation failure triggers fallback.
            if step.check && index + 1 < steps.len() {
                format!("{command} || exit $?")
            } else {
                command
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}
pub fn activate(
    system: &mut impl System,
    interface: &str,
    steps: &[Step],
    fallback: &[Step],
) -> io::Result<bool> {
    if ssh_device(system)? == interface {
        let mut script = shell(steps);
        if !fallback.is_empty() {
            script.push_str(&format!(" || {{ {}; }}", shell(fallback)));
        }
        let unit = format!("--unit=w4studica-net-{}", uuid()?);
        run(
            system,
            &[
                "systemd-run",
                "--collect",
                &unit,
                &format!("--on-active={DEFER_SECONDS}"),
                "--description=W4Studica: apply the network change",
                "/bin/sh",
                "-c",
                &script,
            ],
            true,
        )?;
        return Ok(true);
    }
    for (index, step) in steps.iter().enumerate() {
        let last = index + 1 == steps.len();
        if let Err(error) = system.run(&step.args, step.check || (last && !fallback.is_empty())) {
            if !last || fallback.is_empty() {
                return Err(error);
            }
            for step in fallback {
                let _ = system.run(&step.args, false);
            }
            return Err(io::Error::other(format!(
                "{error}; fallback to 2.4 GHz channel 6 was attempted"
            )));
        }
    }
    Ok(false)
}
fn reload(system: &mut impl System, name: &str, uuid: &str) -> io::Result<Vec<Step>> {
    let mut steps = vec![Step::new(&["nmcli", "connection", "reload"], true)];
    for line in run(
        system,
        &["nmcli", "-t", "-f", "NAME,UUID", "connection", "show"],
        false,
    )?
    .lines()
    {
        if let Some((profile, other)) = line.rsplit_once(':') {
            if profile == name && other != uuid {
                steps.push(Step::new(
                    &["nmcli", "connection", "delete", "uuid", other],
                    false,
                ));
            }
        }
    }
    steps.push(Step::new(
        &["nmcli", "connection", "up", "uuid", uuid],
        true,
    ));
    Ok(steps)
}
pub struct Network {
    pub directory: PathBuf,
}
impl Default for Network {
    fn default() -> Self {
        Self {
            directory: std::env::var_os("VMX_NM_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| NM_DIR.into()),
        }
    }
}
#[derive(Debug)]
pub struct HotspotInfo {
    pub interface: String,
    pub address: String,
    pub ssid: String,
    pub band: String,
    pub channel: u16,
    pub deferred: bool,
}
#[derive(Debug)]
pub struct EthernetInfo {
    pub interface: String,
    pub address: &'static str,
    pub deferred: bool,
}
pub struct Hotspot<'a> {
    pub prefix: &'a str,
    pub team: &'a str,
    pub band: &'a str,
    pub password: &'a str,
    pub channel: Option<u16>,
    pub apply: bool,
}
impl Network {
    fn path(&self, name: &str) -> PathBuf {
        self.directory.join(format!("{name}.nmconnection"))
    }
    pub fn hotspot(
        &self,
        system: &mut impl System,
        options: &Hotspot<'_>,
    ) -> io::Result<HotspotInfo> {
        require_network(system, options.apply)?;
        let address = profiles::team_address(options.team)?;
        let band = profiles::band(options.band)?;
        profiles::password(options.password)?;
        let ssid = format!(
            "{}-{}",
            options.prefix,
            options.team.parse::<u16>().unwrap()
        );
        profiles::ssid(&ssid)?;
        let interface = interface(system, true);
        validate_interface(&interface)?;
        let allowed = if band == "a" {
            profiles::parse_channels(&run(system, &["iw", "phy"], false)?)
        } else {
            vec![]
        };
        let channels = profiles::channels(band, &allowed);
        let default=profiles::default_channel(&channels).ok_or_else(|| io::Error::other("no usable 5 GHz channel here (set the country first: sudo iw reg set KR); use band 2.4"))?;
        let channel = match options.channel {
            Some(channel) => channel,
            None if system.interactive() => {
                let answer = system.ask(&format!(
                    "channel {} [{default}]: ",
                    channels
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join("/")
                ))?;
                if answer.trim().is_empty() {
                    default
                } else {
                    answer
                        .trim()
                        .parse()
                        .map_err(|_| io::Error::other("channel must be a number"))?
                }
            }
            None => default,
        };
        if !channels.contains(&channel) {
            return Err(io::Error::other(format!(
                "channel {channel} is not valid here; valid: {channels:?}"
            )));
        }
        let path = self.path(AP_NAME);
        let uuid = read_uuid(&path).map(Ok).unwrap_or_else(uuid)?;
        write_secret(
            &path,
            &profiles::access_point(&profiles::AccessPoint {
                ssid: &ssid,
                address: &address,
                band,
                channel,
                password: options.password,
                interface: &interface,
                uuid: &uuid,
            }),
        )?;
        let deferred = if options.apply {
            let steps = reload(system, AP_NAME, &uuid)?;
            let fallback = if band == "a" {
                vec![
                    Step::new(
                        &[
                            "nmcli",
                            "connection",
                            "modify",
                            "uuid",
                            &uuid,
                            "802-11-wireless.band",
                            "bg",
                            "802-11-wireless.channel",
                            "6",
                        ],
                        false,
                    ),
                    Step::new(&["nmcli", "connection", "up", "uuid", &uuid], false),
                ]
            } else {
                vec![]
            };
            activate(system, &interface, &steps, &fallback)?
        } else {
            false
        };
        Ok(HotspotInfo {
            interface,
            address,
            ssid,
            band: band.into(),
            channel,
            deferred,
        })
    }
    pub fn ethernet(
        &self,
        system: &mut impl System,
        device: Option<&str>,
        apply: bool,
    ) -> io::Result<EthernetInfo> {
        require_network(system, apply)?;
        let interface = device
            .map(str::to_owned)
            .unwrap_or_else(|| interface(system, false));
        validate_interface(&interface)?;
        let path = self.path(ETH_NAME);
        let uuid = read_uuid(&path).map(Ok).unwrap_or_else(uuid)?;
        write_secret(&path, &profiles::ethernet(&interface, &uuid))?;
        let deferred = if apply {
            let steps = reload(system, ETH_NAME, &uuid)?;
            activate(system, &interface, &steps, &[])?
        } else {
            false
        };
        Ok(EthernetInfo {
            interface,
            address: "172.22.11.2/24",
            deferred,
        })
    }
    pub fn toggle(
        &self,
        system: &mut impl System,
        mode: &str,
        ssid: Option<&str>,
        password: Option<&str>,
    ) -> io::Result<bool> {
        if !matches!(mode, "ap" | "wifi") {
            return Err(io::Error::other("the mode is ap or wifi"));
        }
        require_network(system, true)?;
        let interface = interface(system, true);
        validate_interface(&interface)?;
        let path = self.path(AP_NAME);
        let target = if mode == "ap" {
            if !path.exists() {
                return Err(io::Error::other(
                    "no access point is set up yet: sudo setupHotspot VMX 1234 2.4 password",
                ));
            }
            run(
                system,
                &[
                    "nmcli",
                    "connection",
                    "modify",
                    AP_NAME,
                    "connection.autoconnect",
                    "yes",
                ],
                true,
            )?;
            for name in wifi_connections(system)? {
                if name != AP_NAME {
                    run(
                        system,
                        &[
                            "nmcli",
                            "connection",
                            "modify",
                            &name,
                            "connection.autoconnect",
                            "no",
                        ],
                        false,
                    )?;
                }
            }
            AP_NAME.into()
        } else {
            let target = if let Some(ssid) = ssid.filter(|s| !s.is_empty()) {
                profiles::ssid(ssid)?;
                if let Some(password) = password.filter(|p| !p.is_empty()) {
                    profiles::password(password)?;
                }
                let client_path = self.path(CLIENT_NAME);
                let uuid = read_uuid(&client_path).map(Ok).unwrap_or_else(uuid)?;
                write_secret(
                    &client_path,
                    &profiles::client(ssid, password, &interface, &uuid),
                )?;
                run(system, &["nmcli", "connection", "reload"], true)?;
                CLIENT_NAME.into()
            } else {
                wifi_connections(system)?
                    .into_iter()
                    .find(|n| n != AP_NAME)
                    .ok_or_else(|| {
                        io::Error::other("no router is saved: toggleNetMode wifi SSID PASSWORD")
                    })?
            };
            if path.exists() {
                run(
                    system,
                    &[
                        "nmcli",
                        "connection",
                        "modify",
                        AP_NAME,
                        "connection.autoconnect",
                        "no",
                    ],
                    false,
                )?;
            }
            run(
                system,
                &[
                    "nmcli",
                    "connection",
                    "modify",
                    &target,
                    "connection.autoconnect",
                    "yes",
                ],
                false,
            )?;
            target
        };
        activate(
            system,
            &interface,
            &[Step::new(&["nmcli", "connection", "up", &target], true)],
            &[],
        )
    }
}
fn validate_interface(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name.len() >= libc::IFNAMSIZ
        || name
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '/' | ':' | '\0'))
    {
        return Err(io::Error::other("invalid network interface name"));
    }
    Ok(())
}
fn wifi_connections(system: &mut impl System) -> io::Result<Vec<String>> {
    Ok(run(
        system,
        &[
            "nmcli",
            "-t",
            "--escape",
            "no",
            "-f",
            "NAME,TYPE",
            "connection",
            "show",
        ],
        false,
    )?
    .lines()
    .filter_map(|line| {
        let (name, kind) = line.rsplit_once(':')?;
        matches!(kind, "wifi" | "802-11-wireless").then(|| name.into())
    })
    .collect())
}
pub fn status(system: &mut impl System) -> io::Result<String> {
    run(
        system,
        &[
            "nmcli",
            "-t",
            "-f",
            "NAME,TYPE,DEVICE,STATE",
            "connection",
            "show",
        ],
        false,
    )
}
