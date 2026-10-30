//! MrcComm daemon: FRC Driver Station communication.
//!
//! Owns the Driver Station sockets (UDP 1110 for control packets, 1150 for the
//! robot status reply), forwards the decoded state to the robot program's
//! `halsim_ds_relay` extension over loopback UDP (127.0.0.1:1742), takes the
//! battery voltage back from that relay, and sends notable events as alert
//! lines (127.0.0.1:1743) so they appear in RioLog.
//!
//! Wire formats:
//! - Driver Station packets: sequence (u16), protocol version (u8), control
//!   byte (u8), then six joystick blocks (axis count + i8 axes, button byte
//!   count + bytes, POV count + i16 degrees). This is the documented 2014+
//!   layout; the assumption still needs a check against a real Driver Station.
//! - Relay frames: see `RelayState`/`RelayStatus` below; the byte layout is
//!   shared with `allwpilib/simulation/halsim_ds_relay` (little-endian, fixed
//!   size, no padding).
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const DEFAULT_ROBOT_PORT: u16 = 1110;
pub const DEFAULT_DS_PORT: u16 = 1150;
/// Loopback port of the in-process `halsim_ds_relay` extension.
pub const DEFAULT_RELAY_PORT: u16 = 1742;
/// Loopback port where `robot_manager` republishes alert lines into RioLog.
pub const DEFAULT_ALERT_PORT: u16 = crate::alerts::DEFAULT_ALERT_PORT;
/// A Driver Station packet older than this disables the robot.
pub const DS_TIMEOUT: Duration = Duration::from_millis(500);
/// State frames are sent at the Driver Station rate (50 Hz).
pub const LOOP_PERIOD: Duration = Duration::from_millis(20);

pub const RELAY_STATE_LEN: usize = 248;
pub const RELAY_STATUS_LEN: usize = 16;
const RELAY_STATE_MAGIC: u8 = 0x57;
const RELAY_STATUS_MAGIC: u8 = 0x53;
const RELAY_VERSION: u8 = 1;
const FLAG_DS_ATTACHED: u8 = 1 << 0;
const FLAG_ENABLED: u8 = 1 << 1;
const FLAG_ESTOP: u8 = 1 << 2;
const FLAG_FMS: u8 = 1 << 3;
const JOYSTICK_COUNT: usize = 6;
const AXIS_COUNT: usize = 12;
const POV_COUNT: usize = 8;
const JOYSTICK_BLOCK: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RobotMode {
    Disabled,
    Teleop,
    Autonomous,
    Test,
    Estop,
}

/// One joystick as received from the Driver Station.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JoystickState {
    pub available_axes: u16,
    pub raw_axes: [i8; AXIS_COUNT],
    pub buttons: u64,
    pub available_buttons: u64,
    pub available_povs: u8,
    pub povs: [u8; POV_COUNT],
}

/// Decoded Driver Station control packet.
#[derive(Clone, Copy, Debug)]
pub struct DsControl {
    pub sequence: u16,
    pub mode: RobotMode,
    pub enabled: bool,
    pub estop: bool,
    pub fms_attached: bool,
    /// Alliance/station are not carried by the subset of the packet we decode
    /// yet, so they stay unknown until a real Driver Station is available.
    pub alliance_station: u8,
    pub match_time_tenths: i16,
    pub joysticks: [JoystickState; JOYSTICK_COUNT],
}

impl Default for DsControl {
    fn default() -> Self {
        Self {
            sequence: 0,
            mode: RobotMode::Disabled,
            enabled: false,
            estop: false,
            fms_attached: false,
            alliance_station: 0,
            match_time_tenths: -1,
            joysticks: [JoystickState::default(); JOYSTICK_COUNT],
        }
    }
}

