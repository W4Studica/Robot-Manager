use super::{backup, Hotspot, Network, System};
use std::io;
use std::path::Path;

pub const HELP: &str = "vmx_net setupHotspot [--channel N] [--no-activate] SSID TEAM BAND [PASSWORD]\nvmx_net setupEthernet [--no-activate] [INTERFACE]\nvmx_net toggleNetMode ap | wifi [SSID [PASSWORD]]\nvmx_net status | sshdevice\nnetwork_backup snapshot | restore\n\nThe same commands are available as robot_manager network COMMAND and\nrobot_manager network-backup ACTION. Legacy --force is accepted.";

/// Leave supervisor arguments as OS strings, including non-UTF-8 config paths.
pub fn dispatch_os(
    args: &[std::ffi::OsString],
    network: &Network,
    system: &mut impl System,
) -> Option<io::Result<String>> {
    let name = Path::new(args.first()?)
        .file_name()
        .and_then(|name| name.to_str());
    let command = args.get(1).and_then(|arg| arg.to_str());
    if !matches!(
        name,
        Some("vmx_net" | "network_backup" | "setupHotspot" | "setupEthernet" | "toggleNetMode")
    ) && !matches!(command, Some("network" | "network-backup"))
    {
        return None;
    }
    let text = args
        .iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_owned)
                .ok_or_else(|| io::Error::other("network arguments must be UTF-8"))
        })
        .collect::<io::Result<Vec<_>>>();
    match text {
        Ok(args) => dispatch(&args[0], &args[1..], network, system),
        Err(error) => Some(Err(error)),
    }
}

/// Select a command by executable name (installed symlinks) or explicit subcommand.
pub fn dispatch(
    program: &str,
    args: &[String],
    network: &Network,
    system: &mut impl System,
) -> Option<io::Result<String>> {
    let name = Path::new(program).file_name()?.to_str()?;
    match name {
        "setupHotspot" | "setupEthernet" | "toggleNetMode" => {
            let mut command = vec![name.into()];
            command.extend_from_slice(args);
            Some(run(&command, network, system))
        }
        "vmx_net" => Some(run(args, network, system)),
        "network_backup" => Some(backup_command(args, system)),
        _ => match args.first().map(String::as_str) {
            Some("network") => Some(run(&args[1..], network, system)),
            Some("network-backup") => Some(backup_command(&args[1..], system)),
            _ => None,
        },
    }
}
fn backup_command(args: &[String], system: &mut impl System) -> io::Result<String> {
    if args.len() == 1 && matches!(args[0].as_str(), "--help" | "-h") {
        return Ok("network_backup snapshot | restore\n".into());
    }
    if args.len() != 1 || !matches!(args[0].as_str(), "snapshot" | "restore") {
        return Err(io::Error::other("usage: network_backup snapshot | restore"));
    }
    if !system.is_root() {
        return Err(io::Error::other("run on the robot with sudo"));
    }
    let path = if args[0] == "snapshot" {
        backup::snapshot(Path::new("/"), system)?
    } else {
        backup::restore(Path::new("/"), system)?
    };
    Ok(format!("Network {}: {}\n", args[0], path.display()))
}
fn notice(deferred: bool) -> &'static str {
    if deferred {
        "This ssh session ends in 3 seconds: the change is made by the robot itself. Reconnect through the new network.\n"
    } else {
        ""
    }
}
pub fn run(args: &[String], network: &Network, system: &mut impl System) -> io::Result<String> {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        return Ok(format!("{HELP}\n"));
    }
    let (command, tail) = args.split_first().ok_or_else(|| io::Error::other(HELP))?;
    let mut positionals = vec![];
    let mut apply = true;
    let mut channel = None;
    let mut options = true;
    let mut iter = tail.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--" if options => options = false,
            "--force"
                if options
                    && matches!(
                        command.as_str(),
                        "setupHotspot" | "setupEthernet" | "toggleNetMode"
                    ) => {}
            "--no-activate"
                if options && matches!(command.as_str(), "setupHotspot" | "setupEthernet") =>
            {
                apply = false
            }
            "--channel" if options && command == "setupHotspot" => {
                channel = Some(
                    iter.next()
                        .ok_or_else(|| io::Error::other("--channel requires a number"))?
                        .parse::<u16>()
                        .map_err(|_| io::Error::other("channel must be a number"))?,
                )
            }
            value if options && value.starts_with("--channel=") && command == "setupHotspot" => {
                channel = Some(
                    value[10..]
                        .parse::<u16>()
                        .map_err(|_| io::Error::other("channel must be a number"))?,
                )
            }
            value if options && value.starts_with('-') => {
                return Err(io::Error::other(format!("unknown option: {value}")))
            }
            _ => positionals.push(arg.as_str()),
        }
    }
    match command.as_str() {
        "setupHotspot" if (3..=4).contains(&positionals.len()) => {
            let info = network.hotspot(
                system,
                &Hotspot {
                    prefix: positionals[0],
                    team: positionals[1],
                    band: positionals[2],
                    password: positionals.get(3).copied().unwrap_or("password"),
                    channel,
                    apply,
                },
            )?;
            Ok(format!(
                "Wi-Fi access point '{}' on {} (channel {}): the VMX is {}\n{}",
                info.ssid,
                info.interface,
                info.channel,
                info.address,
                notice(info.deferred)
            ))
        }
        "setupEthernet" if positionals.len() <= 1 => {
            let info = network.ethernet(system, positionals.first().copied(), apply)?;
            Ok(format!(
                "wired port {}: {} with a DHCP server\n{}",
                info.interface,
                info.address,
                notice(info.deferred)
            ))
        }
        "toggleNetMode" if (1..=3).contains(&positionals.len()) => {
            if positionals[0] == "ap" && positionals.len() != 1 {
                return Err(io::Error::other("AP mode does not take SSID or password"));
            }
            let deferred = network.toggle(
                system,
                positionals[0],
                positionals.get(1).copied(),
                positionals.get(2).copied(),
            )?;
            Ok(format!(
                "network mode: {}\n{}",
                positionals[0],
                notice(deferred)
            ))
        }
        "status" if positionals.is_empty() => super::status(system),
        "sshdevice" if positionals.is_empty() => Ok(format!("{}\n", super::ssh_device(system)?)),
        _ => Err(io::Error::other(HELP)),
    }
}
