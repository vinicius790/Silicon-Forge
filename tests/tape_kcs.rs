//! KCS framing over all byte values, leaders, gaps and every error kind.

use mos6502::audio::rising_edge_period;
use mos6502::tape::{bits, decode, encode, encode_with_leader, to_pcm, TapeError, FRAME_BITS};

#[test]
fn every_byte_value_round_trips_alone_and_together() {
    let all: Vec<u8> = (0..=255).collect();
    for &b in &all {
        assert_eq!(decode(&encode(&[b])).unwrap(), [b]);
        let marks = encode(&[b]);
        let ones = b.count_ones() as usize + 2;
        assert_eq!(marks.len(), ones * 8 + (FRAME_BITS - ones) * 4, "{b:02X}");
    }
    assert_eq!(decode(&encode(&all)).unwrap(), all);
    assert_eq!(decode(&encode_with_leader(&all, 100)).unwrap(), all);
}

#[test]
fn idle_bits_between_frames_are_skipped() {
    let mut marks = encode_with_leader(b"A", 3);
    marks.extend(encode_with_leader(b"BC", 7));
    marks.extend(encode_with_leader(b"", 5));
    assert_eq!(decode(&marks).unwrap(), b"ABC");
}

#[test]
fn errors_are_typed_and_located() {
    let good = encode(&[0x5A]);
    let mut level = good.clone();
    level[5] = 2;
    assert_eq!(decode(&level), Err(TapeError::BadLevel { at: 5, level: 2 }));
    let mut broken = good.clone();
    broken[1] = 1; // inside the start bit's four zeros
    assert_eq!(decode(&broken), Err(TapeError::BrokenCell { at: 0 }));
    assert_eq!(
        decode(&good[..good.len() - 8]),
        Err(TapeError::Truncated { bit: 0 })
    );
    assert_eq!(bits(&good[..2]), Err(TapeError::BrokenCell { at: 0 }));
    // A frame with stop bits 0: start, eight 1s, then two 0s.
    let mut bad_stop = vec![0u8; 4];
    for _ in 0..8 {
        bad_stop.extend([1u8; 8]);
    }
    bad_stop.extend([0u8; 8]);
    assert_eq!(decode(&bad_stop), Err(TapeError::StopBits { bit: 0 }));
}

#[test]
fn pcm_uses_the_two_kcs_frequencies() {
    let zeros = to_pcm(&[0; 40], 48_000).unwrap();
    assert_eq!(zeros.len(), 1600, "40 cycles of 1200 Hz");
    assert_eq!(rising_edge_period(&zeros), Some(40));
    let ones = to_pcm(&[1; 40], 48_000).unwrap();
    assert_eq!((ones.len(), rising_edge_period(&ones)), (800, Some(20)));
    let frame = to_pcm(&encode(&[0x00]), 48_000).unwrap();
    assert_eq!(frame.len(), 11 * 160, "every bit lasts 1/300 s");
    assert!(to_pcm(&[0], 4000).is_err());
    assert!(to_pcm(&[3], 48_000).is_err());
}
