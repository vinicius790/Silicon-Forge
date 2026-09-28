//! 2C02 model: registers, memory map, timing, rendering and sprite flags, with
//! original test patterns written through the CPU-visible registers.

use mos6502::mapper::{Cartridge, Mmc3};
use mos6502::ppu::{Mirroring, Ppu, PpuMemory, DOTS, HEIGHT, LINES, WIDTH};

struct Rig {
    ppu: Ppu,
    chr: Vec<u8>,
    mirroring: Mirroring,
}

impl Rig {
    fn new(m: Mirroring) -> Self {
        Self {
            ppu: Ppu::new(),
            chr: vec![0; 0x2000],
            mirroring: m,
        }
    }
    fn mem(&mut self) -> (&mut Ppu, PpuMemory<'_>) {
        (
            &mut self.ppu,
            PpuMemory {
                cart: None,
                chr: &mut self.chr,
                chr_writable: true,
                mirroring: self.mirroring,
            },
        )
    }
    fn w(&mut self, reg: u16, v: u8) {
        let (p, mut m) = self.mem();
        p.write_register(&mut m, reg, v);
    }
    fn r(&mut self, reg: u16) -> u8 {
        let (p, mut m) = self.mem();
        p.read_register(&mut m, reg)
    }
    fn addr(&mut self, a: u16) {
        self.w(0x2006, (a >> 8) as u8);
        self.w(0x2006, a as u8);
    }
    fn poke(&mut self, a: u16, data: &[u8]) {
        self.addr(a);
        for &b in data {
            self.w(0x2007, b);
        }
    }
    fn tick(&mut self, n: u64) {
        for _ in 0..n {
            let (p, mut m) = self.mem();
            p.tick(&mut m);
        }
    }
    fn to(&mut self, line: u16, dot: u16) {
        while !(self.ppu.line == line && self.ppu.dot == dot) {
            self.tick(1);
        }
    }
    fn frame(&mut self) {
        let f = self.ppu.frame;
        while self.ppu.frame == f {
            self.tick(1);
        }
    }
    fn px(&self, x: usize, y: usize) -> u8 {
        self.ppu.framebuffer[y * WIDTH + x]
    }
}

#[test]
fn mirroring_folds_every_nametable_address() {
    for m in [
        Mirroring::Horizontal,
        Mirroring::Vertical,
        Mirroring::FourScreen,
        Mirroring::SingleLow,
        Mirroring::SingleHigh,
    ] {
        for addr in 0x2000u16..0x3F00 {
            let f = m.fold(addr);
            let t = usize::from((addr & 0xFFF) >> 10);
            let page = match m {
                Mirroring::Horizontal => t / 2,
                Mirroring::Vertical => t % 2,
                Mirroring::FourScreen => t,
                Mirroring::SingleLow => 0,
                Mirroring::SingleHigh => 1,
            };
            assert_eq!(
                f,
                page * 0x400 + usize::from(addr & 0x3FF),
                "{m:?} {addr:04X}"
            );
        }
        let mut r = Rig::new(m);
        r.poke(0x2000, &[0x11]);
        r.poke(0x2C00, &[0x44]);
        r.addr(0x2400);
        r.r(0x2007);
        let at_2400 = r.r(0x2007);
        let expect = match m {
            Mirroring::Horizontal => 0x11,
            Mirroring::Vertical | Mirroring::SingleLow | Mirroring::SingleHigh => 0x44,
            Mirroring::FourScreen => 0,
        };
        assert_eq!(at_2400, expect, "{m:?}");
    }
}

