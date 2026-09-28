//! PPU pattern-bus traffic as cartridges see it: every real fetch reaches
//! `Cartridge::ppu_address`, MMC3 counts true A12 rises, MMC2/MMC4 latch on the
//! documented fetch addresses. Original patterns only.

use mos6502::mapper::{Cartridge, Mmc24, Mmc3};
use mos6502::ppu::{Mirroring, Ppu, PpuMemory, WIDTH};
use std::cell::RefCell;
use std::rc::Rc;

/// Cartridge that records `(line, dot, addr)` of every PPU bus address.
struct Probe {
    log: Rc<RefCell<Vec<u16>>>,
}

impl Cartridge for Probe {
    fn peek(&self, _: u16) -> u8 {
        0
    }
    fn write(&mut self, _: u16, _: u8) {}
    fn ppu_address(&mut self, addr: u16) {
        self.log.borrow_mut().push(addr);
    }
}

struct Rig<C: Cartridge> {
    ppu: Ppu,
    chr: Vec<u8>,
    cart: C,
    dots: u64,
}

impl<C: Cartridge> Rig<C> {
    fn new(cart: C, chr: Vec<u8>) -> Self {
        Self {
            ppu: Ppu::new(),
            chr,
            cart,
            dots: 0,
        }
    }
    fn tick(&mut self) {
        let mut m = PpuMemory {
            cart: Some(&mut self.cart),
            chr: &mut self.chr,
            chr_writable: false,
            mirroring: Mirroring::Vertical,
        };
        self.ppu.tick(&mut m);
        self.dots += 1;
        if self.dots % 3 == 0 {
            self.cart.clock();
        }
    }
    fn to(&mut self, line: u16, dot: u16) {
        while !(self.ppu.line == line && self.ppu.dot == dot) {
            self.tick();
        }
    }
    fn frame(&mut self) {
        let f = self.ppu.frame;
        while self.ppu.frame == f {
            self.tick();
        }
    }
}

fn probe_rig() -> (Rig<Probe>, Rc<RefCell<Vec<u16>>>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let r = Rig::new(Probe { log: log.clone() }, vec![0; 0x2000]);
    (r, log)
}

#[test]
fn one_rendered_line_puts_every_documented_fetch_on_the_bus() {
    for ctrl in [0x00u8, 0x08, 0x10, 0x18, 0x20, 0x30] {
        let (mut r, log) = probe_rig();
        r.ppu.ctrl = ctrl;
        r.ppu.mask = 0x18;
        r.to(20, 0);
        log.borrow_mut().clear();
        r.to(21, 0);
        let l = log.borrow();
        // Idle-dot CHR address, 34 tiles x (NT, AT, low, high), 8 sprite slots x 4,
        // 2 trailing NT reads.
        assert_eq!(l.len(), 1 + 34 * 4 + 8 * 4 + 2, "ctrl {ctrl:02X}");
        let patterns: Vec<u16> = l.iter().copied().filter(|&a| a < 0x2000).collect();
        assert_eq!(patterns.len(), 1 + 34 * 2 + 16);
        let patterns = &patterns[1..];
        let bg = u16::from(ctrl & 0x10) << 8;
        for (i, &a) in patterns[..64].iter().enumerate() {
            assert_eq!(a & 0x1000, bg, "bg fetch {i}");
            assert_eq!(a & 8, if i % 2 == 0 { 0 } else { 8 }, "plane order");
        }
        // Empty sprite slots fetch tile $FF: table from PPUCTRL bit 3, or bit 0 of
        // the tile ($FF: $1000) with 8x16 sprites.
        let sp = if ctrl & 0x20 != 0 {
            0x1000
        } else {
            u16::from(ctrl & 8) << 9
        };
        for &a in &patterns[64..80] {
            assert_eq!(a & 0x1000, sp);
            assert_eq!(a & 0x0FF0, 0x0FF0, "tile $FF");
        }
        assert!(l.iter().filter(|&&a| a >= 0x2000).all(|&a| a < 0x3000));
    }
}

