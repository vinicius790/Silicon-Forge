//! 6522 VIA: PB7 timer output, input latching, shift-register modes, IFR/IER edge
//! cases and part variants, checked against the datasheet behavior.

use mos6502::via::{IrqOutput, Via6522, ViaVariant};

const ORB: u8 = 0x0;
const ORA: u8 = 0x1;
const DDRB: u8 = 0x2;
const DDRA: u8 = 0x3;
const T1CL: u8 = 0x4;
const T1CH: u8 = 0x5;
const T2CL: u8 = 0x8;
const SR: u8 = 0xA;
const ACR: u8 = 0xB;
const PCR: u8 = 0xC;
const IFR: u8 = 0xD;
const IER: u8 = 0xE;
const ORA_NH: u8 = 0xF;

fn ticks(v: &mut Via6522, n: u32) {
    for _ in 0..n {
        v.tick();
    }
}

fn start_t1(v: &mut Via6522, acr: u8, n: u16) {
    v.write(ACR, acr);
    v.write(T1CL, n as u8);
    v.write(T1CH, (n >> 8) as u8);
}

// ---- PB7 -------------------------------------------------------------------

#[test]
fn pb7_is_an_ordinary_port_bit_while_acr7_is_clear() {
    let mut v = Via6522::new();
    v.write(DDRB, 0x80);
    v.write(ORB, 0x80);
    start_t1(&mut v, 0x00, 3);
    assert_eq!(v.pb7_output(), None);
    assert_eq!(v.port_b_pins() & 0x80, 0x80);
    ticks(&mut v, 10);
    assert_eq!(
        v.port_b_pins() & 0x80,
        0x80,
        "timeout leaves ORB bit 7 alone"
    );
}

#[test]
fn pb7_one_shot_is_a_single_low_pulse_of_n_plus_one_and_a_half_cycles() {
    for n in [0u16, 1, 2, 5, 100, 1000] {
        let mut v = Via6522::new();
        v.write(ACR, 0x80);
        assert_eq!(v.pb7_output(), Some(true), "idle high once enabled");
        start_t1(&mut v, 0x80, n);
        assert_eq!(v.pb7_output(), Some(false), "T1C-H write drives PB7 low");
        let mut low = 0;
        for _ in 0..u32::from(n) + 10 {
            v.tick();
            if v.pb7_output() == Some(false) {
                low += 1;
            }
        }
        assert_eq!(
            low,
            u32::from(n),
            "low for N whole ticks after the write, n={n}"
        );
        assert_eq!(v.pb7_output(), Some(true));
        ticks(&mut v, 70_000);
        assert_eq!(v.pb7_output(), Some(true), "one-shot never pulses again");
    }
}

#[test]
fn pb7_one_shot_retriggers_on_each_t1ch_write() {
    let mut v = Via6522::new();
    start_t1(&mut v, 0x80, 4);
    ticks(&mut v, 2);
    v.write(T1CH, 0); // reload 4 before timeout: pulse stretches
    ticks(&mut v, 4);
    assert_eq!(v.pb7_output(), Some(false));
    v.tick();
    assert_eq!(v.pb7_output(), Some(true));
}

#[test]
fn pb7_free_run_square_wave_has_period_two_n_plus_four() {
    for n in [1u16, 3, 10, 50] {
        let mut v = Via6522::new();
        start_t1(&mut v, 0xC0, n);
        let mut edges = Vec::new();
        let mut last = v.pb7_output();
        for t in 1..=(6 * (u32::from(n) + 2)) {
            v.tick();
            let now = v.pb7_output();
            if now != last {
                edges.push(t);
            }
            last = now;
        }
        assert_eq!(edges[0], u32::from(n) + 1, "first edge n={n}");
        for w in edges.windows(2) {
            assert_eq!(w[1] - w[0], u32::from(n) + 2, "half period n={n}");
        }
    }
}

#[test]
fn pb7_overrides_ddrb_and_orb_in_the_pin_read() {
    let mut v = Via6522::new();
    v.write(DDRB, 0x00);
    v.drive_port_b(0xFF, 0xFF);
    start_t1(&mut v, 0x80, 10);
    assert_eq!(
        v.port_b_pins(),
        0x7F,
        "PB7 low even though the pin is an input"
    );
    assert_eq!(v.read(ORB) & 0x80, 0);
    v.write(ACR, 0x00);
    assert_eq!(v.port_b_pins(), 0xFF, "released back to the port");
}

