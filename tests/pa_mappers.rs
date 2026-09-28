//! MMC2 (9), MMC4 (10), NINA-001 (34), Camerica (71), VRC1 (75) over every register
//! value, NES 2.0 submappers, bus-conflict options, MMC3 IRQ revisions, and mapper
//! save states. PRG images are generated: each byte holds its bank number.

use mos6502::ines::{Ines, InesError, MAGIC};
use mos6502::mapper::{
    Axrom, Camerica, Cartridge, Latch, MapperError, Mmc24, Mmc3, Nina001, Uxrom, Vrc1,
};
use mos6502::ppu::Mirroring;
use mos6502::systems::nes2a03::Nes2A03;

fn prg(len: usize, unit: usize) -> Vec<u8> {
    (0..len).map(|i| (i / unit) as u8).collect()
}

fn image(mapper: u8, sub: u8, prg16: u8, chr8: u8, flags6: u8) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([
        prg16,
        chr8,
        ((mapper & 15) << 4) | flags6,
        (mapper & 0xF0) | 0x08,
        sub << 4,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]);
    let mut p = prg(usize::from(prg16) * 0x4000, 0x2000);
    // Reset vector at the end of the last bank: $8000.
    let n = p.len();
    p[n - 4] = 0x00;
    p[n - 3] = 0x80;
    f.extend(p);
    f.extend((0..usize::from(chr8) * 0x2000).map(|i| (i / 0x1000) as u8));
    f
}

#[test]
fn mmc2_prg_every_value_and_fixed_banks() {
    for banks in [4usize, 8, 16] {
        let mut m = Mmc24::mmc2(prg(banks * 0x2000, 0x2000)).unwrap();
        for v in 0..=255u8 {
            m.write(0xA000 | (u16::from(v) << 4), v);
            let want = [usize::from(v & 15) % banks, banks - 3, banks - 2, banks - 1];
            for (slot, &b) in want.iter().enumerate() {
                assert_eq!(usize::from(m.peek(0x8000 + slot as u16 * 0x2000 + 5)), b);
            }
            assert_eq!(m.peek(0x6000), 0, "MMC2 has no PRG RAM");
        }
    }
    assert_eq!(
        Mmc24::mmc2(vec![0; 0x6000]).err(),
        Some(MapperError::BadPrgSize(0x6000))
    );
    assert_eq!(
        Mmc24::mmc2(vec![0; 0x22000]).err(),
        Some(MapperError::TooManyBanks(17))
    );
}

#[test]
fn mmc4_prg_ram_and_16k_banking() {
    for banks in [2usize, 8, 16] {
        let mut m = Mmc24::mmc4(prg(banks * 0x4000, 0x4000)).unwrap();
        for v in 0..=255u8 {
            m.write(0xA000, v);
            assert_eq!(usize::from(m.peek(0x8123)), usize::from(v & 15) % banks);
            assert_eq!(usize::from(m.peek(0xC123)), banks - 1);
        }
        m.write(0x6789, 0x5A);
        assert_eq!(m.peek(0x6789), 0x5A);
    }
}

#[test]
fn mmc2_chr_registers_mirroring_and_latch_selection() {
    let mut m = Mmc24::mmc2(prg(0x8000, 0x2000)).unwrap();
    for v in 0..=255u8 {
        for (i, reg) in [0xB000u16, 0xC000, 0xD000, 0xE000].iter().enumerate() {
            m.write(reg + 0x0ABC, v.wrapping_add(i as u8 * 5));
        }
        let r = |i: u8| usize::from(v.wrapping_add(i * 5) & 0x1F);
        for (l0, l1) in [(false, false), (false, true), (true, false), (true, true)] {
            m.latch = [l0, l1];
            let lo = if l0 { r(1) } else { r(0) };
            let hi = if l1 { r(3) } else { r(2) };
            assert_eq!(m.chr_offset(0x0123), lo * 0x1000 + 0x123);
            assert_eq!(m.chr_offset(0x1FFF), hi * 0x1000 + 0xFFF);
        }
        m.write(0xF000, v);
        let want = if v & 1 == 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };
        assert_eq!(m.mirroring(), Some(want));
    }
}

#[test]
fn nina001_registers_live_in_prg_ram() {
    let mut m = Nina001::new(prg(0x10000, 0x8000)).unwrap();
    for v in 0..=255u8 {
        m.write(0x7FFD, v);
        m.write(0x7FFE, v ^ 0x33);
        m.write(0x7FFF, v ^ 0x5C);
        assert_eq!(m.peek(0x8000), v & 1);
        assert_eq!(m.peek(0xFFFF), v & 1);
        assert_eq!(m.peek(0x7FFD), v, "the write also lands in RAM");
        assert_eq!(
            m.chr_offset(0x0010),
            usize::from((v ^ 0x33) & 15) * 0x1000 + 0x10
        );
        assert_eq!(
            m.chr_offset(0x1010),
            usize::from((v ^ 0x5C) & 15) * 0x1000 + 0x10
        );
    }
    let before = (m.prg_bank, m.chr);
    m.write(0x8000, 0xFF);
    m.write(0x7FFC, 0xFF);
    assert_eq!((m.prg_bank, m.chr), before, "no other register");
    assert_eq!(
        Nina001::new(vec![0; 0x18000]).err(),
        Some(MapperError::TooManyBanks(3))
    );
}