#[test]
fn ppudata_buffer_increment_and_palette_mirrors() {
    let mut r = Rig::new(Mirroring::Vertical);
    let data: Vec<u8> = (0..=255).collect();
    r.poke(0x2100, &data);
    r.addr(0x2100);
    assert_eq!(r.r(0x2007), 0, "first read returns the stale buffer");
    for i in 0..255u8 {
        assert_eq!(r.r(0x2007), i);
    }
    // Increment 32.
    r.w(0x2000, 4);
    r.addr(0x2100);
    r.r(0x2007);
    for row in 0..7u8 {
        assert_eq!(r.r(0x2007), row * 32, "buffer lags one read");
    }
    r.w(0x2000, 0);
    // Palette: every write masked to 6 bits, 3F10/14/18/1C alias 3F00/04/08/0C.
    for i in 0..32u16 {
        r.poke(0x3F00 + i, &[0xC0 | i as u8]);
    }
    for i in 0..32u16 {
        let expect = if i & 0x13 == 0x10 {
            (i & 0x0F) + 0x10
        } else {
            i
        } as u8;
        r.addr(0x3F00 + i);
        let v = r.r(0x2007) & 0x3F;
        let alias = if i & 0x13 == 0x10 { i & 0x0F } else { i };
        r.addr(0x3F00 + alias);
        assert_eq!(v, r.r(0x2007) & 0x3F, "{i:02X}");
        let _ = expect;
    }
    // Palette read is immediate and loads the nametable byte underneath ($2F00).
    r.poke(0x2F00, &[0x5A]);
    r.addr(0x3F00);
    r.r(0x2007);
    r.addr(0x2000);
    assert_eq!(r.ppu.buffer, 0x5A);
    // Mirrors of the whole PPU space and of the registers.
    r.poke(0x3F01, &[0x21]);
    r.addr(0x7F01);
    assert_eq!(r.r(0x3FFF) & 0x3F, 0x21);
    // Grayscale masks palette reads.
    r.w(0x2001, 1);
    r.addr(0x3F01);
    assert_eq!(r.r(0x2007) & 0x3F, 0x20);
}

#[test]
fn scroll_and_address_share_the_toggle() {
    let mut r = Rig::new(Mirroring::Horizontal);
    r.w(0x2000, 3);
    assert_eq!(r.ppu.t, 0x0C00);
    r.w(0x2005, 0b1011_1101);
    assert_eq!(
        (r.ppu.t & 0x1F, r.ppu.fine_x, r.ppu.w),
        (0b10111, 0b101, true)
    );
    r.w(0x2005, 0b0101_1110);
    assert_eq!(r.ppu.t, 0x0C00 | 0b10111 | (0b01011 << 5) | (0b110 << 12));
    assert!(!r.ppu.w);
    r.w(0x2006, 0xFF);
    assert_eq!(r.ppu.t >> 8, 0x3F, "bit 14 cleared");
    r.r(0x2002);
    assert!(!r.ppu.w, "status read resets the toggle");
    r.w(0x2006, 0x12);
    r.w(0x2006, 0x34);
    assert_eq!((r.ppu.v, r.ppu.t), (0x1234, 0x1234));
    // Every value of the first $2005 write.
    for v in 0..=255u8 {
        r.r(0x2002);
        r.w(0x2005, v);
        assert_eq!((r.ppu.t & 0x1F, r.ppu.fine_x), (u16::from(v >> 3), v & 7));
    }
}

#[test]
fn open_bus_oam_and_status_bits() {
    let mut r = Rig::new(Mirroring::Horizontal);
    r.w(0x2001, 0x1F);
    for reg in [0x2000, 0x2001, 0x2003, 0x2005, 0x2006] {
        assert_eq!(r.r(reg), 0x1F, "write-only reads the latch");
    }
    r.ppu.status = 0xE0;
    assert_eq!(r.r(0x2002), 0xE0 | 0x1F);
    assert_eq!(r.ppu.status & 0x80, 0);
    // OAMDATA writes while rendering are glitch increments (tests/pa_sprite_eval.rs).
    r.ppu.mask = 0;
    r.w(0x2003, 0xFF);
    r.w(0x2004, 0xAA);
    r.w(0x2004, 0xBB);
    assert_eq!(r.ppu.oam_addr, 0x01);
    r.w(0x2003, 0xFF);
    assert_eq!(
        (r.r(0x2004), r.r(0x2004)),
        (0xAA, 0xAA),
        "read does not increment"
    );
    for v in 0..=255u8 {
        r.w(0x2003, 2);
        r.w(0x2004, v);
        r.w(0x2003, 2);
        assert_eq!(r.r(0x2004), v & 0xE3, "attribute bits 4-2");
    }
    let page: [u8; 256] = std::array::from_fn(|i| i as u8);
    r.w(0x2003, 0x10);
    r.ppu.oam_dma(&page);
    assert_eq!(
        (r.ppu.oam[0x10], r.ppu.oam[0x0F], r.ppu.oam_addr),
        (0, 0xFF, 0x10)
    );
}