#[test]
fn reset_returns_pb7_to_the_port() {
    let mut v = Via6522::new();
    start_t1(&mut v, 0xC0, 2);
    v.reset();
    assert_eq!(v.pb7_output(), None);
    v.write(ACR, 0x80);
    assert_eq!(v.pb7_output(), Some(true));
}

// ---- input latching ---------------------------------------------------------

#[test]
fn pa_latch_captures_on_the_selected_ca1_edge_only() {
    for rising in [false, true] {
        let mut v = Via6522::new();
        v.write(PCR, u8::from(rising));
        v.write(ACR, 0x01);
        v.set_ca1(!rising);
        v.set_port_a_in(0x11);
        v.set_ca1(rising); // active edge
        v.set_port_a_in(0x22);
        assert_eq!(v.read(ORA_NH), 0x11);
        v.set_ca1(!rising); // inactive edge
        assert_eq!(v.read(ORA_NH), 0x11, "inactive edge does not latch");
        v.set_port_a_in(0x33);
        v.set_ca1(rising);
        assert_eq!(v.read(ORA), 0x33);
    }
}

#[test]
fn pb_latch_keeps_output_bits_live_and_latches_inputs() {
    let mut v = Via6522::new();
    v.write(ACR, 0x02);
    v.write(DDRB, 0xF0);
    v.write(ORB, 0xA0);
    v.set_port_b_in(0x05);
    v.set_cb1(false);
    v.set_port_b_in(0x0A);
    assert_eq!(v.read(ORB), 0xA5);
    v.write(ORB, 0x50);
    assert_eq!(v.read(ORB), 0x55, "outputs follow ORB, inputs stay latched");
}

#[test]
fn disabling_latching_returns_live_pins_and_reenabling_shows_the_old_latch() {
    let mut v = Via6522::new();
    v.write(ACR, 0x01);
    v.set_port_a_in(0x42);
    v.set_ca1(false);
    v.set_port_a_in(0x99);
    v.write(ACR, 0x00);
    assert_eq!(v.read(ORA_NH), 0x99);
    v.write(ACR, 0x01);
    assert_eq!(v.read(ORA_NH), 0x42);
}

#[test]
fn ira_latch_records_output_pins_too_on_port_a() {
    let mut v = Via6522::new();
    v.write(ACR, 0x01);
    v.write(DDRA, 0xFF);
    v.write(ORA, 0x5A);
    v.set_ca1(false);
    v.write(ORA, 0x00);
    assert_eq!(v.read(ORA_NH), 0x5A);
}

// ---- shift register --------------------------------------------------------

fn shift_out_phi2(byte: u8) -> (Via6522, Vec<bool>) {
    let mut v = Via6522::new();
    v.write(ACR, 0b110 << 2);
    v.write(SR, byte);
    let mut bits = Vec::new();
    for _ in 0..8 {
        v.tick();
        bits.push(v.cb2_shift_data().unwrap());
    }
    (v, bits)
}

#[test]
fn mode_110_puts_each_bit_on_cb2_msb_first_one_per_cycle() {
    for byte in [0x00u8, 0xFF, 0xA5, 0x81, 0x3C] {
        let (mut v, bits) = shift_out_phi2(byte);
        let expect: Vec<bool> = (0..8).rev().map(|i| (byte >> i) & 1 != 0).collect();
        assert_eq!(bits, expect);
        assert_eq!(v.read(IFR) & 0x04, 0x04);
        assert_eq!(v.sr_shifted_out(), byte);
    }
}

#[test]
fn cb2_shift_data_is_none_in_input_and_disabled_modes() {
    for mode in 0u8..4 {
        let mut v = Via6522::new();
        v.write(ACR, mode << 2);
        assert_eq!(v.cb2_shift_data(), None, "mode {mode:03b}");
    }
    for mode in 4u8..8 {
        let mut v = Via6522::new();
        v.write(ACR, mode << 2);
        assert!(v.cb2_shift_data().is_some(), "mode {mode:03b}");
    }
}

#[test]
fn mode_010_shift_in_phi2_assembles_any_byte_in_eight_cycles() {
    for byte in [0x00u8, 0xFF, 0x96, 0x01, 0x80] {
        let mut v = Via6522::new();
        v.write(ACR, 0b010 << 2);
        v.write(SR, 0);
        for i in (0..8).rev() {
            v.set_sr_in_bit((byte >> i) & 1 != 0);
            v.tick();
        }
        assert_eq!(v.peek(IFR) & 0x04, 0x04);
        assert_eq!(v.read(SR), byte);
        assert_eq!(v.peek(IFR) & 0x04, 0, "SR read clears IFR2");
        assert_eq!(v.sr_bits_left(), 8, "SR read rearms shift-in");
    }
}

