use robot_manager::mrccomm::{
    build_robot_status_packet, decode_status_frame, degrees_to_hal_pov, encode_state_frame,
    parse_ds_packet, usable_battery, DsControl, JoystickState, RobotMode, RELAY_STATE_LEN,
    RELAY_STATUS_LEN,
};

/// Build a Driver Station control packet: header plus six joystick blocks.
fn ds_packet(sequence: u16, control: u8, joysticks: &[&[u8]]) -> Vec<u8> {
    let mut packet = Vec::new();
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.push(0x01); // protocol version
    packet.push(control);
    for block in joysticks {
        packet.extend_from_slice(block);
    }
    for _ in joysticks.len()..6 {
        packet.push(0); // no axes
        packet.push(0); // no button bytes
        packet.push(0); // no POVs
    }
    packet
}

#[test]
fn parse_ds_packet_extracts_mode_enable_and_estop() {
    let teleop = parse_ds_packet(&ds_packet(42, 0x04, &[]));
    assert_eq!(teleop.sequence, 42);
    assert!(teleop.enabled);
    assert_eq!(teleop.mode, RobotMode::Teleop);

    let auto = parse_ds_packet(&ds_packet(43, 0x06, &[]));
    assert!(auto.enabled);
    assert_eq!(auto.mode, RobotMode::Autonomous);

    let test = parse_ds_packet(&ds_packet(44, 0x05, &[]));
    assert_eq!(test.mode, RobotMode::Test);

    let disabled = parse_ds_packet(&ds_packet(45, 0x00, &[]));
    assert!(!disabled.enabled);
    assert_eq!(disabled.mode, RobotMode::Disabled);

    let estop = parse_ds_packet(&ds_packet(46, 0x84, &[]));
    assert!(estop.estop);
    assert!(!estop.enabled);
    assert_eq!(estop.mode, RobotMode::Estop);

    let fms = parse_ds_packet(&ds_packet(47, 0x0C, &[]));
    assert!(fms.fms_attached);
    assert!(fms.enabled);
}

#[test]
fn parse_ds_packet_reads_axis_buttons_and_povs() {
    // Joystick 0: two axes (127, -128), one button byte (0b0000_0010),
    // one POV at 90 degrees. Joystick 1: unused.
    let stick0: &[u8] = &[2, 127, 0x80, 1, 0b0000_0010, 1, 90, 0];
    let control = parse_ds_packet(&ds_packet(1, 0x04, &[stick0]));

    let joystick = control.joysticks[0];
    assert_eq!(joystick.available_axes, 0b11);
    assert_eq!(joystick.raw_axes[0], 127);
    assert_eq!(joystick.raw_axes[1], -128);
    assert_eq!(joystick.raw_axes[2], 0);
    assert_eq!(joystick.buttons, 0b10);
    assert_eq!(joystick.available_buttons, 0xFF);
    assert_eq!(joystick.available_povs, 0b1);
    assert_eq!(joystick.povs[0], 0x02); // right
    assert_eq!(control.joysticks[1], JoystickState::default());
}

#[test]
fn parse_ds_packet_never_panics_on_truncated_or_oversized_counts() {
    // Header only, then a block that promises far more than it delivers.
    let mut packet = ds_packet(1, 0x04, &[]);
    packet.extend_from_slice(&[200, 1, 2, 3]); // axis count 200, three bytes
    let control = parse_ds_packet(&packet);
    assert_eq!(control.sequence, 1);

    // A byte-truncated packet must not panic either.
    for length in 0..packet.len() {
        let _ = parse_ds_packet(&packet[..length]);
    }
    assert_eq!(parse_ds_packet(&[0, 1]).mode, RobotMode::Disabled);
}

#[test]
fn degrees_map_to_hal_direction_bits() {
    assert_eq!(degrees_to_hal_pov(-1), 0);
    assert_eq!(degrees_to_hal_pov(0), 0x01);
    assert_eq!(degrees_to_hal_pov(45), 0x01);
    assert_eq!(degrees_to_hal_pov(90), 0x02);
    assert_eq!(degrees_to_hal_pov(180), 0x04);
    assert_eq!(degrees_to_hal_pov(270), 0x08);
    assert_eq!(degrees_to_hal_pov(350), 0x01);
}