#[test]
fn camerica_bank_register_and_fire_hawk_mirroring() {
    for fire_hawk in [false, true] {
        let mut m = Camerica::new(prg(16 * 0x4000, 0x4000), fire_hawk).unwrap();
        for v in 0..=255u8 {
            for addr in [0xC000u16, 0xD555, 0xFFFF] {
                m.write(addr, v);
                assert_eq!(m.peek(0x8000), v & 15);
                assert_eq!(m.peek(0xC000), 15);
            }
            m.write(0xA000, v ^ 0x0F);
            assert_eq!(m.peek(0x8000), v & 15, "$A000-$BFFF is not a register");
            m.write(0x9000, v);
            let want = fire_hawk.then_some(if v & 0x10 == 0 {
                Mirroring::SingleLow
            } else {
                Mirroring::SingleHigh
            });
            assert_eq!(m.mirroring(), want);
        }
    }
}

#[test]
fn vrc1_every_register_value() {
    for four in [false, true] {
        let mut m = Vrc1::new(prg(16 * 0x2000, 0x2000), four).unwrap();
        for v in 0..=255u8 {
            m.write(0x8000, v);
            m.write(0xA000, v ^ 3);
            m.write(0xC000, v ^ 6);
            assert_eq!(
                [
                    m.peek(0x8000),
                    m.peek(0xA000),
                    m.peek(0xC000),
                    m.peek(0xE000)
                ],
                [v & 15, (v ^ 3) & 15, (v ^ 6) & 15, 15]
            );
            m.write(0xE000, v);
            m.write(0xF000, v >> 4);
            m.write(0x9000, v);
            let c0 = usize::from((v & 15) | ((v & 2) << 3));
            let c1 = usize::from(((v >> 4) & 15) | ((v & 4) << 2));
            assert_eq!(m.chr_offset(0x0042), c0 * 0x1000 + 0x42);
            assert_eq!(m.chr_offset(0x1042), c1 * 0x1000 + 0x42);
            let mir = if v & 1 == 0 {
                Mirroring::Vertical
            } else {
                Mirroring::Horizontal
            };
            assert_eq!(m.mirroring(), (!four).then_some(mir));
            let before = m.chr;
            m.write(0xB000, 0xFF);
            m.write(0xD000, 0xFF);
            assert_eq!(m.chr, before);
        }
    }
}

#[test]
fn bus_conflict_options() {
    let rom = |b: u8| -> Vec<u8> { vec![b; 0x8000] };
    let mut u = Uxrom::new(rom(0x05)).unwrap();
    u.write(0x8000, 0x07);
    assert_eq!(u.bank, 0x05 & 0x07 & 1, "AND with ROM, 2 banks");
    let mut u = Uxrom::new(prg(0x40000, 0x4000)).unwrap();
    u.bus_conflicts = false;
    for v in 0..=255u8 {
        u.write(0x8000, v);
        assert_eq!(
            u.bank,
            usize::from(v & 15),
            "submapper 1: the byte as written"
        );
    }
    let mut c = Latch::cnrom(rom(0x01), 4).unwrap();
    c.write(0x8000, 3);
    assert_eq!(c.chr_bank, 1);
    c.bus_conflicts = false;
    c.write(0x8000, 3);
    assert_eq!(c.chr_bank, 3);
    let mut a = Axrom::new(prg(0x40000, 0x8000)).unwrap();
    a.write(0x8000, 6);
    assert_eq!(a.bank, 6, "ANROM: no conflicts");
    a.bus_conflicts = true;
    a.write(0x8000, 0x05); // ROM byte here is 6
    assert_eq!(a.bank, 4);
}

