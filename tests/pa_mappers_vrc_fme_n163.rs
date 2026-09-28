//! VRC2/VRC4 (21, 22, 23, 25) over every documented wiring, the VRC IRQ, Sunsoft
//! FME-7 (69) with its cycle IRQ, and Namco 163 (19) banking, nametable RAM mapping,
//! sound RAM port, write protect and IRQ. Generated PRG: each byte is its bank.

use mos6502::ines::{Ines, InesError, MAGIC};
use mos6502::mapper::{Cartridge, Fme7, Namco163, PpuTarget, Vrc24, VrcWiring};
use mos6502::ppu::{Mirroring, Ppu, PpuMemory};
use mos6502::systems::nes2a03::Nes2A03;

fn prg(banks8: usize) -> Vec<u8> {
    (0..banks8 * 0x2000).map(|i| (i / 0x2000) as u8).collect()
}

const WIRINGS: [(u16, u8); 12] = [
    (21, 0),
    (21, 1),
    (21, 2),
    (22, 0),
    (23, 0),
    (23, 1),
    (23, 2),
    (23, 3),
    (25, 0),
    (25, 1),
    (25, 2),
    (25, 3),
];

fn low(mask: u16) -> u16 {
    mask & mask.wrapping_neg()
}

fn at(w: &VrcWiring, base: u16, reg: u8) -> u16 {
    base | if reg & 1 != 0 { low(w.a0) } else { 0 } | if reg & 2 != 0 { low(w.a1) } else { 0 }
}

#[test]
fn vrc_wiring_table() {
    for (m, s) in WIRINGS {
        let w = VrcWiring::for_mapper(m, s).unwrap();
        assert_eq!(
            w.vrc4,
            !matches!((m, s), (22, _) | (23, 3) | (25, 3)),
            "{m}.{s}"
        );
        assert_eq!(w.chr_shift, m == 22);
        assert_eq!(w.a0 & w.a1, 0);
    }
    for (m, s) in [(21, 3), (22, 1), (23, 4), (25, 4), (24, 0)] {
        assert_eq!(VrcWiring::for_mapper(m, s), None);
    }
    // Spot checks against the documented pins.
    let w = VrcWiring::for_mapper(21, 2).unwrap();
    assert_eq!((w.a0, w.a1), (0x40, 0x80));
    let w = VrcWiring::for_mapper(25, 2).unwrap();
    assert_eq!((w.a0, w.a1), (0x08, 0x04));
}

#[test]
fn vrc_chr_registers_every_value_every_wiring() {
    for (m, s) in WIRINGS {
        let w = VrcWiring::for_mapper(m, s).unwrap();
        let mut c = Vrc24::new(prg(16), w).unwrap();
        for v in 0..=255u8 {
            for slot in 0..8u8 {
                let base = 0xB000 + u16::from(slot / 2) * 0x1000;
                let r = (slot % 2) * 2;
                c.write(at(&w, base, r), v.wrapping_add(slot));
                c.write(at(&w, base, r + 1), v.wrapping_add(slot * 7) >> 3);
            }
            for slot in 0..8u8 {
                let lo = u16::from(v.wrapping_add(slot) & 15);
                let hi_mask = if w.vrc4 { 0x1F } else { 0x0F };
                let hi = u16::from((v.wrapping_add(slot * 7) >> 3) & hi_mask);
                let mut bank = usize::from(lo | (hi << 4));
                if w.chr_shift {
                    bank >>= 1;
                }
                let a = u16::from(slot) * 0x400 + 0x123;
                assert_eq!(c.chr_offset(a), bank * 0x400 + 0x123, "{m}.{s} {v} {slot}");
            }
        }
    }
}