#[test]
fn frame_timing_vblank_and_nmi() {
    let mut r = Rig::new(Mirroring::Horizontal);
    r.w(0x2000, 0x80);
    r.to(241, 1);
    assert_eq!(r.ppu.status & 0x80, 0);
    r.tick(1);
    assert_ne!(r.ppu.status & 0x80, 0);
    assert!(r.ppu.take_nmi());
    assert!(!r.ppu.take_nmi(), "one edge");
    // Re-enabling NMI inside vblank fires again; disabling then enabling too.
    r.w(0x2000, 0);
    r.w(0x2000, 0x80);
    assert!(r.ppu.take_nmi());
    r.to(261, 1);
    r.tick(1);
    assert_eq!(r.ppu.status & 0xE0, 0, "pre-render clears flags");
    // Frame lengths: 89342 dots, odd frames one shorter with rendering on.
    r.frame();
    let mut lens = Vec::new();
    for render in [false, false, true, true, true, true] {
        r.w(0x2001, if render { 0x08 } else { 0 });
        let mut n = 0u64;
        let f = r.ppu.frame;
        while r.ppu.frame == f {
            r.tick(1);
            n += 1;
        }
        lens.push((r.ppu.frame & 1, n));
    }
    let full = u64::from(DOTS) * u64::from(LINES);
    for (parity_after, n) in &lens[..2] {
        assert_eq!(*n, full, "{parity_after}");
    }
    for (parity_after, n) in &lens[2..] {
        // The frame just finished is odd when the new one is even.
        let odd = *parity_after == 0;
        assert_eq!(*n, if odd { full - 1 } else { full });
    }
}

#[test]
fn status_read_just_before_vblank_suppresses_it() {
    let mut r = Rig::new(Mirroring::Horizontal);
    r.w(0x2000, 0x80);
    r.to(241, 0);
    assert_eq!(r.r(0x2002) & 0x80, 0);
    r.tick(5);
    assert_eq!(r.ppu.status & 0x80, 0);
    assert!(!r.ppu.take_nmi());
    r.frame();
    r.to(241, 2);
    assert!(r.ppu.take_nmi(), "next frame is normal");
}

/// Tile 1: solid colour 1; tile 2: solid colour 3; tile 3: left column colour 2.
fn patterns(r: &mut Rig) {
    r.poke(0x0010, &[0xFF; 8]);
    r.poke(0x0018, &[0x00; 8]);
    r.poke(0x0020, &[0xFF; 16]);
    r.poke(0x0030, &[0; 8]);
    r.poke(0x0038, &[0x80; 8]);
    // Sprites in the $1000 table: same shapes.
    r.poke(0x1010, &[0xFF; 8]);
    r.poke(0x1018, &[0; 8]);
    r.poke(0x1020, &[0xFF; 16]);
    r.poke(0x1030, &[0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01]);
    r.poke(0x1038, &[0; 8]);
    let pal: Vec<u8> = (0..32).map(|i| 0x20 + i).collect();
    r.poke(0x3F00, &pal);
}

