//! 2C02 background and sprite pipeline, dot by dot: fetch dots, shift registers,
//! coarse x/y increments, horizontal/vertical copies, sprite evaluation and
//! hblank sprite fetches. Original patterns only.

use mos6502::ppu::{Mirroring, Ppu, PpuMemory, WIDTH};

struct Rig {
    ppu: Ppu,
    chr: Vec<u8>,
}

impl Rig {
    fn new() -> Self {
        Self {
            ppu: Ppu::new(),
            chr: vec![0; 0x2000],
        }
    }
    fn mem(&mut self) -> (&mut Ppu, PpuMemory<'_>) {
        (
            &mut self.ppu,
            PpuMemory {
                cart: None,
                chr: &mut self.chr,
                chr_writable: true,
                mirroring: Mirroring::Vertical,
            },
        )
    }
    fn w(&mut self, reg: u16, v: u8) {
        let (p, mut m) = self.mem();
        p.write_register(&mut m, reg, v);
    }
    fn tick(&mut self) {
        let (p, mut m) = self.mem();
        p.tick(&mut m);
    }
    /// Run until the PPU is about to execute (line, dot).
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
    fn px(&self, x: usize, y: usize) -> u8 {
        self.ppu.framebuffer[y * WIDTH + x]
    }
}

/// Tile 1 solid colour 1, tile 2 solid colour 2, tile 3 solid colour 3; palette
/// entry i holds `$20 + i`. Nametable 0 all tile 1, attributes 0.
fn setup() -> Rig {
    let mut r = Rig::new();
    r.chr[0x10..0x18].fill(0xFF);
    r.chr[0x28..0x30].fill(0xFF);
    r.chr[0x30..0x40].fill(0xFF);
    for i in 0..32 {
        r.ppu.palette[i] = 0x20 + i as u8;
    }
    r.ppu.palette[0x10] = 0x20;
    for b in r.ppu.vram.iter_mut() {
        *b = 1;
    }
    for i in 0..64 {
        r.ppu.vram[0x3C0 + i] = 0;
        r.ppu.vram[0x7C0 + i] = 0;
    }
    r.ppu.oam = [0xF8; 256];
    r.w(0x2001, 0x1E);
    r
}

#[test]
fn nametable_byte_is_read_on_its_documented_dot() {
    // Column c >= 2 of line 20: NT fetched on dot 8(c-2)+2, shown from dot 8c+1.
    for c in 2..32usize {
        for (change_at, takes) in [
            (8 * (c as u16 - 2) + 2, true),
            (8 * (c as u16 - 2) + 3, false),
        ] {
            let mut r = setup();
            r.frame();
            r.to(20, change_at);
            // Row 2 (lines 16-23) of nametable 0.
            r.ppu.vram[2 * 32 + c] = 3;
            r.to(22, 0);
            let want = if takes { 0x23 } else { 0x21 };
            assert_eq!(
                r.px(c * 8 + 4, 20),
                want,
                "column {c}, change before dot {change_at}"
            );
            assert_eq!(
                r.px(c * 8 + 4, 21),
                0x23,
                "column {c}: next line always sees it"
            );
        }
    }
}

#[test]
fn pattern_planes_are_read_on_dots_6_and_8_of_the_group() {
    // Column 5 of line 40 (fine y 0): low plane read on dot 8*3+6, high on 8*3+8.
    let base = 8 * 3;
    for (dot, lo_in, hi_in) in [
        (base + 6, true, true),
        (base + 7, false, true),
        (base + 9, false, false),
    ] {
        let mut r = setup();
        r.frame();
        r.to(40, dot);
        // Tile 1, row 0: becomes colour 2 (low clear, high set) if both changes land.
        r.chr[0x10] = 0x00;
        r.chr[0x18] = 0xFF;
        r.to(41, 0);
        let want = 0x20 + u8::from(!lo_in) + if hi_in { 2 } else { 0 };
        assert_eq!(r.px(5 * 8 + 3, 40), want, "change before dot {dot}");
    }
}