#[test]
fn state_frame_matches_the_documented_c_golden_vector() {
    let mut control = DsControl::default();
    control.joysticks[0].available_axes = 0x0001;
    control.joysticks[0].raw_axes[0] = 127;
    control.joysticks[0].buttons = 0x2;
    control.joysticks[0].available_buttons = 0x2;

    let frame = encode_state_frame(&control, false);
    assert_eq!(frame.len(), RELAY_STATE_LEN);
    assert_eq!(frame.len(), 248);

    let mut expected = vec![0u8; 248];
    expected[0] = 0x57;
    expected[1] = 0x01;
    expected[2] = 0x00; // no flags
    expected[3] = 0x00; // unknown mode
    expected[4] = 0x00; // unknown station
    expected[6] = 0xFF; // matchTime -1
    expected[7] = 0xFF;
    expected[8] = 0x01; // availableAxes = 0x0001
    expected[10] = 127; // rawAxes[0]
    expected[8 + 14] = 0x02; // buttons = 2
    expected[8 + 22] = 0x02; // availableButtons = 2
    assert_eq!(frame[..], expected[..]);
}

#[test]
fn state_frame_carries_flags_mode_and_joystick_availability() {
    let mut control = DsControl::default();
    control.enabled = true;
    control.mode = RobotMode::Autonomous;
    control.fms_attached = true;
    control.alliance_station = 4; // BLUE_1
    control.match_time_tenths = 1234;
    control.joysticks[5].available_povs = 0b10;
    control.joysticks[5].povs[1] = 0x04;

    let frame = encode_state_frame(&control, true);
    // bit0 attached | bit1 enabled | bit3 fms = 0b1011
    assert_eq!(frame[2], 0b0000_1011);
    assert_eq!(frame[3], 1); // autonomous
    assert_eq!(frame[4], 4);
    assert_eq!(i16::from_le_bytes([frame[6], frame[7]]), 1234);
    let base = 8 + 5 * 40;
    assert_eq!(frame[base + 30], 0b10);
    assert_eq!(frame[base + 32 + 1], 0x04);
}

#[test]
fn status_frame_round_trips_battery_and_liveness() {
    let mut frame = [0u8; RELAY_STATUS_LEN];
    frame[0] = 0x53;
    frame[1] = 0x01;
    frame[2] = 0x01; // robot program alive
    frame[4..8].copy_from_slice(&12.34f32.to_le_bytes());
    frame[8..12].copy_from_slice(&77u32.to_le_bytes());

    let status = decode_status_frame(&frame).unwrap();
    assert!(status.robot_program_alive);
    assert!((status.battery_volts - 12.34).abs() < 1e-6);
    assert_eq!(status.status_count, 77);

    assert!(decode_status_frame(&frame[..15]).is_none());
    assert!(decode_status_frame(&[0u8; 16]).is_none());
}

#[test]
fn battery_is_only_forwarded_when_plausible() {
    assert_eq!(usable_battery(12.34, 12.5), 12.34);
    assert_eq!(usable_battery(0.0, 12.5), 12.5);
    assert_eq!(usable_battery(f32::NAN, 12.5), 12.5);
    assert_eq!(usable_battery(99.0, 12.5), 12.5);
}

#[test]
fn robot_status_packet_reports_mode_and_battery() {
    let mut control = DsControl::default();
    control.enabled = true;
    control.mode = RobotMode::Teleop;

    let packet = build_robot_status_packet(100, &control, 12.5);
    assert_eq!(u16::from_be_bytes([packet[0], packet[1]]), 100);
    assert_eq!(packet[2], 0x01);
    assert_eq!(packet[3] & 0x04, 0x04);
    assert_eq!(packet[4], 12);
    assert_eq!(packet[5], 128); // 0.5 * 256
}