#[test]
fn background_tiles_attributes_and_scroll() {
    let mut r = Rig::new(Mirroring::Vertical);
    patterns(&mut r);
    // Checkerboard of tiles 1/2 in nametable 0, attribute quadrant palettes 0..3.
    let nt: Vec<u8> = (0..960)
        .map(|i| if (i % 32 + i / 32) % 2 == 0 { 1 } else { 2 })
        .collect();
    r.poke(0x2000, &nt);
    r.poke(0x23C0, &[0b11_10_01_00; 64]);
    r.poke(0x2400, &[2; 960]);
    r.w(0x2001, 0x0A);
    r.addr(0);
    r.w(0x2005, 0);
    r.w(0x2005, 0);
    r.w(0x2000, 0);
    r.frame();
    r.frame();
    let pal = |p: u8, c: u8| 0x20 + p * 4 + c;
    for ty in 0..30usize {
        for tx in 0..32usize {
            let c = if (tx + ty) % 2 == 0 { 1 } else { 3 };
            let q = ((ty % 4) / 2) * 2 + (tx % 4) / 2;
            let expect = pal(q as u8, c) & 0x3F;
            assert_eq!(r.px(tx * 8 + 3, ty * 8 + 4), expect, "tile {tx},{ty}");
        }
    }
    // Fine/coarse x scroll by 13 shows tile column 1 starting at x = 3, and the
    // right edge reads into nametable 1 (all tile 2, palette 0).
    r.r(0x2002);
    r.w(0x2005, 13);
    r.w(0x2005, 0);
    r.frame();
    r.frame();
    assert_eq!(r.px(0, 0), pal(0, 3), "tile 1 of row 0 is tile 2");
    assert_eq!(r.px(2, 8), pal(0, 1), "row 1, tile column 1");
    assert_eq!(
        r.px(3, 8),
        pal(1, 3),
        "row 1, tile column 2, right attribute quadrant"
    );
    assert_eq!(r.px(255, 100), pal(0, 3), "nametable 1");
    // Left clip hides the background in x < 8 (backdrop shows).
    r.w(0x2001, 0x08);
    r.frame();
    r.frame();
    assert_eq!(
        r.px(7, 50),
        r.ppu.palette[0],
        "backdrop ($3F10 aliases $3F00)"
    );
    assert_ne!(r.px(8, 50), r.ppu.palette[0]);
}

fn sprite(r: &mut Rig, n: usize, y: u8, tile: u8, attr: u8, x: u8) {
    r.ppu.oam[n * 4..n * 4 + 4].copy_from_slice(&[y, tile, attr, x]);
}

#[test]
fn sprites_priority_flip_and_left_clip() {
    let mut r = Rig::new(Mirroring::Horizontal);
    patterns(&mut r);
    r.poke(0x2000, &[1; 960]);
    r.poke(0x23C0, &[0; 64]);
    for n in 0..64 {
        sprite(&mut r, n, 0xF0, 0, 0, 0);
    }
    sprite(&mut r, 0, 19, 2, 0x01, 40); // in front, palette 5
    sprite(&mut r, 1, 19, 2, 0x22, 44); // behind bg, overlaps
    sprite(&mut r, 2, 59, 3, 0x40, 100); // diagonal, h-flipped
    sprite(&mut r, 3, 59, 3, 0x80, 120); // v-flipped
    sprite(&mut r, 4, 79, 2, 0x00, 2); // left edge
    r.w(0x2000, 0x08);
    r.w(0x2001, 0x1E);
    r.frame();
    r.frame();
    let sp = |p: u8, c: u8| 0x20 + 0x10 + p * 4 + c;
    let bg1 = 0x21;
    assert_eq!(r.px(40, 20), sp(1, 3));
    assert_eq!(
        r.px(43, 27),
        sp(1, 3),
        "lower-numbered sprite wins the overlap"
    );
    assert_eq!(
        r.px(48, 20),
        bg1,
        "behind-background sprite hidden by opaque bg"
    );
    assert_eq!(r.px(40, 19), bg1, "OAM y = 19 starts on line 20");
    for row in 0..8usize {
        assert_eq!(r.px(100 + 7 - row, 60 + row), sp(0, 1), "h-flip row {row}");
        assert_eq!(r.px(120 + 7 - row, 60 + row), sp(0, 1), "v-flip row {row}");
    }
    assert_eq!(r.px(2, 80), sp(0, 3));
    r.w(0x2001, 0x18);
    r.frame();
    r.frame();
    assert_eq!(r.px(2, 80), r.ppu.palette[0], "both clipped -> backdrop");
    assert_eq!(r.px(8, 80), sp(0, 3));
}