#[test]
fn coarse_x_increments_on_every_eighth_dot_and_copies_at_257() {
    let mut r = setup();
    r.frame();
    r.to(30, 1);
    let start = r.ppu.v & 0x1F;
    assert_eq!(start, 2, "two tiles were prefetched on dots 321-336");
    for d in 1..=256u16 {
        r.tick();
        let inc = d / 8;
        let want = (start + inc) & 0x1F;
        let nt = ((start + inc) >> 5) & 1;
        assert_eq!(r.ppu.v & 0x1F, want, "dot {d}");
        assert_eq!(
            (r.ppu.v >> 10) & 1,
            nt,
            "dot {d}: nametable bit flips on wrap"
        );
    }
    r.ppu.t = (r.ppu.t & !0x041F) | 0x0407;
    r.tick(); // dot 257
    assert_eq!(r.ppu.v & 0x041F, 0x0407, "horizontal t copied on dot 257");
    r.to(30, 329);
    assert_eq!(r.ppu.v & 0x1F, 8);
    r.to(30, 337);
    assert_eq!(r.ppu.v & 0x1F, 9);
}

#[test]
fn fine_y_increments_on_dot_256_only() {
    let mut r = setup();
    r.frame();
    r.to(10, 255);
    let fy = (r.ppu.v >> 12) & 7;
    r.tick();
    assert_eq!((r.ppu.v >> 12) & 7, fy, "dot 255 leaves y");
    r.tick();
    assert_eq!((r.ppu.v >> 12) & 7, (fy + 1) & 7, "dot 256 increments y");
}

#[test]
fn vertical_copy_spans_pre_render_dots_280_to_304() {
    for (write_at, copied) in [(279u16, true), (300, true), (304, true), (305, false)] {
        let mut r = setup();
        r.frame();
        r.to(261, write_at);
        r.ppu.t = (r.ppu.t & !0x7BE0) | 0x5A60;
        r.tick();
        r.to(261, 306);
        assert_eq!(
            r.ppu.v & 0x7BE0 == 0x5A60,
            copied,
            "t written before dot {write_at}"
        );
    }
}

#[test]
fn shift_registers_hold_two_tiles_and_reload_on_dot_9() {
    let mut r = setup();
    r.ppu.vram[0] = 2;
    r.ppu.vram[1] = 3;
    r.ppu.vram[2] = 1;
    r.frame();
    r.to(0, 1);
    // Tile 0 (colour 2) in the high byte, tile 1 (colour 3) in the low byte.
    assert_eq!(r.ppu.bg_lo, 0x00FF);
    assert_eq!(r.ppu.bg_hi, 0xFFFF);
    r.to(0, 10);
    assert_eq!(
        r.ppu.bg_lo, 0xFFFF,
        "tile 1 shifted up, tile 2 loaded on dot 9"
    );
    assert_eq!(r.ppu.bg_hi, 0xFF00);
}

#[test]
fn fine_x_change_mid_line_acts_on_the_next_dot() {
    let mut r = setup();
    r.ppu.vram[2 * 32 + 9] = 2; // row 2 column 9 colour 2, column 10 colour 1
    r.frame();
    r.to(20, 77); // pixel 76 drawn on dot 77
    r.w(0x2005, 4); // fine x = 4 (coarse x in t only)
    r.to(21, 0);
    assert_eq!(r.px(75, 20), 0x22, "before the write: column 9");
    assert_eq!(r.px(76, 20), 0x21, "x 76 + 4 = 80: column 10 at once");
    assert_eq!(r.ppu.fine_x, 4);
}

#[test]
fn attribute_latch_changes_palette_at_tile_granularity() {
    let mut r = setup();
    r.frame();
    r.to(50, 8 * 4 + 4); // column 6 attribute read on dot 8*4+4
    r.ppu.vram[0x3C0 + 9] = 0xFF; // tile rows 4-7, columns 4-7
    r.to(51, 0);
    assert_eq!(r.px(5 * 8, 50), 0x21, "column 5 already fetched");
    assert_eq!(r.px(6 * 8, 50), 0x2D, "column 6 sees palette 3");
    assert_eq!(r.px(7 * 8, 50), 0x2D);
}

