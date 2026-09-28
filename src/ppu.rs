//! Original model of the 2C02 picture processing unit (NTSC timing).
//!
//! # CPU interface (`$2000`–`$2007`, mirrored every 8 bytes up to `$3FFF`)
//! * `$2000` PPUCTRL: bits 1–0 base nametable (into `t`), 2 VRAM increment 1/32,
//!   3 sprite pattern table (8×8), 4 background pattern table, 5 8×16 sprites,
//!   7 NMI at vblank. Setting bit 7 while the vblank flag is set raises NMI at once.
//! * `$2001` PPUMASK: 0 grayscale, 1/2 show background/sprites in the left 8 pixels,
//!   3/4 show background/sprites, 7–5 emphasis (kept, not applied to indices).
//! * `$2002` PPUSTATUS (read): 7 vblank, 6 sprite 0 hit, 5 sprite overflow, bits 4–0
//!   from the open-bus latch. Reading clears vblank and the `w` toggle. A read on the
//!   dot before vblank starts suppresses both the flag and its NMI for that frame.
//! * `$2003` OAMADDR, `$2004` OAMDATA (write increments OAMADDR; read does not).
//! * `$2005` PPUSCROLL and `$2006` PPUADDR: two writes each through the shared `w`
//!   toggle into the loopy `t`/`v`/fine-x registers.
//! * `$2007` PPUDATA: reads below `$3F00` return the previous buffer and refill it;
//!   palette reads return at once and refill the buffer with the nametable byte
//!   underneath. `v` advances by 1 or 32.
//! * Every write, and every read of a readable register, refreshes the open-bus latch;
//!   write-only registers read back the latch.
//!
//! # Memory
//! Pattern tables `$0000`–`$1FFF` come from the cartridge through
//! [`crate::mapper::Cartridge::chr_offset`] (CHR RAM when the cart has no CHR ROM).
//! Nametables `$2000`–`$3EFF` use 4 KiB of internal memory folded by [`Mirroring`].
//! Palette `$3F00`–`$3FFF` is 32 bytes; `$3F10/14/18/1C` mirror `$3F00/04/08/0C`.
//!
//! # Timing and rendering
//! 341 dots × 262 scanlines; lines 0–239 visible, 241 starts vblank at dot 1, 261 is
//! the pre-render line (clears the flags at dot 1, copies vertical `t` into `v` over
//! dots 280–304). With rendering enabled the pre-render line of odd frames is one dot
//! short.
//!
//! ## Background pipeline (dot by dot)
//! On visible and pre-render lines with rendering on, every 8-dot group of dots
//! 1–256 and 321–336 reads, each on the second dot of its 2-dot access: the
//! nametable byte (dot 2 of the group) from `$2000 | (v & $0FFF)`, the attribute byte
//! (dot 4) from `$23C0 | (v & $0C00) | ((v >> 4) & $38) | ((v >> 2) & 7)`, the pattern
//! low plane (dot 6) and high plane (dot 8) from the background table, tile and fine
//! y of `v`. Coarse x of `v` increments on dots 8, 16, …, 256, 328 and 336; fine/coarse
//! y on dot 256; horizontal bits of `t` are copied into `v` on dot 257 and vertical
//! bits on pre-render dots 280–304. Two 16-bit pattern shift registers and two
//! attribute shift registers (the 1-bit attribute latch expanded to 8 bits) shift
//! once per dot on dots 2–257 and 322–337, and their low byte reloads from the
//! latches on dots 9, 17, …, 257, 329 and 337. Pixel `x` is composed on dot `x + 1`
//! from bit `15 - fine_x`, after that dot's shift and reload, with PPUMASK and the
//! palette as they are on that dot. So a `$2006`/`$2005`/`$2000` write or a CHR/
//! nametable change mid-line affects the first tile fetched after it, two tiles
//! (16 dots) before that tile reaches the screen, exactly as on the 2C02; fine x
//! changes act at once. With rendering off each dot shows the backdrop (or the
//! palette entry `v` points into) and the shift registers stand still.
//!
//! ## Sprites
//! Secondary OAM is cleared to `$FF` over dots 1–64; evaluation of the current line
//! for the next one runs from dot 65, reading OAM as it stands at dot 65: 2 dots per
//! y check and 6 more for each in-range sprite copied (up to 8). After the eighth,
//! the overflow scan reproduces the hardware bug: it advances the byte index with
//! the sprite index, so it compares tile, attribute and x bytes as y values (false
//! positives and negatives), and the overflow flag rises on the second dot of the
//! check that matched. Dots 257–320 fetch each slot's pattern (low plane on dot
//! 262 + 8i, high on 264 + 8i), so pattern writes during hblank land per slot; the
//! line's sprite pixels are latched at dot 320. A sprite with OAM y = Y shows on
//! lines Y+1..; nothing is evaluated on the pre-render line, so line 0 has no
//! sprites. Sprite 0 hit is set on the dot of the first overlapping opaque pixel
//! (never at x = 255, not in clipped left pixels), from the shift-register pixel.
//!
//! Every real fetch is put on the cartridge bus through [`Cartridge::ppu_address`]
//! (idle-dot CHR address, NT, AT, both pattern planes, the eight sprite slots, the
//! two trailing NT reads), so A12 watchers see the true edges.
//!
//! [`Region`] selects the 2C02 (NTSC: 262 lines, vblank at 241, odd-frame dot skip),
//! the 2C07 (PAL: 312 lines, vblank at 241, no skip, emphasis red/green swapped) or
//! the Dendy timing (312 lines, vblank flag at line 291, no skip).
//!
//! The framebuffer holds 256×240 6-bit palette indices (grayscale applied), with the
//! emphasis bits of each pixel in [`Ppu::emphasis`]. [`ntsc_rgb`] turns an index and
//! emphasis into RGB from a model of the composite signal (no captured palette file).

use crate::mapper::Cartridge;

pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 240;
pub const DOTS: u16 = 341;
/// NTSC line count; see [`Region::lines`] for the others.
pub const LINES: u16 = 262;

/// Console timing family (NES 2.0 header byte 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Region {
    /// RP2C02 + RP2A03: master clock / 4 per dot, / 12 per CPU cycle (3 dots).
    #[default]
    Ntsc,
    /// RP2C07 + RP2A07: master / 5 per dot, / 16 per CPU cycle (3.2 dots).
    Pal,
    /// UA6538-style clone: PAL dot rate, CPU at master / 15 (3 dots), 51 post-render
    /// lines before vblank so NMI timing matches NTSC software.
    Dendy,
}

