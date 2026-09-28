//! Color Dreams (11) and BNROM (34): every written value, bus conflicts, limits,
//! iNES wiring and save-state round trips.

use mos6502::ines::Ines;
use mos6502::mapper::{Cartridge, Latch, MapperError};

fn prg(banks: usize) -> Vec<u8> {
    // Each byte is its bank number in the high bits and all-ones elsewhere except at
    // the conflict probe, so plain writes see no bus conflict.
    (0..banks * 0x8000)
        .map(|i| ((i / 0x8000) as u8).wrapping_mul(0x40) | 0x3F)
        .collect()
}

#[test]
fn color_dreams_every_value_and_conflicts() {
    for (pb, cb) in [(1usize, 1usize), (2, 16), (4, 8), (4, 16)] {
        let mut p: Vec<u8> = (0..pb * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        for b in 0..pb {
            p[b * 0x8000 + 0x10] = 0xFF;
            p[b * 0x8000 + 0x20] = 0x53;
        }
        let mut c = Latch::color_dreams(p.clone(), cb).unwrap();
        for v in 0..=255u8 {
            c.write(0x8010, v);
            let (wp, wc) = (usize::from(v & 3) % pb, usize::from(v >> 4) % cb);
            assert_eq!((c.prg_bank, c.chr_bank), (wp, wc), "{v:02X}");
            assert_eq!(usize::from(c.peek(0x8000)), wp);
            assert_eq!(usize::from(c.peek(0xFFFF)), wp);
            assert_eq!(c.chr_offset(0x0123), wc * 0x2000 + 0x123);
            c.write(0x8020, v);
            let m = v & 0x53;
            assert_eq!(
                (c.prg_bank, c.chr_bank),
                (usize::from(m & 3) % pb, usize::from(m >> 4) % cb)
            );
            c.write(0x7FFF, 0xFF);
            assert_eq!(c.chr_bank, usize::from(m >> 4) % cb, "below $8000 ignored");
        }
    }
    assert_eq!(
        Latch::color_dreams(vec![0; 0x4000], 1).err(),
        Some(MapperError::BadPrgSize(0x4000))
    );
    assert_eq!(
        Latch::color_dreams(vec![0; 5 * 0x8000], 1).err(),
        Some(MapperError::TooManyBanks(5))
    );
    assert_eq!(
        Latch::color_dreams(vec![0; 0x8000], 17).err(),
        Some(MapperError::TooManyBanks(17))
    );
}

#[test]
fn bnrom_every_value_up_to_256_banks() {
    for banks in [1usize, 2, 3, 8, 256] {
        let mut p: Vec<u8> = (0..banks * 0x8000).map(|i| (i / 0x8000) as u8).collect();
        for b in 0..banks {
            p[b * 0x8000 + 0x41] = 0xFF;
            p[b * 0x8000 + 0x42] = 0x0F;
        }
        let mut c = Latch::bnrom(p.clone()).unwrap();
        for v in 0..=255u8 {
            c.write(0x8041, v);
            let want = usize::from(v) % banks;
            assert_eq!(c.prg_bank, want);
            assert_eq!(usize::from(c.peek(0xC000)), want);
            assert_eq!(c.chr_offset(0x1FFF), 0x1FFF, "CHR not banked");
            c.write(0x8042, v);
            assert_eq!(c.prg_bank, usize::from(v & 0x0F) % banks, "bus conflict");
        }
    }
    assert_eq!(
        Latch::bnrom(vec![0; 257 * 0x8000]).err(),
        Some(MapperError::TooManyBanks(257))
    );
    assert_eq!(
        Latch::bnrom(vec![0; 0x4000]).err(),
        Some(MapperError::BadPrgSize(0x4000))
    );
}

fn ines(mapper: u8, prg16: u8, chr8: u8) -> Vec<u8> {
    let mut f = mos6502::ines::MAGIC.to_vec();
    f.extend([
        prg16,
        chr8,
        (mapper & 15) << 4,
        mapper & 0xF0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]);
    f.extend(prg(usize::from(prg16) / 2));
    f.extend(vec![0u8; usize::from(chr8) * 0x2000]);
    f
}

#[test]
fn ines_wiring_and_nina_is_refused() {
    let c = Ines::parse(&ines(11, 8, 16)).unwrap().cartridge().unwrap();
    assert_eq!(c.state_kind(), 11);
    let c = Ines::parse(&ines(34, 16, 0)).unwrap().cartridge().unwrap();
    assert_eq!(c.state_kind(), 34);
    // 16 KiB CHR ROM: NINA-001 (modelled since; see tests/pa_mappers.rs).
    assert!(Ines::parse(&ines(34, 4, 2)).unwrap().cartridge().is_ok());
}

#[test]
fn save_states_round_trip_every_bank() {
    use mos6502::systems::nes2a03::Nes2A03;
    for (m, p16, c8) in [(11u8, 8u8, 16u8), (34, 32, 0)] {
        let file = ines(m, p16, c8);
        for v in [0u8, 1, 3, 0x10, 0x5A, 0xF3, 0xFF] {
            let mut a = Nes2A03::from_ines(&file).unwrap();
            a.cpu.bus.cart.as_mut().unwrap().write(0x8000, v);
            let s = a.save_state();
            let mut b = Nes2A03::from_ines(&file).unwrap();
            b.load_state(&s).unwrap();
            assert_eq!(b.save_state(), s, "{m} {v:02X}");
        }
    }
}
