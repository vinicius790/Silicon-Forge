//! VRC6 nametable/A10 rules for every `$B003` value, VRC7 banking and IRQ on both
//! boards, and the MMC3 boards MMC6 (HKROM), TxSROM (118) and TQROM (119).

use mos6502::ines::MAGIC;
use mos6502::mapper::{Cartridge, Mmc3, Mmc3Board, PpuTarget, Vrc6, Vrc7};
use mos6502::ppu::{Mirroring, Ppu, PpuMemory};
use mos6502::systems::nes2a03::Nes2A03;

fn prg(banks8: usize) -> Vec<u8> {
    (0..banks8 * 0x2000).map(|i| (i / 0x2000) as u8).collect()
}

/// NESdev VRC6 nametable tables, transcribed: register per quadrant from
/// `$B003 & 7` and, with bit 5 set, the forced LSB per mode/mirroring
/// ('a' = PPU A10, 'b' = PPU A11, 'r' = register LSB, '0'/'1').
fn wiki(ctrl: u8) -> [(usize, char); 4] {
    let regs = match ctrl & 7 {
        0 | 6 | 7 => [6, 6, 7, 7],
        1 | 5 => [4, 5, 6, 7],
        _ => [6, 7, 6, 7],
    };
    let rule = if ctrl & 0x20 == 0 {
        'r'
    } else {
        match (ctrl & 3, (ctrl >> 2) & 3) {
            (0, 0) => 'a',
            (0, 1) => 'b',
            (0, 2) => '0',
            (0, 3) => '1',
            (3, 0) => 'b',
            (3, 1) => 'a',
            (3, 2) => '1',
            (3, 3) => '0',
            _ => 'r',
        }
    };
    [0, 1, 2, 3].map(|q| (regs[q], rule))
}

#[test]
fn vrc6_nametables_every_control_value() {
    let mut c = Vrc6::new(prg(16), false).unwrap();
    let vals = [0x10u8, 0x21, 0x32, 0x43, 0x54, 0x65, 0x76, 0x87];
    for (i, &v) in vals.iter().enumerate() {
        let r = i as u16;
        c.write(if r < 4 { 0xD000 + r } else { 0xE000 + r - 4 }, v);
    }
    for ctrl in 0..=0x3Fu8 {
        c.write(0xB003, ctrl);
        for (q, (reg, rule)) in wiki(ctrl).iter().enumerate() {
            let rv = usize::from(vals[*reg]);
            let lsb = match rule {
                'a' => q & 1,
                'b' => q >> 1,
                '0' => 0,
                '1' => 1,
                _ => rv & 1,
            };
            let page = (rv & !1) | lsb;
            let addr = 0x2000 + 0x400 * q as u16 + 0x123;
            assert_eq!(c.nametable_page(addr), page, "ctrl {ctrl:02X} q{q}");
            let want = if ctrl & 0x10 != 0 {
                PpuTarget::Chr(page * 0x400 + 0x123)
            } else {
                PpuTarget::Ciram((page & 1) * 0x400 + 0x123)
            };
            assert_eq!(c.ppu_target(addr), want);
            assert_eq!(
                c.ppu_target(addr + 0x1000),
                c.ppu_target(addr),
                "$3xxx mirrors"
            );
        }
        assert_eq!(c.ppu_target(0x1234), PpuTarget::Default);
    }
}

#[test]
fn vrc6_rom_nametables_through_the_ppu() {
    let mut c = Vrc6::new(prg(16), false).unwrap();
    c.write(0xE002, 5); // R6
    c.write(0xE003, 9); // R7
    c.write(0xB003, 0x32); // bit 5, bit 4 (ROM), mode 2, vertical
    let mut chr: Vec<u8> = (0..0x8000).map(|i| (i >> 10) as u8).collect();
    let ppu = Ppu::new();
    let m = PpuMemory {
        cart: Some(&mut c),
        chr: &mut chr,
        chr_writable: false,
        mirroring: Mirroring::Horizontal,
    };
    assert_eq!(ppu.mem_read(&m, 0x2000), 5);
    assert_eq!(ppu.mem_read(&m, 0x2400), 9);
    assert_eq!(ppu.mem_read(&m, 0x2800), 5);
    assert_eq!(ppu.mem_read(&m, 0x2C00), 9);
}

