//! Bit-level RxD receiver: every byte through every word length and parity mode,
//! glitch rejection, parity and framing errors, back-to-back frames.

use mos6502::acia::{data_bits, frame_levels, programmed_frame, Acia6551};
use mos6502::bus::IoDevice;

fn acia(control: u8, command: u8) -> Acia6551 {
    let mut a = Acia6551::new();
    a.write(3, control);
    a.write(2, command | 0x0B);
    a
}

fn feed(a: &mut Acia6551, levels: &[bool]) {
    for &l in levels {
        a.rxd_16x(l);
    }
}

#[test]
fn every_byte_every_format_round_trips() {
    for len in 0..4u8 {
        for stop2 in [0u8, 0x80] {
            for parity in [0x00u8, 0x20, 0x60, 0xA0, 0xE0] {
                let control = (len << 5) | stop2;
                let mut a = acia(control, parity);
                let bits = data_bits(control);
                feed(&mut a, &[true; 20]);
                for byte in 0..=255u8 {
                    let f = programmed_frame(byte, a.peek(2), control);
                    feed(&mut a, &frame_levels(&f, bits));
                    feed(&mut a, &[true; 3]);
                    let status = a.read(1);
                    assert_eq!(
                        status & 0x0B,
                        0x08,
                        "len {bits} par {parity:02X} byte {byte:02X} status {status:02X}"
                    );
                    assert_eq!(a.read(0), f.data);
                    assert!(!a.rx_line_busy());
                }
            }
        }
    }
}

#[test]
fn glitch_shorter_than_half_a_bit_is_not_a_start() {
    for width in 1..8usize {
        let mut a = acia(0, 0);
        feed(&mut a, &[true; 4]);
        feed(&mut a, &vec![false; width]);
        feed(&mut a, &[true; 200]);
        assert_eq!(a.read(1) & 0x08, 0, "glitch {width}");
        assert!(!a.rx_line_busy());
    }
    let mut a = acia(0, 0);
    feed(&mut a, &[true; 4]);
    feed(&mut a, &[false; 8]);
    assert!(a.rx_line_busy(), "eight low samples confirm the start bit");
}

#[test]
fn parity_and_framing_errors_are_reported() {
    let control = 0x00; // 8 bits, 1 stop
    for parity in [0x20u8, 0x60, 0xA0, 0xE0] {
        for byte in [0x00u8, 0x01, 0x7F, 0xFF] {
            let mut a = acia(control, parity);
            let f = programmed_frame(byte, a.peek(2), control);
            let mut lv = frame_levels(&f, 8);
            for s in &mut lv[16 * 9..16 * 10] {
                *s = !*s; // flip the parity bit
            }
            feed(&mut a, &[true; 4]);
            feed(&mut a, &lv);
            assert_eq!(a.read(1) & 0x03, 0x01, "parity {parity:02X} {byte:02X}");
        }
    }
    let mut a = acia(control, 0);
    let f = programmed_frame(0x55, a.peek(2), control);
    let mut lv = frame_levels(&f, 8);
    let n = lv.len();
    for s in &mut lv[n - 16..] {
        *s = false;
    }
    feed(&mut a, &[true; 4]);
    feed(&mut a, &lv);
    assert_eq!(a.read(1) & 0x03, 0x02, "low stop bit is a framing error");
    assert_eq!(a.read(0), 0x55);
}

#[test]
fn back_to_back_frames_without_idle() {
    let mut a = acia(0, 0);
    feed(&mut a, &[true; 4]);
    let mut got = Vec::new();
    for byte in [0x00u8, 0xFF, 0xA5, 0x5A] {
        let f = programmed_frame(byte, a.peek(2), 0);
        feed(&mut a, &frame_levels(&f, 8));
        got.push(a.read(0));
    }
    assert_eq!(got, [0x00, 0xFF, 0xA5, 0x5A]);
}