impl DsControl {
    fn disabled(&self) -> Self {
        Self {
            sequence: self.sequence,
            ..Self::default()
        }
    }
    fn robot_mode_byte(&self) -> u8 {
        match self.mode {
            RobotMode::Disabled | RobotMode::Estop => 0,
            RobotMode::Autonomous => 1,
            RobotMode::Teleop => 2,
            RobotMode::Test => 3,
        }
    }
    fn flags(&self) -> u8 {
        let mut flags = 0u8;
        if self.enabled {
            flags |= FLAG_ENABLED;
        }
        if self.estop {
            flags |= FLAG_ESTOP;
        }
        if self.fms_attached {
            flags |= FLAG_FMS;
        }
        flags
    }
}

/// Encode a relay STATE frame (mrccomm -> `halsim_ds_relay`).
pub fn encode_state_frame(control: &DsControl, ds_attached: bool) -> [u8; RELAY_STATE_LEN] {
    let mut frame = [0u8; RELAY_STATE_LEN];
    frame[0] = RELAY_STATE_MAGIC;
    frame[1] = RELAY_VERSION;
    let mut flags = control.flags();
    if ds_attached {
        flags |= FLAG_DS_ATTACHED;
    }
    frame[2] = flags;
    frame[3] = control.robot_mode_byte();
    frame[4] = control.alliance_station;
    frame[6..8].copy_from_slice(&control.match_time_tenths.to_le_bytes());
    for (index, joystick) in control.joysticks.iter().enumerate() {
        let base = 8 + index * JOYSTICK_BLOCK;
        frame[base..base + 2].copy_from_slice(&joystick.available_axes.to_le_bytes());
        for axis in 0..AXIS_COUNT {
            frame[base + 2 + axis] = joystick.raw_axes[axis] as u8;
        }
        frame[base + 14..base + 22].copy_from_slice(&joystick.buttons.to_le_bytes());
        frame[base + 22..base + 30].copy_from_slice(&joystick.available_buttons.to_le_bytes());
        frame[base + 30] = joystick.available_povs;
        frame[base + 31] = 0;
        frame[base + 32..base + 32 + POV_COUNT].copy_from_slice(&joystick.povs);
    }
    frame
}

/// Decoded relay STATUS frame (battery voltage and liveness).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelayStatus {
    pub robot_program_alive: bool,
    pub battery_volts: f32,
    pub status_count: u32,
}

/// Decode a relay STATUS frame; `None` for anything malformed.
pub fn decode_status_frame(bytes: &[u8]) -> Option<RelayStatus> {
    if bytes.len() != RELAY_STATUS_LEN
        || bytes[0] != RELAY_STATUS_MAGIC
        || bytes[1] != RELAY_VERSION
    {
        return None;
    }
    Some(RelayStatus {
        robot_program_alive: bytes[2] & 1 != 0,
        battery_volts: f32::from_le_bytes(bytes[4..8].try_into().ok()?),
        status_count: u32::from_le_bytes(bytes[8..12].try_into().ok()?),
    })
}

/// Battery voltage is only forwarded to the Driver Station when plausible
/// (the relay reports 0.0 until the robot program has polled the hardware).
pub fn usable_battery(reported: f32, fallback: f32) -> f32 {
    if reported.is_finite() && (5.0..=20.0).contains(&reported) {
        reported
    } else {
        fallback
    }
}

/// Parse a Driver Station control packet. Truncated or oversized values are
/// clamped so a malformed packet can never panic the daemon.
pub fn parse_ds_packet(data: &[u8]) -> DsControl {
    if data.len() < 4 {
        return DsControl::default();
    }
    let sequence = u16::from_be_bytes([data[0], data[1]]);
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

    let mut control = DsControl {
        sequence,
        mode,
        enabled,
        estop,
        fms_attached,
        ..DsControl::default()
    };

    let mut index = 4;
    for joystick in control.joysticks.iter_mut() {
        let Some(&axis_count) = data.get(index) else {
            break;
        };
        index += 1;
        let axis_count = (axis_count as usize).min(AXIS_COUNT);
        for axis in 0..axis_count {
            match data.get(index) {
                Some(&value) => joystick.raw_axes[axis] = value as i8,
                None => return control,
            }
            joystick.available_axes |= 1 << axis;
            index += 1;
        }
        let Some(&button_bytes) = data.get(index) else {
            return control;
        };
        index += 1;
        let button_bytes = (button_bytes as usize).min(8);
        for byte in 0..button_bytes {
            match data.get(index) {
                Some(&value) => joystick.buttons |= (value as u64) << (byte * 8),
                None => return control,
            }
            index += 1;
        }
        joystick.available_buttons = if button_bytes == 8 {
            u64::MAX
        } else {
            (1u64 << (button_bytes * 8)) - 1
        };
        let Some(&pov_count) = data.get(index) else {
            return control;
        };
        index += 1;
        let pov_count = (pov_count as usize).min(POV_COUNT);
        for pov in 0..pov_count {
            let (Some(&low), Some(&high)) = (data.get(index), data.get(index + 1)) else {
                return control;
            };
            let degrees = i16::from_le_bytes([low, high]);
            joystick.povs[pov] = degrees_to_hal_pov(degrees);
            joystick.available_povs |= 1 << pov;
            index += 2;
        }
    }
    control
}