fn vrc7_addr(sub: u8, base: u16, second: bool) -> u16 {
    base | if second {
        if sub == 1 {
            0x08
        } else {
            0x10
        }
    } else {
        0
    }
}

#[test]
fn vrc7_banking_mirroring_ram_both_boards() {
    for sub in [0u8, 1, 2] {
        let mut c = Vrc7::new(prg(32), sub).unwrap();
        for b in 0..64u8 {
            c.write(0x8000, b | 0xC0);
            c.write(vrc7_addr(sub.max(1), 0x8000, true), b ^ 1);
            c.write(0x9000, b ^ 2);
            assert_eq!(c.peek(0x8000), b % 32, "sub {sub}");
            assert_eq!(c.peek(0xA000), (b ^ 1) % 32);
            assert_eq!(c.peek(0xDFFF), (b ^ 2) % 32);
            assert_eq!(c.peek(0xE000), 31);
        }
        for slot in 0..8u16 {
            let base = 0xA000 + (slot / 2) * 0x1000;
            c.write(
                vrc7_addr(sub.max(1), base, slot % 2 == 1),
                0x40 + slot as u8,
            );
        }
        for a in (0..0x2000u16).step_by(0x1FF) {
            assert_eq!(
                c.chr_offset(a),
                usize::from(0x40 + (a >> 10)) * 0x400 + usize::from(a & 0x3FF)
            );
        }
        for (v, m) in [
            (0u8, Mirroring::Vertical),
            (1, Mirroring::Horizontal),
            (2, Mirroring::SingleLow),
            (3, Mirroring::SingleHigh),
        ] {
            c.write(0xE000, v);
            assert_eq!(c.mirroring(), Some(m));
        }
        c.write(0x6000, 7);
        assert_eq!(c.peek(0x6000), 0, "RAM protected");
        c.write(0xE000, 0x80);
        c.write(0x6000, 7);
        assert_eq!(c.peek(0x6000), 7);
        c.write(0x9010, 0x25);
        c.write(0x9030, 0x99);
        assert_eq!((c.audio_select, c.audio_regs[0x25]), (0x25, 0x99));
        assert_eq!(c.audio(), 0.0, "OPLL not synthesized");
    }
    // On VRC7b A4 is not a register line: $8010 writes PRG 0.
    let mut c = Vrc7::new(prg(32), 1).unwrap();
    c.write(0x8010, 3);
    assert_eq!((c.peek(0x8000), c.peek(0xA000)), (3, 0));
    let mut c = Vrc7::new(prg(32), 2).unwrap();
    c.write(0x8008, 3);
    assert_eq!((c.peek(0x8000), c.peek(0xA000)), (3, 0));
}

#[test]
fn vrc7_irq_cycle_and_ack() {
    for sub in [1u8, 2] {
        for latch in [0xF0u8, 0x00, 0x7F] {
            let mut c = Vrc7::new(prg(8), sub).unwrap();
            c.write(vrc7_addr(sub, 0xE000, true), latch);
            c.write(0xF000, 0x06);
            let mut k = 0;
            while !c.irq() {
                c.clock();
                k += 1;
                assert!(k < 1000);
            }
            assert_eq!(k, 256 - u32::from(latch));
            c.write(vrc7_addr(sub, 0xF000, true), 0);
            assert!(!c.irq());
        }
    }
}