impl Region {
    pub fn lines(self) -> u16 {
        match self {
            Self::Ntsc => 262,
            _ => 312,
        }
    }
    /// Line whose dot 1 sets the vblank flag.
    pub fn vblank_line(self) -> u16 {
        match self {
            Self::Dendy => 291,
            _ => 241,
        }
    }
    pub fn prerender(self) -> u16 {
        self.lines() - 1
    }
    /// PPU dots per CPU cycle as `(dots, cycles)`.
    pub fn dots_per_cpu(self) -> (u32, u32) {
        match self {
            Self::Pal => (16, 5),
            _ => (3, 1),
        }
    }
    /// CPU clock in Hz (master clock / divider).
    pub fn cpu_hz(self) -> f64 {
        match self {
            Self::Ntsc => 236_250_000.0 / 11.0 / 12.0,
            Self::Pal => 26_601_712.5 / 16.0,
            Self::Dendy => 26_601_712.5 / 15.0,
        }
    }
    /// Stable code for save states.
    pub fn code(self) -> u8 {
        self as u8
    }
    pub fn from_code(c: u8) -> Option<Self> {
        [Self::Ntsc, Self::Pal, Self::Dendy]
            .get(usize::from(c))
            .copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    Horizontal,
    Vertical,
    FourScreen,
    SingleLow,
    SingleHigh,
}

impl Mirroring {
    /// Offset into the 4 KiB nametable memory for `$2000`–`$3EFF`.
    pub fn fold(self, addr: u16) -> usize {
        let a = usize::from(addr & 0x0FFF);
        let table = a >> 10;
        let page = match self {
            Self::Horizontal => table >> 1,
            Self::Vertical => table & 1,
            Self::FourScreen => table,
            Self::SingleLow => 0,
            Self::SingleHigh => 1,
        };
        page * 0x400 + (a & 0x3FF)
    }
}

/// What the PPU sees outside itself: the cartridge, its CHR memory, and mirroring.
pub struct PpuMemory<'a> {
    pub cart: Option<&'a mut dyn Cartridge>,
    pub chr: &'a mut [u8],
    pub chr_writable: bool,
    pub mirroring: Mirroring,
}

impl PpuMemory<'_> {
    fn chr_index(&self, addr: u16) -> usize {
        let off = match &self.cart {
            Some(c) => c.chr_offset(addr & 0x1FFF),
            None => usize::from(addr & 0x1FFF),
        };
        off % self.chr.len().max(1)
    }

    fn mirroring(&self) -> Mirroring {
        self.cart
            .as_ref()
            .and_then(|c| c.mirroring())
            .unwrap_or(self.mirroring)
    }

    fn a12(&mut self, addr: u16) {
        if let Some(c) = self.cart.as_mut() {
            c.ppu_address(addr);
        }
    }

    /// A real rendering read of `addr`: the address goes on the bus and /RD
    /// pulses (MMC5 counts these reads; the idle dot 0 only drives the address).
    fn fetch(&mut self, addr: u16) {
        if let Some(c) = self.cart.as_mut() {
            c.ppu_address(addr);
            c.ppu_read(addr);
        }
    }

    fn pattern(&self, addr: u16) -> u8 {
        if self.chr.is_empty() {
            return 0;
        }
        self.chr[self.chr_index(addr)]
    }

    fn target(&self, addr: u16) -> PpuTarget {
        self.cart
            .as_ref()
            // Patterns keep all 13 bits; $3000-$3EFF mirror the nametables.
            .map_or(PpuTarget::Default, |c| {
                c.ppu_target(if addr & 0x3FFF < 0x2000 {
                    addr & 0x1FFF
                } else {
                    0x2000 | (addr & 0x0FFF)
                })
            })
    }
}

pub use crate::mapper::PpuTarget;

#[derive(Clone)]
pub struct Ppu {
    pub ctrl: u8,
    pub mask: u8,
    pub status: u8,
    pub oam_addr: u8,
    pub oam: [u8; 256],
    pub vram: Box<[u8; 0x1000]>,
    pub palette: [u8; 32],
    /// Loopy registers: current and temporary VRAM address, fine x, write toggle.
    pub v: u16,
    pub t: u16,
    pub fine_x: u8,
    pub w: bool,
    pub buffer: u8,
    pub latch: u8,
    pub dot: u16,
    pub line: u16,
    pub frame: u64,
    pub framebuffer: Vec<u8>,
    nmi_out: bool,
    nmi_edge: bool,
    suppress: bool,
    /// Dot on which the overflow flag rises this line (0: none).
    overflow_dot: u16,
    /// Emphasis bits (PPUMASK 7–5) in effect for each framebuffer pixel.
    pub emphasis: Vec<u8>,
    /// Background shift registers (pattern planes, expanded attribute bits).
    pub bg_lo: u16,
    pub bg_hi: u16,
    pub at_lo: u16,
    pub at_hi: u16,
    /// Latches filled by the fetches of the tile being read.
    pub nt_latch: u8,
    pub at_latch: u8,
    pub pt_lo: u8,
    pub pt_hi: u8,
    /// Secondary OAM and the number of sprites copied into it.
    pub secondary: [u8; 32],
    pub sec_count: u8,
    sec_has0: bool,
    sp_lo: [u8; 8],
    sp_hi: [u8; 8],
    line_sp: [u8; WIDTH],
    line_sp0: [bool; WIDTH],
    /// Sprite evaluation, run dot by dot on dots 65–256: sprites examined since dot
    /// 65 (`n`, 0–64), byte within the sprite (`m`), the byte read on the last odd
    /// dot (what `$2004` returns), the OAMADDR evaluation started from, and the
    /// phase ([`EVAL_COPY`], [`EVAL_OVERFLOW`], [`EVAL_DONE`]).
    pub eval_n: u8,
    pub eval_m: u8,
    pub eval_latch: u8,
    pub eval_base: u8,
    pub eval_phase: u8,
    /// OAM rows (8 bytes each) to be overwritten by row 0 when rendering resumes,
    /// set by turning rendering off mid-line (see [`Ppu::write_register`]).
    pub corrupt_rows: u32,
    pub region: Region,
    /// 2C02G: if OAMADDR >= 8 when rendering starts (pre-render dot 1), the eight
    /// bytes at `OAMADDR & $F8` are copied over OAM bytes 0–7. On by default for
    /// NTSC; not documented for the 2C07 and Dendy parts, so off there.
    pub oamaddr_bug: bool,
}

