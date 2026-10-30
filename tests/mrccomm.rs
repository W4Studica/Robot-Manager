use robot_manager::mrccomm::{
    build_robot_status_packet, parse_ds_packet, DsControl, RobotMode,
};

#[test]
fn parse_ds_packet_extracts_teleop_and_autonomous() {
    // Sequence 42, version 1, control 0x04 (Enabled, Teleop), team 0, station 1 (Red 2)
    let teleop_data = [0x00, 0x2a, 0x01, 0x04, 0x00, 0x01];
    let ctrl = parse_ds_packet(&teleop_data);
    assert_eq!(ctrl.sequence, 42);
    assert!(ctrl.enabled);
    assert_eq!(ctrl.mode, RobotMode::Teleop);
    assert_eq!(ctrl.red_alliance, true);
    assert_eq!(ctrl.station, 2);

    // Sequence 43, control 0x06 (Enabled, Auto)
    let auto_data = [0x00, 0x2b, 0x01, 0x06, 0x00, 0x04]; // station 4 (Blue 2)
    let auto_ctrl = parse_ds_packet(&auto_data);
    assert_eq!(auto_ctrl.sequence, 43);
    assert!(auto_ctrl.enabled);
    assert_eq!(auto_ctrl.mode, RobotMode::Autonomous);
    assert_eq!(auto_ctrl.red_alliance, false);
}

#[test]
fn parse_ds_packet_handles_estop_and_disabled() {
    // Disabled packet (control 0x00)
    let dis_data = [0x00, 0x01, 0x01, 0x00];
    let ctrl = parse_ds_packet(&dis_data);
    assert!(!ctrl.enabled);
    assert_eq!(ctrl.mode, RobotMode::Disabled);

    // Estop packet (control bit 0x80)
    let estop_data = [0x00, 0x02, 0x01, 0x84];
    let estop_ctrl = parse_ds_packet(&estop_data);
    assert_eq!(estop_ctrl.mode, RobotMode::Estop);
    assert!(estop_ctrl.estop);
    assert!(!estop_ctrl.enabled);
}

#[test]
fn build_robot_status_packet_encodes_battery_and_mode() {
    let mut ctrl = DsControl::default();
    ctrl.enabled = true;
    ctrl.mode = RobotMode::Teleop;

    let packet = build_robot_status_packet(100, &ctrl, 12.5);
    assert_eq!(u16::from_be_bytes([packet[0], packet[1]]), 100);
    assert_eq!(packet[2], 0x01); // Comm version
    assert_eq!(packet[3] & 0x04, 0x04); // Enabled
    assert_eq!(packet[4], 12); // Integer volts
    assert_eq!(packet[5], 128); // 0.5 * 256 = 128 fractional volts
}