#[test]
fn overflow_rises_on_the_dot_its_check_finishes() {
    for extra in 0..4usize {
        let mut r = setup();
        for n in 0..8 {
            r.ppu.oam[n * 4] = 50;
        }
        r.ppu.oam[(8 + extra) * 4 + extra] = 50; // the byte the buggy scan reads
        r.frame();
        r.to(50, 0);
        let mut at = None;
        while r.ppu.line == 50 {
            r.tick();
            if at.is_none() && r.ppu.status & 0x20 != 0 {
                at = Some(r.ppu.dot - 1);
            }
        }
        assert_eq!(at, Some(65 + 8 * 8 + 2 * extra as u16 + 1), "extra {extra}");
    }
}

#[test]
fn secondary_oam_holds_the_in_range_sprites_after_evaluation() {
    let mut r = setup();
    for (slot, n) in [3usize, 7, 40].iter().enumerate() {
        r.ppu.oam[n * 4..n * 4 + 4].copy_from_slice(&[60, slot as u8, 0, 10]);
    }
    r.frame();
    r.to(62, 257); // evaluation runs dot by dot over 65-256
    assert_eq!(r.ppu.sec_count, 3);
    assert_eq!(
        &r.ppu.secondary[0..12],
        &[60, 0, 0, 10, 60, 1, 0, 10, 60, 2, 0, 10]
    );
    // The 2C02 copies every Y it examines into the next free slot, so slot 3
    // byte 0 holds the last sprite's Y; the rest stays $FF.
    assert_eq!(r.ppu.secondary[12], r.ppu.oam[63 * 4]);
    assert!(r.ppu.secondary[13..].iter().all(|&b| b == 0xFF));
    r.to(70, 257);
    assert_eq!(r.ppu.sec_count, 0, "sprites end after 8 lines");
}

#[test]
fn oam_written_after_dot_65_misses_the_next_line() {
    let mut r = setup();
    r.frame();
    r.to(90, 66);
    r.ppu.oam[0..4].copy_from_slice(&[90, 3, 0, 100]);
    r.to(92, 0);
    assert_eq!(r.px(100, 91), 0x21, "line 91 evaluated before the write");
    r.to(93, 0);
    assert_eq!(r.px(100, 92), 0x33, "line 92 has it");
}

#[test]
fn sprite_patterns_are_fetched_per_slot_in_hblank() {
    for (slot_dot, lands) in [(262u16, true), (263, false)] {
        let mut r = setup();
        r.chr[0x1000..0x2000].fill(0);
        r.w(0x2000, 0x08);
        r.ppu.oam[0..4].copy_from_slice(&[100, 4, 0, 30]);
        r.frame();
        r.to(100, slot_dot);
        r.chr[0x1040] = 0xFF; // tile 4 row 0 low plane
        r.to(102, 0);
        let want = if lands { 0x31 } else { 0x21 };
        assert_eq!(r.px(30, 101), want, "change before dot {slot_dot}");
    }
}

#[test]
fn sprite_zero_hit_follows_fine_scroll_of_the_background() {
    // Background opaque only at column 12; sprite 0 spans x 92..100.
    for fine in 0..8u8 {
        let mut r = setup();
        for b in r.ppu.vram.iter_mut().take(0x3C0) {
            *b = 0;
        }
        for row in 0..30 {
            r.ppu.vram[row * 32 + 12] = 1;
        }
        r.ppu.oam[0..4].copy_from_slice(&[119, 3, 0, 92]);
        r.w(0x2005, fine);
        r.w(0x2005, 0);
        r.frame();
        r.to(120, 0);
        let mut at = None;
        while r.ppu.line == 120 {
            r.tick();
            if at.is_none() && r.ppu.status & 0x40 != 0 {
                at = Some(r.ppu.dot - 1);
            }
        }
        // Column 12 starts at screen x 96 - fine.
        let first = 96 - u16::from(fine);
        assert_eq!(at, Some(first.max(92) + 1), "fine x {fine}");
    }
}

#[test]
fn rendering_off_freezes_the_pipeline() {
    let mut r = setup();
    r.frame();
    r.to(10, 100);
    r.w(0x2001, 0);
    let (v, lo, hi) = (r.ppu.v, r.ppu.bg_lo, r.ppu.bg_hi);
    r.to(11, 100);
    assert_eq!((r.ppu.v, r.ppu.bg_lo, r.ppu.bg_hi), (v, lo, hi));
}