/// Sprite evaluation phases.
pub const EVAL_COPY: u8 = 0;
pub const EVAL_OVERFLOW: u8 = 1;
pub const EVAL_DONE: u8 = 2;

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            ctrl: 0,
            mask: 0,
            status: 0,
            oam_addr: 0,
            oam: [0; 256],
            vram: Box::new([0; 0x1000]),
            palette: [0; 32],
            v: 0,
            t: 0,
            fine_x: 0,
            w: false,
            buffer: 0,
            latch: 0,
            dot: 0,
            line: 0,
            frame: 0,
            framebuffer: vec![0; WIDTH * HEIGHT],
            nmi_out: false,
            nmi_edge: false,
            suppress: false,
            overflow_dot: 0,
            emphasis: vec![0; WIDTH * HEIGHT],
            bg_lo: 0,
            bg_hi: 0,
            at_lo: 0,
            at_hi: 0,
            nt_latch: 0,
            at_latch: 0,
            pt_lo: 0,
            pt_hi: 0,
            secondary: [0xFF; 32],
            sec_count: 0,
            sec_has0: false,
            sp_lo: [0; 8],
            sp_hi: [0; 8],
            line_sp: [0; WIDTH],
            line_sp0: [false; WIDTH],
            eval_n: 64,
            eval_m: 0,
            eval_latch: 0xFF,
            eval_base: 0,
            eval_phase: EVAL_DONE,
            corrupt_rows: 0,
            region: Region::Ntsc,
            oamaddr_bug: true,
        }
    }

    /// Switch timing family; resets the beam to line 0, dot 0.
    pub fn set_region(&mut self, region: Region) {
        self.region = region;
        self.oamaddr_bug = region == Region::Ntsc;
        self.line = 0;
        self.dot = 0;
    }

    fn chr_read(&self, mem: &PpuMemory, addr: u16) -> u8 {
        match mem.target(addr) {
            PpuTarget::Ciram(i) => self.vram[i & 0x7FF],
            PpuTarget::Chr(i) if !mem.chr.is_empty() => mem.chr[i % mem.chr.len()],
            PpuTarget::Chr(_) => 0,
            PpuTarget::Mapper(v) => v,
            PpuTarget::Default => mem.pattern(addr),
        }
    }

    fn nt_read(&self, mem: &PpuMemory, addr: u16) -> u8 {
        match mem.target(addr) {
            PpuTarget::Ciram(i) => self.vram[i & 0x7FF],
            PpuTarget::Chr(i) if !mem.chr.is_empty() => mem.chr[i % mem.chr.len()],
            PpuTarget::Chr(_) => 0,
            PpuTarget::Mapper(v) => v,
            PpuTarget::Default => self.vram[mem.mirroring().fold(addr)],
        }
    }

    fn ppu_write(&mut self, mem: &mut PpuMemory, addr: u16, value: u8) {
        match mem.target(addr) {
            PpuTarget::Ciram(i) => self.vram[i & 0x7FF] = value,
            PpuTarget::Mapper(_) => {
                if let Some(c) = mem.cart.as_mut() {
                    c.ppu_write(addr, value);
                }
            }
            PpuTarget::Chr(i) => {
                if mem.chr_writable && !mem.chr.is_empty() {
                    let n = mem.chr.len();
                    mem.chr[i % n] = value;
                }
            }
            PpuTarget::Default if addr < 0x2000 => {
                if mem.chr_writable && !mem.chr.is_empty() {
                    let i = mem.chr_index(addr);
                    mem.chr[i] = value;
                }
            }
            PpuTarget::Default => self.vram[mem.mirroring().fold(addr)] = value,
        }
    }

    /// True on lines where the PPU renders (visible and pre-render) with rendering on.
    fn rendering_line(&self) -> bool {
        self.rendering() && (self.line < 240 || self.line == self.region.prerender())
    }

    /// 2C07 forced OAM refresh: with rendering enabled the PAL PPU starts
    /// re-reading OAM 24 lines into vblank (line 265) and keeps going until the
    /// pre-render line, so OAMDATA writes (and OAM DMA) only work in the first 24
    /// lines of PAL vblank; outside that window they take the glitchy rendering path.
    pub fn oam_refresh(&self) -> bool {
        self.region == Region::Pal
            && self.rendering()
            && (265..self.region.prerender()).contains(&self.line)
    }

    pub fn rendering(&self) -> bool {
        self.mask & 0x18 != 0
    }

    /// True once per rising edge of the NMI output; clears on read.
    pub fn take_nmi(&mut self) -> bool {
        std::mem::take(&mut self.nmi_edge)
    }

    fn update_nmi(&mut self) {
        let out = self.ctrl & 0x80 != 0 && self.status & 0x80 != 0;
        if out && !self.nmi_out {
            self.nmi_edge = true;
        }
        self.nmi_out = out;
    }

    fn palette_index(addr: u16) -> usize {
        let i = usize::from(addr & 0x1F);
        if i & 0x13 == 0x10 {
            i & 0x0F
        } else {
            i
        }
    }

    /// PPU-bus read (`$0000`–`$3FFF`), no side effects besides A12 notification.
    pub fn mem_read(&self, mem: &PpuMemory, addr: u16) -> u8 {
        let addr = addr & 0x3FFF;
        match addr {
            0x0000..=0x1FFF => self.chr_read(mem, addr),
            0x2000..=0x3EFF => self.nt_read(mem, addr),
            _ => {
                let gray = if self.mask & 1 != 0 { 0x30 } else { 0x3F };
                self.palette[Self::palette_index(addr)] & gray
            }
        }
    }

    pub fn mem_write(&mut self, mem: &mut PpuMemory, addr: u16, value: u8) {
        let addr = addr & 0x3FFF;
        match addr {
            0x0000..=0x3EFF => self.ppu_write(mem, addr, value),
            _ => self.palette[Self::palette_index(addr)] = value & 0x3F,
        }
    }

    fn increment(&mut self, mem: &mut PpuMemory) {
        self.v = (self.v + if self.ctrl & 4 != 0 { 32 } else { 1 }) & 0x7FFF;
        mem.a12(self.v & 0x3FFF);
    }

    /// CPU read of `$2000 + (reg & 7)`.
    pub fn read_register(&mut self, mem: &mut PpuMemory, reg: u16) -> u8 {
        match reg & 7 {
            2 => {
                if self.line == self.region.vblank_line() && self.dot == 0 {
                    self.suppress = true;
                }
                let value = (self.status & 0xE0) | (self.latch & 0x1F);
                self.status &= 0x7F;
                self.w = false;
                self.update_nmi();
                self.latch = value;
                value
            }
            4 => {
                let mut value = if self.rendering_line() {
                    self.oam_bus()
                } else {
                    self.oam[usize::from(self.oam_addr)]
                };
                if self.oam_addr & 3 == 2 {
                    value &= 0xE3; // attribute bits 4-2 do not exist
                }
                self.latch = value;
                value
            }
            7 => {
                let addr = self.v & 0x3FFF;
                let value = if addr >= 0x3F00 {
                    self.buffer = self.nt_read(mem, addr - 0x1000);
                    (self.mem_read(mem, addr) & 0x3F) | (self.latch & 0xC0)
                } else {
                    {
                        let next = self.mem_read(mem, addr);
                        std::mem::replace(&mut self.buffer, next)
                    }
                };
                self.increment(mem);
                self.latch = value;
                value
            }
            _ => self.latch,
        }
    }

    /// Side-effect-free view of a register (debuggers).
    pub fn peek_register(&self, reg: u16) -> u8 {
        match reg & 7 {
            2 => (self.status & 0xE0) | (self.latch & 0x1F),
            4 => self.oam[usize::from(self.oam_addr)],
            7 => self.buffer,
            _ => self.latch,
        }
    }

    pub fn write_register(&mut self, mem: &mut PpuMemory, reg: u16, value: u8) {
        self.latch = value;
        match reg & 7 {
            0 => {
                self.ctrl = value;
                self.t = (self.t & !0x0C00) | (u16::from(value & 3) << 10);
                self.update_nmi();
            }
            1 => {
                if self.rendering_line() && value & 0x18 == 0 {
                    self.mark_corruption();
                }
                self.mask = value;
            }
            2 => {}
            3 => self.oam_addr = value,
            4 if self.rendering_line() || self.oam_refresh() => {
                // Glitchy increment: the write is dropped and only the sprite
                // index (OAMADDR bits 7-2) advances.
                self.oam_addr = self.oam_addr.wrapping_add(4);
            }
            4 => {
                self.oam[usize::from(self.oam_addr)] = value;
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            5 => {
                if self.w {
                    self.t = (self.t & 0x0C1F)
                        | (u16::from(value & 7) << 12)
                        | (u16::from(value >> 3) << 5);
                } else {
                    self.t = (self.t & !0x1F) | u16::from(value >> 3);
                    self.fine_x = value & 7;
                }
                self.w = !self.w;
            }
            6 => {
                if self.w {
                    self.t = (self.t & 0x7F00) | u16::from(value);
                    self.v = self.t;
                    mem.a12(self.v & 0x3FFF);
                } else {
                    self.t = (self.t & 0x00FF) | (u16::from(value & 0x3F) << 8);
                }
                self.w = !self.w;
            }
            _ => {
                self.mem_write(mem, self.v, value);
                self.increment(mem);
            }
        }
        debug_assert!(self.v <= 0x7FFF && self.t <= 0x7FFF);
    }

    /// What the OAM data bus holds now, as `$2004` reads it while rendering: `$FF`
    /// on dots 1–64 (secondary OAM clear), the byte evaluation last read on 65–256,
    /// and the secondary OAM byte being fetched on 257–320 (Y, tile, attribute, X,
    /// then X again for the rest of the slot). Elsewhere it is OAM at OAMADDR.
    pub fn oam_bus(&self) -> u8 {
        match self.dot {
            1..=64 => 0xFF,
            65..=256 => self.eval_latch,
            257..=320 => {
                let d = usize::from(self.dot - 257);
                self.secondary[(d / 8) * 4 + (d % 8).min(3)]
            }
            _ => self.oam[usize::from(self.oam_addr)],
        }
    }

    /// Rendering switched off mid-line: record the secondary-OAM address being
    /// accessed (dots 0–63: the clear pointer `dot / 2`; dots 256–319: slot `(dot -
    /// 256) / 8`, byte `min(3, (dot - 256) % 8)`). That OAM row is overwritten with
    /// row 0 when rendering resumes. This is the 2C02 row corruption as documented on
    /// the NESdev wiki and modelled by Mesen; turning rendering off during sprite
    /// evaluation (dots 64–255) or HBlank after dot 320 corrupts nothing.
    fn mark_corruption(&mut self) {
        let row = match self.dot {
            0..=63 => Some(self.dot >> 1),
            256..=319 => {
                let d = self.dot - 256;
                Some((d / 8) * 4 + (d % 8).min(3))
            }
            _ => None,
        };
        if let Some(row) = row {
            self.corrupt_rows |= 1 << row;
        }
    }

    fn apply_corruption(&mut self) {
        let rows = std::mem::take(&mut self.corrupt_rows);
        for row in 1..32 {
            if rows & (1 << row) != 0 {
                self.oam.copy_within(0..8, row * 8);
            }
        }
    }

    /// OAM DMA body: 256 bytes written through OAMDATA starting at OAMADDR.
    pub fn oam_dma(&mut self, page: &[u8; 256]) {
        if self.oam_refresh() {
            // Every DMA write lands on a refreshing OAM and is dropped with a +4
            // address step; 256 such steps bring OAMADDR back to where it was.
            return;
        }
        for &b in page {
            self.oam[usize::from(self.oam_addr)] = b;
            self.oam_addr = self.oam_addr.wrapping_add(1);
        }
    }

    fn increment_y(&mut self) {
        if self.v & 0x7000 != 0x7000 {
            self.v += 0x1000;
            return;
        }
        self.v &= !0x7000;
        let mut y = (self.v & 0x3E0) >> 5;
        if y == 29 {
            y = 0;
            self.v ^= 0x0800;
        } else if y == 31 {
            y = 0;
        } else {
            y += 1;
        }
        self.v = (self.v & !0x3E0) | (y << 5);
    }

    fn sprite_height(&self) -> u16 {
        if self.ctrl & 0x20 != 0 {
            16
        } else {
            8
        }
    }

    /// Pattern address of `row` of a sprite (vertical flip applied).
    fn sprite_addr(&self, tile: u8, attr: u8, row: u16) -> u16 {
        let h = self.sprite_height();
        let row = if attr & 0x80 != 0 { h - 1 - row } else { row };
        let base = if h == 16 {
            (u16::from(tile & 1) << 12)
                + u16::from(tile & 0xFE) * 16
                + if row >= 8 { 16 } else { 0 }
        } else {
            (u16::from(self.ctrl & 8) << 9) + u16::from(tile) * 16
        };
        base + (row & 7)
    }

    fn increment_x(&mut self) {
        if self.v & 0x1F == 31 {
            self.v = (self.v & !0x1F) ^ 0x0400;
        } else {
            self.v += 1;
        }
    }

    fn shift_bg(&mut self) {
        self.bg_lo <<= 1;
        self.bg_hi <<= 1;
        self.at_lo <<= 1;
        self.at_hi <<= 1;
    }

    fn reload_bg(&mut self) {
        let fill = |b: bool| if b { 0xFF } else { 0 };
        self.bg_lo = (self.bg_lo & 0xFF00) | u16::from(self.pt_lo);
        self.bg_hi = (self.bg_hi & 0xFF00) | u16::from(self.pt_hi);
        self.at_lo = (self.at_lo & 0xFF00) | fill(self.at_latch & 1 != 0);
        self.at_hi = (self.at_hi & 0xFF00) | fill(self.at_latch & 2 != 0);
    }

    /// One dot of the background pipeline (dots 1–257 and 321–337).
    fn background_dot(&mut self, mem: &mut PpuMemory) {
        let d = self.dot;
        if (2..=257).contains(&d) || (322..=337).contains(&d) {
            self.shift_bg();
        }
        let phase = (d - 1) % 8;
        if phase == 0 {
            self.reload_bg();
        }
        if d == 257 || d == 337 {
            return;
        }
        let v = self.v;
        let table = u16::from(self.ctrl & 0x10) << 8;
        let pattern = table + u16::from(self.nt_latch) * 16 + ((v >> 12) & 7);
        match phase {
            1 => {
                let nt = 0x2000 | (v & 0x0FFF);
                self.nt_latch = self.nt_read(mem, nt);
                mem.fetch(nt);
            }
            3 => {
                let at_addr = 0x23C0 | (v & 0x0C00) | ((v >> 4) & 0x38) | ((v >> 2) & 7);
                let at = self.nt_read(mem, at_addr);
                self.at_latch = (at >> (((v >> 4) & 4) | (v & 2))) & 3;
                mem.fetch(at_addr);
            }
            5 => {
                self.pt_lo = self.chr_read(mem, pattern);
                mem.fetch(pattern);
            }
            7 => {
                self.pt_hi = self.chr_read(mem, pattern + 8);
                mem.fetch(pattern + 8);
                self.increment_x();
            }
            _ => {}
        }
    }

    fn sprite_in_range(&self, y: u8) -> bool {
        let y = u16::from(y);
        y <= self.line && self.line < y + self.sprite_height()
    }

    /// One dot of sprite evaluation (dots 1–256 of a rendering line), as on the
    /// 2C02: dots 1–64 write `$FF` to secondary OAM on even dots; from dot 65 odd
    /// dots read OAM at `OAMADDR(dot 65) + 4n + m` and even dots act on that byte.
    /// While fewer than eight sprites are found the Y byte is copied to the next free
    /// slot and, if in range, the other three follow; with eight found, secondary OAM
    /// is read-only and each out-of-range check advances both `n` and `m` (the
    /// hardware's overflow bug), so the flag can rise or miss on tile and attribute
    /// bytes. OAM is read live, so writes landing before a sprite's read are seen.
    fn evaluation_dot(&mut self) {
        let d = self.dot;
        if d <= 64 {
            self.eval_latch = 0xFF;
            if d % 2 == 0 {
                self.secondary[usize::from(d / 2 - 1)] = 0xFF;
            }
            return;
        }
        if d == 65 {
            self.sec_count = 0;
            self.sec_has0 = false;
            self.overflow_dot = 0;
            self.eval_n = 0;
            self.eval_m = 0;
            self.eval_base = self.oam_addr;
            self.eval_phase = if self.line < 240 {
                EVAL_COPY
            } else {
                EVAL_DONE
            };
        }
        let addr = self
            .eval_base
            .wrapping_add(self.eval_n.wrapping_mul(4))
            .wrapping_add(self.eval_m);
        if d % 2 == 1 {
            self.eval_latch = self.oam[usize::from(addr)];
            return;
        }
        let byte = self.eval_latch;
        match self.eval_phase {
            EVAL_COPY => {
                let slot = usize::from(self.sec_count) * 4 + usize::from(self.eval_m);
                self.secondary[slot] = byte;
                if self.eval_m == 0 {
                    if self.sprite_in_range(byte) {
                        self.sec_has0 |= self.eval_n == 0;
                        self.eval_m = 1;
                    } else {
                        self.eval_n += 1;
                    }
                } else {
                    self.eval_m += 1;
                    if self.eval_m == 4 {
                        self.eval_m = 0;
                        self.eval_n += 1;
                        self.sec_count += 1;
                    }
                }
                if self.eval_n >= 64 {
                    self.eval_phase = EVAL_DONE;
                } else if self.sec_count == 8 {
                    self.eval_phase = EVAL_OVERFLOW;
                }
            }
            EVAL_OVERFLOW => {
                if self.sprite_in_range(byte) {
                    self.overflow_dot = d;
                    self.eval_phase = EVAL_DONE;
                } else {
                    self.eval_n += 1;
                    self.eval_m = (self.eval_m + 1) & 3;
                    if self.eval_n >= 64 {
                        self.eval_phase = EVAL_DONE;
                    }
                }
            }
            _ => {
                self.eval_n = (self.eval_n + 1) & 63;
            }
        }
        debug_assert!(self.sec_count <= 8 && self.eval_m < 4);
    }

    /// Dots 257–320: pattern fetches for each secondary-OAM slot, latched at 320.
    /// Every slot fetches, including empty ones (secondary OAM `$FF`: tile `$FF`),
    /// so the pattern-table address and A12 behave as on the bus; empty slots'
    /// data is discarded. Dots 1 and 3 of each slot are garbage nametable reads.
    fn sprite_fetch_dot(&mut self, mem: &mut PpuMemory) {
        let d = self.dot;
        let i = usize::from((d - 257) / 8);
        let phase = (d - 257) % 8;
        if phase == 1 || phase == 3 {
            mem.fetch(0x2000 | (self.v & 0x0FFF));
        }
        if phase == 5 || phase == 7 {
            let o = &self.secondary[i * 4..i * 4 + 4];
            let (y, tile, attr) = (o[0], o[1], o[2]);
            let row = self.line.wrapping_sub(u16::from(y)) & (self.sprite_height() - 1);
            let addr = self.sprite_addr(tile, attr, row) + if phase == 7 { 8 } else { 0 };
            let data = self.chr_read(mem, addr);
            mem.fetch(addr);
            if i < usize::from(self.sec_count) {
                if phase == 5 {
                    self.sp_lo[i] = data;
                } else {
                    self.sp_hi[i] = data;
                }
            }
        }
        if d == 320 {
            self.latch_sprites();
        }
    }

    fn latch_sprites(&mut self) {
        self.line_sp = [0; WIDTH];
        self.line_sp0 = [false; WIDTH];
        for i in (0..usize::from(self.sec_count)).rev() {
            let (attr, x) = (
                self.secondary[i * 4 + 2],
                usize::from(self.secondary[i * 4 + 3]),
            );
            let (mut lo, mut hi) = (self.sp_lo[i], self.sp_hi[i]);
            if attr & 0x40 != 0 {
                lo = lo.reverse_bits();
                hi = hi.reverse_bits();
            }
            for bit in 0..8 {
                let px = ((lo >> (7 - bit)) & 1) | (((hi >> (7 - bit)) & 1) << 1);
                let sx = x + bit;
                if px != 0 && sx < WIDTH {
                    self.line_sp[sx] = px | ((attr & 3) << 2) | (attr & 0x20);
                    self.line_sp0[sx] = i == 0 && self.sec_has0;
                }
            }
        }
    }

    /// Background pixel (palette bits 3-2, colour bits 1-0) under fine x.
    pub fn bg_pixel(&self) -> u8 {
        let bit = 15 - u16::from(self.fine_x);
        let b = |r: u16| ((r >> bit) & 1) as u8;
        b(self.bg_lo) | (b(self.bg_hi) << 1) | (b(self.at_lo) << 2) | (b(self.at_hi) << 3)
    }

    /// Compose pixel `x` of the current line with the mask and palette as they are now.
    fn compose(&mut self, x: usize) {
        let y = usize::from(self.line);
        let left = x < 8;
        let bgv = if self.mask & 8 != 0 && !(left && self.mask & 2 == 0) {
            self.bg_pixel()
        } else {
            0
        };
        let spv = if self.mask & 0x10 != 0 && !(left && self.mask & 4 == 0) {
            self.line_sp[x]
        } else {
            0
        };
        let (b, s) = (bgv & 3, spv & 3);
        if self.line_sp0[x] && b != 0 && s != 0 && x != 255 {
            self.status |= 0x40;
        }
        let addr = match (b, s) {
            (0, 0) => 0,
            (0, _) => 0x10 | u16::from(spv & 0x0F),
            (_, 0) => u16::from(bgv & 0x0F),
            _ if spv & 0x20 != 0 => u16::from(bgv & 0x0F),
            _ => 0x10 | u16::from(spv & 0x0F),
        };
        self.put(x, y, addr);
    }

    fn put(&mut self, x: usize, y: usize, addr: u16) {
        let gray = if self.mask & 1 != 0 { 0x30 } else { 0x3F };
        self.framebuffer[y * WIDTH + x] = self.palette[Self::palette_index(addr)] & gray;
        let e = self.mask >> 5;
        // The 2C07 wires PPUMASK bit 5 to green and bit 6 to red.
        self.emphasis[y * WIDTH + x] = if self.region == Region::Pal {
            (e & 4) | ((e & 1) << 1) | ((e >> 1) & 1)
        } else {
            e
        };
    }

    /// Advance one dot.
    pub fn tick(&mut self, mem: &mut PpuMemory) {
        let visible = self.line < 240;
        let pre = self.line == self.region.prerender();
        if self.rendering() && (visible || pre) {
            if pre && self.dot == 1 && self.oamaddr_bug && self.oam_addr >= 8 {
                let from = usize::from(self.oam_addr & 0xF8);
                self.oam.copy_within(from..from + 8, 0);
            }
            if self.corrupt_rows != 0 {
                self.apply_corruption();
            }
            let d = self.dot;
            if (1..=257).contains(&d) || (321..=337).contains(&d) {
                self.background_dot(mem);
            }
            if d == 0 {
                // Idle dot: the bus already shows the CHR address of the low
                // pattern fetch on dot 5 (documented 2C02 behaviour).
                let nt = self.nt_read(mem, 0x2000 | (self.v & 0x0FFF));
                let table = u16::from(self.ctrl & 0x10) << 8;
                mem.a12(table + u16::from(nt) * 16 + ((self.v >> 12) & 7));
            }
            if d == 338 || d == 340 {
                // The two unused nametable fetches at the end of the line.
                mem.fetch(0x2000 | (self.v & 0x0FFF));
            }
            if (1..=256).contains(&d) {
                self.evaluation_dot();
            }
            if (257..=320).contains(&d) {
                self.oam_addr = 0;
            }
            if d != 0 && d == self.overflow_dot {
                self.status |= 0x20;
                self.overflow_dot = 0;
            }
            if (257..=320).contains(&d) {
                self.sprite_fetch_dot(mem);
            }
            if visible && (1..=256).contains(&self.dot) {
                self.compose(usize::from(self.dot - 1));
            }
            if self.dot == 256 {
                self.increment_y();
            }
            if self.dot == 257 {
                self.v = (self.v & !0x041F) | (self.t & 0x041F);
            }
            if pre && (280..=304).contains(&self.dot) {
                self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
            }
        } else if visible && (1..=256).contains(&self.dot) {
            // Rendering off: the backdrop, or the palette entry v points at.
            let addr = if (self.v & 0x3F00) == 0x3F00 {
                self.v
            } else {
                0
            };
            self.put(usize::from(self.dot - 1), usize::from(self.line), addr);
        }
        if self.line == self.region.vblank_line() && self.dot == 1 {
            if !std::mem::take(&mut self.suppress) {
                self.status |= 0x80;
            }
            self.update_nmi();
        }
        if pre && self.dot == 1 {
            self.status &= 0x1F;
            self.update_nmi();
        }
        // Advance, skipping the last pre-render dot on odd frames with rendering on.
        let skip = pre
            && self.region == Region::Ntsc
            && self.dot == 339
            && self.frame & 1 == 1
            && self.rendering();
        if skip {
            // The skipped dot is the second half of the last dummy nametable fetch;
            // the read still completes (merged into dot 0 of line 0), so mappers
            // counting reads (MMC5) still see three identical fetches.
            mem.fetch(0x2000 | (self.v & 0x0FFF));
        }
        self.dot += 1;
        if self.dot == DOTS || skip {
            self.dot = 0;
            self.line += 1;
            if self.line == self.region.lines() {
                self.line = 0;
                self.frame += 1;
            }
        }
    }
}

