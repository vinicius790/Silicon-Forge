//! What differs between the 6530 (KIM-1 RRIOT) and 6532 (RIOT) models, side by side.

use mos6502::bus::IoDevice;
use mos6502::pia::Riot6532;
use mos6502::systems::kim1::Rriot6530;

fn timeout_6530(start: u8, div: usize) -> u32 {
    let mut r = Rriot6530::new();
    r.write(4 + div as u16, start);
    let mut n = 0;
    while r.peek(0x05) & 0x80 == 0 {
        r.tick();
        n += 1;
    }
    n
}

fn timeout_6532(start: u8, div: usize) -> u32 {
    let mut r = Riot6532::new();
    r.write(0x94 + div as u16, start);
    let mut n = 0;
    while r.peek(0x85) & 0x80 == 0 {
        r.tick();
        n += 1;
    }
    n
}

#[test]
fn first_decrement_after_a_full_interval_vs_one_clock() {
    for (i, d) in [1u32, 8, 64, 1024].into_iter().enumerate() {
        for start in [0u8, 1, 2, 0x55, 0xFF] {
            let n = u32::from(start);
            assert_eq!(timeout_6530(start, i), (n + 1) * d, "6530 d{d} n{n}");
            assert_eq!(timeout_6532(start, i), n * d + 1, "6532 d{d} n{n}");
        }
    }
}

#[test]
fn both_free_run_one_per_clock_after_timeout() {
    let mut a = Rriot6530::new();
    a.write(5, 3);
    let mut b = Riot6532::new();
    b.write(0x95, 3);
    while a.peek(5) & 0x80 == 0 {
        a.tick();
    }
    while b.peek(0x85) & 0x80 == 0 {
        b.tick();
    }
    let (ta, tb) = (a.peek(6), b.peek(0x84));
    for k in 1..=300u32 {
        a.tick();
        b.tick();
        assert_eq!(a.peek(6), ta.wrapping_sub(k as u8));
        assert_eq!(b.peek(0x84), tb.wrapping_sub(k as u8));
    }
}

#[test]
fn interrupt_output_and_flag_register() {
    // 6530: the timer interrupt appears on PB7 (pin pulled low), flag bit 7 only.
    let mut a = Rriot6530::new();
    a.write(0x0C, 0);
    a.tick();
    assert!(a.irq_asserted());
    assert_eq!(a.peek(0x02) & 0x80, 0);
    assert_eq!(a.peek(0x05), 0x80);
    // PB7 as an output with a 1 latched still reads low while the interrupt holds.
    a.write(0x03, 0x80);
    a.write(0x02, 0x80);
    assert_eq!(a.peek(0x02) & 0x80, 0);
    a.read(0x06);
    assert_eq!(a.peek(0x02) & 0x80, 0x80);
    // 6532: dedicated IRQ, PB7 untouched, flag bit 6 for the PA7 edge.
    let mut b = Riot6532::new();
    b.write(0x9C, 0);
    b.tick();
    b.tick();
    assert!(b.irq_asserted());
    assert_eq!(b.peek(0x82) & 0x80, 0x80 & b.peek(0x82));
    assert_eq!(b.peek(0x85) & 0xC0, 0x80);
}

#[test]
fn ram_sizes() {
    let mut a = Rriot6530::new();
    let mut b = Riot6532::new();
    assert_eq!(a.ram.len(), 64);
    assert_eq!(b.ram.len(), 128);
    for i in 0..64u16 {
        a.write(0x40 + i, i as u8);
    }
    for i in 0..128u16 {
        b.write(i, i as u8);
    }
    assert!((0..64u16).all(|i| a.peek(0x40 + i) == i as u8));
    assert!((0..128u16).all(|i| b.peek(i) == i as u8));
}
