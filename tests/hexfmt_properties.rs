//! Intel HEX and S-record: round trips over many lengths and origins, every one-digit
//! corruption is caught, and each documented record type and error is exercised.

use mos6502::asm::to_intel_hex;
use mos6502::hexfmt::{from_intel_hex, from_srec, to_srec, HexErrorKind, HexImage};

fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(37).wrapping_add(seed))
        .collect()
}

#[test]
fn round_trips_for_every_length_up_to_300_and_edge_origins() {
    for len in 1..=300usize {
        for origin in [0x0000u16, 0x00F7, 0x8000, (0x10000 - len) as u16] {
            let bytes = data(len, origin as u8);
            let srec = from_srec(&to_srec(&bytes, origin, 0x1234)).unwrap();
            assert_eq!(
                srec,
                HexImage {
                    origin,
                    bytes: bytes.clone(),
                    start: Some(0x1234)
                }
            );
            let intel = from_intel_hex(&to_intel_hex(&bytes, origin)).unwrap();
            assert_eq!(
                (intel.origin, &intel.bytes),
                (origin, &bytes),
                "{len} at {origin:04X}"
            );
        }
    }
}

#[test]
fn every_single_digit_change_is_rejected() {
    let bytes = data(20, 7);
    for text in [
        to_srec(&bytes, 0x0400, 0x0400),
        to_intel_hex(&bytes, 0x0400),
    ] {
        for (i, c) in text.char_indices() {
            if !c.is_ascii_hexdigit() || (text.starts_with('S') && text.as_bytes()[i - 1] == b'S') {
                continue;
            }
            let replacement = if c == '0' { '1' } else { '0' };
            let mut bad = text.clone();
            bad.replace_range(i..i + 1, &replacement.to_string());
            let parsed = if text.starts_with('S') {
                from_srec(&bad)
            } else {
                from_intel_hex(&bad)
            };
            assert!(parsed.is_err(), "digit {i} of {text:?}");
        }
    }
}

#[test]
fn record_types_and_typed_errors() {
    // Intel: start linear address 05, extended linear 04 = 0 accepted, nonzero rejected.
    let start = from_intel_hex(":0400000500001234B1\n:00000001FF\n").unwrap();
    assert_eq!(start.start, Some(0x1234));
    assert!(from_intel_hex(":020000040000FA\n:00000001FF\n").is_ok());
    let wide = from_intel_hex(":020000040001F9\n").unwrap_err();
    assert_eq!(wide.kind, HexErrorKind::WideAddress(0x10));
    assert_eq!(
        from_intel_hex("0000").unwrap_err().kind,
        HexErrorKind::MissingMark(':')
    );
    assert_eq!(
        from_intel_hex(":05000000AA").unwrap_err().kind,
        HexErrorKind::BadLength
    );
    assert_eq!(
        from_intel_hex(":0G").unwrap_err().kind,
        HexErrorKind::BadDigit
    );
    assert_eq!(
        from_intel_hex(":01FFFF00").unwrap_err().kind,
        HexErrorKind::BadLength
    );
    let past = from_intel_hex(":02FFFF00AABB9B\n").unwrap_err();
    assert_eq!(past.kind, HexErrorKind::PastEnd(0xFFFF));
    let conflict = from_intel_hex(":0100100011DE\n:0100100022CD\n").unwrap_err();
    assert_eq!(
        (conflict.line, conflict.kind),
        (
            2,
            HexErrorKind::Conflict {
                address: 0x10,
                first: 0x11,
                second: 0x22
            }
        )
    );
    assert!(
        from_intel_hex(":0100100011DE\n:0100100011DE\n").is_ok(),
        "same value twice"
    );
    // S-record: S5 count must match.
    let good = to_srec(&data(40, 1), 0x2000, 0x2000);
    let miscounted = good.replace("S5030003F9", "S5030002FA");
    assert_eq!(
        from_srec(&miscounted).unwrap_err().kind,
        HexErrorKind::RecordCount {
            declared: 2,
            seen: 3
        }
    );
    assert_eq!(
        from_srec("S2040000AA51").unwrap_err().kind,
        HexErrorKind::UnsupportedType("S2".into())
    );
    assert_eq!(
        from_srec("").unwrap(),
        HexImage {
            origin: 0,
            bytes: vec![],
            start: None
        }
    );
}