#[test]
fn sprite_zero_hit_timing_and_rules() {
    for (x, mask, hit) in [
        (50u8, 0x1E, true),
        (255, 0x1E, false),
        (4, 0x18, true),
        (4, 0x1E, true),
        (50, 0x10, false),
    ] {
        let mut r = Rig::new(Mirroring::Horizontal);
        patterns(&mut r);
        r.poke(0x2000, &[1; 960]);
        for n in 0..64 {
            sprite(&mut r, n, 0xF0, 0, 0, 0);
        }
        sprite(&mut r, 0, 99, 1, 0, x);
        r.w(0x2000, 0x08);
        r.w(0x2001, mask);
        r.frame();
        r.to(100, 0);
        let mut at = None;
        while r.ppu.line == 100 {
            r.tick(1);
            if at.is_none() && r.ppu.status & 0x40 != 0 {
                at = Some(r.ppu.dot);
            }
        }
        assert_eq!(at.is_some(), hit, "x {x} mask {mask:02X}");
        if hit {
            let first = if mask & 6 == 6 { x } else { x.max(8) };
            assert_eq!(
                at,
                Some(u16::from(first) + 2),
                "set on dot x+1, seen after it"
            );
        }
        r.to(261, 2);
        assert_eq!(r.ppu.status & 0x40, 0);
    }
}

#[test]
fn sprite_overflow_counts_per_line() {
    for count in 0..=12usize {
        for h16 in [false, true] {
            let mut r = Rig::new(Mirroring::Horizontal);
            for n in 0..64 {
                if n < count {
                    sprite(&mut r, n, 50, 0, 0, (n * 8) as u8);
                } else {
                    sprite(&mut r, n, 0xF8, 0xF8, 0xF8, 0xF8);
                }
            }
            r.w(0x2000, if h16 { 0x20 } else { 0 });
            r.w(0x2001, 0x18);
            r.frame();
            r.to(240, 0);
            assert_eq!(r.ppu.status & 0x20 != 0, count > 8, "{count} {h16}");
        }
    }
    // Rendering off: no overflow.
    let mut r = Rig::new(Mirroring::Horizontal);
    for n in 0..64 {
        sprite(&mut r, n, 50, 0, 0, 0);
    }
    r.frame();
    r.to(240, 0);
    assert_eq!(r.ppu.status & 0x20, 0);
}

#[test]
fn eight_by_sixteen_sprites_pick_table_from_bit_0() {
    let mut r = Rig::new(Mirroring::Horizontal);
    r.poke(0x1020, &[0xFF; 8]); // tile $02 top, plane 0, in $1000 because tile $03 has bit 0
    r.poke(0x1030, &[0x00; 8]);
    r.poke(0x1038, &[0xFF; 8]); // bottom half plane 1
    r.poke(0x3F11, &[0x05, 0x06, 0x07]);
    for n in 0..64 {
        sprite(&mut r, n, 0xF0, 0, 0, 0);
    }
    sprite(&mut r, 0, 9, 0x03, 0, 16);
    r.w(0x2000, 0x20);
    r.w(0x2001, 0x14);
    r.frame();
    r.frame();
    assert_eq!(r.px(16, 10), 0x05, "top: colour 1");
    assert_eq!(r.px(16, 18), 0x06, "bottom: colour 2");
    assert_eq!(r.px(16, 26), r.ppu.palette[0]);
    assert_eq!(HEIGHT, 240);
}