#[test]
fn vrc_prg_swap_mirroring_and_ram() {
    for (m, s) in WIRINGS {
        let w = VrcWiring::for_mapper(m, s).unwrap();
        let mut c = Vrc24::new(prg(32), w).unwrap();
        for v in 0..32u8 {
            c.write(at(&w, 0x8000, 0), v);
            c.write(at(&w, 0xA000, 0), 31 - v);
            for swap in [false, true] {
                c.write(at(&w, 0x9000, 2), u8::from(swap) << 1);
                let swapped = swap && w.vrc4;
                let want = if swapped {
                    [30, 31 - v, v, 31]
                } else {
                    [v, 31 - v, 30, 31]
                };
                for (slot, &b) in want.iter().enumerate() {
                    assert_eq!(c.peek(0x8000 + slot as u16 * 0x2000), b, "{m}.{s}");
                }
            }
        }
        use Mirroring::*;
        for v in 0..4u8 {
            c.write(at(&w, 0x9000, 0), v);
            let want = if w.vrc4 {
                [Vertical, Horizontal, SingleLow, SingleHigh][usize::from(v)]
            } else {
                [Vertical, Horizontal][usize::from(v & 1)]
            };
            assert_eq!(c.mirroring(), Some(want));
        }
        if w.vrc4 {
            c.write(at(&w, 0x9000, 2), 0);
            c.write(0x6000, 0x12);
            assert_eq!(c.peek(0x6000), 0, "RAM disabled");
            c.write(at(&w, 0x9000, 2), 1);
            c.write(0x6000, 0x12);
            assert_eq!(c.peek(0x6000), 0x12);
        } else {
            c.write(0x6000, 0xFF);
            assert_eq!(c.peek(0x6000), 0x61, "VRC2 one-bit latch");
            c.write(0x6000, 0xFE);
            assert_eq!(c.peek(0x6FFF), 0x60);
        }
    }
}

fn vrc4() -> (Vrc24, VrcWiring) {
    let w = VrcWiring::for_mapper(21, 1).unwrap();
    (Vrc24::new(prg(8), w).unwrap(), w)
}

#[test]
fn vrc_irq_cycle_mode_every_latch() {
    for latch in 0..=255u8 {
        let (mut c, w) = vrc4();
        c.write(at(&w, 0xF000, 0), latch);
        c.write(at(&w, 0xF000, 1), latch >> 4);
        c.write(at(&w, 0xF000, 2), 0x06);
        let mut n = 0u32;
        while !c.irq() {
            c.clock();
            n += 1;
        }
        assert_eq!(n, 256 - u32::from(latch), "latch {latch}");
        assert_eq!(c.irq.counter, latch, "reloads on overflow");
    }
}

#[test]
fn vrc_irq_scanline_prescaler_and_ack() {
    for clocks in 1..=6u32 {
        let (mut c, w) = vrc4();
        let latch = (256 - clocks) as u8;
        c.write(at(&w, 0xF000, 0), latch);
        c.write(at(&w, 0xF000, 1), latch >> 4);
        c.write(at(&w, 0xF000, 2), 0x03);
        let mut n = 0u32;
        while !c.irq() {
            c.clock();
            n += 1;
        }
        assert_eq!(n, (341 * clocks).div_ceil(3), "{clocks} lines");
        // Ack copies A (1) into E: still counting.
        c.write(at(&w, 0xF000, 3), 0);
        assert!(!c.irq() && c.irq.enabled);
        // Control with E clear stops it.
        c.write(at(&w, 0xF000, 2), 0x01);
        let before = c.irq.counter;
        for _ in 0..10_000 {
            c.clock();
        }
        assert_eq!((c.irq.counter, c.irq()), (before, false));
        c.write(at(&w, 0xF000, 3), 0);
        assert!(c.irq.enabled, "A was set");
    }
}

#[test]
fn vrc2_has_no_irq() {
    let w = VrcWiring::for_mapper(23, 3).unwrap();
    let mut c = Vrc24::new(prg(8), w).unwrap();
    c.write(at(&w, 0xF000, 2), 0x06);
    for _ in 0..1000 {
        c.clock();
    }
    assert!(!c.irq());
}