#[test]
fn mmc3_old_irq_revision_ignores_a_zero_reload() {
    for alt in [false, true] {
        let mut m = Mmc3::new(vec![0; 0x8000], false).unwrap();
        m.alt_irq = alt;
        m.write(0xC000, 0);
        m.write(0xE001, 0);
        let clock = |m: &mut Mmc3| {
            for _ in 0..4 {
                m.clock();
            }
            m.ppu_address(0x1000);
            m.ppu_address(0x0000);
        };
        clock(&mut m);
        // Counter was 0 and reloaded 0: Sharp fires, NEC does not.
        assert_eq!(m.irq(), !alt, "alt {alt}");
        m.write(0xE000, 0);
        m.write(0xE001, 0);
        m.write(0xC001, 0);
        clock(&mut m);
        assert!(m.irq(), "reload flag: both fire");
        m.write(0xE000, 0);
        m.write(0xE001, 0);
        m.write(0xC000, 2);
        m.write(0xC001, 0);
        let mut fired = Vec::new();
        for _ in 0..6 {
            clock(&mut m);
            fired.push(m.irq());
            m.write(0xE000, 0);
            m.write(0xE001, 0);
        }
        // reload 2, 1, 0 (fires), reload 2, 1, 0 (fires).
        assert_eq!(fired, vec![false, false, true, false, false, true]);
    }
}

#[test]
fn ines_submappers_select_boards() {
    let kind = |m: u8, s: u8, p: u8, c: u8| {
        Ines::parse(&image(m, s, p, c, 0))
            .unwrap()
            .cartridge()
            .map(|c| c.state_kind())
    };
    assert_eq!(kind(9, 0, 8, 2), Ok(9));
    assert_eq!(kind(10, 0, 8, 2), Ok(10));
    assert_eq!(kind(71, 0, 8, 0), Ok(71));
    assert_eq!(kind(71, 1, 8, 0), Ok(71));
    assert_eq!(kind(75, 0, 8, 4), Ok(75));
    assert_eq!(kind(34, 1, 4, 1), Ok(34));
    assert_eq!(kind(34, 2, 4, 0), Ok(34));
    assert_eq!(kind(4, 1, 4, 1), Ok(4), "MMC6");
    for (m, s) in [(2u8, 3u8), (3, 5), (4, 2), (7, 3), (34, 3), (71, 2)] {
        assert_eq!(
            kind(m, s, 4, 1),
            Err(InesError::UnsupportedSubmapper(u16::from(m), s))
        );
    }
    // NINA-001 vs BNROM: submapper 2 forces BNROM even with CHR ROM present, and
    // the machine boots the right one (BNROM ignores $7FFD).
    for (sub, nina) in [(1u8, true), (2, false), (0, true)] {
        let mut nes = Nes2A03::from_ines(&image(34, sub, 4, 2, 0)).unwrap();
        let cart = nes.cpu.bus.cart.as_mut().unwrap();
        cart.write(0x7FFD, 1);
        // 64 KiB PRG of 8 KiB-numbered bytes: 32 KiB bank 1 starts with 4.
        assert_eq!(cart.peek(0x8000) == 4, nina, "sub {sub}");
    }
}

#[test]
fn fire_hawk_submapper_boots_with_mirroring_control() {
    let mut nes = Nes2A03::from_ines(&image(71, 1, 4, 0, 0)).unwrap();
    let c = nes.cpu.bus.cart.as_mut().unwrap();
    c.write(0x9000, 0x10);
    assert_eq!(c.mirroring(), Some(Mirroring::SingleHigh));
    let mut nes = Nes2A03::from_ines(&image(71, 0, 4, 0, 0)).unwrap();
    let c = nes.cpu.bus.cart.as_mut().unwrap();
    c.write(0x9000, 0x10);
    assert_eq!(c.mirroring(), None);
}

#[test]
fn new_mappers_round_trip_through_machine_states() {
    for (m, p, c) in [
        (9u8, 8u8, 4u8),
        (10, 8, 4),
        (34, 4, 2),
        (71, 8, 0),
        (75, 8, 4),
    ] {
        let data = image(m, 0, p, c, 0);
        let mut a = Nes2A03::from_ines(&data).unwrap();
        {
            let cart = a.cpu.bus.cart.as_mut().unwrap();
            for (addr, v) in [
                (0xA000u16, 3u8),
                (0xB000, 5),
                (0xE000, 6),
                (0x9000, 0x17),
                (0x7FFD, 1),
                (0x7FFE, 7),
                (0xC000, 2),
                (0x6000, 0x99),
            ] {
                cart.write(addr, v);
            }
        }
        a.run_cycles(3000).unwrap();
        let s = a.save_state();
        let mut b = Nes2A03::from_ines(&data).unwrap();
        b.load_state(&s).unwrap();
        assert_eq!(b.save_state(), s, "mapper {m}");
        a.run_cycles(5000).unwrap();
        b.run_cycles(5000).unwrap();
        assert_eq!(a.save_state(), b.save_state(), "mapper {m}");
        // Corrupt a register byte past its range: refused, machine unchanged.
        let mut bad = s.clone();
        let n = bad.len();
        bad[n - 1] = 0xFF;
        let before = b.save_state();
        if b.load_state(&bad).is_err() {
            assert_eq!(b.save_state(), before);
        }
    }
}
