//! MOS Technology paper tape and 32-bit Intel HEX.

use mos6502::hexfmt::*;

#[test]
fn mos_record_layout_and_checksum() {
    let text = to_mos_hex(&[0xA9, 0x01, 0x00], 0x0200);
    // 03 + 02 + 00 + A9 + 01 + 00 = $00AF
    assert_eq!(text, ";030200A9010000AF\n;0000010001\n");
}

#[test]
fn mos_splits_at_24_bytes_and_counts_records() {
    let data: Vec<u8> = (0..50).collect();
    let text = to_mos_hex(&data, 0x1000);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 4);
    assert!(lines[0].starts_with(";181000"));
    assert!(lines[1].starts_with(";181018"));
    assert!(lines[2].starts_with(";021030"));
    assert_eq!(lines[3], ";0000030003");
    let back = from_mos_hex(&text).unwrap();
    assert_eq!((back.origin, back.bytes), (0x1000, data));
}

#[test]
fn mos_rejects_bad_checksum_count_and_missing_end() {
    let good = to_mos_hex(&[1, 2, 3], 0x0300);
    let bad = good.replacen("010203", "010204", 1);
    assert!(matches!(
        from_mos_hex(&bad).unwrap_err().kind,
        HexErrorKind::Checksum { .. }
    ));
    let wrong_count = good.replace(";0000010001", ";0000020002");
    assert_eq!(
        from_mos_hex(&wrong_count).unwrap_err().kind,
        HexErrorKind::RecordCount {
            declared: 2,
            seen: 1
        }
    );
    let cut = good.lines().next().unwrap().to_string();
    assert_eq!(
        from_mos_hex(&cut).unwrap_err().kind,
        HexErrorKind::RecordCount {
            declared: 0,
            seen: 1
        }
    );
    assert_eq!(
        from_mos_hex(":00000001FF").unwrap_err().kind,
        HexErrorKind::MissingMark(';')
    );
}

#[test]
fn intel32_emits_extended_linear_records_at_64k_boundaries() {
    let seg = Segment32 {
        address: 0x0001_FFF8,
        bytes: (0..16).collect(),
    };
    let text = to_intel_hex32(&[seg.clone()], Some(0x0002_0004));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], ":020000040001F9");
    assert!(lines[1].starts_with(":08FFF800"));
    assert_eq!(lines[2], ":020000040002F8");
    assert!(lines[3].starts_with(":08000000"));
    assert_eq!(lines[4], ":0400000500020004F1");
    assert_eq!(lines[5], ":00000001FF");
    let back = from_intel_hex32(&text).unwrap();
    assert_eq!(back.segments, vec![seg]);
    assert_eq!(back.start, Some(0x0002_0004));
    assert_eq!(back.byte(0x0002_0000), Some(8));
    assert_eq!(back.byte(0x0002_0008), None);
    assert_eq!(back.len(), 16);
}

#[test]
fn intel32_segment_records_and_conflicts() {
    // Type 02 base $1000 << 4 = $10000.
    let text = ":020000021000EC\n:02000000AABB99\n:00000001FF\n";
    let img = from_intel_hex32(text).unwrap();
    assert_eq!(img.segments[0].address, 0x10000);
    assert_eq!(img.segments[0].bytes, vec![0xAA, 0xBB]);
    let clash = ":0100000011EE\n:0100000022DD\n";
    assert!(matches!(
        from_intel_hex32(clash).unwrap_err().kind,
        HexErrorKind::Conflict {
            first: 0x11,
            second: 0x22,
            ..
        }
    ));
    // The 16-bit loader still refuses a nonzero upper address.
    assert!(from_intel_hex(":020000040001F9\n").is_err());
}

#[test]
fn detect_and_read_any() {
    assert_eq!(detect_format("  :00"), Some(HexFormat::Intel));
    assert_eq!(detect_format("S1"), Some(HexFormat::SRecord));
    assert_eq!(detect_format(";00"), Some(HexFormat::Mos));
    assert_eq!(detect_format("\u{1}"), None);
    let mos = to_mos_hex(&[9, 8], 0x40);
    assert_eq!(read_any(&mos).unwrap().bytes, vec![9, 8]);
    let srec = to_srec(&[7], 0x50, 0x50);
    assert_eq!(read_any(&srec).unwrap().origin, 0x50);
}
