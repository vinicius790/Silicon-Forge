//! 2C02 sprite evaluation dot by dot: live OAM reads, the `$2004` bus during
//! rendering, glitchy OAMDATA writes, OAMADDR reset and start address, the overflow
//! bug both ways, and OAM row corruption when rendering stops mid-line.

use mos6502::ppu::{Mirroring, Ppu, PpuMemory, EVAL_DONE};

struct Rig {
    ppu: Ppu,
    chr: Vec<u8>,
}

impl Rig {
    fn new() -> Self {
        let mut ppu = Ppu::new();
        ppu.oam = [0xF8; 256];
        ppu.mask = 0x18;
        Self {
            ppu,
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
    fn r(&mut self, reg: u16) -> u8 {
        let (p, mut m) = self.mem();
        p.read_register(&mut m, reg)
    }
    fn tick(&mut self) {
        let (p, mut m) = self.mem();
        p.tick(&mut m);
    }
    fn to(&mut self, line: u16, dot: u16) {
        while !(self.ppu.line == line && self.ppu.dot == dot) {
            self.tick();
        }
    }
}

#[test]
fn oam_is_read_live_each_sprite_on_its_own_dot() {
    // No sprite in range: sprite n's Y is read on dot 65 + 2n.
    for n in 0..64u16 {
        for d in [65 + 2 * n - 1, 65 + 2 * n, 65 + 2 * n + 1] {
            let mut r = Rig::new();
            r.to(90, d);
            r.ppu.oam[usize::from(n) * 4] = 90;
            r.to(90, 257);
            let seen = d <= 65 + 2 * n;
            assert_eq!(r.ppu.sec_count, u8::from(seen), "sprite {n} poked on {d}");
        }
    }
}

#[test]
fn copy_takes_eight_dots_per_in_range_sprite() {
    let mut r = Rig::new();
    for n in [3usize, 4, 10, 40] {
        r.ppu.oam[n * 4..n * 4 + 4].copy_from_slice(&[50, n as u8, 0x23, 7]);
    }
    r.to(50, 65);
    let mut counts = Vec::new();
    while r.ppu.dot <= 256 {
        r.tick();
        counts.push((r.ppu.dot - 1, r.ppu.sec_count));
    }
    // Sprite 3: Y checked on dot 72, bytes copied on 74, 76, 78; count rises on 78.
    let rise: Vec<u16> = counts
        .windows(2)
        .filter(|w| w[1].1 > w[0].1)
        .map(|w| w[1].0)
        .collect();
    assert_eq!(
        rise,
        vec![78, 86, 6 + 66 + 2 * 10 + 12, 6 + 66 + 2 * 40 + 18]
    );
    assert_eq!(
        &r.ppu.secondary[..16],
        &[50, 3, 0x23, 7, 50, 4, 0x23, 7, 50, 10, 0x23, 7, 50, 40, 0x23, 7]
    );
    assert_eq!(r.ppu.eval_phase, EVAL_DONE);
}

#[test]
fn oamdata_reads_the_evaluation_bus_while_rendering() {
    let mut r = Rig::new();
    for (i, b) in r.ppu.oam.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(7) | 0x80; // Y >= $80: never in range on line 20
    }
    r.to(20, 1);
    for d in 1..=340u16 {
        let got = r.r(0x2004);
        let want = match d {
            1..=65 => 0xFF,
            66..=256 => {
                // Last odd dot already run reads sprite (k - 65) / 2's Y byte.
                let k = if d % 2 == 1 { d - 2 } else { d - 1 };
                r.ppu.oam[usize::from((k - 65) / 2 % 64) * 4]
            }
            // No sprite found: secondary OAM is $FF except slot 0's Y byte, which
            // holds the last Y the copy step wrote there (sprite 63's).
            257..=320 if (d - 257) % 8 == 0 && d < 265 => r.ppu.oam[63 * 4],
            257..=320 => 0xFF,
            _ => r.ppu.oam[usize::from(r.ppu.oam_addr)],
        };
        let mask = if r.ppu.oam_addr & 3 == 2 { 0xE3 } else { 0xFF };
        assert_eq!(got, want & mask, "dot {d}");
        r.tick();
    }
}

#[test]
fn oamdata_writes_while_rendering_only_bump_the_sprite_index() {
    for start in 0..=255u8 {
        let mut r = Rig::new();
        r.to(100, 300);
        r.ppu.oam_addr = start;
        let before = r.ppu.oam;
        r.w(0x2004, 0x55);
        assert_eq!(r.ppu.oam, before, "no write");
        assert_eq!(r.ppu.oam_addr, start.wrapping_add(4));
    }
    // In vblank the write lands and OAMADDR steps by one.
    let mut r = Rig::new();
    r.to(245, 0);
    r.w(0x2003, 0x10);
    r.w(0x2004, 0x55);
    assert_eq!((r.ppu.oam[0x10], r.ppu.oam_addr), (0x55, 0x11));
}

#[test]
fn oamaddr_is_cleared_on_dots_257_to_320_only_when_rendering() {
    let mut r = Rig::new();
    r.to(30, 257);
    r.ppu.oam_addr = 0x44;
    r.tick();
    assert_eq!(r.ppu.oam_addr, 0);
    r.ppu.mask = 0;
    r.to(31, 260);
    r.ppu.oam_addr = 0x44;
    r.to(31, 330);
    assert_eq!(r.ppu.oam_addr, 0x44);
}

#[test]
fn evaluation_starts_at_oamaddr_and_wraps() {
    let mut r = Rig::new();
    r.ppu.mask = 0;
    for n in 0..64 {
        r.ppu.oam[n * 4] = if n % 9 == 0 { 40 } else { 0xF0 };
        r.ppu.oam[n * 4 + 1] = n as u8;
    }
    r.to(40, 60);
    r.ppu.oam_addr = 8 * 4;
    r.ppu.mask = 0x18;
    r.to(40, 257);
    assert_eq!(r.ppu.eval_base, 32);
    let tiles: Vec<u8> = (0..usize::from(r.ppu.sec_count))
        .map(|i| r.ppu.secondary[i * 4 + 1])
        .collect();
    assert_eq!(tiles, vec![9, 18, 27, 36, 45, 54, 63, 0]);
}

fn overflow_on_line(set: impl Fn(&mut [u8; 256])) -> bool {
    let mut r = Rig::new();
    for n in 0..8 {
        r.ppu.oam[n * 4] = 60;
    }
    set(&mut r.ppu.oam);
    r.to(60, 0);
    r.to(61, 0);
    r.ppu.status & 0x20 != 0
}

#[test]
fn overflow_bug_false_negative_and_false_positive() {
    assert!(overflow_on_line(|o| o[8 * 4] = 60), "true ninth sprite");
    // Sprite 8 out of range moves the scan to sprite 9's tile byte (m = 1): a ninth
    // sprite with Y in range but a tile byte out of range is missed.
    assert!(!overflow_on_line(|o| {
        o[9 * 4] = 60;
        o[9 * 4 + 1] = 0xF0;
    }));
    // Y out of range but the tile byte read by the bug is "in range": flagged.
    assert!(overflow_on_line(|o| o[9 * 4 + 1] = 60));
    // Diagonal: sprite 8 + k is read at byte k & 3.
    for k in 0..56usize {
        assert!(
            overflow_on_line(|o| o[(8 + k) * 4 + (k & 3)] = 60),
            "diagonal {k}"
        );
        if k & 3 != 0 {
            assert!(
                !overflow_on_line(|o| o[(8 + k) * 4] = 60),
                "off-diagonal {k}"
            );
        }
    }
}

fn corrupted_row(dot: u16) -> Option<usize> {
    let mut r = Rig::new();
    for (i, b) in r.ppu.oam.iter_mut().enumerate() {
        *b = 0xC0 | (i as u8 & 0x3F) ^ (i as u8 >> 6);
    }
    r.ppu.oam[..8].copy_from_slice(&[0xEE; 8]);
    let original = r.ppu.oam;
    r.to(50, dot);
    r.w(0x2001, 0);
    r.to(52, 0);
    assert_eq!(
        r.ppu.oam, original,
        "nothing changes while rendering is off"
    );
    r.w(0x2001, 0x18);
    r.tick();
    let rows: Vec<usize> = (0..32)
        .filter(|&i| r.ppu.oam[i * 8..i * 8 + 8] != original[i * 8..i * 8 + 8])
        .collect();
    assert!(rows.len() <= 1);
    for &i in &rows {
        assert_eq!(r.ppu.oam[i * 8..i * 8 + 8], [0xEE; 8], "row 0 copied");
    }
    rows.first().copied()
}

#[test]
fn rendering_off_mid_line_corrupts_the_documented_row() {
    for dot in 0..=340u16 {
        let want = match dot {
            0..=63 => usize::from(dot >> 1),
            256..=319 => {
                let d = usize::from(dot - 256);
                (d / 8) * 4 + (d % 8).min(3)
            }
            _ => 0,
        };
        let want = (want != 0).then_some(want);
        assert_eq!(corrupted_row(dot), want, "dot {dot}");
    }
}

#[test]
fn rendering_off_in_vblank_corrupts_nothing() {
    let mut r = Rig::new();
    r.to(245, 10);
    r.w(0x2001, 0);
    assert_eq!(r.ppu.corrupt_rows, 0);
}