#[test]
fn rendering_off_puts_nothing_on_the_bus() {
    let (mut r, log) = probe_rig();
    r.frame();
    assert!(log.borrow().is_empty());
}

#[test]
fn mmc3_counts_true_a12_rises_per_frame() {
    // Rises per frame: one per rendering line when exactly one of the two tables is
    // $1000 (the pre-render line adds the rise out of vblank when the background is
    // at $1000); none with both at $0000; one (out of vblank) with both at $1000,
    // since the short lows between fetches are filtered.
    for (ctrl, want) in [
        (0x00u8, 0u64),
        (0x08, 241),
        (0x10, 242),
        (0x18, 1),
        (0x20, 241),
        (0x28, 241),
        (0x38, 1),
    ] {
        let mut r = Rig::new(Mmc3::new(vec![0; 0x8000], false).unwrap(), vec![0; 0x2000]);
        // Power-up OAM of zeros puts eight 8x16 tile-0 sprites ($0000) on lines 0-15.
        r.ppu.oam = [0xFF; 256];
        r.ppu.ctrl = ctrl;
        r.ppu.mask = 0x18;
        r.to(241, 0);
        for _ in 0..3 {
            let before = r.cart.edges;
            r.frame();
            r.to(241, 0);
            assert_eq!(r.cart.edges - before, want, "ctrl {ctrl:02X}");
        }
    }
}

#[test]
fn eight_by_sixteen_sprites_on_table_0_hide_the_rise() {
    // Zeroed OAM: tile 0 sprites ($0000) fill all eight slots on lines 0-15, so
    // those lines put no $1000 address on the bus and the counter is not clocked.
    let mut r = Rig::new(Mmc3::new(vec![0; 0x8000], false).unwrap(), vec![0; 0x2000]);
    r.ppu.ctrl = 0x20;
    r.ppu.mask = 0x18;
    r.to(241, 0);
    let before = r.cart.edges;
    r.frame();
    r.to(241, 0);
    assert_eq!(r.cart.edges - before, 241 - 16);
}

#[test]
fn mmc3_irq_fires_on_the_programmed_line() {
    for latch in [0u8, 1, 5, 30, 100, 200, 238] {
        let mut m = Mmc3::new(vec![0; 0x8000], false).unwrap();
        m.write(0xC000, latch);
        m.write(0xC001, 0);
        m.write(0xE001, 0);
        let mut r = Rig::new(m, vec![0; 0x2000]);
        r.ppu.ctrl = 0x08;
        r.ppu.mask = 0x18;
        r.to(261, 0);
        r.cart.write(0xE000, 0);
        r.cart.write(0xC001, 0);
        r.cart.write(0xE001, 0);
        let mut first = None;
        while first.is_none() {
            r.tick();
            if r.cart.irq() {
                first = Some((r.ppu.line, r.ppu.dot));
            }
        }
        // Reload on the pre-render rise, then `latch` decrements: line latch - 1.
        let (line, dot) = first.unwrap();
        let want = if latch == 0 {
            261
        } else {
            u16::from(latch) - 1
        };
        assert_eq!(line, want, "latch {latch}");
        assert!(
            (262..=270).contains(&dot),
            "rise on the sprite low fetch, {dot}"
        );
    }
}

#[test]
fn ppudata_toggles_reach_the_mapper_but_are_filtered() {
    let mut r = Rig::new(Mmc3::new(vec![0; 0x8000], false).unwrap(), vec![0; 0x2000]);
    r.to(241, 10);
    let edges = r.cart.edges;
    // Two $2006 writes in a row put $1000 then $0000 on the bus within one CPU cycle.
    for _ in 0..4 {
        let mut m = PpuMemory {
            cart: Some(&mut r.cart),
            chr: &mut r.chr,
            chr_writable: false,
            mirroring: Mirroring::Vertical,
        };
        r.ppu.write_register(&mut m, 0x2006, 0x10);
        r.ppu.write_register(&mut m, 0x2006, 0x00);
        r.ppu.write_register(&mut m, 0x2006, 0x00);
        r.ppu.write_register(&mut m, 0x2006, 0x00);
    }
    assert_eq!(
        r.cart.edges,
        edges + 1,
        "only the first rise passes the filter"
    );
}

