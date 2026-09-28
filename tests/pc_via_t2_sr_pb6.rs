//! 6522 data sheet edge cases: T2 one-shot reload rules, SR mode 000 and the IFR2
//! flag, PB6 pulse counting edges.

use mos6502::via::Via6522;

const T2CL: u8 = 8;
const T2CH: u8 = 9;
const SR: u8 = 10;
const ACR: u8 = 11;
const IFR: u8 = 13;
const IER: u8 = 14;

fn ticks(v: &mut Via6522, n: u32) {
    for _ in 0..n {
        v.tick();
    }
}

#[test]
fn t2_low_latch_write_does_not_touch_the_counter_or_flag() {
    for n in [0u16, 1, 2, 0x80, 0x1234, 0xFFFF] {
        let mut v = Via6522::new();
        v.write(T2CL, n as u8);
        v.write(T2CH, (n >> 8) as u8);
        let c = v.t2_counter();
        v.write(T2CL, 0x55);
        assert_eq!(v.t2_counter(), c, "{n:04X}");
        assert!(!v.t2_flag());
    }
}

#[test]
fn t2_one_shot_flags_once_then_keeps_counting_without_reload() {
    for n in [0u16, 1, 3, 10, 200] {
        let mut v = Via6522::new();
        v.write(T2CL, n as u8);
        v.write(T2CH, (n >> 8) as u8);
        assert_eq!(v.t2_counter(), n, "high write transfers the low latch");
        ticks(&mut v, u32::from(n) + 1);
        assert!(v.t2_flag(), "flag after N+1 ticks, N={n}");
        assert_eq!(v.t2_counter(), 0xFFFF, "rolls, does not reload the latch");
        v.read(T2CL); // clears the flag
        assert!(!v.t2_flag());
        ticks(&mut v, 0x1_0000 + 5);
        assert!(
            !v.t2_flag(),
            "one shot: no second flag without a T2C-H write"
        );
    }
}

#[test]
fn t2_high_write_rearms_and_clears_the_flag_for_every_high_byte() {
    for hi in 0..=255u8 {
        let mut v = Via6522::new();
        v.write(T2CL, 2);
        v.write(T2CH, 0);
        ticks(&mut v, 3);
        assert!(v.t2_flag());
        v.write(T2CH, hi);
        assert!(!v.t2_flag(), "T2C-H write clears IFR5");
        assert_eq!(v.t2_counter(), u16::from_le_bytes([2, hi]));
        assert!(v.t2_running());
    }
}

#[test]
fn reading_t2_high_does_not_clear_the_flag_but_low_does() {
    let mut v = Via6522::new();
    v.write(T2CL, 0);
    v.write(T2CH, 0);
    ticks(&mut v, 1);
    assert!(v.t2_flag());
    v.read(T2CH);
    assert!(v.t2_flag());
    v.read(T2CL);
    assert!(!v.t2_flag());
}

#[test]
fn pb6_counts_only_falling_edges_and_only_in_pulse_mode() {
    let mut v = Via6522::new();
    v.write(ACR, 0x20);
    v.write(T2CL, 5);
    v.write(T2CH, 0);
    let mut level = true;
    v.set_pb6(true);
    // Rising edges and repeated levels never count.
    for _ in 0..10 {
        v.set_pb6(true);
    }
    assert_eq!(v.t2_counter(), 5);
    for expected in (0..5u16).rev() {
        level = !level;
        v.set_pb6(level); // falling
        assert_eq!(v.t2_counter(), expected);
        assert_eq!(
            v.t2_flag(),
            expected == 0,
            "flag exactly when the count reaches 0"
        );
        level = !level;
        v.set_pb6(level); // rising
        assert_eq!(v.t2_counter(), expected);
    }
    // Phi2 does not decrement in pulse mode.
    ticks(&mut v, 100);
    assert_eq!(v.t2_counter(), 0);
}

#[test]
fn pb6_after_zero_keeps_counting_without_a_second_flag() {
    let mut v = Via6522::new();
    v.write(ACR, 0x20);
    v.write(T2CL, 1);
    v.write(T2CH, 0);
    v.set_pb6(true);
    v.set_pb6(false);
    assert!(v.t2_flag());
    v.read(T2CL);
    for k in 1..=300u32 {
        v.set_pb6(true);
        v.set_pb6(false);
        assert_eq!(v.t2_counter(), 0u16.wrapping_sub(k as u16));
        assert!(!v.t2_flag(), "pulse {k}");
    }
}

#[test]
fn pb6_ignored_in_timed_mode() {
    let mut v = Via6522::new();
    v.write(T2CL, 0x40);
    v.write(T2CH, 0);
    for _ in 0..10 {
        v.set_pb6(false);
        v.set_pb6(true);
    }
    assert_eq!(v.t2_counter(), 0x40);
}

#[test]
fn pb6_edge_timing_is_independent_of_phi2_phase() {
    for gap in 0..20u32 {
        let mut v = Via6522::new();
        v.write(ACR, 0x20);
        v.write(T2CL, 3);
        v.write(T2CH, 0);
        v.set_pb6(true);
        for _ in 0..3 {
            ticks(&mut v, gap);
            v.set_pb6(false);
            ticks(&mut v, gap);
            v.set_pb6(true);
        }
        assert!(v.t2_flag(), "gap {gap}");
        assert_eq!(v.t2_counter(), 0);
    }
}

#[test]
fn sr_mode_000_never_shifts_and_never_sets_ifr2() {
    for value in 0..=255u8 {
        let mut v = Via6522::new();
        v.write(ACR, 0x00);
        v.write(SR, value);
        v.set_sr_in_bit(true);
        for k in 0..64 {
            v.set_cb1(k % 2 == 0);
            v.tick();
        }
        assert_eq!(v.read(SR), value, "SR holds in mode 000");
        assert_eq!(v.read(IFR) & 0x04, 0);
        assert_eq!(v.sr_bits_left(), 0);
    }
}

#[test]
fn switching_to_mode_000_stops_a_shift_and_keeps_an_earlier_flag_until_sr_access() {
    let mut v = Via6522::new();
    v.write(ACR, 0x18); // 110: shift out under PHI2
    v.write(SR, 0x81);
    ticks(&mut v, 8);
    assert_ne!(v.read(IFR) & 0x04, 0, "eight bits shifted");
    v.write(ACR, 0x00);
    ticks(&mut v, 50);
    assert_ne!(v.read(IFR) & 0x04, 0, "mode 000 does not clear IFR2");
    v.read(SR);
    assert_eq!(v.read(IFR) & 0x04, 0, "an SR access clears it");
    // Mid-shift switch: no further bits move.
    v.write(ACR, 0x18);
    v.write(SR, 0xF0);
    ticks(&mut v, 3);
    let left = v.sr_bits_left();
    let sr = v.sr();
    v.write(ACR, 0x00);
    ticks(&mut v, 40);
    assert_eq!((v.sr_bits_left(), v.sr()), (left, sr));
    assert_eq!(v.read(IFR) & 0x04, 0);
}

#[test]
fn ifr2_drives_irq_only_when_ier2_is_set() {
    for ier in [0x00u8, 0x04] {
        let mut v = Via6522::new();
        v.write(IER, 0x7F);
        v.write(IER, 0x80 | ier);
        v.write(ACR, 0x18);
        v.write(SR, 0x00);
        ticks(&mut v, 8);
        assert_ne!(v.read(IFR) & 0x04, 0);
        assert_eq!(v.irq_asserted(), ier != 0);
        assert_eq!(v.read(IFR) & 0x80 != 0, ier != 0, "IFR7 summary");
    }
}