#[test]
fn mode_011_external_cb1_shift_in_samples_on_rising_edges_only() {
    let byte = 0xC3u8;
    let mut v = Via6522::new();
    v.write(ACR, 0b011 << 2);
    v.write(SR, 0);
    ticks(&mut v, 100);
    assert_eq!(v.sr_bits_left(), 8, "PHI2 does not clock mode 011");
    for i in (0..8).rev() {
        v.set_sr_in_bit((byte >> i) & 1 != 0);
        v.set_cb1(false);
        assert_eq!(v.sr_bits_left(), 8 - (7 - i), "falling edge does not shift");
        v.set_cb1(true);
    }
    assert_eq!(v.sr(), byte);
    assert_eq!(v.peek(IFR) & 0x04, 0x04);
}

#[test]
fn mode_011_keeps_shifting_past_eight_pulses_without_counting() {
    let mut v = Via6522::new();
    v.write(ACR, 0b011 << 2);
    v.write(SR, 0);
    v.set_sr_in_bit(true);
    for _ in 0..8 {
        v.set_cb1(false);
        v.set_cb1(true);
    }
    assert_eq!(v.sr(), 0xFF);
    v.set_sr_in_bit(false);
    v.set_cb1(false);
    v.set_cb1(true);
    assert_eq!(
        v.sr(),
        0xFE,
        "ninth pulse still shifts (datasheet: counter does not stop shifting)"
    );
    assert_eq!(v.sr_bits_left(), 0, "but is not counted");
    assert_eq!(v.read(SR), 0xFE);
    assert_eq!(v.sr_bits_left(), 8, "SR access re-arms eight");
    assert_eq!(v.peek(IFR) & 0x04, 0);
}

#[test]
fn mode_111_shifts_out_on_falling_cb1_edges_only() {
    for byte in [0x00u8, 0xFF, 0xA5, 0x3C, 0x81] {
        let mut v = Via6522::new();
        v.write(ACR, 0b111 << 2);
        v.write(SR, byte);
        let mut bits = Vec::new();
        for _ in 0..8 {
            v.set_cb1(true);
            v.set_cb1(true);
            let left = v.sr_bits_left();
            v.set_cb1(false);
            assert_eq!(v.sr_bits_left(), left - 1);
            bits.push(v.cb2_shift_data().unwrap());
            v.set_cb1(true); // rising edge: no shift
            assert_eq!(v.sr_bits_left(), left - 1);
        }
        let expect: Vec<bool> = (0..8).rev().map(|i| (byte >> i) & 1 != 0).collect();
        assert_eq!(bits, expect);
        assert_eq!(v.sr_shifted_out(), byte);
        assert_eq!(v.peek(IFR) & 0x04, 0x04);
    }
}

#[test]
fn mode_111_needs_eight_pulses_not_eight_edges() {
    let mut v = Via6522::new();
    v.write(ACR, 0b111 << 2);
    v.write(SR, 0xA5);
    let mut level = true;
    for _ in 0..8 {
        level = !level;
        v.set_cb1(level);
    }
    assert_eq!(v.sr_bits_left(), 4, "eight edges are four pulses");
    assert_eq!(v.peek(IFR) & 0x04, 0);
    for _ in 0..8 {
        level = !level;
        v.set_cb1(level);
    }
    assert_eq!(v.sr_bits_left(), 0);
    assert_eq!(v.sr_shifted_out(), 0xA5);
}

#[test]
fn mode_111_extra_pulses_shift_uncounted_and_an_sr_write_rearms() {
    let mut v = Via6522::new();
    v.write(ACR, 0b111 << 2);
    v.write(SR, 0xF0);
    for _ in 0..10 {
        v.set_cb1(false);
        v.set_cb1(true);
    }
    assert_eq!(
        v.sr_shifted_out(),
        0xC0,
        "two more bits out after the byte (zeros)"
    );
    assert_eq!(v.peek(IFR) & 0x04, 0x04);
    v.write(SR, 0x0F);
    assert_eq!(v.peek(IFR) & 0x04, 0);
    assert_eq!(v.sr_bits_left(), 8);
}

