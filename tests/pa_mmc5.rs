//! MMC5 core: PRG modes with ROM/RAM selection and write protect, CHR modes with
//! both register sets and 8×16 sprite/background switching, ExRAM modes, nametable
//! mapping and fill mode, the scanline detector and IRQ driven by real PPU reads,
//! extended attributes, the multiplier, save states and battery RAM.

use mos6502::ines::MAGIC;
use mos6502::mapper::{Cartridge, Mmc5, PpuTarget};
use mos6502::ppu::{Mirroring, Ppu, PpuMemory};
use mos6502::systems::nes2a03::Nes2A03;

const BANKS: usize = 32; // 256 KiB PRG

fn prg() -> Vec<u8> {
    (0..BANKS * 0x2000).map(|i| (i / 0x2000) as u8).collect()
}

fn mmc5() -> Mmc5 {
    Mmc5::new(prg()).unwrap()
}

/// Reference: (is_ram, 8 KiB bank) of CPU address `addr` from the documented
/// mode table.
fn reference(mode: u8, regs: [u8; 5], addr: u16) -> (bool, usize) {
    if addr < 0x8000 {
        return (true, usize::from(regs[0] & 7));
    }
    let q = usize::from((addr - 0x8000) / 0x2000);
    let (r, size, sub) = match mode {
        0 => (4, 4, q),
        1 => (if q < 2 { 2 } else { 4 }, 2, q % 2),
        2 => match q {
            0 | 1 => (2, 2, q),
            2 => (3, 1, 0),
            _ => (4, 1, 0),
        },
        _ => (q + 1, 1, 0),
    };
    let v = regs[r];
    let bank = (usize::from(v & 0x7F) / size) * size + sub;
    if r != 4 && v & 0x80 == 0 {
        (true, bank % 8)
    } else {
        (false, bank % BANKS)
    }
}

#[test]
fn prg_modes_banks_and_rom_ram_bits() {
    let mut c = mmc5();
    assert_eq!(
        c.peek(0xFFFF),
        (BANKS - 1) as u8,
        "power-on: mode 3, $5117 = $FF"
    );
    c.write(0x5102, 2);
    c.write(0x5103, 1);
    for b in 0..8u8 {
        c.write(0x5113, b);
        c.write(0x6000, 0xA0 | b);
    }
    let seeds = [0x00u8, 0x01, 0x05, 0x7F, 0x80, 0x81, 0x93, 0xFF, 0x3C, 0xC6];
    for mode in 0..4u8 {
        c.write(0x5100, mode | 0xFC);
        for (k, &a) in seeds.iter().enumerate() {
            let regs = [
                a & 7,
                seeds[(k + 1) % 10],
                seeds[(k + 2) % 10],
                seeds[(k + 3) % 10],
                seeds[(k + 4) % 10],
            ];
            for (i, &v) in regs.iter().enumerate() {
                c.write(0x5113 + i as u16, v);
            }
            for addr in [
                0x6000u16, 0x7FFF, 0x8000, 0x9FFF, 0xA000, 0xBFFF, 0xC000, 0xDFFF, 0xE000, 0xFFFF,
            ] {
                let (ram, bank) = reference(mode, regs, addr);
                let want = if ram { 0xA0 | bank as u8 } else { bank as u8 };
                let got = c.peek(addr);
                if ram && addr >= 0x8000 {
                    // Only the first byte of each RAM bank carries the marker.
                    let (r, off) = c.prg_target(addr);
                    assert!(r);
                    assert_eq!(off / 0x2000, bank, "mode {mode} {addr:04X}");
                    continue;
                }
                if ram && addr != 0x6000 {
                    continue;
                }
                assert_eq!(got, want, "mode {mode} regs {regs:02X?} {addr:04X}");
            }
        }
    }
}

#[test]
fn prg_ram_write_protect_needs_2_then_1() {
    for a in 0..4u8 {
        for b in 0..4u8 {
            let mut c = mmc5();
            c.write(0x5102, a | 0xFC);
            c.write(0x5103, b | 0xFC);
            c.write(0x6000, 0x77);
            c.write(0x5100, 3);
            c.write(0x5114, 0x01); // RAM bank 1 at $8000
            c.write(0x8000, 0x66);
            let ok = (a, b) == (2, 1);
            assert_eq!(c.peek(0x6000) == 0x77, ok, "{a} {b}");
            assert_eq!(c.peek(0x8000) == 0x66, ok);
            c.write(0x5114, 0x81);
            c.write(0x8000, 0x55);
            assert_eq!(c.peek(0x8000), 1, "ROM is never written");
        }
    }
}