#[test]
fn mmc6_ram_protect_every_value() {
    for enable in [false, true] {
        for p in 0..16u8 {
            let mut c = Mmc3::with_board(prg(8), false, Mmc3Board::Mmc6).unwrap();
            c.write(0x8000, if enable { 0x20 } else { 0 });
            c.write(0xA001, 0xF0);
            c.write(0x7000, 0x11);
            c.write(0x7200, 0x22);
            c.write(0xA001, p << 4);
            let (wl, rl, wh, rh) = (p & 1 != 0, p & 2 != 0, p & 4 != 0, p & 8 != 0);
            let open = !enable || !(rl || rh);
            let lo = if open {
                0
            } else if rl {
                if enable {
                    0x11
                } else {
                    0
                }
            } else {
                0
            };
            let hi = if open {
                0
            } else if rh {
                0x22
            } else {
                0
            };
            assert_eq!(c.mmc6_read(0x7000).unwrap_or(0), lo, "en {enable} p {p:X}");
            assert_eq!(c.mmc6_read(0x7200).unwrap_or(0), hi);
            assert_eq!(c.mmc6_read(0x7000).is_none(), open);
            // Mirrors every 1 KiB.
            assert_eq!(c.peek(0x7C00), c.peek(0x7000));
            c.write(0x7001, 0x33);
            c.write(0x7201, 0x44);
            c.write(0xA001, 0xF0);
            let can_lo = enable && wl && rl;
            let can_hi = enable && wh && rh;
            assert_eq!(
                c.mmc6_read(0x7001).unwrap_or(0) == 0x33,
                can_lo,
                "write lo {p:X}"
            );
            assert_eq!(
                c.mmc6_read(0x7201).unwrap_or(0) == 0x44,
                can_hi,
                "write hi {p:X}"
            );
        }
    }
    // Clearing $8000 bit 5 forces $A001 to 0 and ignores later writes.
    let mut c = Mmc3::with_board(prg(8), false, Mmc3Board::Mmc6).unwrap();
    c.write(0x8000, 0x20);
    c.write(0xA001, 0xF0);
    c.write(0x8000, 0x00);
    assert_eq!(c.ram_protect, 0);
    c.write(0xA001, 0xF0);
    assert_eq!(c.ram_protect, 0);
    c.write(0x6000, 1);
    assert_eq!(c.peek(0x6000), 0, "no RAM at $6000 on MMC6");
    assert_eq!(c.save_ram().unwrap().len(), 0x400);
}

#[test]
fn txsrom_chr_bit_7_selects_ciram() {
    let mut c = Mmc3::with_board(prg(8), false, Mmc3Board::TxSrom).unwrap();
    for (inv, regs) in [
        (0u8, [0x80u8, 0x00, 0x80, 0x00, 0x80, 0x00]),
        (0x80, [0x00, 0x80, 0x00, 0x80, 0x00, 0x80]),
    ] {
        c.write(0x8000, inv);
        for (r, &v) in regs.iter().enumerate() {
            c.write(0x8000, inv | r as u8);
            c.write(0x8001, v);
        }
        for q in 0..4u16 {
            let addr = 0x2000 + q * 0x400 + 7;
            let bank = c.chr_bank(usize::from(addr & 0x0FFF) >> 10);
            let want = PpuTarget::Ciram(((bank >> 7) & 1) * 0x400 + 7);
            assert_eq!(c.ppu_target(addr), want, "inv {inv:02X} q {q}");
        }
    }
    c.write(0x8000, 0);
    c.write(0x8001, 0x80); // R0: 2 KiB at $0000 -> $2000/$2400 on page 1
    c.write(0x8000, 1);
    c.write(0x8001, 0x00);
    let pages: Vec<_> = (0..4u16)
        .map(|q| c.ppu_target(0x2000 + q * 0x400))
        .collect();
    assert_eq!(
        pages,
        [
            PpuTarget::Ciram(0x400),
            PpuTarget::Ciram(0x400),
            PpuTarget::Ciram(0),
            PpuTarget::Ciram(0)
        ]
    );
    c.write(0xA000, 1);
    assert_eq!(
        c.ppu_target(0x2000),
        PpuTarget::Ciram(0x400),
        "$A000 bypassed"
    );
}