#[test]
fn mode_101_flag_waits_for_cb1_to_return_high_then_cb1_stays_high() {
    let mut v = Via6522::new();
    v.write(0x8, 3);
    v.write(ACR, 0b101 << 2);
    v.write(SR, 0x55);
    ticks(&mut v, 15 * 5);
    assert_eq!(v.sr_bits_left(), 0, "eighth bit out at 15(N+2)");
    assert_eq!(v.peek(IFR) & 0x04, 0);
    assert!(!v.sr_clock_high());
    ticks(&mut v, 4);
    assert_eq!(v.peek(IFR) & 0x04, 0);
    v.tick();
    assert_eq!(v.peek(IFR) & 0x04, 0x04, "flag at 16(N+2)");
    assert!(v.sr_clock_high());
    ticks(&mut v, 100);
    assert!(v.sr_clock_high(), "CB1 stays high after the byte");
    assert_eq!(v.sr_shifted_out(), 0x55);
}

#[test]
fn mode_001_flag_also_lands_at_sixteen_half_periods() {
    for n in [0u8, 2, 9] {
        let mut v = Via6522::new();
        v.write(0x8, n);
        v.write(ACR, 0b001 << 2);
        v.write(SR, 0);
        let total = 16 * (u32::from(n) + 2);
        ticks(&mut v, total - 1);
        assert_eq!(v.peek(IFR) & 0x04, 0, "n={n}");
        v.tick();
        assert_eq!(v.peek(IFR) & 0x04, 0x04, "n={n}");
    }
}

#[test]
fn t2_rate_modes_take_two_n_plus_four_cycles_per_bit() {
    for n in [0u8, 1, 4, 20] {
        // 101: CB1 starts high and toggles every N+2 ticks; bits leave on falling
        // edges at (2k-1)(N+2) and IFR2 sets on the rising edge at 16(N+2).
        let mut v = Via6522::new();
        v.write(T2CL, n);
        v.write(ACR, 0b101 << 2);
        v.write(SR, 0xFF);
        let half = u32::from(n) + 2;
        let mut out_at = Vec::new();
        let mut t = 0;
        while v.peek(IFR) & 0x04 == 0 {
            let before = v.sr_bits_left();
            v.tick();
            t += 1;
            if v.sr_bits_left() != before {
                out_at.push(t);
            }
            assert!(t < 10_000);
        }
        let want: Vec<u32> = (1..=8).map(|k| (2 * k - 1) * half).collect();
        assert_eq!(out_at, want, "n={n}");
        assert_eq!(t, 16 * half, "n={n}");
        assert!(v.sr_clock_high());
        assert_eq!(v.sr_shifted_out(), 0xFF);
    }
}

#[test]
fn mode_100_free_run_recirculates_forever_without_a_flag() {
    let mut v = Via6522::new();
    v.write(T2CL, 0);
    v.write(ACR, 0b100 << 2);
    v.write(SR, 0x81);
    ticks(&mut v, 4 * 64);
    assert_eq!(v.sr(), 0x81, "after 64 bits the byte is back in place");
    assert_eq!(v.peek(IFR) & 0x04, 0);
}

#[test]
fn mode_001_shift_in_at_t2_rate_samples_on_the_rising_cb1() {
    let byte = 0x5Au8;
    let mut v = Via6522::new();
    v.write(T2CL, 2);
    v.write(ACR, 0b001 << 2);
    v.write(SR, 0);
    let mut got = 0;
    while v.sr_bits_left() > 0 {
        let bit = 7 - (8 - v.sr_bits_left());
        v.set_sr_in_bit((byte >> bit) & 1 != 0);
        v.tick();
        got += 1;
        assert!(got < 1000);
    }
    assert_eq!(v.sr(), byte);
    assert_eq!(got, 8 * 2 * 4);
}

#[test]
fn mode_000_and_sr_access_never_set_ifr2() {
    let mut v = Via6522::new();
    v.write(SR, 0x12);
    ticks(&mut v, 50);
    let _ = v.read(SR);
    assert_eq!(v.read(IFR) & 0x04, 0);
}

// ---- IFR / IER ---------------------------------------------------------------

#[test]
fn ier_set_and_clear_only_touch_the_named_bits_and_read_bit7_as_one() {
    let mut v = Via6522::new();
    v.write(IER, 0x80 | 0x7F);
    assert_eq!(v.read(IER), 0xFF);
    v.write(IER, 0x41); // clear T1 and CA2
    assert_eq!(v.read(IER), 0xBE);
    v.write(IER, 0x80); // set nothing
    assert_eq!(v.read(IER), 0xBE);
    v.write(IER, 0x00); // clear nothing
    assert_eq!(v.read(IER), 0xBE);
}