#[test]
fn fme7_commands_every_value() {
    let mut c = Fme7::new(prg(64)).unwrap();
    for v in 0..=255u8 {
        for cmd in 0..8u8 {
            c.write(0x8000, cmd);
            c.write(0xA000, v ^ cmd);
            assert_eq!(
                c.chr_offset(u16::from(cmd) * 0x400 + 9),
                usize::from(v ^ cmd) * 0x400 + 9
            );
        }
        for (cmd, addr) in [(9u8, 0x8000u16), (10, 0xA000), (11, 0xC000)] {
            c.write(0x9FFF, cmd);
            c.write(0xBFFF, v);
            assert_eq!(c.peek(addr), v & 0x3F);
        }
        assert_eq!(c.peek(0xE000), 63);
        c.write(0x8000, 12);
        c.write(0xA000, v);
        use Mirroring::*;
        assert_eq!(
            c.mirroring(),
            Some([Vertical, Horizontal, SingleLow, SingleHigh][usize::from(v & 3)])
        );
    }
    // $6000: ROM bank, RAM disabled (open bus 0), RAM enabled.
    c.write(0x8000, 8);
    c.write(0xA000, 0x05);
    assert_eq!(c.peek(0x6000), 5);
    c.write(0x6000, 0x77);
    assert_eq!(c.peek(0x6000), 5, "ROM is not writable");
    c.write(0xA000, 0x40);
    assert_eq!(c.peek(0x6000), 0);
    c.write(0xA000, 0xC0);
    c.write(0x6000, 0x77);
    assert_eq!(c.peek(0x6000), 0x77);
}

#[test]
fn fme7_irq_counts_cpu_cycles_down() {
    for n in [0u16, 1, 2, 100, 0x1234, 0xFFFF] {
        let mut c = Fme7::new(prg(8)).unwrap();
        c.write(0x8000, 14);
        c.write(0xA000, n as u8);
        c.write(0x8000, 15);
        c.write(0xA000, (n >> 8) as u8);
        c.write(0x8000, 13);
        c.write(0xA000, 0x81);
        let mut k = 0u32;
        while !c.irq() {
            c.clock();
            k += 1;
        }
        assert_eq!(k, u32::from(n) + 1, "fires on the wrap to $FFFF");
        assert_eq!(c.counter, 0xFFFF);
        c.write(0xA000, 0x81);
        assert!(!c.irq(), "a write to D acknowledges");
    }
    let mut c = Fme7::new(prg(8)).unwrap();
    c.write(0x8000, 13);
    c.write(0xA000, 0x80);
    for _ in 0..70_000 {
        c.clock();
    }
    assert!(!c.irq(), "counter runs, IRQ disabled");
    c.write(0xA000, 0x01);
    let k = c.counter;
    c.clock();
    assert_eq!(c.counter, k, "counter disabled");
}

#[test]
fn n163_prg_chr_and_nametable_targets() {
    let mut c = Namco163::new(prg(64)).unwrap();
    for v in 0..=255u8 {
        c.write(0xE000, v);
        c.write(0xE800, v ^ 0x15);
        c.write(0xF000, v ^ 0x2A);
        assert_eq!(c.peek(0x8000), v & 0x3F);
        assert_eq!(c.peek(0xA000), (v ^ 0x15) & 0x3F);
        assert_eq!(c.peek(0xC000), (v ^ 0x2A) & 0x3F);
        assert_eq!(c.peek(0xE000), 63);
        for slot in 0..12u16 {
            c.write(0x8000 + slot * 0x800, v);
        }
        for half in 0..2u16 {
            let disabled = (v ^ 0x15) & (0x40 << half) != 0;
            let a = half * 0x1000 + 0x0456;
            let want = if v >= 0xE0 && !disabled {
                PpuTarget::Ciram(usize::from(v & 1) * 0x400 + 0x56)
            } else {
                PpuTarget::Chr(usize::from(v) * 0x400 + 0x56)
            };
            assert_eq!(c.ppu_target(a), want, "{v:02X} half {half}");
        }
        let want = if v >= 0xE0 {
            PpuTarget::Ciram(usize::from(v & 1) * 0x400 + 0x56)
        } else {
            PpuTarget::Chr(usize::from(v) * 0x400 + 0x56)
        };
        for nt in 0..4u16 {
            assert_eq!(c.ppu_target(0x2056 + nt * 0x400), want);
        }
    }
}