#[test]
fn tqrom_mixes_chr_rom_and_ram() {
    let mut c = Mmc3::with_board(prg(8), false, Mmc3Board::Tqrom).unwrap();
    let mut chr: Vec<u8> = (0..0x10000).map(|i| (i >> 10) as u8).collect();
    for (r, v) in [(2u8, 0x40u8), (3, 0x41), (4, 0x05), (5, 0x47)] {
        c.write(0x8000, r);
        c.write(0x8001, v);
    }
    let mut ppu = Ppu::new();
    {
        let mut m = PpuMemory {
            cart: Some(&mut c),
            chr: &mut chr,
            chr_writable: false,
            mirroring: Mirroring::Vertical,
        };
        assert_eq!(ppu.mem_read(&m, 0x1400), 0, "RAM page 1 starts cleared");
        ppu.mem_write(&mut m, 0x1000, 0xAA);
        ppu.mem_write(&mut m, 0x1400, 0xBB);
        ppu.mem_write(&mut m, 0x1800, 0xCC); // ROM: ignored
        ppu.mem_write(&mut m, 0x1C00, 0xDD); // RAM page 7
        assert_eq!(ppu.mem_read(&m, 0x1000), 0xAA);
        assert_eq!(ppu.mem_read(&m, 0x1400), 0xBB);
        assert_eq!(ppu.mem_read(&m, 0x1800), 5, "CHR ROM bank 5");
        assert_eq!(ppu.mem_read(&m, 0x1C00), 0xDD);
    }
    assert_eq!(
        (c.chr_ram[0], c.chr_ram[0x400], c.chr_ram[0x1C00]),
        (0xAA, 0xBB, 0xDD)
    );
    // The same RAM page through another register.
    c.write(0x8000, 4);
    c.write(0x8001, 0x41);
    assert_eq!(c.ppu_target(0x1800), PpuTarget::Mapper(0xBB));
}

fn image(mapper: u8, sub: u8, chr8: u8) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([
        8,
        chr8,
        ((mapper & 15) << 4) | 2,
        (mapper & 0xF0) | 8,
        sub << 4,
        0,
        0x07,
        0,
        0,
        0,
        0,
        0,
    ]);
    let mut p = vec![0xEAu8; 8 * 0x4000];
    let n = p.len();
    p[n - 0x2000..n - 0x1FFD].copy_from_slice(&[0x4C, 0x00, 0xE0]);
    p[n - 4..].copy_from_slice(&[0x00, 0xE0, 0x00, 0xE0]);
    f.extend(p);
    f.extend(vec![0; usize::from(chr8) * 0x2000]);
    f
}

#[test]
fn boards_from_ines_and_states() {
    for (m, sub) in [
        (85u8, 0u8),
        (85, 1),
        (85, 2),
        (118, 0),
        (119, 0),
        (4, 1),
        (4, 0),
    ] {
        let data = image(m, sub, 8);
        let mut a = Nes2A03::from_ines(&data).unwrap();
        {
            let c = a.cpu.bus.cart.as_mut().unwrap();
            for (addr, v) in [
                (0x8000u16, 0x22u8),
                (0x8001, 0x41),
                (0xA001, 0xF0),
                (0x7003, 9),
                (0xE000, 0x80),
                (0x6000, 4),
                (0xF000, 6),
            ] {
                c.write(addr, v);
            }
            c.ppu_write(0x0802, 0x5E);
        }
        a.run_cycles(30_000).unwrap();
        let s = a.save_state();
        let mut b = Nes2A03::from_ines(&data).unwrap();
        b.load_state(&s).unwrap();
        a.run_cycles(20_000).unwrap();
        b.run_cycles(20_000).unwrap();
        assert_eq!(a.save_state(), b.save_state(), "{m}.{sub}");
        if m == 4 {
            let v6 = a.save_state_version(6);
            Nes2A03::from_ines(&data).unwrap().load_state(&v6).unwrap();
        }
    }
    assert!(
        Nes2A03::from_ines(&image(85, 3, 8)).is_err(),
        "unknown VRC7 submapper"
    );
    let mmc6 = Nes2A03::from_ines(&image(4, 1, 8)).unwrap();
    assert_eq!(mmc6.save_ram().unwrap().len(), 0x400);
}
