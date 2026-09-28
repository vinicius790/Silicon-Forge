//! RES pin and next-interrupt scheduling.

use mos6502::via::Via6522;

#[test]
fn reset_keeps_timers_latches_and_sr_only() {
    for seed in 0..=255u8 {
        let mut v = Via6522::new();
        v.write(0x02, seed);
        v.write(0x03, !seed);
        v.write(0x00, seed ^ 0x5A);
        v.write(0x01, seed ^ 0xA5);
        v.write(0x0C, seed);
        v.write(0x0B, 0x08 | (seed & 0xE0));
        v.write(0x0A, seed.rotate_left(3));
        v.write(0x0E, 0xFF);
        v.write(0x04, seed);
        v.write(0x05, 0x01);
        v.write(0x08, !seed);
        v.write(0x09, 0x02);
        v.set_port_a_in(seed);
        for _ in 0..u32::from(seed) {
            v.tick();
        }
        let (t1, t2, sr) = (v.t1_counter(), v.t2_counter(), v.sr());
        let latch = (v.peek(0x06), v.peek(0x07));
        v.reset();
        assert_eq!(
            (v.t1_counter(), v.t2_counter(), v.sr()),
            (t1, t2, sr),
            "{seed}"
        );
        assert_eq!((v.peek(0x06), v.peek(0x07)), latch);
        for reg in [0x02, 0x03, 0x0B, 0x0C, 0x0D] {
            assert_eq!(v.peek(reg), 0, "reg {reg:X} seed {seed}");
        }
        assert_eq!(v.peek(0x0E), 0x80, "IER reads bit 7 set");
        assert!(!v.irq_asserted() && !v.t1_running() && !v.t2_running());
        assert_eq!(v.port_a_pins(), seed, "inputs follow the host pins");
    }
}

#[test]
fn ticks_until_irq_is_linear_in_the_latch_and_matches_ticking() {
    let mut offset = None;
    for n in (0..=0x3000u16).step_by(37) {
        let mut v = Via6522::new();
        v.write(0x0E, 0xC0);
        v.write(0x04, n as u8);
        v.write(0x05, (n >> 8) as u8);
        let k = v.ticks_until_irq(0x10000).unwrap();
        let c = *offset.get_or_insert(i64::from(k) - i64::from(n));
        assert_eq!(i64::from(k) - i64::from(n), c, "{n}");
        for _ in 0..k - 1 {
            v.tick();
        }
        assert!(!v.irq_asserted());
        assert_eq!(v.ticks_until_irq(5), Some(1));
        v.tick();
        assert_eq!(v.ticks_until_irq(0), Some(0));
    }
    assert!(
        (1..=2).contains(&offset.unwrap()),
        "N + 1 or N + 2 cycles: {offset:?}"
    );
    // No enabled source: none within the limit; disabled flag never asserts.
    let mut v = Via6522::new();
    v.write(0x04, 10);
    v.write(0x05, 0);
    assert_eq!(v.ticks_until_irq(1000), None);
    v.write(0x0E, 0xA0);
    v.write(0x08, 3);
    v.write(0x09, 0);
    assert!(v.ticks_until_irq(10).is_some(), "timer 2");
}