/// Composite signal levels (relative to sync tip, in volts) for the four luma rows,
/// low and high halves of the square wave, as documented for the 2C02.
const LOW: [f64; 4] = [0.228, 0.312, 0.552, 0.880];
const HIGH: [f64; 4] = [0.616, 0.840, 1.100, 1.100];
const BLACK: f64 = 0.312;
const WHITE: f64 = 1.100;
/// Emphasis attenuates the signal during the phases of the emphasized colours.
const ATTENUATION: f64 = 0.746;

/// Luma and the two chroma components (`y`, `i`, `q` on the NTSC axes) of colour
/// `index` with emphasis, by sampling the 12 phases of the 2C02/2C07 square wave.
fn composite(index: u8, emphasis: u8) -> (f64, f64, f64) {
    let hue = i32::from(index & 0x0F);
    let row = usize::from((index >> 4) & 3);
    let (lo, hi) = if hue >= 14 {
        (LOW[1], LOW[1])
    } else {
        (LOW[row], HIGH[row])
    };
    let level = |phase: i32| -> f64 {
        let on = match hue {
            0 => true,
            13..=15 => false,
            // Offset so that hue 1 decodes blue, 6 red and 12 cyan, as on the console.
            h => (h + 8 + phase).rem_euclid(12) < 6,
        };
        let mut v = if on { hi } else { lo };
        // Each emphasis bit attenuates the six phases opposite its colour (red near
        // phase 0, green near 8, blue near 4), tinting the picture toward that colour.
        for (bit, centre) in [(1u8, 0i32), (2, 8), (4, 4)] {
            if emphasis & bit != 0 && hue < 14 && (phase - centre + 3).rem_euclid(12) >= 6 {
                v *= ATTENUATION;
            }
        }
        (v - BLACK) / (WHITE - BLACK)
    };
    let (mut y, mut i, mut q) = (0.0, 0.0, 0.0);
    for p in 0..12 {
        let v = level(p);
        let a = std::f64::consts::PI * f64::from(p) / 6.0;
        y += v;
        i += v * a.cos();
        q += v * a.sin();
    }
    (y / 12.0, i / 6.0, q / 6.0)
}

