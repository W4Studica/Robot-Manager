use robot_manager::network::system::System;
use std::io;

pub const IW: &str = "* 2412.0 MHz [1] (20 dBm)\n* 5170.0 MHz [34] (20 dBm)\n* 5180.0 MHz [36] (20 dBm)\n* 5200.0 MHz [40] (20 dBm)\n* 5260.0 MHz [52] (no IR, radar detection)\n* 5500.0 MHz [100] (radar detection)\n* 5700.0 MHz [140] (disabled)\n* 5745.0 MHz [149] (20 dBm)\n";
pub struct Fake {
    pub calls: Vec<Vec<String>>,
    pub checked: Vec<bool>,
    pub root: bool,
    pub active: bool,
    pub missing_nmcli: bool,
    pub ssh: Option<String>,
    pub route: String,
    pub wifi: Vec<String>,
    pub profiles: Vec<String>,
    pub tty: bool,
    pub answer: String,
    pub fail_up: bool,
    pub iw: String,
    pub manager: String,
    pub enabled: String,
}
impl Default for Fake {
    fn default() -> Self {
        Self {
            calls: vec![],
            checked: vec![],
            root: true,
            active: true,
            missing_nmcli: false,
            ssh: None,
            route: "eth0".into(),
            wifi: vec![],
            profiles: vec![],
            tty: false,
            answer: String::new(),
            fail_up: false,
            iw: IW.into(),
            manager: "NetworkManager.service".into(),
            enabled: "enabled".into(),
        }
    }
}
impl Fake {
    pub fn ran(&self, args: &[&str]) -> bool {
        self.calls.iter().any(|c| {
            c.iter()
                .map(String::as_str)
                .take(args.len())
                .eq(args.iter().copied())
        })
    }
    pub fn ssh(device: &str) -> Self {
        Self {
            ssh: Some("192.168.0.220 5555 192.168.0.222 22".into()),
            route: device.into(),
            ..Self::default()
        }
    }
}
impl System for Fake {
    fn run(&mut self, args: &[String], check: bool) -> io::Result<String> {
        self.calls.push(args.to_vec());
        self.checked.push(check);
        if args[0] == "nmcli" && self.missing_nmcli {
            return Err(io::ErrorKind::NotFound.into());
        }
        if args[0] == "systemctl" && args[1] == "is-active" {
            return Ok(
                if self.active && (args[2] == "NetworkManager" || args[2] == self.manager) {
                    "active\n"
                } else {
                    "inactive\n"
                }
                .into(),
            );
        }
        if args[0] == "systemctl" && args[1] == "is-enabled" {
            return Ok(if args[2] == self.manager {
                self.enabled.clone()
            } else {
                "disabled".into()
            });
        }
        if args[0] == "nmcli" {
            if args.iter().any(|v| v == "DEVICE,TYPE") {
                return Ok("lo:loopback\neth0:ethernet\nwlan0:wifi\n".into());
            }
            if args.iter().any(|v| v == "NAME,UUID") {
                return Ok(self.profiles.join("\n"));
            }
            if args.iter().any(|v| v == "NAME,TYPE") {
                return Ok(self
                    .wifi
                    .iter()
                    .map(|n| format!("{n}:802-11-wireless\n"))
                    .collect());
            }
            if args[1..].starts_with(&["connection".into(), "up".into()]) && self.fail_up {
                return Err(io::Error::other("activation refused"));
            }
        }
        if args[0] == "ip" {
            return Ok(format!(
                "192.168.0.220 dev {} src 192.168.0.222 uid 0\n",
                self.route
            ));
        }
        if args[0] == "iw" {
            return Ok(self.iw.clone());
        }
        Ok(String::new())
    }
    fn environ(&self, name: &str) -> Option<String> {
        if name == "SSH_CONNECTION" {
            self.ssh.clone()
        } else {
            None
        }
    }
    fn interfaces(&self) -> Vec<String> {
        vec!["eth0".into(), "lo".into(), "wlan0".into()]
    }
    fn is_wireless(&self, name: &str) -> bool {
        name == "wlan0"
    }
    fn is_root(&self) -> bool {
        self.root
    }
    fn interactive(&self) -> bool {
        self.tty
    }
    fn ask(&mut self, _: &str) -> io::Result<String> {
        Ok(self.answer.clone())
    }
}