#[test]
fn mmc3_counts_one_edge_per_rendered_line() {
    let mut cart = Mmc3::new(vec![0; 0x8000], false).unwrap();
    let mut ppu = Ppu::new();
    let mut chr = vec![0u8; 0x2000];
    cart.write(0xC000, 9);
    cart.write(0xC001, 0);
    cart.write(0xE001, 0);
    let run = |ppu: &mut Ppu, cart: &mut Mmc3, chr: &mut Vec<u8>, ctrl: u8, mask: u8| {
        let mut m = PpuMemory {
            cart: Some(cart),
            chr,
            chr_writable: true,
            mirroring: Mirroring::Vertical,
        };
        ppu.write_register(&mut m, 0x2000, ctrl);
        ppu.write_register(&mut m, 0x2001, mask);
        let mut irq_line = None;
        let f = ppu.frame;
        let mut dots = 0u32;
        while ppu.frame == f {
            ppu.tick(&mut m);
            dots += 1;
            if dots % 3 == 0 {
                m.cart.as_mut().unwrap().clock();
            }
            if irq_line.is_none() && m.cart.as_ref().unwrap().irq() {
                irq_line = Some(ppu.line);
            }
        }
        irq_line
    };
    let base = cart.edges;
    // Sprites at $1000, background at $0000: 241 edges (240 visible + pre-render).
    assert_eq!(run(&mut ppu, &mut cart, &mut chr, 0x08, 0x18), Some(9));
    assert_eq!(cart.edges - base, 241);
    cart.write(0xE000, 0);
    assert!(!cart.irq());
    let base = cart.edges;
    assert_eq!(
        run(&mut ppu, &mut cart, &mut chr, 0x00, 0x18),
        None,
        "same table: no edges"
    );
    assert_eq!(cart.edges, base);
    assert_eq!(
        run(&mut ppu, &mut cart, &mut chr, 0x08, 0x00),
        None,
        "rendering off"
    );
    assert_eq!(cart.edges, base);
}

fn overflow_after(oam: impl Fn(&mut Rig)) -> bool {
    let mut r = Rig::new(Mirroring::Horizontal);
    r.ppu.oam = [0xF8; 256];
    oam(&mut r);
    r.w(0x2001, 0x18);
    r.frame();
    r.to(240, 0);
    r.ppu.status & 0x20 != 0
}

#[test]
fn overflow_scan_reproduces_the_hardware_bug() {
    let eight = |r: &mut Rig| {
        for n in 0..8 {
            sprite(r, n, 50, 0xF8, 0xF8, 0xF8);
        }
    };
    assert!(!overflow_after(eight), "eight never overflow");
    assert!(overflow_after(|r: &mut Rig| {
        eight(r);
        sprite(r, 8, 50, 0xF8, 0xF8, 0xF8);
    }));
    // False negative: sprite 8 is off the line, so the scan reads byte 1 of sprite 9.
    assert!(!overflow_after(|r: &mut Rig| {
        eight(r);
        sprite(r, 9, 50, 0xF8, 0xF8, 0xF8);
    }));
    // False positive: sprite 10's attribute byte (m = 2 by then) reads as a y on the line.
    assert!(overflow_after(|r: &mut Rig| {
        eight(r);
        sprite(r, 10, 0xF8, 0xF8, 52, 0xF8);
    }));
    for k in 0..4usize {
        assert!(
            overflow_after(|r: &mut Rig| {
                eight(r);
                r.ppu.oam[(8 + k) * 4 + k] = 53;
            }),
            "sprite {} is checked at byte {k}",
            8 + k
        );
        assert!(
            !overflow_after(|r: &mut Rig| {
                eight(r);
                r.ppu.oam[(8 + k) * 4 + (k + 1) % 4] = 53;
            }),
            "sprite {} byte {} is skipped",
            8 + k,
            (k + 1) % 4
        );
    }
}

#[test]
fn mid_line_mask_and_palette_changes_split_the_line() {
    let mut r = Rig::new(Mirroring::Horizontal);
    r.poke(0x3F00, &[0x0F]);
    r.addr(0x2000);
    r.frame();
    r.to(100, 1);
    r.tick(100);
    r.addr(0x3F00);
    r.w(0x2007, 0x21);
    r.addr(0x2000);
    r.w(0x2001, 0xE0);
    r.to(101, 0);
    assert_eq!(r.px(99, 100), 0x0F);
    assert_eq!(r.px(100, 100), 0x21);
    assert_eq!(r.ppu.emphasis[100 * WIDTH + 99], 0);
    assert_eq!(r.ppu.emphasis[100 * WIDTH + 100], 7);
    // Rendering off with v in palette space shows that entry.
    r.poke(0x3F05, &[0x16]);
    r.addr(0x3F05);
    r.to(102, 0);
    assert_eq!(r.px(10, 101), 0x16);
}

