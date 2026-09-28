//! 6551 ACIA: W65C51N transmit-status variant, bit-level receiver noise rejection
//! and break send/detect.

use mos6502::acia::{data_bits, frame_levels, programmed_frame, Acia6551, AciaVariant};
use mos6502::bus::IoDevice;

const DATA: u16 = 0;
const STATUS: u16 = 1;
const COMMAND: u16 = 2;
const CONTROL: u16 = 3;
const TDRE: u8 = 0x10;
const RDRF: u8 = 0x08;

fn acia(variant: AciaVariant, command: u8, control: u8) -> Acia6551 {
    let mut a = Acia6551::with_variant(variant);
    IoDevice::write(&mut a, CONTROL, control);
    IoDevice::write(&mut a, COMMAND, command);
    a
}

/// Feed one frame on RxD at 16 clocks per bit, plus `idle` high clocks.
fn send_frame(a: &mut Acia6551, byte: u8, idle: usize) {
    let control = IoDevice::peek(a, CONTROL);
    let command = IoDevice::peek(a, COMMAND);
    let frame = programmed_frame(byte, command, control);
    for level in frame_levels(&frame, data_bits(control)) {
        a.rxd_16x(level);
    }
    for _ in 0..idle {
        a.rxd_16x(true);
    }
}

#[test]
fn classic_part_clears_tdre_while_a_byte_waits() {
    let mut a = acia(AciaVariant::Classic, 0x0B, 0x1E);
    assert_eq!(a.status() & TDRE, TDRE);
    IoDevice::write(&mut a, DATA, 0x55);
    assert_eq!(a.status() & TDRE, 0);
}

#[test]
fn w65c51n_tdre_reads_one_at_all_times() {
    let mut a = acia(AciaVariant::WdcW65c51n, 0x0B, 0x1E);
    for byte in 0..=255u8 {
        IoDevice::write(&mut a, DATA, byte);
        assert_eq!(a.status() & TDRE, TDRE);
        for _ in 0..10 {
            a.tick();
            assert_eq!(a.status() & TDRE, TDRE);
        }
    }
    assert_eq!(a.variant(), AciaVariant::WdcW65c51n);
}

#[test]
fn w65c51n_never_raises_a_transmit_interrupt() {
    // Command $07: DTR on, RX IRQ disabled (bit 1 set), TX control 01, so only the
    // transmitter could interrupt.
    for variant in [AciaVariant::Classic, AciaVariant::WdcW65c51n] {
        let mut a = acia(variant, 0x07, 0x1E);
        IoDevice::write(&mut a, DATA, 0x41);
        let mut fired = false;
        for _ in 0..20_000 {
            a.tick();
            let _ = a.pop_tx();
            fired |= a.irq_asserted();
        }
        assert_eq!(fired, variant == AciaVariant::Classic, "{variant:?}");
    }
}

#[test]
fn w65c51n_receiver_interrupt_still_works() {
    let mut a = acia(AciaVariant::WdcW65c51n, 0x09, 0x10);
    a.push_rx(0x7E);
    assert!(a.irq_asserted());
    assert_eq!(IoDevice::read(&mut a, STATUS) & (RDRF | 0x80), RDRF | 0x80);
    assert_eq!(IoDevice::read(&mut a, DATA), 0x7E);
}

#[test]
fn glitches_shorter_than_half_a_bit_are_counted_as_false_starts() {
    let mut a = acia(AciaVariant::Classic, 0x0B, 0x10);
    for width in 1..=7 {
        for _ in 0..width {
            a.rxd_16x(false);
        }
        for _ in 0..40 {
            a.rxd_16x(true);
        }
    }
    assert_eq!(a.false_starts(), 7);
    assert_eq!(a.status() & RDRF, 0, "no byte from noise");
    // A low pulse lasting past the start-bit midpoint is a real start bit.
    send_frame(&mut a, 0xA7, 32);
    assert_eq!(a.false_starts(), 7);
    assert_eq!(IoDevice::read(&mut a, DATA), 0xA7);
}

#[test]
fn noise_between_frames_does_not_corrupt_the_next_byte() {
    let mut a = acia(AciaVariant::Classic, 0x0B, 0x10);
    let mut got = Vec::new();
    for byte in [0x00u8, 0xFF, 0x55, 0xAA, 0x0F] {
        a.rxd_16x(false);
        a.rxd_16x(false);
        a.rxd_16x(true);
        for _ in 0..20 {
            a.rxd_16x(true);
        }
        send_frame(&mut a, byte, 20);
        got.push(IoDevice::read(&mut a, DATA));
    }
    assert_eq!(got, [0x00, 0xFF, 0x55, 0xAA, 0x0F]);
    assert_eq!(a.false_starts(), 5);
    assert_eq!(a.status() & 0x03, 0, "no parity or framing errors");
}