fn mmc2_rig(mmc4: bool) -> Rig<Mmc24> {
    let prg = vec![0; 0x20000];
    let mut m = if mmc4 {
        Mmc24::mmc4(prg).unwrap()
    } else {
        Mmc24::mmc2(prg).unwrap()
    };
    // $0000 FD -> bank 0, FE -> bank 1.
    m.write(0xB000, 0);
    m.write(0xC000, 1);
    let mut chr = vec![0u8; 0x2000];
    // Tile 1: low plane solid in bank 0 (colour 1), high plane solid in bank 1 (colour 2).
    chr[0x0010..0x0018].fill(0xFF);
    chr[0x1018..0x1020].fill(0xFF);
    let mut r = Rig::new(m, chr);
    for col in 0..32 {
        r.ppu.vram[col] = [0xFD, 1, 0xFE, 1][col % 4];
        r.ppu.vram[32 + col] = r.ppu.vram[col];
    }
    r.ppu.palette[0] = 0x0F;
    r.ppu.palette[1] = 0x21;
    r.ppu.palette[2] = 0x22;
    r.ppu.mask = 0x0A;
    r
}

#[test]
fn mmc2_and_mmc4_switch_chr_after_the_fd_fe_fetch() {
    for mmc4 in [false, true] {
        let mut r = mmc2_rig(mmc4);
        r.frame();
        r.frame();
        let px = |x: usize, y: usize| r.ppu.framebuffer[y * WIDTH + x];
        for k in 0..8 {
            assert_eq!(
                px(32 * k + 8, 0),
                0x21,
                "after $FD: bank 0, col {}",
                4 * k + 1
            );
            assert_eq!(px(32 * k + 24, 0), 0x22, "after $FE: bank 1");
            assert_eq!(px(32 * k, 0), 0x0F, "$FD itself blank");
        }
        // Line 1 (fine y 1): the $FD high plane is $0FD9. MMC2 latch 0 only reacts
        // to $0FD8 exactly, MMC4 to $0FD8-$0FDF.
        let want = if mmc4 { 0x21 } else { 0x22 };
        assert_eq!(px(8, 1), want, "mmc4 {mmc4}");
    }
}

#[test]
fn mmc2_mmc4_latch_trigger_addresses_exhaustive() {
    for mmc4 in [false, true] {
        for addr in 0u16..0x4000 {
            let prg = vec![0; 0x20000];
            let mut m = if mmc4 {
                Mmc24::mmc4(prg).unwrap()
            } else {
                Mmc24::mmc2(prg).unwrap()
            };
            m.latch = [false; 2];
            m.ppu_address(addr);
            let fe = m.latch;
            m.latch = [true; 2];
            m.ppu_address(addr);
            let fd = m.latch;
            let a = addr & 0x1FFF;
            let ranged = |base: u16| (base..base + 8).contains(&a);
            let exact0 = if mmc4 { ranged(0x0FD8) } else { a == 0x0FD8 };
            let exact0e = if mmc4 { ranged(0x0FE8) } else { a == 0x0FE8 };
            let in_pt = addr < 0x2000;
            assert_eq!(fe[0], in_pt && exact0e, "{addr:04X}");
            assert_eq!(!fd[0], in_pt && exact0, "{addr:04X}");
            assert_eq!(fe[1], in_pt && ranged(0x1FE8), "{addr:04X}");
            assert_eq!(!fd[1], in_pt && ranged(0x1FD8), "{addr:04X}");
        }
    }
}