#[test]
fn n163_nametables_through_the_ppu() {
    let mut c = Namco163::new(prg(8)).unwrap();
    // $2000 -> CIRAM page 1, $2400 -> CIRAM page 0, $2800 -> CHR ROM 1 KiB bank 3.
    c.write(0xC000, 0xE1);
    c.write(0xC800, 0xE0);
    c.write(0xD000, 0x03);
    let mut chr: Vec<u8> = (0..0x2000).map(|i| (i >> 10) as u8 | 0x80).collect();
    let mut ppu = Ppu::new();
    let mut m = PpuMemory {
        cart: Some(&mut c),
        chr: &mut chr,
        chr_writable: false,
        mirroring: Mirroring::Horizontal,
    };
    ppu.mem_write(&mut m, 0x2005, 0x11);
    ppu.mem_write(&mut m, 0x2405, 0x22);
    assert_eq!(ppu.vram[0x405], 0x11);
    assert_eq!(ppu.vram[0x005], 0x22);
    assert_eq!(ppu.mem_read(&m, 0x2005), 0x11);
    assert_eq!(ppu.mem_read(&m, 0x2805), 0x83, "nametable from CHR ROM");
    ppu.mem_write(&mut m, 0x2805, 0);
    assert_eq!(ppu.mem_read(&m, 0x2805), 0x83, "CHR ROM is read-only");
}

#[test]
fn n163_sound_port_protect_and_irq() {
    let mut c = Namco163::new(prg(8)).unwrap();
    c.write(0xF800, 0x80 | 0x7E);
    for v in [1u8, 2, 3, 4] {
        c.write(0x4800, v);
    }
    assert_eq!(&c.sound_ram[0x7E..], &[1, 2]);
    assert_eq!(&c.sound_ram[..2], &[3, 4], "auto-increment wraps in 7 bits");
    c.write(0xF800, 0x7E);
    assert_eq!((c.read(0x4800), c.read(0x4800)), (1, 1), "no increment");
    for port in 0..=255u8 {
        let mut c = Namco163::new(prg(8)).unwrap();
        c.write(0xF800, port);
        for sec in 0..4u16 {
            c.write(0x6000 + sec * 0x800, 0xAB);
            let ok = port & 0xF0 == 0x40 && port & (1 << sec) == 0;
            assert_eq!(c.peek(0x6000 + sec * 0x800) == 0xAB, ok, "{port:02X} {sec}");
        }
    }
    for start in [0u16, 1, 0x7000, 0x7FFE] {
        let mut c = Namco163::new(prg(8)).unwrap();
        c.write(0x5000, start as u8);
        c.write(0x5800, 0x80 | (start >> 8) as u8);
        assert_eq!(
            u16::from(c.peek(0x5000)) | (u16::from(c.peek(0x5800) & 0x7F) << 8),
            start
        );
        let mut k = 0u32;
        while !c.irq() {
            c.clock();
            k += 1;
        }
        assert_eq!(k, u32::from(0x7FFF - start));
        c.clock();
        assert_eq!(c.counter, 0x7FFF, "stops");
        c.write(0x5000, 0);
        assert!(!c.irq(), "write acknowledges");
    }
}

fn image(mapper: u8, sub: u8) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([
        8,
        4,
        (mapper & 15) << 4,
        (mapper & 0xF0) | 8,
        sub << 4,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]);
    let mut p = prg(16);
    let n = p.len();
    p[n - 4..].copy_from_slice(&[0x00, 0x80, 0x00, 0x80]);
    f.extend(p);
    f.extend(vec![0; 4 * 0x2000]);
    f
}

#[test]
fn ines_builds_the_new_boards_and_states_round_trip() {
    let mut cases: Vec<(u8, u8)> = WIRINGS.iter().map(|&(m, s)| (m as u8, s)).collect();
    cases.extend([(69, 0), (19, 0)]);
    for (m, s) in cases {
        let data = image(m, s);
        let mut a = Nes2A03::from_ines(&data).unwrap();
        {
            let c = a.cpu.bus.cart.as_mut().unwrap();
            for (addr, v) in [
                (0x8000u16, 3u8),
                (0xA000, 5),
                (0xB003, 0x1F),
                (0xF002, 6),
                (0xF001, 2),
                (0x5800, 0x81),
            ] {
                c.write(addr, v);
            }
        }
        a.run_cycles(20_000).unwrap();
        let s1 = a.save_state();
        let mut b = Nes2A03::from_ines(&data).unwrap();
        b.load_state(&s1).unwrap();
        a.run_cycles(30_000).unwrap();
        b.run_cycles(30_000).unwrap();
        assert_eq!(a.save_state(), b.save_state(), "{m}.{s}");
    }
    assert_eq!(
        Ines::parse(&image(22, 1)).unwrap().cartridge().err(),
        Some(InesError::UnsupportedSubmapper(22, 1))
    );
}
