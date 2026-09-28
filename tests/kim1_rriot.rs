//! 6530 RRIOT timer and ports over every divider, start value and register offset.

use mos6502::bus::IoDevice;
use mos6502::systems::kim1::Rriot6530;

#[test]
fn every_divider_and_start_value_times_out_on_schedule() {
    for (offset, div) in [(4u16, 1u32), (5, 8), (6, 64), (7, 1024)] {
        for start in [0u8, 1, 2, 0x10, 0x7F, 0xFF] {
            let mut r = Rriot6530::new();
            r.write(offset, start);
            let mut ticks = 0u32;
            while r.peek(0x05) & 0x80 == 0 {
                r.tick();
                ticks += 1;
            }
            assert_eq!(
                ticks,
                (u32::from(start) + 1) * div,
                "div {div} start {start}"
            );
            assert_eq!(r.peek(0x06), 0xFF);
            r.tick();
            r.tick();
            assert_eq!(
                r.peek(0x06),
                0xFD,
                "free run at one per cycle after timeout"
            );
            assert!(!r.irq_asserted(), "IRQ disabled by A3 = 0");
            r.read(0x0E); // read counter with A3 = 1: clears flag, enables IRQ
            assert_eq!(r.peek(0x05), 0);
        }
    }
}

#[test]
fn irq_enable_follows_a3_of_the_timer_access() {
    let mut r = Rriot6530::new();
    r.write(0x0C, 0);
    r.tick();
    assert!(r.irq_asserted());
    r.read(0x06);
    assert!(!r.irq_asserted() && !r.timer_flag);
}

#[test]
fn ports_mix_latch_and_pins_for_every_ddr_and_ram_is_64_bytes() {
    for ddr in 0..=255u8 {
        let mut r = Rriot6530::new();
        r.write(1, ddr);
        r.write(0, 0xA5);
        r.pins_a = 0x3C;
        assert_eq!(r.read(0), (0xA5 & ddr) | (0x3C & !ddr));
        r.write(3, ddr);
        r.write(2, 0x5A);
        r.pins_b = 0xC3;
        assert_eq!(r.read(2), (0x5A & ddr) | (0xC3 & !ddr));
        assert_eq!(r.display.data, 0xA5);
    }
    let mut r = Rriot6530::new();
    for i in 0..64u16 {
        r.write(0x40 + i, i as u8 ^ 0x55);
    }
    for i in 0..64u16 {
        assert_eq!(r.read(0x40 + i), i as u8 ^ 0x55);
    }
}
