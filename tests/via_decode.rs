//! ACR/PCR decoding against the datasheet, over every register value, and against
//! the live chip.

use mos6502::via::{Acr, C2Mode, Pcr, ShiftMode, Via6522};

#[test]
fn acr_every_value_round_trips_and_matches_bits() {
    for v in 0..=255u8 {
        let a = Acr::decode(v);
        assert_eq!(a.encode(), v);
        assert_eq!(
            (a.t1_pb7, a.t1_free_run, a.t2_counts_pb6),
            (v & 0x80 != 0, v & 0x40 != 0, v & 0x20 != 0)
        );
        let shifting_in = matches!(
            a.shift,
            ShiftMode::InUnderT2 | ShiftMode::InUnderPhi2 | ShiftMode::InUnderCb1
        );
        assert_eq!(shifting_in, matches!((v >> 2) & 7, 1..=3), "{v:02X}");
        assert_eq!(a.shift == ShiftMode::Disabled, v & 0x1C == 0);
        let mut via = Via6522::new();
        via.write(0x0B, v);
        assert_eq!(via.acr(), a);
    }
}

#[test]
fn pcr_every_value_round_trips_and_matches_the_table() {
    for v in 0..=255u8 {
        let p = Pcr::decode(v);
        assert_eq!(p.encode(), v, "{v:02X}");
        for (mode, bits) in [(p.ca2, (v >> 1) & 7), (p.cb2, (v >> 5) & 7)] {
            let output = matches!(
                mode,
                C2Mode::Handshake | C2Mode::Pulse | C2Mode::Manual { .. }
            );
            assert_eq!(output, bits & 4 != 0);
            if let C2Mode::Manual { high } = mode {
                assert_eq!(high, bits & 1 != 0);
            }
        }
        let mut via = Via6522::new();
        via.write(0x0C, v);
        assert_eq!(via.pcr(), p);
        // A manual CA2/CB2 level is what the pin shows.
        if let C2Mode::Manual { high } = p.ca2 {
            assert_eq!(via.ca2(), high);
        }
        if let C2Mode::Manual { high } = p.cb2 {
            assert_eq!(via.cb2(), high);
        }
    }
}

#[test]
fn pending_sources_follow_ifr_and_ier() {
    let mut via = Via6522::new();
    via.write(0x04, 2);
    via.write(0x05, 0);
    for _ in 0..10 {
        via.tick();
    }
    assert_eq!(via.pending_sources(), 0, "T1 flagged but disabled");
    via.write(0x0E, 0xC0);
    assert_eq!(via.pending_sources(), 0x40);
    via.write(0x0E, 0x40);
    assert_eq!(via.pending_sources(), 0);
}
