//! 6551 receiver error priority (overrun vs parity/framing) and the modem-line
//! IRQ masking matrix over every command value.

use mos6502::bus::IoDevice;
use mos6502::Acia6551;

const DATA: u16 = 0;
const STATUS: u16 = 1;
const CMD: u16 = 2;
const CTRL: u16 = 3;

/// 8 data bits, 1 stop; command with DTR on and the given parity bits 7–5.
fn acia(parity: u8) -> Acia6551 {
    let mut a = Acia6551::new();
    a.write(CTRL, 0x1F);
    a.write(CMD, (parity & 0xE0) | 0x0B);
    a.set_baud_period(0); // frames land at once
    a
}

fn even(b: u8) -> bool {
    b.count_ones() % 2 == 1 // parity bit making the total even
}

#[test]
fn error_bits_describe_the_stored_byte_for_every_combination() {
    for pe in [false, true] {
        for fe in [false, true] {
            let mut a = acia(0x60); // even parity
            let b = 0x5A;
            a.receive_framed(b, Some(even(b) ^ pe), !fe);
            let s = a.peek(STATUS);
            assert_eq!(s & 1 != 0, pe);
            assert_eq!(s & 2 != 0, fe);
            assert_ne!(s & 0x08, 0, "byte stored even with errors");
            assert_eq!(s & 0x04, 0, "no overrun");
            assert_eq!(a.read(DATA), b);
        }
    }
}

#[test]
fn an_overrunning_byte_sets_only_overrun_and_leaves_the_error_bits_of_the_unread_byte() {
    for first in 0..4u8 {
        for second in 0..4u8 {
            let mut a = acia(0x60);
            let (pe1, fe1) = (first & 1 != 0, first & 2 != 0);
            let (pe2, fe2) = (second & 1 != 0, second & 2 != 0);
            a.receive_framed(0x11, Some(even(0x11) ^ pe1), !fe1);
            a.receive_framed(0x22, Some(even(0x22) ^ pe2), !fe2);
            let s = a.peek(STATUS);
            assert_ne!(s & 0x04, 0, "overrun");
            assert_eq!((s & 1 != 0, s & 2 != 0), (pe1, fe1), "{first} {second}");
            assert_eq!(a.read(DATA), 0x11, "the unread byte stays");
        }
    }
}

#[test]
fn status_read_clears_errors_and_the_next_good_byte_is_clean() {
    let mut a = acia(0x60);
    a.receive_framed(0x01, Some(!even(0x01)), false);
    a.read(DATA);
    a.receive_framed(0x02, Some(!even(0x02)), true);
    let s = a.read(STATUS);
    assert_eq!(s & 0x07, 0x01, "second byte: parity only");
    assert_eq!(a.peek(STATUS) & 0x07, 0, "status read clears PE/FE/OVRN");
    a.read(DATA);
    a.receive_framed(0x03, Some(even(0x03)), true);
    assert_eq!(a.peek(STATUS) & 0x07, 0);
}

#[test]
fn parity_modes_mark_space_and_disabled() {
    // 001 odd, 011 even, 101 mark (check 1), 111 space (check 0), xx0 none.
    for b in [0x00u8, 0x01, 0x7F, 0xFF] {
        for (bits, want) in [
            (0x20u8, Some(!even(b))),
            (0x60, Some(even(b))),
            (0xA0, Some(true)),
            (0xE0, Some(false)),
            (0x00, None),
        ] {
            let mut ok = acia(bits);
            ok.receive_framed(b, want, true);
            assert_eq!(ok.peek(STATUS) & 1, 0, "{bits:02X} {b:02X}");
            if let Some(p) = want {
                let mut bad = acia(bits);
                bad.receive_framed(b, Some(!p), true);
                assert_eq!(bad.peek(STATUS) & 1, 1, "{bits:02X} {b:02X}");
            }
        }
    }
}

#[test]
fn modem_line_irq_matrix_over_every_command_value() {
    for cmd in 0..=255u8 {
        for line in 0..2 {
            for to_high in [true, false] {
                let mut a = Acia6551::new();
                a.write(CTRL, 0x1F);
                a.write(CMD, cmd);
                // Start from the opposite level without counting that change.
                if line == 0 {
                    a.set_dcd(!to_high);
                } else {
                    a.set_dsr(!to_high);
                }
                a.read(STATUS);
                if line == 0 {
                    a.set_dcd(to_high);
                } else {
                    a.set_dsr(to_high);
                }
                // Data sheet: DCD/DSR changes interrupt only with DTR on (bit 0)
                // and the receiver interrupt enabled (bit 1 clear).
                let want = cmd & 0x03 == 0x01;
                assert_eq!(
                    a.irq_asserted(),
                    want,
                    "cmd {cmd:02X} line {line} high {to_high}"
                );
                let s = a.read(STATUS);
                assert_eq!(s & 0x80 != 0, want);
                let bit = if line == 0 { 0x20 } else { 0x40 };
                assert_eq!(s & bit != 0, to_high, "status shows the level");
                assert!(!a.irq_asserted(), "status read clears IRQ");
            }
        }
    }
}

#[test]
fn unchanged_levels_and_cts_never_interrupt() {
    for cmd in 0..=255u8 {
        let mut a = Acia6551::new();
        a.write(CMD, cmd);
        a.read(STATUS); // drop a TDRE interrupt of a transmit-IRQ command
        let before = a.irq_asserted();
        a.set_dcd(false);
        a.set_dsr(false);
        assert_eq!(a.irq_asserted(), before, "{cmd:02X}: same level as reset");
        a.set_cts(true);
        a.read(STATUS);
        a.set_cts(false);
        // Releasing CTS may let TDRE rise (a transmit interrupt), never a modem one.
        let tx_irq = cmd & 0x0D == 0x05;
        assert!(
            !a.irq_asserted() || tx_irq,
            "{cmd:02X}: CTS has no IRQ of its own"
        );
    }
}

#[test]
fn dcd_high_stops_the_receiver() {
    let mut a = acia(0x00);
    a.set_dcd(true);
    a.receive_framed(0x42, None, true);
    assert_eq!(a.peek(STATUS) & 0x08, 0);
    a.set_dcd(false);
    a.receive_framed(0x42, None, true);
    assert_ne!(a.peek(STATUS) & 0x08, 0);
}
