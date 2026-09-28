//! WAV 8/16-bit mono/stereo; C64 PRG and Atari XEX load formats.

use mos6502::audio::*;
use mos6502::hexfmt::*;

fn fmt(rate: u32, channels: u16, bits: u16) -> WavFormat {
    WavFormat {
        sample_rate: rate,
        channels,
        bits,
    }
}

#[test]
fn u8_conversion_endpoints_and_rounding() {
    assert_eq!(to_u8_sample(0), 128);
    assert_eq!(to_u8_sample(i16::MAX), 255);
    assert_eq!(to_u8_sample(i16::MIN), 0);
    assert_eq!(to_u8_sample(127), 128);
    assert_eq!(to_u8_sample(128), 129);
    assert_eq!(to_u8_sample(-129), 127);
    assert_eq!(from_u8_sample(128), 0);
    assert_eq!(from_u8_sample(0), -32768);
    assert_eq!(from_u8_sample(255), 127 << 8);
    for b in 0..=255u8 {
        assert_eq!(to_u8_sample(from_u8_sample(b)), b);
    }
}

#[test]
fn stereo_16_bit_header_and_round_trip() {
    let pcm = interleave(&[1, 2, 3], &[-1, -2, -3]).unwrap();
    assert_eq!(pcm, [1, -1, 2, -2, 3, -3]);
    let wav = wav_bytes_with(&pcm, fmt(48_000, 2, 16)).unwrap();
    assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 2);
    assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 192_000);
    assert_eq!(u16::from_le_bytes([wav[32], wav[33]]), 4);
    assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 12);
    assert_eq!(parse_wav_any(&wav).unwrap(), (fmt(48_000, 2, 16), pcm));
}

#[test]
fn eight_bit_mono_is_padded_to_even_length() {
    let wav = wav_bytes_with(&[0, i16::MAX, i16::MIN], fmt(8000, 1, 8)).unwrap();
    assert_eq!(&wav[44..47], &[128, 255, 0]);
    assert_eq!(wav.len(), 48, "pad byte");
    assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 40);
    assert_eq!(u32::from_le_bytes(wav[40..44].try_into().unwrap()), 3);
    let (f, s) = parse_wav_any(&wav).unwrap();
    assert_eq!(f, fmt(8000, 1, 8));
    assert_eq!(s, [0, 127 << 8, -32768]);
}

#[test]
fn legacy_writer_output_parses_with_the_new_reader() {
    let wav = wav_bytes(&[5, -5], 22050).unwrap();
    assert_eq!(
        parse_wav_any(&wav).unwrap(),
        (fmt(22050, 1, 16), vec![5, -5])
    );
}

#[test]
fn reader_skips_unknown_chunks() {
    let wav = wav_bytes_with(&[7, 8], fmt(8000, 1, 16)).unwrap();
    let mut with_list = wav[..36].to_vec();
    with_list.extend_from_slice(b"LIST\x03\x00\x00\x00abc\x00");
    with_list.extend_from_slice(&wav[36..]);
    assert_eq!(parse_wav_any(&with_list).unwrap().1, vec![7, 8]);
}

#[test]
fn invalid_formats_and_files() {
    assert!(wav_bytes_with(&[1], fmt(8000, 3, 16)).is_err());
    assert!(wav_bytes_with(&[1], fmt(8000, 1, 24)).is_err());
    assert!(wav_bytes_with(&[1], fmt(0, 1, 16)).is_err());
    assert!(
        wav_bytes_with(&[1, 2, 3], fmt(8000, 2, 16)).is_err(),
        "half frame"
    );
    assert!(interleave(&[1], &[]).is_err());
    let good = wav_bytes_with(&[1, 2], fmt(8000, 1, 16)).unwrap();
    let mut float = good.clone();
    float[20] = 3;
    assert!(parse_wav_any(&float).unwrap_err().contains("only PCM"));
    let mut align = good.clone();
    align[32] = 9;
    assert!(parse_wav_any(&align).is_err());
    assert!(parse_wav_any(&good[..40]).is_err());
    assert!(parse_wav_any(b"RIFX0000WAVE").is_err());
}

