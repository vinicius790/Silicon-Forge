//! MMC3 banking/IRQ over full register ranges, and the PPU wired into the 2A03 machine:
//! NMI, OAM DMA stall, CHR ROM from iNES, MMC3 scanline IRQ reaching the CPU.

use mos6502::bus::MemoryBus;
use mos6502::ines::MAGIC;
use mos6502::mapper::{Cartridge, MapperError, Mmc3};
use mos6502::ppu::Mirroring;
use mos6502::systems::nes2a03::Nes2A03;

fn prg(banks8: usize) -> Vec<u8> {
    (0..banks8 * 0x2000).map(|i| (i / 0x2000) as u8).collect()
}

#[test]
fn prg_banking_every_value_both_modes() {
    for banks in [2usize, 4, 16, 64] {
        let mut m = Mmc3::new(prg(banks), false).unwrap();
        for mode in [0u8, 0x40] {
            for v in 0..=255u8 {
                m.write(0x8000, mode | 6);
                m.write(0x8001, v);
                m.write(0x8000, mode | 7);
                m.write(0x8001, v ^ 0x15);
                let r6 = usize::from(v & 0x3F) % banks;
                let r7 = usize::from((v ^ 0x15) & 0x3F) % banks;
                let want = if mode == 0 {
                    [r6, r7, banks - 2, banks - 1]
                } else {
                    [banks - 2, r7, r6, banks - 1]
                };
                for (slot, &bank) in want.iter().enumerate() {
                    assert_eq!(
                        usize::from(m.peek(0x8000 + slot as u16 * 0x2000 + 0x123)),
                        bank,
                        "{banks} {mode:02X} {v:02X} {slot}"
                    );
                }
            }
        }
    }
    assert_eq!(
        Mmc3::new(vec![0; 0x2000], false).err(),
        Some(MapperError::BadPrgSize(0x2000))
    );
    assert_eq!(
        Mmc3::new(vec![0; 0x5000], false).err(),
        Some(MapperError::BadPrgSize(0x5000))
    );
}

#[test]
fn chr_banking_every_value_and_inversion() {
    let mut m = Mmc3::new(prg(4), false).unwrap();
    for inv in [0u8, 0x80] {
        for v in 0..=255u8 {
            for r in 0..6u8 {
                m.write(0x8000, inv | r);
                m.write(0x8001, v.wrapping_add(r * 3));
            }
            let reg = |r: u8| usize::from(v.wrapping_add(r * 3));
            let mut want = [
                reg(0) & !1,
                (reg(0) & !1) + 1,
                reg(1) & !1,
                (reg(1) & !1) + 1,
                reg(2),
                reg(3),
                reg(4),
                reg(5),
            ];
            if inv != 0 {
                want.rotate_left(4);
            }
            for slot in 0..8u16 {
                assert_eq!(
                    m.chr_offset(slot * 0x400 + 0x3FF),
                    want[usize::from(slot)] * 0x400 + 0x3FF
                );
            }
        }
    }
}

#[test]
fn mirroring_ram_protect_and_irq_counter() {
    let mut m = Mmc3::new(prg(4), false).unwrap();
    for v in 0..=255u8 {
        m.write(0xA000, v);
        assert_eq!(
            m.mirroring(),
            Some(if v & 1 != 0 {
                Mirroring::Horizontal
            } else {
                Mirroring::Vertical
            })
        );
    }
    let four = Mmc3::new(prg(4), true).unwrap();
    assert_eq!(four.mirroring(), Some(Mirroring::FourScreen));
    for (p, readable, writable) in [
        (0x80u8, true, true),
        (0xC0, true, false),
        (0x00, false, false),
        (0x40, false, false),
    ] {
        m.write(0xA001, 0x80);
        m.write(0x6000, 0x11);
        m.write(0xA001, p);
        m.write(0x6000, 0x22);
        m.write(0xA001, 0x80);
        assert_eq!(
            m.peek(0x6000),
            if writable { 0x22 } else { 0x11 },
            "{p:02X}"
        );
        m.write(0xA001, p);
        assert_eq!(m.peek(0x6000) != 0, readable);
    }
    // Counter: latch N fires after N+1 edges from reload, then every N+1.
    for latch in [0u8, 1, 5, 255] {
        let mut m = Mmc3::new(prg(4), false).unwrap();
        m.write(0xC000, latch);
        m.write(0xC001, 0);
        m.write(0xE001, 0);
        let mut fires = Vec::new();
        for edge in 1..=600u32 {
            m.ppu_address(0x0000);
            for _ in 0..Mmc3::A12_FILTER {
                m.clock();
            }
            m.ppu_address(0x1000);
            if m.irq() {
                fires.push(edge);
                m.write(0xE000, 0);
                m.write(0xE001, 0);
            }
        }
        let period = u32::from(latch) + 1;
        if latch == 0 {
            assert_eq!(
                fires,
                (1..=600).collect::<Vec<_>>(),
                "Sharp: latch 0 fires every clock"
            );
        } else {
            assert_eq!(fires[0], period, "{latch}: reload edge counts as the first");
            assert!(
                fires.windows(2).all(|w| w[1] - w[0] == period),
                "{latch} {fires:?}"
            );
        }
        // Held A12 high never clocks again.
        let e = m.edges;
        m.ppu_address(0x1000);
        m.ppu_address(0x1FFF);
        assert_eq!(m.edges, e);
    }
}