#[test]
fn ntsc_palette_properties_over_all_indices_and_emphasis() {
    use mos6502::ppu::ntsc_rgb;
    let luma = |c: [u8; 3]| 299 * u32::from(c[0]) + 587 * u32::from(c[1]) + 114 * u32::from(c[2]);
    for e in 0..8u8 {
        for i in 0..64u8 {
            let c = ntsc_rgb(i, e);
            if i & 0x0F >= 0x0E {
                assert_eq!(c, [0, 0, 0], "{i:02X} {e}");
            }
            if e == 0 && (i & 0x0F == 0 || i & 0x0F == 0x0D) {
                assert!(
                    c[0].abs_diff(c[1]) <= 1 && c[1].abs_diff(c[2]) <= 1,
                    "gray {i:02X} {e} {c:?}"
                );
            }
        }
        // Gray columns brighten down the rows.
        for col in [0x00u8, 0x0D] {
            for row in 0..3u8 {
                assert!(
                    luma(ntsc_rgb(col + row * 16, e)) <= luma(ntsc_rgb(col + (row + 1) * 16, e))
                );
            }
        }
    }
    assert_eq!(
        ntsc_rgb(0x0D, 0),
        [0, 0, 0],
        "$0D is the blacker-than-black level, clamped"
    );
    assert_eq!(ntsc_rgb(0x30, 0), [255, 255, 255]);
    // Twelve hues in a row are pairwise distinct and saturated.
    let row: Vec<[u8; 3]> = (1..=12).map(|h| ntsc_rgb(0x10 + h, 0)).collect();
    for (a, x) in row.iter().enumerate() {
        for y in &row[a + 1..] {
            assert_ne!(x, y);
        }
        assert!(
            x.iter().max().unwrap() - x.iter().min().unwrap() > 40,
            "{x:?}"
        );
    }
    // Hue order: 1 blue, 6 red, $A green, $C cyan (row 1).
    let dominant = |c: [u8; 3]| (0..3).max_by_key(|&k| c[k]).unwrap();
    assert_eq!(dominant(ntsc_rgb(0x11, 0)), 2);
    assert_eq!(dominant(ntsc_rgb(0x16, 0)), 0);
    assert_eq!(dominant(ntsc_rgb(0x1A, 0)), 1);
    let cyan = ntsc_rgb(0x1C, 0);
    assert!(cyan[1] > cyan[0] && cyan[2] > cyan[0], "{cyan:?}");
    // Emphasis only darkens (hue $D sits at black, where clamping hides it).
    for i in (0..64u8).filter(|i| i & 0x0F != 0x0D) {
        for e in 1..8u8 {
            assert!(
                luma(ntsc_rgb(i, e)) <= luma(ntsc_rgb(i, 0)) + 1000,
                "{i:02X} {e}"
            );
        }
    }
    for (bit, ch) in [(1u8, 0usize), (2, 1), (4, 2)] {
        let c = ntsc_rgb(0x20, bit);
        let others = (0..3).filter(|&k| k != ch).map(|k| c[k]).max().unwrap();
        assert!(
            c[ch] >= others,
            "emphasis {bit} tints toward channel {ch}: {c:?}"
        );
    }
    let mut r = Rig::new(Mirroring::Horizontal);
    r.ppu.framebuffer[5] = 0x30;
    let rgb = r.ppu.rgb_frame();
    assert_eq!(rgb.len(), WIDTH * HEIGHT * 3);
    assert_eq!(&rgb[15..18], &[255, 255, 255]);
    r.tick(0);
}