/// RGB for palette `index` (6 bits) with PPUMASK emphasis bits `emphasis` (bit 0 red,
/// 1 green, 2 blue on NTSC). The 12 phases are sampled from the square wave of hue
/// `index & 15` (hue 0 is the high level, 13 the low level, 14–15 black), decoded as
/// YIQ and gamma-free. Emphasis attenuates the half-cycle opposite each emphasized
/// colour's phase. Deterministic and computed here.
pub fn ntsc_rgb(index: u8, emphasis: u8) -> [u8; 3] {
    let (y, i, q) = composite(index, emphasis);
    let rgb = [
        y + 0.956 * i + 0.621 * q,
        y - 0.272 * i - 0.647 * q,
        y - 1.106 * i + 1.703 * q,
    ];
    rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// Hue rotation of the 2C07's chroma relative to the 2C02's, in degrees of the
/// decoded U/V angle.
pub const PAL_HUE_SHIFT_DEG: f64 = -15.0;

/// RGB for a 2C07 colour through a PAL decoder.
///
/// The 2C07 produces the same 12-phase square-wave composite as the 2C02 (same
/// levels, emphasis attenuating six phases), but it inverts the V phase on
/// alternate lines and its hues sit [`PAL_HUE_SHIFT_DEG`] away from NTSC. A PAL
/// set decodes Y/U/V and its delay line averages each line with the previous one,
/// so the alternation cancels and a single Y/U/V decode equals the averaged pair;
/// the PAL YUV→RGB matrix then gives RGB. Emphasis bits arrive already
/// swapped (the PAL red/green swap happens when the pixel is stored).
pub fn pal_rgb(index: u8, emphasis: u8) -> [u8; 3] {
    let (y, i, q) = composite(index, emphasis);
    let (s33, c33) = 33f64.to_radians().sin_cos();
    let (sh, ch) = PAL_HUE_SHIFT_DEG.to_radians().sin_cos();
    // Chroma rotated by the 2C07 offset, then the I/Q axes taken back to U/V.
    // (The I/Q axes run opposite to U/V's angle, hence the sign.)
    let (i, q) = (i * ch + q * sh, q * ch - i * sh);
    let u = -i * s33 + q * c33;
    let v = i * c33 + q * s33;
    // Line n carries +V and line n+1 -V of the same colour; the receiver re-inverts
    // the second and averages, which returns exactly (u, v) for this ideal signal.
    let rgb = [y + 1.140 * v, y - 0.395 * u - 0.581 * v, y + 2.032 * u];
    rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
}

impl Ppu {
    /// Framebuffer as packed RGB (256×240×3), through [`ntsc_rgb`].
    pub fn rgb_frame(&self) -> Vec<u8> {
        self.framebuffer
            .iter()
            .zip(&self.emphasis)
            .flat_map(|(&c, &e)| match self.region {
                Region::Ntsc => ntsc_rgb(c, e),
                Region::Pal | Region::Dendy => pal_rgb(c, e),
            })
            .collect()
    }
}

impl Ppu {
    /// Every PPU field, for [`crate::nes_state`]. Version 4 adds the dot-by-dot
    /// sprite evaluation state and pending OAM row corruption. Version 1 stops before the
    /// framebuffer, emphasis, pipeline (shift registers, latches, secondary OAM,
    /// sprite patterns, overflow dot) and sprite line buffers.
    pub(crate) fn save_into(&self, w: &mut crate::nes_state::StateWriter) {
        for v in [self.ctrl, self.mask, self.status, self.oam_addr] {
            w.u8(v);
        }
        w.bytes(&self.oam);
        w.bytes(&self.vram[..]);
        w.bytes(&self.palette);
        w.u16(self.v);
        w.u16(self.t);
        w.u8(self.fine_x);
        w.bool(self.w);
        w.u8(self.buffer);
        w.u8(self.latch);
        w.u16(self.dot);
        w.u16(self.line);
        w.u64(self.frame);
        for f in [
            self.nmi_out,
            self.nmi_edge,
            self.suppress,
            self.overflow_dot != 0,
        ] {
            w.bool(f);
        }
        if w.version >= 2 {
            w.bytes(&self.framebuffer);
            w.bytes(&self.emphasis);
            for r in [
                self.bg_lo,
                self.bg_hi,
                self.at_lo,
                self.at_hi,
                self.overflow_dot,
            ] {
                w.u16(r);
            }
            for b in [
                self.nt_latch,
                self.at_latch,
                self.pt_lo,
                self.pt_hi,
                self.sec_count,
            ] {
                w.u8(b);
            }
            w.bool(self.sec_has0);
            w.bytes(&self.secondary);
            w.bytes(&self.sp_lo);
            w.bytes(&self.sp_hi);
            w.bytes(&self.line_sp);
            let sp0: Vec<u8> = self.line_sp0.iter().map(|&b| u8::from(b)).collect();
            w.bytes(&sp0);
        }
        if w.version >= 4 {
            for b in [
                self.eval_n,
                self.eval_m,
                self.eval_latch,
                self.eval_base,
                self.eval_phase,
            ] {
                w.u8(b);
            }
            w.u32(self.corrupt_rows);
        }
    }

    pub(crate) fn load_from(
        &mut self,
        r: &mut crate::nes_state::StateReader,
    ) -> Result<(), crate::nes_state::NesStateError> {
        use crate::nes_state::NesStateError;
        self.ctrl = r.u8("PPUCTRL")?;
        self.mask = r.u8("PPUMASK")?;
        self.status = r.u8("PPUSTATUS")?;
        self.oam_addr = r.u8("OAMADDR")?;
        r.fill(&mut self.oam, "OAM")?;
        r.fill(&mut self.vram[..], "VRAM")?;
        r.fill(&mut self.palette, "palette")?;
        self.v = r.u16("v")?;
        self.t = r.u16("t")?;
        if self.v > 0x7FFF || self.t > 0x7FFF {
            return Err(NesStateError::BadValue {
                what: "loopy register",
                value: u64::from(self.v.max(self.t)),
            });
        }
        self.fine_x = r.small(7, "fine x")?;
        self.w = r.bool("write toggle")?;
        self.buffer = r.u8("read buffer")?;
        self.latch = r.u8("open bus")?;
        self.dot = r.u16("dot")?;
        self.line = r.u16("scanline")?;
        if self.dot > 340 || self.line >= self.region.lines() {
            return Err(NesStateError::BadValue {
                what: "PPU position",
                value: (u64::from(self.line) << 16) | u64::from(self.dot),
            });
        }
        self.frame = r.u64("frame")?;
        self.nmi_out = r.bool("NMI output")?;
        self.nmi_edge = r.bool("NMI edge")?;
        self.suppress = r.bool("vblank suppress")?;
        let overflow = r.bool("overflow pending")?;
        self.overflow_dot = 0;
        if r.version >= 2 {
            r.fill(&mut self.framebuffer, "framebuffer")?;
            r.fill(&mut self.emphasis, "emphasis")?;
            self.bg_lo = r.u16("pattern shift low")?;
            self.bg_hi = r.u16("pattern shift high")?;
            self.at_lo = r.u16("attribute shift low")?;
            self.at_hi = r.u16("attribute shift high")?;
            self.overflow_dot = r.u16("overflow dot")?;
            if self.overflow_dot > 256 || (self.overflow_dot != 0) != overflow {
                return Err(NesStateError::BadValue {
                    what: "overflow dot",
                    value: u64::from(self.overflow_dot),
                });
            }
            self.nt_latch = r.u8("nametable latch")?;
            self.at_latch = r.small(3, "attribute latch")?;
            self.pt_lo = r.u8("pattern latch low")?;
            self.pt_hi = r.u8("pattern latch high")?;
            self.sec_count = r.small(8, "secondary OAM count")?;
            self.sec_has0 = r.bool("sprite 0 in range")?;
            r.fill(&mut self.secondary, "secondary OAM")?;
            r.fill(&mut self.sp_lo, "sprite pattern low")?;
            r.fill(&mut self.sp_hi, "sprite pattern high")?;
            r.fill(&mut self.line_sp, "sprite line")?;
            let mut sp0 = [0u8; WIDTH];
            r.fill(&mut sp0, "sprite 0 line")?;
            for (d, &s) in self.line_sp0.iter_mut().zip(&sp0) {
                *d = match s {
                    0 => false,
                    1 => true,
                    v => {
                        return Err(NesStateError::BadValue {
                            what: "sprite 0 line",
                            value: u64::from(v),
                        })
                    }
                };
            }
        } else {
            self.framebuffer.fill(0);
            self.emphasis.fill(0);
            let _ = overflow;
            self.bg_lo = 0;
            self.bg_hi = 0;
            self.at_lo = 0;
            self.at_hi = 0;
            self.nt_latch = 0;
            self.at_latch = 0;
            self.pt_lo = 0;
            self.pt_hi = 0;
            self.sec_count = 0;
            self.sec_has0 = false;
            self.secondary = [0xFF; 32];
            self.line_sp.fill(0);
            self.line_sp0.fill(false);
        }
        if r.version >= 4 {
            self.eval_n = r.small(64, "evaluation n")?;
            self.eval_m = r.small(3, "evaluation m")?;
            self.eval_latch = r.u8("OAM bus")?;
            self.eval_base = r.u8("evaluation base")?;
            self.eval_phase = r.small(EVAL_DONE, "evaluation phase")?;
            self.corrupt_rows = r.u32("OAM corruption rows")?;
        } else {
            // Older states evaluated each line whole at dot 65: nothing in flight.
            self.eval_n = 64;
            self.eval_m = 0;
            self.eval_latch = 0xFF;
            self.eval_base = 0;
            self.eval_phase = EVAL_DONE;
            self.corrupt_rows = 0;
        }
        Ok(())
    }
}
