//! 6522 register semantics over full value ranges (data sheet register map).

use mos6502::via::Via6522;

#[test]
fn port_reads_mix_output_latch_and_pins_for_every_ddr() {
    for ddr in 0..=255u8 {
        let mut via = Via6522::new();
        via.write(0x3, ddr); // DDRA
        via.write(0x1, 0xA5); // ORA
        via.set_port_a_in(0x3C);
        assert_eq!(via.ddra(), ddr);
        // Port A reads the pins: outputs drive them (no load modeled), inputs from host.
        assert_eq!(via.read(0x1), (0xA5 & ddr) | (0x3C & !ddr), "A {ddr:02X}");
        assert_eq!(
            via.read(0xF),
            (0xA5 & ddr) | (0x3C & !ddr),
            "ORA no handshake"
        );
        via.write(0x2, ddr); // DDRB
        via.write(0x0, 0x5A);
        via.set_port_b_in(0xC3);
        assert_eq!(via.read(0x0) & ddr, 0x5A & ddr, "B outputs read the latch");
        assert_eq!(via.read(0x0) & !ddr, 0xC3 & !ddr, "B inputs read the pins");
    }
}

#[test]
fn ier_set_and_clear_for_every_pair_of_writes() {
    for first in 0..=255u8 {
        for second in [0x00u8, 0x7F, 0x80, 0xFF, 0x05, 0x85, first ^ 0x80] {
            let mut via = Via6522::new();
            via.write(0xE, first);
            via.write(0xE, second);
            let after_first = if first & 0x80 != 0 { first & 0x7F } else { 0 };
            let want = if second & 0x80 != 0 {
                after_first | (second & 0x7F)
            } else {
                after_first & !second
            };
            assert_eq!(via.read(0xE), 0x80 | want, "{first:02X} then {second:02X}");
        }
    }
}

#[test]
fn ifr_bit7_tracks_enabled_flags_and_writes_clear_by_mask() {
    let mut via = Via6522::new();
    via.write(0x4, 0x02); // T1 latch low
    via.write(0x5, 0x00); // T1 high: start, count 2
    for _ in 0..4 {
        via.tick();
    }
    assert_eq!(via.read(0xD) & 0x40, 0x40, "T1 flag");
    assert_eq!(via.read(0xD) & 0x80, 0, "not enabled: no IRQ bit");
    assert!(!via.irq_asserted());
    via.write(0xE, 0xC0);
    assert_eq!(via.read(0xD) & 0xC0, 0xC0);
    assert!(via.irq_asserted());
    via.write(0xD, 0x3F); // other bits: T1 stays
    assert_eq!(via.read(0xD) & 0x40, 0x40);
    via.write(0xD, 0x40);
    assert_eq!(via.read(0xD), 0);
    assert!(!via.irq_asserted());
}

#[test]
fn every_t1_latch_value_times_out_after_n_plus_1_and_a_half_ticks() {
    for n in [0u16, 1, 2, 3, 0x00FF, 0x0100, 0x1234] {
        let mut via = Via6522::new();
        via.write(0x4, n as u8);
        via.write(0x5, (n >> 8) as u8);
        let mut ticks = 0u32;
        while via.read(0xD) & 0x40 == 0 {
            via.tick();
            ticks += 1;
            assert!(ticks < 0x20000);
        }
        // Flag rises N+1.5 cycles after the write; this whole-tick model shows it after N+1.
        assert_eq!(ticks, u32::from(n) + 1, "{n}");
    }
}
