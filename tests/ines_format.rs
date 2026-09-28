//! iNES / NES 2.0 parsing, building, every flag bit, every error, and mapper wiring.

use mos6502::ines::{Ines, InesError, Mirroring, Timing, MAGIC};
use mos6502::mapper::{Axrom, Cartridge, Latch, MapperError};

fn header(b4: u8, b5: u8, f6: u8, f7: u8) -> Vec<u8> {
    let mut h = MAGIC.to_vec();
    h.extend([b4, b5, f6, f7, 0, 0, 0, 0, 0, 0, 0, 0]);
    h
}

fn file(f6: u8, f7: u8, prg: u8, chr: u8) -> Vec<u8> {
    let mut d = header(prg, chr, f6, f7);
    if f6 & 4 != 0 {
        d.extend((0..512).map(|i| i as u8));
    }
    d.extend((0..usize::from(prg) * 0x4000).map(|i| (i >> 8) as u8 ^ i as u8));
    d.extend(vec![0xCC; usize::from(chr) * 0x2000]);
    d
}

#[test]
fn every_flag6_value_decodes() {
    for f6 in 0..=255u8 {
        let c = Ines::parse(&file(f6, 0, 1, 1)).unwrap();
        assert_eq!(c.mapper, u16::from(f6 >> 4));
        assert_eq!(c.battery, f6 & 2 != 0);
        assert_eq!(c.trainer.is_some(), f6 & 4 != 0);
        let m = match (f6 & 8 != 0, f6 & 1 != 0) {
            (true, _) => Mirroring::FourScreen,
            (_, true) => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        };
        assert_eq!(c.mirroring, m, "{f6:02X}");
        assert_eq!(c.prg.len(), 0x4000);
        assert_eq!(c.chr, vec![0xCC; 0x2000]);
        if let Some(t) = &c.trainer {
            assert_eq!(t[511], 0xFF);
        }
        assert!(!c.nes2 && c.trailing.is_empty());
    }
}

#[test]
fn every_flag7_value_decodes() {
    for f7 in 0..=255u8 {
        let c = Ines::parse(&file(0x10, f7, 1, 0)).unwrap();
        assert_eq!(c.nes2, f7 & 0x0C == 0x08, "{f7:02X}");
        assert_eq!(c.mapper, u16::from(f7 & 0xF0) | 1);
        assert_eq!((c.vs, c.playchoice), (f7 & 1 != 0, f7 & 2 != 0));
    }
}

#[test]
fn diskdude_headers_ignore_byte_7() {
    let mut d = file(0x20, 0x40, 1, 0);
    d[7..16].copy_from_slice(b"\x40DiskDude");
    let c = Ines::parse(&d).unwrap();
    assert_eq!((c.mapper, c.vs), (2, false));
}

#[test]
fn nes2_fields_and_exponent_sizes() {
    let mut d = header(2, 0, 0x71, 0x08 | 0x30);
    d[8] = 0x5A; // submapper 5, mapper bits 11-8 = $A
    d[10] = 0x77; // 8 KiB + 8 KiB
    d[12] = 3;
    d.extend(vec![1; 0x8000]);
    let c = Ines::parse(&d).unwrap();
    assert_eq!(
        (c.mapper, c.submapper, c.prg_ram, c.timing),
        (0xA37, 5, 0x4000, Timing::Dendy)
    );
    assert_eq!(c.mirroring, Mirroring::Vertical);
    // Exponent form: E=13, M=1 -> 8 KiB * 3 = 24 KiB of PRG.
    let mut d = header((13 << 2) | 1, 0, 0, 0x08);
    d[9] = 0x0F;
    d.extend(vec![2; 24 * 1024]);
    assert_eq!(Ines::parse(&d).unwrap().prg.len(), 24 * 1024);
    // E=63 overflows.
    let mut d = header(0xFF, 0, 0, 0x08);
    d[9] = 0x0F;
    assert_eq!(Ines::parse(&d), Err(InesError::SizeOverflow));
    // MSB nibble extends the unit count.
    let mut d = header(0, 0, 0, 0x08);
    d[9] = 0x01;
    assert_eq!(
        Ines::parse(&d),
        Err(InesError::Truncated {
            need: 16 + 256 * 0x4000,
            have: 16
        })
    );
}

#[test]
fn errors() {
    for n in 0..16 {
        assert_eq!(Ines::parse(&vec![0; n]), Err(InesError::TooShort(n)));
    }
    let mut d = file(0, 0, 1, 0);
    d[3] = 0x1B;
    assert_eq!(Ines::parse(&d), Err(InesError::BadMagic(*b"NES\x1B")));
    assert_eq!(Ines::parse(&header(0, 1, 0, 0)), Err(InesError::NoPrg));
    let full = file(4, 0, 2, 1);
    for cut in [16, 527, 528, 528 + 0x8000, full.len() - 1] {
        assert_eq!(
            Ines::parse(&full[..cut]),
            Err(InesError::Truncated {
                need: full.len(),
                have: cut
            })
        );
    }
    let mut extra = full.clone();
    extra.extend([9, 8, 7]);
    assert_eq!(Ines::parse(&extra).unwrap().trailing, [9, 8, 7]);
}

#[test]
fn round_trip_both_formats() {
    for (f6, f7, prg, chr) in [
        (0x00, 0x00, 1, 0),
        (0x17, 0x23, 2, 3),
        (0x2E, 0x08, 4, 0),
        (0xF9, 0xF8, 1, 1),
    ] {
        let d = file(f6, f7, prg, chr);
        let c = Ines::parse(&d).unwrap();
        let again = Ines::parse(&c.to_bytes().unwrap()).unwrap();
        assert_eq!(again, c);
    }
    let mut c = Ines::parse(&file(0, 0, 1, 0)).unwrap();
    c.mapper = 0x100;
    assert_eq!(c.to_bytes(), Err(InesError::UnsupportedMapper(0x100)));
    c.mapper = 0;
    c.prg.push(0);
    assert_eq!(c.to_bytes(), Err(InesError::SizeOverflow));
}