#[test]
fn chr_modes_sets_and_upper_bits() {
    let mut c = mmc5();
    c.write(0x5130, 2);
    for i in 0..8u16 {
        c.write(0x5120 + i, 0x10 + i as u8);
    }
    c.write(0x5130, 1);
    for i in 0..4u16 {
        c.write(0x5128 + i, 0x40 + i as u8);
    }
    assert_eq!(c.chr_a[3], 0x213, "upper bits latched at the write");
    assert_eq!(c.chr_b[2], 0x142);
    let a_reg = |i: usize| 0x210 + i;
    let b_reg = |i: usize| 0x140 + i;
    for mode in 0..4u8 {
        c.write(0x5101, mode);
        for addr in (0..0x2000u16).step_by(0x1F3) {
            let a = usize::from(addr);
            let (size, ia) = match mode {
                0 => (0x2000, 7),
                1 => (0x1000, (a >> 12) * 4 + 3),
                2 => (0x800, (a >> 11) * 2 + 1),
                _ => (0x400, a >> 10),
            };
            // Last write was set B: 8×8 sprites use it for everything.
            c.last_set_b = true;
            assert_eq!(
                c.chr_page(addr),
                b_reg(ia & 3) * size + (a & (size - 1)),
                "B m{mode} {addr:04X}"
            );
            c.last_set_b = false;
            assert_eq!(
                c.chr_page(addr),
                a_reg(ia) * size + (a & (size - 1)),
                "A m{mode} {addr:04X}"
            );
            assert_eq!(c.ppu_target(addr), PpuTarget::Chr(c.chr_page(addr)));
        }
    }
    c.write(0x5120, 0);
    assert!(!c.last_set_b);
    c.write(0x5128, 0);
    assert!(c.last_set_b);
}

#[test]
fn multiplier_every_pair() {
    let mut c = mmc5();
    assert_eq!(
        (c.peek(0x5205), c.peek(0x5206)),
        (0x01, 0xFE),
        "power-on $FF × $FF"
    );
    for a in 0..=255u8 {
        c.write(0x5205, a);
        for b in 0..=255u8 {
            c.write(0x5206, b);
            let p = u16::from(a) * u16::from(b);
            assert_eq!(
                u16::from(c.read(0x5205)) | (u16::from(c.read(0x5206)) << 8),
                p
            );
        }
    }
}

#[test]
fn exram_cpu_modes() {
    for mode in 0..4u8 {
        for in_frame in [false, true] {
            let mut c = mmc5();
            c.exram = [0x99; 1024];
            c.write(0x5104, mode | 0xFC);
            c.in_frame = in_frame;
            c.write(0x5C10, 0x42);
            let stored = c.exram[0x10];
            let want = match mode {
                2 => 0x42,
                3 => 0x99,
                _ if in_frame => 0x42,
                _ => 0,
            };
            assert_eq!(stored, want, "mode {mode} in frame {in_frame}");
            let read = c.peek(0x5C10);
            assert_eq!(
                read,
                if mode >= 2 { stored } else { 0 },
                "CPU reads only in modes 2/3"
            );
        }
    }
}

#[test]
fn nametable_map_exram_and_fill_through_the_ppu() {
    let mut c = mmc5();
    // $2000 CIRAM 0, $2400 CIRAM 1, $2800 ExRAM, $2C00 fill.
    c.write(0x5105, 0b11_10_01_00);
    c.write(0x5106, 0x5A);
    c.write(0x5107, 0xFE);
    let mut chr = vec![0u8; 0x2000];
    let mut ppu = Ppu::new();
    {
        let mut m = PpuMemory {
            cart: Some(&mut c),
            chr: &mut chr,
            chr_writable: true,
            mirroring: Mirroring::Horizontal,
        };
        ppu.mem_write(&mut m, 0x2003, 0x11);
        ppu.mem_write(&mut m, 0x2403, 0x22);
        ppu.mem_write(&mut m, 0x2803, 0x33);
        ppu.mem_write(&mut m, 0x2C03, 0x44);
        assert_eq!(ppu.vram[0x003], 0x11);
        assert_eq!(ppu.vram[0x403], 0x22);
        assert_eq!(ppu.mem_read(&m, 0x2803), 0x33);
        assert_eq!(ppu.mem_read(&m, 0x2C03), 0x5A, "fill tile");
        assert_eq!(
            ppu.mem_read(&m, 0x2FC7),
            0xAA,
            "fill attribute 2 in every quadrant"
        );
    }
    assert_eq!(c.exram[3], 0x33);
    c.write(0x5104, 2);
    assert_eq!(
        c.ppu_target(0x2803),
        PpuTarget::Mapper(0),
        "ExRAM as CPU RAM reads 0 as nametable"
    );
    let before = c.exram[3];
    c.ppu_write(0x2803, 0x77);
    assert_eq!(c.exram[3], before);
}