#[test]
fn prg_round_trip_and_limits() {
    let prg = to_prg(&[0xA9, 0x00], 0x0801).unwrap();
    assert_eq!(prg, [0x01, 0x08, 0xA9, 0x00]);
    let img = from_prg(&prg).unwrap();
    assert_eq!(
        (img.origin, img.bytes, img.start),
        (0x0801, vec![0xA9, 0], None)
    );
    assert!(from_prg(&[1]).is_err());
    assert!(to_prg(&[0; 3], 0xFFFE).is_err());
    assert!(from_prg(&[0xFE, 0xFF, 1, 2, 3]).is_err());
    assert_eq!(from_prg(&[0x00, 0xC0]).unwrap().bytes, Vec::<u8>::new());
}

#[test]
fn c64_sys_stub_is_a_valid_basic_line() {
    let stub = c64_sys_stub(2061);
    // next-line pointer, line 10, SYS, "2061", 0, end marker.
    assert_eq!(
        stub,
        [0x0B, 0x08, 0x0A, 0x00, 0x9E, b'2', b'0', b'6', b'1', 0, 0, 0]
    );
    // Code placed right after the stub starts at $0801 + 12 = 2061.
    assert_eq!(0x0801 + stub.len(), 2061);
}

#[test]
fn xex_parse_with_run_init_and_repeated_headers() {
    let mut file = vec![0xFF, 0xFF];
    file.extend_from_slice(&[0x00, 0x20, 0x02, 0x20, 1, 2, 3]); // $2000-$2002
    file.extend_from_slice(&[0xE2, 0x02, 0xE3, 0x02, 0x00, 0x20]); // INITAD $2000
    file.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x30, 0x00, 0x30, 9]); // $FFFF again
    file.extend_from_slice(&[0xE0, 0x02, 0xE1, 0x02, 0x00, 0x30]); // RUNAD $3000
    let x = from_xex(&file).unwrap();
    assert_eq!(x.segments.len(), 4);
    assert_eq!(x.inits, vec![0x2000]);
    assert_eq!(x.run, Some(0x3000));
    assert_eq!(
        x.segments[2],
        XexSegment {
            start: 0x3000,
            bytes: vec![9]
        }
    );
    let img = x.image();
    assert_eq!(img.origin, 0x02E0);
    assert_eq!(img.start, Some(0x3000));
    assert_eq!(img.bytes[0x2000 - 0x02E0..0x2003 - 0x02E0], [1, 2, 3]);
}

#[test]
fn xex_writer_round_trips_and_errors() {
    let segs = vec![
        XexSegment {
            start: 0x0600,
            bytes: vec![0xA9, 0x01],
        },
        XexSegment {
            start: 0x0700,
            bytes: vec![0x60],
        },
    ];
    let file = to_xex(&segs, Some(0x0600)).unwrap();
    assert_eq!(&file[..6], &[0xFF, 0xFF, 0x00, 0x06, 0x01, 0x06]);
    let x = from_xex(&file).unwrap();
    assert_eq!(&x.segments[..2], &segs[..]);
    assert_eq!(x.run, Some(0x0600));
    assert!(to_xex(
        &[XexSegment {
            start: 0,
            bytes: vec![]
        }],
        None
    )
    .is_err());
    assert!(from_xex(&[0x00, 0x20]).unwrap_err().contains("$FFFF"));
    assert!(from_xex(&[0xFF, 0xFF])
        .unwrap_err()
        .contains("sem segmentos"));
    assert!(from_xex(&[0xFF, 0xFF, 0x10, 0x20, 0x00, 0x20])
        .unwrap_err()
        .contains("invertido"));
    assert!(from_xex(&[0xFF, 0xFF, 0x00, 0x20, 0x05, 0x20, 1])
        .unwrap_err()
        .contains("truncado"));
}
