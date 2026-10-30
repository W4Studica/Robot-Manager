use robot_manager::irqconf::{configure_irq_affinity, NETWORK_CPU_MASK};

#[test]
fn configure_irq_affinity_dry_run_parses_interrupts() {
    let pinned = configure_irq_affinity(true).unwrap();
    assert_eq!(NETWORK_CPU_MASK, "3");
    // Should complete cleanly on any Linux system with /proc/interrupts
    for p in pinned {
        assert!(p.irq > 0);
        assert!(!p.name.is_empty());
    }
}
