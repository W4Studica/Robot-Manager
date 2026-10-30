//! MrcComm daemon: FRC Driver Station communication protocol handler.
//! Listens on UDP port 1110 for DS control packets (mode, enable/disable, joysticks),
//! and transmits robot status/battery telemetry back on UDP port 1150 at 50 Hz.
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_ROBOT_PORT: u16 = 1110;
pub const DEFAULT_DS_PORT: u16 = 1150;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RobotMode {
    Disabled,
    Teleop,
    Autonomous,
    Test,
    Estop,
}

#[derive(Clone, Copy, Debug)]
pub struct DsControl {
    pub sequence: u16,
    pub mode: RobotMode,
    pub enabled: bool,
    pub estop: bool,
    pub fms_attached: bool,
    pub red_alliance: bool,
    pub station: u8,
}

impl Default for DsControl {
    fn default() -> Self {
        Self {
            sequence: 0,
            mode: RobotMode::Disabled,
            enabled: false,
            estop: false,
            fms_attached: false,
            red_alliance: true,
            station: 1,
        }
    }
}

pub struct MrcCommDaemon {
    stopping: Arc<AtomicBool>,
    state: Arc<AtomicU8>,
}

impl MrcCommDaemon {
    pub fn new() -> Self {
        Self {
            stopping: Arc::new(AtomicBool::new(false)),
            state: Arc::new(AtomicU8::new(0)),
        }
    }

    pub fn run(
        &self,
        listen_port: u16,
        reply_port: u16,
        battery_volts: f32,
    ) -> io::Result<()> {
        // System component: stay on the reserved core even when launched by hand.
        let core = crate::realtime::pin_to_core(crate::realtime::SUPERVISOR_CORE).unwrap_or(0);
        let socket = UdpSocket::bind(("0.0.0.0", listen_port))?;
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        socket.set_nonblocking(false)?;

        crate::log(
            "INFO",
            format_args!(
                "MrcComm daemon listening on UDP port {listen_port}, replying on {reply_port} (CPU core {core}, Core {})",
                core + 1
            ),
        );

        let mut buf = [0u8; 1024];
        let mut last_ds_addr: Option<SocketAddr> = None;
        let mut last_packet_time = Instant::now();
        let mut ds_connected = false;
        let mut seq_out = 0u16;
        let mut control = DsControl::default();

        while !self.stopping.load(Ordering::Relaxed) {
            // Receive DS packet
            match socket.recv_from(&mut buf) {
                Ok((size, peer)) => {
                    if size >= 4 {
                        last_ds_addr = Some(SocketAddr::new(peer.ip(), reply_port));
                        last_packet_time = Instant::now();
                        if !ds_connected {
                            ds_connected = true;
                            crate::log("INFO", format_args!("Driver Station connected from {}", peer.ip()));
                        }
                        control = parse_ds_packet(&buf[..size]);
                        self.state.store(encode_state(&control), Ordering::Relaxed);
                    }
                }
                Err(ref err)
                    if err.kind() == io::ErrorKind::WouldBlock
                        || err.kind() == io::ErrorKind::TimedOut => {}
                Err(err) => {
                    crate::log("WARNING", format_args!("MrcComm recv error: {err}"));
                }
            }

            // Check watchdog
            if ds_connected && last_packet_time.elapsed() > Duration::from_millis(500) {
                ds_connected = false;
                control.enabled = false;
                control.mode = RobotMode::Disabled;
                self.state.store(encode_state(&control), Ordering::Relaxed);
                crate::log("WARNING", "Driver Station packet timeout: robot automatically disabled");
            }

            // Send telemetry back to DS at 50 Hz if DS is known
            if let Some(target) = last_ds_addr {
                seq_out = seq_out.wrapping_add(1);
                let reply = build_robot_status_packet(seq_out, &control, battery_volts);
                let _ = socket.send_to(&reply, target);
            }

            thread::sleep(Duration::from_millis(20));
        }

        Ok(())
    }

    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }
}

pub fn parse_ds_packet(data: &[u8]) -> DsControl {
    if data.len() < 4 {
        return DsControl::default();
    }
    let seq = u16::from_be_bytes([data[0], data[1]]);
    let ctrl_byte = data[3];

    let estop = (ctrl_byte & 0x80) != 0;
    let enabled = (ctrl_byte & 0x04) != 0 && !estop;
    let fms_attached = (ctrl_byte & 0x08) != 0;

    let mode = if estop {
        RobotMode::Estop
    } else if !enabled {
        RobotMode::Disabled
    } else if (ctrl_byte & 0x02) != 0 {
        RobotMode::Autonomous
    } else if (ctrl_byte & 0x01) != 0 {
        RobotMode::Test
    } else {
        RobotMode::Teleop
    };

    let (red_alliance, station) = if data.len() >= 6 {
        let station_byte = data[5];
        (station_byte < 3, (station_byte % 3) + 1)
    } else {
        (true, 1)
    };

    DsControl {
        sequence: seq,
        mode,
        enabled,
        estop,
        fms_attached,
        red_alliance,
        station,
    }
}

pub fn build_robot_status_packet(sequence: u16, control: &DsControl, battery_volts: f32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(32);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.push(0x01); // Comm version

    // Control reflection byte
    let mut status_byte = 0u8;
    if control.enabled {
        status_byte |= 0x04;
    }
    match control.mode {
        RobotMode::Autonomous => status_byte |= 0x02,
        RobotMode::Test => status_byte |= 0x01,
        _ => {}
    }
    if control.estop {
        status_byte |= 0x80;
    }
    packet.push(status_byte);

    // Battery voltage format: integer part + fractional part
    let int_part = battery_volts.trunc() as u8;
    let frac_part = ((battery_volts.fract() * 256.0).trunc() as u8).min(255);
    packet.push(int_part);
    packet.push(frac_part);

    packet.push(0x00); // CPU & trace info
    packet
}

fn encode_state(ctrl: &DsControl) -> u8 {
    let mut s = 0u8;
    if ctrl.enabled {
        s |= 1;
    }
    match ctrl.mode {
        RobotMode::Disabled => {}
        RobotMode::Teleop => s |= 2,
        RobotMode::Autonomous => s |= 4,
        RobotMode::Test => s |= 8,
        RobotMode::Estop => s |= 16,
    }
    s
}

pub fn dispatch(args: &[String]) -> Option<io::Result<()>> {
    if args.is_empty() || args[0] != "mrccomm" {
        return None;
    }
    let mut port = DEFAULT_ROBOT_PORT;
    let mut ds_port = DEFAULT_DS_PORT;
    let mut battery = 12.5f32;

    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--port" => {
                if let Some(p) = iter.next().and_then(|v| v.parse().ok()) {
                    port = p;
                }
            }
            "--ds-port" => {
                if let Some(p) = iter.next().and_then(|v| v.parse().ok()) {
                    ds_port = p;
                }
            }
            "--battery" => {
                if let Some(b) = iter.next().and_then(|v| v.parse().ok()) {
                    battery = b;
                }
            }
            _ => {
                return Some(Err(io::Error::other(format!(
                    "unknown mrccomm argument: {arg}\nusage: robot_manager mrccomm [--port 1110] [--ds-port 1150] [--battery 12.5]"
                ))))
            }
        }
    }

    let daemon = MrcCommDaemon::new();
    Some(daemon.run(port, ds_port, battery))
}
