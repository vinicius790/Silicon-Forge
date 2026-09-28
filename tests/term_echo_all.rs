//! Line-editor ROM: every byte class, rubout, full buffer, CR, and typed errors.

use mos6502::systems::term::{TermComputer, TermError, LINE_MAX};

const LIMIT: u32 = 10_000;

#[test]
fn every_byte_value_is_stored_or_rung() {
    for b in 0..=255u8 {
        let mut m = TermComputer::new().unwrap();
        let out = m.send(b, LIMIT).unwrap();
        let (echo, line): (Vec<u8>, Vec<u8>) = match b {
            0x0D => (vec![0x0D, 0x0A], vec![]),
            0x08 | 0x7F => (vec![0x07], vec![]),
            0x00..=0x1F => (vec![0x07], vec![]),
            _ => (vec![b], vec![b]),
        };
        assert_eq!((out, m.line()), (echo, line), "{b:02X}");
        assert_eq!(m.cpu.is_halted(), b == 0x0D, "{b:02X}");
    }
}

#[test]
fn rubout_erases_one_byte_each_until_empty() {
    let mut m = TermComputer::new().unwrap();
    m.type_bytes(b"abc", LIMIT).unwrap();
    for (key, rest) in [(0x08u8, &b"ab"[..]), (0x7F, b"a"), (0x08, b"")] {
        assert_eq!(m.send(key, LIMIT).unwrap(), [0x08, 0x20, 0x08]);
        assert_eq!(m.line(), rest);
    }
    assert_eq!(m.send(0x7F, LIMIT).unwrap(), [0x07]);
    assert_eq!(m.type_bytes(b"hi\r", LIMIT).unwrap(), b"hi\r\n");
    assert_eq!(m.line(), b"hi");
}

#[test]
fn full_line_rings_and_keeps_64_bytes() {
    let mut m = TermComputer::new().unwrap();
    let text: Vec<u8> = (0..LINE_MAX as u8).map(|i| 0x20 + i).collect();
    assert_eq!(m.type_bytes(&text, LIMIT).unwrap(), text);
    for b in [b'x', 0xFF, b' '] {
        assert_eq!(m.send(b, LIMIT).unwrap(), [0x07]);
    }
    assert_eq!(m.line(), text);
    assert_eq!(m.send(0x08, LIMIT).unwrap(), [0x08, 0x20, 0x08]);
    assert_eq!(m.send(b'!', LIMIT).unwrap(), b"!");
    assert_eq!(m.line().len(), LINE_MAX);
}

#[test]
fn typed_errors() {
    let mut m = TermComputer::new().unwrap();
    assert_eq!(m.send(b'a', 3), Err(TermError::Timeout { byte: b'a' }));
    let mut m = TermComputer::new().unwrap();
    m.send(0x0D, LIMIT).unwrap();
    assert_eq!(m.send(b'x', LIMIT), Err(TermError::Halted));
    assert!(m.echo_byte(b'x', 100).unwrap_err().contains("parou"));
}