#[test]
fn a_line_held_low_is_detected_as_a_break() {
    let mut a = acia(AciaVariant::Classic, 0x0B, 0x10);
    for _ in 0..16 * 12 {
        a.rxd_16x(false);
    }
    assert!(a.rx_break_detected());
    let status = a.status();
    assert_eq!(status & RDRF, RDRF);
    assert_eq!(status & 0x02, 0x02, "framing error");
    assert_eq!(IoDevice::read(&mut a, DATA), 0x00);
    for _ in 0..40 {
        a.rxd_16x(true);
    }
    send_frame(&mut a, 0x31, 20);
    assert!(!a.rx_break_detected(), "next good frame clears the break");
}

#[test]
fn zero_byte_with_a_good_stop_bit_is_not_a_break() {
    let mut a = acia(AciaVariant::Classic, 0x0B, 0x10);
    send_frame(&mut a, 0x00, 20);
    assert!(!a.rx_break_detected());
    assert_eq!(a.status() & 0x02, 0);
}

#[test]
fn break_with_even_parity_is_detected() {
    // Even parity (command bits 7-5 = 011): parity bit of $00 is 0, so a held-low
    // line is a perfect break apart from the stop bit.
    let mut a = acia(AciaVariant::Classic, 0x6B, 0x10);
    for _ in 0..16 * 13 {
        a.rxd_16x(false);
    }
    assert!(a.rx_break_detected());
    assert_eq!(a.status() & 0x02, 0x02);
}

#[test]
fn transmit_break_holds_txd_low_and_drops_the_byte_in_flight() {
    let mut a = acia(AciaVariant::Classic, 0x0B, 0x10);
    IoDevice::write(&mut a, DATA, 0x55);
    assert!(a.txd_16x() || a.tx_line_busy());
    IoDevice::write(&mut a, COMMAND, 0x0F); // TX control 11: break
    assert!(a.tx_break());
    for _ in 0..16 * 20 {
        assert!(!a.txd_16x());
    }
    assert!(!a.txd_high());
    IoDevice::write(&mut a, COMMAND, 0x0B);
    for _ in 0..16 * 12 {
        assert!(a.txd_16x(), "line returns to idle mark, frame abandoned");
    }
}

#[test]
fn txd_break_looped_into_rxd_of_a_second_acia_is_detected() {
    let mut tx = acia(AciaVariant::Classic, 0x0F, 0x10);
    let mut rx = acia(AciaVariant::WdcW65c51n, 0x0B, 0x10);
    for _ in 0..16 * 12 {
        let level = tx.txd_16x();
        rx.rxd_16x(level);
    }
    assert!(rx.rx_break_detected());
}

#[test]
fn back_to_back_frames_between_two_acias_round_trip() {
    let mut tx = acia(AciaVariant::Classic, 0x0B, 0x10);
    let mut rx = acia(AciaVariant::Classic, 0x0B, 0x10);
    let mut got = Vec::new();
    for byte in 0..=255u8 {
        IoDevice::write(&mut tx, DATA, byte);
        for _ in 0..16 * 11 {
            let level = tx.txd_16x();
            rx.rxd_16x(level);
        }
        if rx.status() & RDRF != 0 {
            got.push(IoDevice::read(&mut rx, DATA));
        }
    }
    assert_eq!(got, (0..=255u8).collect::<Vec<_>>());
    assert_eq!(rx.false_starts(), 0);
}

#[test]
fn a_fresh_receiver_takes_the_line_as_idle_so_the_first_low_is_a_start_bit() {
    let mut a = Acia6551::new();
    IoDevice::write(&mut a, CONTROL, 0x10);
    IoDevice::write(&mut a, COMMAND, 0x0B);
    send_frame(&mut a, 0x5A, 20);
    assert_eq!(a.status() & RDRF, RDRF);
    assert_eq!(IoDevice::read(&mut a, DATA), 0x5A);
    assert_eq!(a.false_starts(), 0);
}

#[test]
fn a_reset_receiver_state_round_trips_through_a_snapshot_as_idle_high() {
    let a = Acia6551::new();
    let mut b = Acia6551::new();
    b.load_state(&a.state());
    IoDevice::write(&mut b, CONTROL, 0x10);
    IoDevice::write(&mut b, COMMAND, 0x0B);
    send_frame(&mut b, 0xC3, 20);
    assert_eq!(IoDevice::read(&mut b, DATA), 0xC3);
}