#[test]
fn mapper_selection() {
    for (m, ok) in [
        (0u8, true),
        (1, true),
        (2, true),
        (3, true),
        (4, true),
        (5, true),
        (6, false),
        (7, true),
        (11, true),
        (34, true),
        (66, true),
    ] {
        let c = Ines::parse(&file((m & 15) << 4, m & 0xF0, 2, 0)).unwrap();
        match c.cartridge() {
            Ok(cart) => {
                assert!(ok, "{m}");
                if m == 5 {
                    // MMC5 powers up with only $5117 = $FF defined: the last bank
                    // sits at $E000-$FFFF.
                    assert_eq!(cart.peek(0xFFFF), c.prg[c.prg.len() - 1]);
                } else {
                    assert_eq!(cart.peek(0x8001), c.prg[1]);
                }
            }
            Err(e) => assert_eq!((ok, e), (false, InesError::UnsupportedMapper(u16::from(m)))),
        }
    }
    let c = Ines::parse(&file(0x70, 0, 1, 0)).unwrap();
    assert_eq!(
        c.cartridge().err(),
        Some(InesError::Mapper(MapperError::BadPrgSize(0x4000)))
    );
}

#[test]
fn axrom_switches_32k_banks_for_every_value() {
    let prg: Vec<u8> = (0..8 * 0x8000)
        .map(|i| (i / 0x8000) as u8 * 0x10 + (i % 7) as u8)
        .collect();
    for banks in [1usize, 2, 4, 8] {
        let mut a = Axrom::new(prg[..banks * 0x8000].to_vec()).unwrap();
        for v in 0..=255u8 {
            a.write(0x8000 | (u16::from(v) << 7), v);
            let bank = usize::from(v & 7) % banks;
            assert_eq!((a.bank, a.page), (bank, (v >> 4) & 1));
            for addr in [0x8000u16, 0xBFFF, 0xC000, 0xFFFF] {
                assert_eq!(
                    a.peek(addr),
                    prg[bank * 0x8000 + usize::from(addr - 0x8000)]
                );
            }
        }
        a.write(0x7FFF, 0xFF);
        assert_eq!(a.peek(0x6000), 0);
    }
    assert_eq!(
        Axrom::new(vec![0; 0x4000]).err(),
        Some(MapperError::BadPrgSize(0x4000))
    );
    assert_eq!(
        Axrom::new(vec![0; 9 * 0x8000]).err(),
        Some(MapperError::TooManyBanks(9))
    );
}

fn tagged(banks: usize) -> Vec<u8> {
    (0..banks * 0x8000)
        .map(|i| (((i / 0x8000) as u8) << 6) | 0x3F)
        .collect()
}

#[test]
fn gxrom_every_value_with_bus_conflicts() {
    for (prg_banks, chr_banks) in [(1usize, 1usize), (2, 4), (4, 2), (4, 4)] {
        let mut prg = tagged(prg_banks);
        // A ROM byte that masks every bit except 5-4 and 1-0 at $8123.
        for b in 0..prg_banks {
            prg[b * 0x8000 + 0x123] = 0x33;
        }
        let mut g = Latch::gxrom(prg.clone(), chr_banks).unwrap();
        for v in 0..=255u8 {
            g.write(0x8000, v);
            let pb = usize::from(((v & 0x3F) >> 4) & 3) % prg_banks;
            let cb = usize::from(v & 0x3F & 3) % chr_banks;
            assert_eq!((g.prg_bank, g.chr_bank), (pb, cb), "{v:02X}");
            assert_eq!(g.peek(0xFFFF), prg[pb * 0x8000 + 0x7FFF]);
            g.write(0x8123, v);
            let m = v & 0x33;
            assert_eq!(g.prg_bank, usize::from((m >> 4) & 3) % prg_banks);
            assert_eq!(g.chr_bank, usize::from(m & 3) % chr_banks);
        }
    }
    assert_eq!(
        Latch::gxrom(vec![0; 0x4000], 1).err(),
        Some(MapperError::BadPrgSize(0x4000))
    );
    assert_eq!(
        Latch::gxrom(vec![0; 5 * 0x8000], 1).err(),
        Some(MapperError::TooManyBanks(5))
    );
}

#[test]
fn cnrom_keeps_prg_fixed_and_conflicts_the_chr_latch() {
    for size in [0x4000usize, 0x8000] {
        let prg: Vec<u8> = (0..size).map(|i| i as u8 | 1).collect();
        let mut c = Latch::cnrom(prg.clone(), 4).unwrap();
        for v in 0..=255u8 {
            let addr = 0x8000 + u16::from(v) * 0x80;
            c.write(addr, v);
            let rom = prg[usize::from(addr - 0x8000) % size];
            assert_eq!(c.chr_bank, usize::from(v & rom & 3), "{v:02X}");
            assert_eq!(c.prg_bank, 0);
            for a in [0x8000u16, 0xC000, 0xFFFF] {
                assert_eq!(c.peek(a), prg[usize::from(a - 0x8000) % size]);
            }
        }
    }
    assert_eq!(
        Latch::cnrom(vec![0; 0xC000], 1).err(),
        Some(MapperError::BadPrgSize(0xC000))
    );
    assert_eq!(
        Latch::cnrom(vec![0; 0x4000], 5).err(),
        Some(MapperError::TooManyBanks(5))
    );
}