#[test]
fn ifr_bit7_is_the_or_of_enabled_flags_and_writing_it_does_nothing() {
    let mut v = Via6522::new();
    v.set_ca1(false);
    assert_eq!(v.read(IFR), 0x02, "flag without IER: no summary bit");
    assert!(!v.irq_asserted());
    v.write(IER, 0x82);
    assert_eq!(v.read(IFR), 0x82);
    v.write(IFR, 0x80);
    assert_eq!(v.read(IFR), 0x82, "bit 7 cannot be cleared directly");
    v.write(IFR, 0x7F);
    assert_eq!(v.read(IFR), 0x00);
}

#[test]
fn disabling_ier_drops_irq_but_keeps_the_flag() {
    let mut v = Via6522::new();
    v.write(IER, 0xC0);
    start_t1(&mut v, 0, 1);
    ticks(&mut v, 2);
    assert!(v.irq_asserted());
    v.write(IER, 0x40);
    assert!(!v.irq_asserted());
    assert_eq!(v.read(IFR), 0x40);
    v.write(IER, 0xC0);
    assert!(
        v.irq_asserted(),
        "re-enabling an already set flag asserts IRQ"
    );
}

#[test]
fn every_flag_has_its_own_clear_path() {
    let mut v = Via6522::new();
    // T1 by T1C-L read, T2 by T2C-L read.
    start_t1(&mut v, 0, 1);
    v.write(T2CL, 1);
    v.write(0x9, 0);
    ticks(&mut v, 3);
    assert_eq!(v.peek(IFR) & 0x60, 0x60);
    let _ = v.read(T1CL);
    assert_eq!(v.peek(IFR) & 0x60, 0x20);
    let _ = v.read(T2CL);
    assert_eq!(v.peek(IFR) & 0x60, 0);
    // CA1 by ORA access but not by the no-handshake mirror.
    v.set_ca1(false);
    let _ = v.read(ORA_NH);
    assert_eq!(v.peek(IFR) & 0x02, 0x02);
    let _ = v.read(ORA);
    assert_eq!(v.peek(IFR) & 0x02, 0);
    // CB1 by ORB write.
    v.set_cb1(false);
    v.write(ORB, 0);
    assert_eq!(v.peek(IFR) & 0x10, 0);
}

#[test]
fn independent_ca2_flag_survives_port_access() {
    let mut v = Via6522::new();
    v.write(PCR, 0b001 << 1); // CA2 independent negative edge
    v.set_ca2(false);
    let _ = v.read(ORA);
    assert_eq!(v.peek(IFR) & 0x01, 0x01);
    v.write(IFR, 0x01);
    assert_eq!(v.peek(IFR) & 0x01, 0);
}

// ---- variants -----------------------------------------------------------------

#[test]
fn variants_differ_only_in_the_irq_output_stage() {
    for (variant, stage, cmos) in [
        (ViaVariant::Mos6522, IrqOutput::OpenDrain, false),
        (ViaVariant::RockwellR6522, IrqOutput::OpenDrain, false),
        (ViaVariant::WdcW65c22N, IrqOutput::OpenDrain, true),
        (ViaVariant::WdcW65c22S, IrqOutput::TotemPole, true),
    ] {
        assert_eq!(variant.irq_output(), stage);
        assert_eq!(variant.is_cmos(), cmos);
        let mut v = Via6522::with_variant(variant);
        assert_eq!(v.variant(), variant);
        let idle = v.irq_pin_drive();
        assert_eq!(idle, (stage == IrqOutput::TotemPole).then_some(true));
        v.write(IER, 0x82);
        v.set_ca1(false);
        assert_eq!(v.irq_pin_drive(), Some(false));
        v.reset();
        assert_eq!(v.variant(), variant, "RES keeps the part");
    }
    assert_eq!(ViaVariant::default(), ViaVariant::Mos6522);
}

#[test]
fn variants_run_identical_register_sequences() {
    let script: Vec<(u8, u8)> = (0..200u32)
        .map(|i| (((i * 7) % 16) as u8, (i * 31 % 256) as u8))
        .collect();
    let mut reference = None;
    for variant in [
        ViaVariant::Mos6522,
        ViaVariant::RockwellR6522,
        ViaVariant::WdcW65c22N,
        ViaVariant::WdcW65c22S,
    ] {
        let mut v = Via6522::with_variant(variant);
        let mut trace = Vec::new();
        for &(r, val) in &script {
            v.write(r, val);
            v.tick();
            trace.push((v.read(r), v.peek(IFR), v.port_b_pins()));
        }
        match &reference {
            None => reference = Some(trace),
            Some(t) => assert_eq!(&trace, t, "{variant:?}"),
        }
    }
}