/// The HAL stores a POV as a direction bitmask; the Driver Station sends
/// degrees (or -1 when centered).
pub fn degrees_to_hal_pov(degrees: i16) -> u8 {
    if degrees < 0 {
        return 0;
    }
    let degrees = degrees.rem_euclid(360);
    if degrees <= 45 || degrees >= 315 {
        0x01 // up
    } else if degrees <= 135 {
        0x02 // right
    } else if degrees <= 225 {
        0x04 // down
    } else {
        0x08 // left
    }
}

/// Build the robot status reply (Robot -> Driver Station, UDP 1150).
pub fn build_robot_status_packet(sequence: u16, control: &DsControl, battery_volts: f32) -> Vec<u8> {
    let mut packet = Vec::with_capacity(8);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.push(0x01); // Comm version

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

    // Battery voltage: integer volts, then the fraction in 256ths
    // (the Driver Station reads byte1 + byte2 / 256).
    let volts = usable_battery(battery_volts, 12.5);
    packet.push(volts.trunc().min(255.0) as u8);
    packet.push(((volts.fract() * 256.0) as u8).min(255));

    packet.push(0x00); // Trace byte
    packet
}

pub struct MrcCommOptions {
    pub listen_port: u16,
    pub ds_port: u16,
    pub relay_port: u16,
    pub alert_port: u16,
    pub battery_fallback: f32,
}

impl Default for MrcCommOptions {
    fn default() -> Self {
        Self {
            listen_port: DEFAULT_ROBOT_PORT,
            ds_port: DEFAULT_DS_PORT,
            relay_port: DEFAULT_RELAY_PORT,
            alert_port: DEFAULT_ALERT_PORT,
            battery_fallback: 12.5,
        }
    }
}

pub struct MrcCommDaemon {
    stopping: Arc<AtomicBool>,
}

impl Default for MrcCommDaemon {
    fn default() -> Self {
        Self::new()
    }
}

