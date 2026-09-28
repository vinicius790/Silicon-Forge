//! zlib/DEFLATE and PNG written here, checked against reference streams.

use mos6502::graphics::*;

fn sample(n: usize) -> Vec<u8> {
    (0..n).map(|i| b"abcdefghij"[(i * i / 7) % 10]).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn adler32_reference() {
    assert_eq!(adler32(b""), 1);
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    // Long input exercises the deferred modulo.
    let big = vec![0xFFu8; 100_000];
    let (mut a, mut b) = (1u64, 0u64);
    for &x in &big {
        a = (a + u64::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    assert_eq!(adler32(&big), ((b << 16) | a) as u32);
}

#[test]
fn inflates_a_dynamic_huffman_stream_from_zlib() {
    // python3: zlib.compress(sample(3000), 9) -> BTYPE 2 (dynamic tables).
    let stream = unhex("78daedcc3911c0000c0341acb2fcc8e60f20098874bafa660104b37551422d272694d7bc296235fd355ab0e6d8977a8fe1165471ea64e0cb8a152b56ac58b162c58a152bbf290fb064a262");
    assert_eq!((stream[2] >> 1) & 3, 2, "dynamic block");
    assert_eq!(zlib_decompress(&stream, 1 << 20).unwrap(), sample(3000));
}

#[test]
fn inflates_known_fixed_and_stored_streams() {
    // python3: zlib.compress(b"hello") (fixed Huffman).
    let fixed = unhex("789ccb48cdc9c90700062c0215");
    assert_eq!(zlib_decompress(&fixed, 100).unwrap(), b"hello");
    // zlib.compress(b"hi", 0): one stored block.
    let stored = unhex("7801010200fdff6869013b00d2");
    assert_eq!(zlib_decompress(&stored, 100).unwrap(), b"hi");
}

#[test]
fn deflate_round_trips_at_every_level() {
    let inputs: Vec<Vec<u8>> = vec![
        vec![],
        vec![42],
        sample(5000),
        vec![0; 100_000],
        (0..70_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect(),
    ];
    for data in &inputs {
        for level in [0, 1, 6, 9] {
            let z = zlib_compress(data, level);
            assert_eq!(
                &zlib_decompress(&z, data.len()).unwrap(),
                data,
                "level {level}"
            );
        }
    }
}

#[test]
fn deflate_compresses_and_stored_blocks_split_at_64k() {
    let zeros = vec![0u8; 100_000];
    assert!(zlib_compress(&zeros, 6).len() < 1000);
    let stored = zlib_compress(&zeros, 0);
    // Two stored blocks: 2 header + 2*(5) + data + 4 Adler.
    assert_eq!(stored.len(), 2 + 10 + 100_000 + 4);
    assert_eq!(&stored[..2], &[0x78, 0x01]);
    assert_eq!(stored[2], 0, "first block not final");
    assert_eq!(stored[2 + 5 + 65535], 1, "second block final");
}

#[test]
fn zlib_rejects_damage() {
    let good = zlib_compress(b"payload payload payload", 6);
    let mut adler = good.clone();
    *adler.last_mut().unwrap() ^= 1;
    assert_eq!(
        zlib_decompress(&adler, 100).unwrap_err(),
        "Adler-32 não confere"
    );
    let mut head = good.clone();
    head[1] ^= 1;
    assert!(zlib_decompress(&head, 100).is_err());
    assert!(zlib_decompress(&good[..4], 100).is_err());
    assert!(zlib_decompress(&good, 5).is_err(), "limit enforced");
    // Reserved block type 3.
    assert!(inflate(&[0x07], 10).is_err());
    // Stored block whose NLEN is not the complement of LEN.
    assert!(inflate(&[0x01, 0x01, 0x00, 0x00, 0x00, 0x41], 10).is_err());
}

fn gradient(w: u32, h: u32) -> Vec<u8> {
    let mut rgb = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgb.extend_from_slice(&[(x * 7) as u8, (y * 11) as u8, ((x ^ y) * 3) as u8]);
        }
    }
    rgb
}

#[test]
fn png_round_trips_and_has_valid_structure() {
    let (w, h) = (37, 23);
    let rgb = gradient(w, h);
    for level in [0, 6] {
        let png = png_rgb(w, h, &rgb, level).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), w);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), h);
        assert_eq!(&png[24..29], &[8, 2, 0, 0, 0]);
        assert_eq!(&png[png.len() - 12..], b"\0\0\0\0IEND\xaeB\x60\x82");
        assert_eq!(decode_png_rgb(&png).unwrap(), (w, h, rgb.clone()));
    }
}

#[test]
fn png_filters_make_smooth_images_small() {
    let (w, h) = (256, 240);
    let rgb = gradient(w, h);
    let png = png_rgb(w, h, &rgb, 6).unwrap();
    assert!(png.len() < rgb.len() / 10, "{} bytes", png.len());
}

#[test]
fn png_rejects_bad_input_and_damage() {
    assert!(png_rgb(2, 2, &[0; 11], 6).is_err());
    assert!(png_rgb(0, 2, &[], 6).is_err());
    let mut png = png_rgb(2, 2, &[9; 12], 6).unwrap();
    png[30] ^= 1; // inside the IHDR CRC
    assert!(decode_png_rgb(&png).unwrap_err().contains("CRC"));
    assert!(decode_png_rgb(b"GIF89a").is_err());
}

#[test]
fn ppm_header() {
    assert_eq!(ppm_rgb(1, 1, &[1, 2, 3]), b"P6\n1 1\n255\n\x01\x02\x03");
}