fn nes_file(mapper: u8, prg_rom: &[u8], chr: &[u8], f6_extra: u8) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([
        (prg_rom.len() / 0x4000) as u8,
        (chr.len() / 0x2000) as u8,
        (mapper << 4) | f6_extra,
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
    f.extend_from_slice(prg_rom);
    f.extend_from_slice(chr);
    f
}

fn program(src: &str, size: usize) -> Vec<u8> {
    let base = (0x10000 - size) as u16;
    let img = mos6502::asm::assemble_with_symbols(src, base).unwrap();
    let mut p = vec![0xEA; size];
    let at = usize::from(img.origin - base);
    p[at..at + img.bytes.len()].copy_from_slice(&img.bytes);
    p
}

#[test]
fn nmi_counts_frames_and_dma_stalls() {
    let src = "
.org $C000
reset: sei
 ldx #$FF
 txs
 lda #$80
 sta $2000      ; NMI on
loop: jmp loop
nmi: inc $10
 lda #$02
 sta $4014      ; OAM DMA from $0200
 rti
irq: rti
.org $FFFA
.word nmi
.word reset
.word irq
";
    let rom = program(src, 0x4000);
    let mut nes = Nes2A03::from_ines(&nes_file(0, &rom, &[0x5A; 0x2000], 1)).unwrap();
    assert_eq!(nes.cpu.bus.mirroring, Mirroring::Vertical);
    assert_eq!(nes.cpu.bus.chr[5], 0x5A);
    assert!(!nes.cpu.bus.chr_writable);
    for i in 0..256u16 {
        nes.cpu.bus.write(0x0200 + i, (i as u8).wrapping_mul(7));
    }
    // ~10 frames of CPU time (29780.67 cycles each).
    nes.run_cycles(297_807).unwrap();
    let frames = nes.cpu.bus.peek(0x10);
    assert!((9..=10).contains(&frames), "{frames}");
    let dmas = u64::from(frames);
    assert!(
        (513 * dmas..=514 * dmas).contains(&nes.cpu.bus.oam_dma_cycles),
        "{}",
        nes.cpu.bus.oam_dma_cycles
    );
    assert_eq!(nes.cpu.bus.ppu.oam[37], 37u8.wrapping_mul(7));
    // CHR ROM ignores PPU writes.
    nes.cpu.bus.write(0x2006, 0);
    nes.cpu.bus.write(0x2006, 5);
    nes.cpu.bus.write(0x2007, 0x00);
    assert_eq!(nes.cpu.bus.chr[5], 0x5A);
}

#[test]
fn mmc3_irq_reaches_the_cpu_on_a_scanline() {
    let src = "
.org $E000
reset: sei
 ldx #$FF
 txs
 lda #$40
 sta $4017      ; APU frame IRQ off (it would otherwise keep IRQ asserted)
 lda #$08
 sta $2000      ; sprites at $1000
 lda #$18
 sta $2001      ; render
 lda #20
 sta $C000      ; latch: line 32 (hex 20)
 sta $C001
 sta $E001
 cli
loop: jmp loop
irq: inc $11
 lda $2002
 sta $E000      ; ack
 sta $E001
 rti
nmi: rti
.org $FFFA
.word nmi
.word reset
.word irq
";
    let rom = program(src, 0x2000);
    let mut prg_rom = vec![0; 0x6000];
    prg_rom.extend(rom);
    let mut nes = Nes2A03::from_ines(&nes_file(4, &prg_rom, &[0; 0x2000], 0)).unwrap();
    nes.run_cycles(29_781 * 3).unwrap();
    let irqs = nes.cpu.bus.peek(0x11);
    // One per 33 rendered lines (latch $20 + 1) over ~241 edges per frame.
    assert!((20..=23).contains(&irqs), "{irqs}");
    assert_eq!(nes.cpu.bus.ppu.frame, 3);
}

#[test]
fn a12_filter_ignores_short_low_pulses() {
    for low in 0..8u32 {
        let mut m = Mmc3::new(prg(4), false).unwrap();
        m.ppu_address(0x1000);
        let e = m.edges;
        m.ppu_address(0x0000);
        for _ in 0..low {
            m.clock();
        }
        m.ppu_address(0x1000);
        assert_eq!(
            m.edges - e,
            u64::from(low >= Mmc3::A12_FILTER),
            "low for {low}"
        );
        // Clocks while high do not count toward the next low period.
        for _ in 0..10 {
            m.clock();
        }
        m.ppu_address(0x0000);
        m.ppu_address(0x1000);
        assert_eq!(m.edges - e, u64::from(low >= Mmc3::A12_FILTER));
    }
}
