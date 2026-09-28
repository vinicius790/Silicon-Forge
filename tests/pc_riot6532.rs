//! MOS 6532 RIOT: RAM, I/O ports, interval timer and PA7 edge detect.

use mos6502::bus::IoDevice;
use mos6502::pia::Riot6532;

const IO: u16 = 0x80;
const TIMER_W: u16 = 0x94; // RS, A4, A2
const TIMER_R: u16 = 0x84;
const FLAGS_R: u16 = 0x85;
const EDGE_W: u16 = 0x84;

#[test]
fn ram_holds_128_bytes_and_ignores_a7_aliasing() {
    let mut r = Riot6532::new();
    for a in 0..128u16 {
        r.write(a, (a as u8).wrapping_mul(7) ^ 0x5A);
    }
    for a in 0..128u16 {
        assert_eq!(r.read(a), (a as u8).wrapping_mul(7) ^ 0x5A);
    }
    assert_eq!(r.peek(IO + 1), 0, "RAM writes never touch I/O");
}

#[test]
fn ports_and_ddrs_over_all_values() {
    for v in 0..=255u8 {
        let mut r = Riot6532::new();
        r.write(IO + 1, v);
        r.write(IO + 3, !v);
        r.write(IO, 0xFF);
        r.write(IO + 2, 0x00);
        r.set_port_a_in(0x00);
        r.set_port_b_in(0xFF);
        assert_eq!((r.read(IO + 1), r.read(IO + 3)), (v, !v));
        assert_eq!(r.read(IO), v, "A: outputs from ORA, inputs from pins");
        assert_eq!(r.read(IO + 2), v, "B: outputs 0 from ORB, inputs high");
    }
}

#[test]
fn timer_every_interval_and_start_value() {
    for (sel, interval) in [(0u16, 1u32), (1, 8), (2, 64), (3, 1024)] {
        for n in [0u8, 1, 2, 5, 0x80, 0xFF] {
            let mut r = Riot6532::new();
            r.write(TIMER_W | sel, n);
            assert_eq!(r.peek(TIMER_R), n);
            r.tick();
            // First decrement one clock after the write.
            assert_eq!(r.peek(TIMER_R), n.wrapping_sub(1), "sel {sel} n {n}");
            assert_eq!(r.timer_flag(), n == 0);
            if n == 0 {
                continue;
            }
            // Then once per interval until passing 0 -> FF.
            let mut clocks = 1u32;
            while !r.timer_flag() {
                r.tick();
                clocks += 1;
                assert!(clocks <= 1 + interval * 256);
            }
            assert_eq!(clocks, 1 + interval * u32::from(n), "sel {sel} n {n}");
            assert_eq!(r.peek(TIMER_R), 0xFF);
            assert_eq!(r.interval(), 1, "after timeout: every clock");
            r.tick();
            assert_eq!(r.peek(TIMER_R), 0xFE);
        }
    }
}

#[test]
fn reading_or_writing_the_timer_clears_its_flag_but_reading_flags_does_not() {
    let mut r = Riot6532::new();
    r.write(TIMER_W, 0);
    r.tick();
    assert_eq!(r.read(FLAGS_R) & 0x80, 0x80);
    assert!(r.timer_flag(), "flag register read leaves the timer flag");
    r.read(TIMER_R);
    assert!(!r.timer_flag());
    r.write(TIMER_W, 0);
    r.tick();
    assert!(r.timer_flag());
    r.write(TIMER_W | 3, 9);
    assert!(!r.timer_flag());
    assert_eq!(r.interval(), 1024, "write restores the interval");
}

#[test]
fn timer_irq_enable_follows_a3_on_write_and_read() {
    for a3 in [0u16, 8] {
        let mut r = Riot6532::new();
        r.write(TIMER_W | a3, 0);
        r.tick();
        assert_eq!(r.irq_asserted(), a3 != 0);
    }
    let mut r = Riot6532::new();
    r.write(TIMER_W, 1);
    r.read(TIMER_R | 8); // enable through a read with A3
    r.tick();
    r.tick();
    assert!(r.irq_asserted());
    r.read(TIMER_R); // read with A3 = 0: disables and clears
    assert!(!r.irq_asserted());
}

#[test]
fn pa7_edge_detect_matrix() {
    for rising in [false, true] {
        for enable in [false, true] {
            for (from, to) in [(true, false), (false, true)] {
                let mut r = Riot6532::new();
                r.set_port_a_in(if from { 0x80 } else { 0 });
                r.write(EDGE_W | u16::from(rising) | (u16::from(enable) << 1), 0);
                r.read(FLAGS_R); // drop an edge from the setup itself
                r.set_port_a_in(if to { 0x80 } else { 0 });
                let want = to == rising;
                assert_eq!(r.pa7_flag(), want, "rising {rising} {from}->{to}");
                assert_eq!(r.irq_asserted(), want && enable);
                assert_eq!(r.read(FLAGS_R) & 0x40 != 0, want);
                assert!(!r.pa7_flag(), "flag read clears PA7");
            }
        }
    }
}

#[test]
fn pa7_as_output_still_edge_detects() {
    let mut r = Riot6532::new();
    r.write(EDGE_W, 0); // falling
    r.write(IO + 1, 0x80);
    r.write(IO, 0x80);
    r.read(FLAGS_R); // DDR write drove PA7 low first: that edge counts too
    assert!(!r.pa7_flag());
    r.write(IO, 0x00);
    assert!(r.pa7_flag());
}

#[test]
fn other_pa_lines_never_flag() {
    let mut r = Riot6532::new();
    r.write(EDGE_W | 1, 0);
    for v in 0..0x80u8 {
        r.set_port_a_in(v);
        assert!(!r.pa7_flag());
    }
}

#[test]
fn reset_clears_io_and_enables_keeps_ram_and_timer() {
    let mut r = Riot6532::new();
    r.write(5, 0x42);
    r.write(IO + 1, 0xFF);
    r.write(TIMER_W | 8, 0x30);
    r.reset();
    assert_eq!((r.peek(5), r.peek(IO + 1), r.timer()), (0x42, 0, 0x30));
    r.write(TIMER_W, 0);
    r.tick();
    assert!(r.timer_flag() && !r.irq_asserted());
}
