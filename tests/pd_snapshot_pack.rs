//! M65Z container: LZ compression, CRC-32 and corruption detection.

use mos6502::snapshot::*;

#[test]
fn crc32_reference_values() {
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(
        crc32(b"The quick brown fox jumps over the lazy dog"),
        0x414F_A339
    );
}

#[test]
fn lz_round_trips_varied_inputs() {
    let mut seed = 0x1234_5678u32;
    let mut noise = Vec::new();
    for _ in 0..5000 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        noise.push(seed as u8);
    }
    let cases: Vec<Vec<u8>> = vec![
        vec![],
        vec![7],
        vec![0; 70_000],
        b"abcabcabcabcabcabcabcabc".repeat(100),
        noise,
        (0..=255u8).cycle().take(3000).collect(),
    ];
    for c in cases {
        let z = lz_compress(&c);
        assert_eq!(lz_decompress(&z, c.len()).unwrap(), c);
    }
}

#[test]
fn lz_compresses_a_zeroed_ram_image_hard() {
    let ram = vec![0u8; 65536];
    let z = lz_compress(&ram);
    assert!(z.len() < 1700, "{} bytes", z.len());
}

#[test]
fn lz_rejects_bad_streams() {
    assert_eq!(lz_decompress(&[0x80, 1, 0], 10), Err(PackError::Corrupt));
    assert_eq!(lz_decompress(&[0x05, 1], 10), Err(PackError::Corrupt));
    assert_eq!(
        lz_decompress(&[0x00, 1, 0x80, 0, 0], 10),
        Err(PackError::Corrupt)
    );
    assert_eq!(
        lz_decompress(&[0x00, 1, 0xFF, 1, 0], 10),
        Err(PackError::Corrupt)
    );
}

#[test]
fn pack_chooses_method_and_round_trips() {
    let json = br#"{"version":8,"ram":"0000000000000000000000000000"}"#.repeat(50);
    let packed = pack(&json).unwrap();
    assert!(is_packed(&packed));
    assert_eq!(&packed[..4], b"M65Z");
    assert_eq!(packed[5], 1, "LZ");
    assert!(packed.len() < json.len() / 4);
    assert_eq!(unpack(&packed).unwrap(), json);
    let tiny = pack(b"ab").unwrap();
    assert_eq!(tiny[5], 0, "stored");
    assert_eq!(tiny.len(), PACK_HEADER + 2);
    assert_eq!(unpack(&tiny).unwrap(), b"ab");
}

#[test]
fn unpack_detects_every_kind_of_damage() {
    let data = vec![0x55u8; 1000];
    let good = pack(&data).unwrap();
    assert_eq!(unpack(&good[..10]), Err(PackError::TooShort));
    let mut m = good.clone();
    m[0] = b'X';
    assert_eq!(unpack(&m), Err(PackError::BadMagic));
    let mut v = good.clone();
    v[4] = 9;
    assert_eq!(unpack(&v), Err(PackError::Version(9)));
    let mut me = good.clone();
    me[5] = 7;
    assert_eq!(unpack(&me), Err(PackError::Method(7)));
    let mut c = good.clone();
    c[10] ^= 1;
    assert!(matches!(unpack(&c), Err(PackError::Crc { .. })));
    let mut l = good.clone();
    l[6] = l[6].wrapping_add(1);
    assert!(matches!(
        unpack(&l),
        Err(PackError::Length { .. }) | Err(PackError::Corrupt)
    ));
    // Stored payload flipped: CRC catches it.
    let mut s = pack(b"xy").unwrap();
    s[PACK_HEADER] ^= 0x20;
    assert!(matches!(unpack(&s), Err(PackError::Crc { .. })));
}