impl MrcCommDaemon {
    pub fn new() -> Self {
        Self {
            stopping: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn run(&self, options: MrcCommOptions) -> io::Result<()> {
        let core = crate::realtime::pin_to_core(crate::realtime::SUPERVISOR_CORE).unwrap_or(0);
        let socket = UdpSocket::bind(("0.0.0.0", options.listen_port))?;
        socket.set_nonblocking(true)?;
        let relay = UdpSocket::bind(("127.0.0.1", 0))?;
        relay.set_nonblocking(true)?;
        let alerts = UdpSocket::bind(("127.0.0.1", 0))?;
        let alert = |line: &str| {
            let _ = alerts.send_to(line.as_bytes(), ("127.0.0.1", options.alert_port));
        };

        crate::log(
            "INFO",
            format_args!(
                "MrcComm daemon listening on UDP port {}, replying on {} (CPU core {core}, Core {}); relay 127.0.0.1:{}",
                options.listen_port,
                options.ds_port,
                core + 1,
                options.relay_port
            ),
        );

        let mut buf = [0u8; 1024];
        let mut relay_buf = [0u8; 64];
        let mut control = DsControl::default();
        let mut control_addr: Option<SocketAddr> = None;
        let mut control_seen: Option<Instant> = None;
        let mut seq_out = 0u16;
        let mut battery = options.battery_fallback;
        let mut robot_program_seen = false;

        while !self.stopping.load(Ordering::Relaxed) {
            let started = Instant::now();

            // Drain all pending Driver Station packets; the newest one wins.
            while let Ok((size, peer)) = socket.recv_from(&mut buf) {
                if size >= 4 {
                    if control_seen.is_none() {
                        alert(&format!("Driver Station connected from {}", peer.ip()));
                    }
                    control_addr = Some(SocketAddr::new(peer.ip(), options.ds_port));
                    control_seen = Some(Instant::now());
                    control = parse_ds_packet(&buf[..size]);
                }
            }

            // Watchdog: a silent Driver Station disables the robot.
            if let Some(seen) = control_seen {
                if seen.elapsed() > DS_TIMEOUT {
                    control = control.disabled();
                    control_seen = None;
                    alert("Driver Station packet timeout: robot automatically disabled");
                }
            }

            // Forward the state to the robot program's relay extension.
            let state = encode_state_frame(&control, control_seen.is_some());
            let _ = relay.send_to(&state, ("127.0.0.1", options.relay_port));

            // Take battery voltage and liveness back from the relay.
            while let Ok((size, _)) = relay.recv_from(&mut relay_buf) {
                if let Some(status) = decode_status_frame(&relay_buf[..size]) {
                    battery = usable_battery(status.battery_volts, battery);
                    if status.robot_program_alive && !robot_program_seen {
                        robot_program_seen = true;
                        alert("Robot program connected to the DS relay");
                    }
                }
            }

            // Robot status reply at the Driver Station rate.
            if let Some(target) = control_addr {
                seq_out = seq_out.wrapping_add(1);
                let reply = build_robot_status_packet(seq_out, &control, battery);
                let _ = socket.send_to(&reply, target);
            }

            let elapsed = started.elapsed();
            if elapsed < LOOP_PERIOD {
                thread::sleep(LOOP_PERIOD - elapsed);
            }
        }

        Ok(())
    }

    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
    }
}

pub fn dispatch(args: &[String]) -> Option<io::Result<()>> {
    if args.is_empty() || args[0] != "mrccomm" {
        return None;
    }
    let mut options = MrcCommOptions::default();
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        let value = arg.split_once('=').map(|(_, value)| value.to_string());
        let mut next = |name: &str| -> Result<String, io::Error> {
            value
                .clone()
                .or_else(|| iter.next().cloned())
                .ok_or_else(|| io::Error::other(format!("{name} needs a value")))
        };
        let parsed: Option<Result<(), io::Error>> = match arg.split('=').next().unwrap_or(arg) {
            "--port" => Some(next("--port").and_then(|v| {
                options.listen_port = v.parse().map_err(io::Error::other)?;
                Ok(())
            })),
            "--ds-port" => Some(next("--ds-port").and_then(|v| {
                options.ds_port = v.parse().map_err(io::Error::other)?;
                Ok(())
            })),
            "--relay-port" => Some(next("--relay-port").and_then(|v| {
                options.relay_port = v.parse().map_err(io::Error::other)?;
                Ok(())
            })),
            "--alert-port" => Some(next("--alert-port").and_then(|v| {
                options.alert_port = v.parse().map_err(io::Error::other)?;
                Ok(())
            })),
            "--battery" => Some(next("--battery").and_then(|v| {
                options.battery_fallback = v.parse().map_err(io::Error::other)?;
                Ok(())
            })),
            _ => None,
        };
        match parsed {
            Some(Ok(())) => {}
            Some(Err(error)) => return Some(Err(error)),
            None => {
                return Some(Err(io::Error::other(format!(
                    "unknown mrccomm argument: {arg}\nusage: robot_manager mrccomm [--port 1110] [--ds-port 1150] [--relay-port 1742] [--alert-port 1743] [--battery 12.5]"
                ))))
            }
        }
    }

    Some(MrcCommDaemon::new().run(options))
}