/// Drive a PPU frame by frame with the MMC5 attached, clocking the mapper once per
/// three dots; `f` sees each dot.
fn run(c: &mut Mmc5, ppu: &mut Ppu, chr: &mut [u8], dots: u32, mut f: impl FnMut(&Ppu, &Mmc5)) {
    for d in 0..dots {
        {
            let mut m = PpuMemory {
                cart: Some(&mut *c),
                chr: &mut *chr,
                chr_writable: true,
                mirroring: Mirroring::Vertical,
            };
            ppu.tick(&mut m);
        }
        if d % 3 == 2 {
            c.clock();
        }
        f(ppu, c);
    }
}

const FRAME: u32 = 341 * 262;

fn rendering_setup(ctrl: u8) -> (Mmc5, Ppu, Vec<u8>) {
    let mut c = mmc5();
    let mut ppu = Ppu::new();
    ppu.mask = 0x18;
    ppu.ctrl = ctrl;
    ppu.oam = [0xFF; 256];
    c.snoop_ppu_register(0x2000, ctrl);
    c.snoop_ppu_register(0x2001, 0x18);
    (c, ppu, vec![0; 0x2000])
}

#[test]
fn scanline_irq_fires_on_the_compare_line() {
    for compare in [1u8, 2, 3, 50, 100, 200, 238, 239] {
        let (mut c, mut ppu, mut chr) = rendering_setup(0);
        c.write(0x5203, compare);
        c.write(0x5204, 0x80);
        run(&mut c, &mut ppu, &mut chr, FRAME, |_, _| {});
        c.read(0x5204); // drop whatever the warm-up frame left pending
        while ppu.line != 261 {
            run(&mut c, &mut ppu, &mut chr, 3, |_, _| {});
        }
        c.read(0x5204);
        let mut fired = vec![];
        let mut was = false;
        run(&mut c, &mut ppu, &mut chr, FRAME, |p, c| {
            if c.irq() && !was {
                fired.push((p.line, p.dot));
            }
            was = c.irq();
        });
        assert_eq!(fired.len(), 1, "once per frame: {fired:?}");
        let (line, dot) = fired[0];
        assert_eq!(line, u16::from(compare), "compare {compare} {fired:?}");
        assert!(
            dot <= 4,
            "detected at the third identical nametable read: dot {dot}"
        );
        assert!(c.irq(), "pending stays set until $5204 is read");
        assert_eq!(c.read(0x5204) & 0x80, 0x80);
        assert_eq!(c.read(0x5204) & 0x80, 0, "read clears pending");
    }
    // Compare 0 never fires; disabled IRQ keeps the flag but not the line.
    let (mut c, mut ppu, mut chr) = rendering_setup(0);
    c.write(0x5204, 0x80);
    run(&mut c, &mut ppu, &mut chr, 2 * FRAME, |_, c| {
        assert!(!c.irq())
    });
    let (mut c, mut ppu, mut chr) = rendering_setup(0);
    c.write(0x5203, 10);
    run(&mut c, &mut ppu, &mut chr, 2 * FRAME, |_, c| {
        assert!(!c.irq())
    });
    assert!(c.irq_pending);
}

#[test]
fn in_frame_flag_follows_rendering() {
    let (mut c, mut ppu, mut chr) = rendering_setup(0);
    run(&mut c, &mut ppu, &mut chr, FRAME, |_, _| {});
    let mut seen = vec![];
    run(&mut c, &mut ppu, &mut chr, FRAME, |p, c| {
        if p.dot == 100 {
            seen.push((p.line, c.in_frame, c.scanline));
        }
    });
    for (line, in_frame, sl) in seen {
        if line < 240 {
            assert!(in_frame, "line {line}");
            assert_eq!(sl, line as u8, "counter = line");
        } else if line < 261 {
            assert!(
                !in_frame,
                "vblank line {line}: no PPU reads for 3 CPU cycles"
            );
        }
    }
    assert_eq!(c.peek(0x5204) & 0x40, 0x40 * u8::from(c.in_frame));
    c.snoop_ppu_register(0x2001, 0);
    assert!(!c.in_frame, "rendering disabled leaves the frame");
}

