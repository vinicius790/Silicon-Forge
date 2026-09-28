//! KCS and Apple II cassette round trips through PCM.

use mos6502::tape::*;

#[test]
fn kcs_pcm_round_trip_at_several_rates() {
    let data: Vec<u8> = (0..=255).collect();
    for rate in [9600, 22050, 44100, 48000] {
        let pcm = to_pcm(&encode_with_leader(&data, 20), rate).unwrap();
        assert_eq!(kcs_decode_pcm(&pcm, rate).unwrap(), data, "rate {rate}");
    }
}

#[test]
fn kcs_marks_from_pcm_recovers_exact_marks() {
    let marks = encode(&[0x5A]);
    let pcm = to_pcm(&marks, 44100).unwrap();
    assert_eq!(kcs_marks_from_pcm(&pcm, 44100).unwrap(), marks);
    assert!(kcs_marks_from_pcm(&pcm, 4000).is_err());
}

#[test]
fn kcs_inverted_polarity_still_decodes() {
    let pcm: Vec<i16> = to_pcm(&encode(b"KIM"), 44100)
        .unwrap()
        .into_iter()
        .map(|s| -s)
        .collect();
    assert_eq!(kcs_decode_pcm(&pcm, 44100).unwrap(), b"KIM");
}

#[test]
fn half_cycles_split_at_sign_changes() {
    assert_eq!(half_cycles(&[1, 1, -1, -1, -1, 0, 5]), vec![2, 3, 2]);
    assert!(half_cycles(&[]).is_empty());
}

#[test]
fn apple_checksum_is_xor_seeded_ff() {
    assert_eq!(apple_checksum(&[]), 0xFF);
    assert_eq!(apple_checksum(&[0x0F, 0xF0]), 0x00);
}

#[test]
fn apple_round_trip() {
    let data: Vec<u8> = (0..200u16).map(|i| (i * 37) as u8).collect();
    for rate in [11025, 22050, 44100] {
        let pcm = apple_encode_pcm(&data, rate, 200).unwrap();
        assert_eq!(apple_decode_pcm(&pcm, rate).unwrap(), data, "rate {rate}");
    }
}

#[test]
fn apple_bit_timing_matches_rom() {
    // One byte $80 + checksum $7F at 1 MHz-like resolution: 1 bit = 1000 us.
    let pcm = apple_encode_pcm(&[0x80], 100_000, 16).unwrap();
    let halves = half_cycles(&pcm);
    assert_eq!(halves[..16], [65; 16]);
    assert_eq!(halves[16..18], [20, 25]);
    assert_eq!(halves[18..20], [50, 50]); // bit 7 of $80 is 1
    assert_eq!(halves[20..22], [25, 25]); // then zeros
}

/// PCM from half-cycle lengths in samples.
fn from_halves(halves: &[usize]) -> Vec<i16> {
    let mut out = Vec::new();
    let mut level = 1000i16;
    for &h in halves {
        out.extend(std::iter::repeat_n(level, h));
        level = -level;
    }
    out
}

#[test]
fn apple_errors() {
    assert!(apple_encode_pcm(&[1], 4000, 10).is_err());
    assert_eq!(
        apple_decode_pcm(&vec![100i16; 5000], 44100).unwrap_err(),
        AppleTapeError::NoSync
    );
    // At 100 kHz: header 65, sync 20+25, zero half 25, one half 50.
    let header = [65usize; 20];
    let byte = |b: u8| -> Vec<usize> {
        (0..8)
            .rev()
            .flat_map(|i| {
                if (b >> i) & 1 != 0 {
                    [50, 50]
                } else {
                    [25, 25]
                }
            })
            .collect()
    };
    let build = |bytes: &[u8], extra: &[usize]| {
        let mut h = header.to_vec();
        h.extend([20, 25]);
        for &b in bytes {
            h.extend(byte(b));
        }
        h.extend_from_slice(extra);
        from_halves(&h)
    };
    assert_eq!(
        apple_decode_pcm(&build(&[0x12, 0x12 ^ 0xFF], &[]), 100_000).unwrap(),
        vec![0x12]
    );
    assert_eq!(
        apple_decode_pcm(&build(&[0x12, 0x00], &[]), 100_000).unwrap_err(),
        AppleTapeError::Checksum {
            expected: 0xED,
            found: 0x00
        }
    );
    assert_eq!(
        apple_decode_pcm(&build(&[0x12], &[25, 25, 50, 50]), 100_000).unwrap_err(),
        AppleTapeError::PartialByte { bits: 2 }
    );
    assert_eq!(
        apple_decode_pcm(&build(&[], &[]), 100_000).unwrap_err(),
        AppleTapeError::Empty
    );
    // Too short a header is not a sync.
    let mut h = vec![65usize; 10];
    h.extend([20, 25]);
    h.extend(byte(0xFF));
    assert_eq!(
        apple_decode_pcm(&from_halves(&h), 100_000).unwrap_err(),
        AppleTapeError::NoSync
    );
}
