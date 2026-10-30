//! VMX network IRQ affinity and CPU isolation.
//! Pins network and I/O interrupts to CPU 0 and 1 (mask 0x3),
//! leaving CPU 2 and 3 unburdened for the robot's real-time control loop.
use std::fs;
use std::io;
use std::path::Path;

pub const NETWORK_CPU_MASK: &str = "3"; // CPU 0 and 1 (binary 0011)

#[derive(Clone, Debug)]
pub struct PinnedIrq {
    pub irq: u32,
    pub name: String,
}

pub fn configure_irq_affinity(dry_run: bool) -> io::Result<Vec<PinnedIrq>> {
    let mut pinned = Vec::new();
    let text = match fs::read_to_string("/proc/interrupts") {
        Ok(t) => t,
        Err(err) => {
            return Err(io::Error::other(format!(
                "could not read /proc/interrupts: {err}"
            )));
        }
    };

    // Target devices to pin to CPU 0 and 1
    let targets = ["bcmgenet", "eth0", "brcmfmac", "mmc1", "xhci-hcd", "dwc_otg"];

    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let irq_str = parts[0].trim_end_matches(':');
        let Ok(irq) = irq_str.parse::<u32>() else {
            continue;
        };

        // Check if any part matches our target devices
        if parts.iter().any(|p| targets.iter().any(|t| p.contains(t))) {
            let desc = parts.last().copied().unwrap_or("unknown");
            let affinity_file = format!("/proc/irq/{irq}/smp_affinity");
            if dry_run {
                println!("[dry-run] set smp_affinity for IRQ {irq} ({desc}) -> {NETWORK_CPU_MASK}");
            } else if Path::new(&affinity_file).exists() {
                if let Err(err) = fs::write(&affinity_file, NETWORK_CPU_MASK) {
                    crate::log(
                        "WARNING",
                        format_args!("could not set affinity for IRQ {irq}: {err}"),
                    );
                }
            }
            pinned.push(PinnedIrq {
                irq,
                name: desc.to_string(),
            });
        }
    }

    // Configure RPS (Receive Packet Steering) on network devices
    for iface in ["eth0", "wlan0"] {
        let rps_path = format!("/sys/class/net/{iface}/queues/rx-0/rps_cpus");
        if Path::new(&rps_path).exists() {
            if dry_run {
                println!("[dry-run] set rps_cpus for {iface} -> {NETWORK_CPU_MASK}");
            } else {
                let _ = fs::write(&rps_path, NETWORK_CPU_MASK);
            }
        }
    }

    Ok(pinned)
}

pub fn dispatch(args: &[String]) -> Option<io::Result<()>> {
    if args.is_empty() || args[0] != "irqconf" {
        return None;
    }
    let mut dry_run = false;
    for arg in args.iter().skip(1) {
        if arg == "--dry-run" {
            dry_run = true;
        } else {
            return Some(Err(io::Error::other(format!(
                "unknown irqconf argument: {arg}\nusage: robot_manager irqconf [--dry-run]"
            ))));
        }
    }

    Some((|| {
        let pinned = configure_irq_affinity(dry_run)?;
        if !dry_run {
            println!(
                "Configured network IRQ affinity to CPU 0,1 (mask 0x3) for {} interrupts; CPU 2,3 reserved for RT control",
                pinned.len()
            );
            for p in pinned {
                println!("  IRQ {:>3}: {}", p.irq, p.name);
            }
        }
        Ok(())
    })())
}