#[test]
fn sprite_fetches_use_set_a_with_8x16_sprites() {
    let (mut c, mut ppu, mut chr) = rendering_setup(0x20);
    c.write(0x5101, 3);
    c.write(0x5120, 0x01);
    c.write(0x5128, 0x02);
    run(&mut c, &mut ppu, &mut chr, FRAME, |_, _| {});
    let mut checked = 0;
    run(&mut c, &mut ppu, &mut chr, FRAME, |p, c| {
        if (1..239).contains(&p.line) {
            let sprite = (259..=320).contains(&p.dot);
            let bg = (10..=256).contains(&p.dot);
            if sprite || bg {
                assert_eq!(c.sprite_phase(), sprite, "line {} dot {}", p.line, p.dot);
                let want = if sprite { 0x400 } else { 0x800 };
                assert_eq!(c.chr_page(0x0010), want + 0x10);
                checked += 1;
            }
        }
    });
    assert!(checked > 70_000);
}

#[test]
fn extended_attributes_per_tile() {
    let mut c = mmc5();
    c.write(0x5104, 1);
    c.write(0x5130, 1);
    c.in_frame = true;
    c.rendering = true;
    for (i, v) in [(5usize, 0xC7u8), (6, 0x01), (900, 0x7E)] {
        c.exram[i] = v;
        c.ppu_read(0x2000 + i as u16);
        assert_eq!(c.ppu_target(0x23C1), PpuTarget::Mapper((v >> 6) * 0x55));
        assert_eq!(
            c.chr_page(0x0123),
            (usize::from(v & 0x3F) | 0x40) * 0x1000 + 0x123
        );
    }
    c.in_frame = false;
    assert_eq!(
        c.ppu_target(0x23C1),
        PpuTarget::Ciram(0x3C1),
        "outside rendering: plain"
    );
}

fn image(battery: bool) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([
        16,
        4,
        0x50 | (u8::from(battery) << 1),
        8,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]);
    let mut p = prg();
    let n = p.len();
    p[n - 0x2000..n - 0x1FFD].copy_from_slice(&[0x4C, 0x00, 0xE0]);
    p[n - 4..].copy_from_slice(&[0x00, 0xE0, 0x00, 0xE0]);
    f.extend(p);
    f.extend(vec![0; 4 * 0x2000]);
    f
}

#[test]
fn machine_states_and_battery_ram() {
    let data = image(true);
    let mut a = Nes2A03::from_ines(&data).unwrap();
    {
        let c = a.cpu.bus.cart.as_mut().unwrap();
        for (addr, v) in [
            (0x5102u16, 2u8),
            (0x5103, 1),
            (0x6123, 0xAB),
            (0x5104, 2),
            (0x5C00, 9),
            (0x5203, 30),
            (0x5204, 0x80),
            (0x5205, 7),
            (0x5206, 9),
            (0x5101, 1),
            (0x5127, 3),
        ] {
            c.write(addr, v);
        }
    }
    a.cpu.bus.ppu.mask = 0x18;
    a.cpu
        .bus
        .cart
        .as_mut()
        .unwrap()
        .snoop_ppu_register(0x2001, 0x18);
    a.run_cycles(40_000).unwrap();
    let s = a.save_state();
    let mut b = Nes2A03::from_ines(&data).unwrap();
    b.load_state(&s).unwrap();
    a.run_cycles(50_000).unwrap();
    b.run_cycles(50_000).unwrap();
    assert_eq!(a.save_state(), b.save_state());
    let sav = a.save_ram().unwrap();
    assert_eq!(sav.len(), 0x10000);
    assert_eq!(sav[0x123], 0xAB);
    let mut fresh = Nes2A03::from_ines(&data).unwrap();
    fresh.load_save_ram(&sav).unwrap();
    assert_eq!(fresh.cpu.bus.cart.as_ref().unwrap().peek(0x6123), 0xAB);
    assert!(Nes2A03::from_ines(&image(false))
        .unwrap()
        .save_ram()
        .is_none());
}
