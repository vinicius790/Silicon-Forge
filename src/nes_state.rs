//! Whole-machine save states for [`Nes2A03`]: CPU (including an in-progress bus
//! cycle sequence), 2 KiB RAM, PPU (registers, VRAM, OAM, palette, loopy registers,
//! scanline buffers, framebuffer), APU (every channel, the frame sequencer and its
//! pending restart), the cartridge mapper and its PRG RAM, CHR RAM, and the
//! machine's DMA bookkeeping. A state taken at any CPU cycle resumes identically.
//!
//! Version 7 adds MMC6 RAM / TQROM CHR RAM, MMC5 audio and split, VRC6 audio
//! fields unchanged, and the extra input devices (version 6 loads them reset).
//! Version 6 adds the Sunsoft 5B and Namco 163 audio state after their mapper
//! registers (version 5 loads them silent and reset).
//!
//! Binary layout: magic `NESS`, a little-endian `u16` version, then the fields in a
//! fixed order. ROM is not stored: PRG and CHR ROM of a cartridge are fingerprinted (FNV-1a 64) and
//! must match the machine the state is loaded into. Version 5 is current: it adds the region
//! (NTSC/PAL/Dendy, which must match), the CPU/PPU divider phase and the DMC halt
//! counters for repeated PPU reads; version 4 loads as NTSC. Version 4 adds the PPU's
//! in-flight sprite evaluation, pending OAM row corruption and the DMC halt flag of a
//! stalled controller read (version 3 loads them idle). Version 3 adds the controller strobe and both pads (buttons
//! and shift register); version 2 loads them released and empty. Version 1
//! (no framebuffer, emphasis or per-line render buffers) still loads; those buffers
//! come back zero and the next rendered frame rewrites them. A machine with no
//! cartridge stores its writable `$8000`-`$FFFF` image instead of a fingerprint. Errors are
//! [`NesStateError`]; a failed load restores the machine's previous state exactly.

use crate::flags::Status;
use crate::ppu::Mirroring;
use crate::systems::nes2a03::Nes2A03;

pub const MAGIC: [u8; 4] = *b"NESS";
pub const NES_STATE_VERSION: u16 = 7;
const DMC_STALL_MAX: u8 = crate::apu::DMC_STALL as u8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NesStateError {
    BadMagic,
    UnsupportedVersion(u16),
    /// The data ended while reading `what`.
    Truncated {
        what: &'static str,
    },
    /// A stored byte that is not a valid value for `what`.
    BadValue {
        what: &'static str,
        value: u64,
    },
    /// A length-prefixed block of the wrong size.
    Length {
        what: &'static str,
        expected: usize,
        got: usize,
    },
    /// The ROM (or mapper) of the target machine is not the one saved.
    RomMismatch(&'static str),
    /// The CPU rejected its bus-cycle state.
    Cpu(String),
    /// Bytes left after the last field.
    Trailing(usize),
}

impl std::fmt::Display for NesStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadMagic => f.write_str("not a NES save state"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NES state version {v}"),
            Self::Truncated { what } => write!(f, "state ends inside {what}"),
            Self::BadValue { what, value } => write!(f, "invalid {what}: {value}"),
            Self::Length {
                what,
                expected,
                got,
            } => {
                write!(f, "{what}: expected {expected} bytes, got {got}")
            }
            Self::RomMismatch(what) => write!(f, "{what} differs from the saved machine"),
            Self::Cpu(m) => write!(f, "CPU state: {m}"),
            Self::Trailing(n) => write!(f, "{n} bytes after the state"),
        }
    }
}

impl std::error::Error for NesStateError {}

pub fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[derive(Default)]
pub struct StateWriter {
    pub buf: Vec<u8>,
    pub version: u16,
}

impl StateWriter {
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }
    pub fn u16(&mut self, v: u16) {
        self.buf.extend(v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend(v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.buf.extend(v.to_le_bytes());
    }
    /// Length (`u32`) then the bytes.
    pub fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.buf.extend_from_slice(v);
    }
    pub fn opt_u8(&mut self, v: Option<u8>) {
        self.bool(v.is_some());
        self.u8(v.unwrap_or(0));
    }
}

pub struct StateReader<'a> {
    data: &'a [u8],
    pos: usize,
    pub version: u16,
}

impl<'a> StateReader<'a> {
    pub fn new(data: &'a [u8], version: u16) -> Self {
        Self {
            data,
            pos: 0,
            version,
        }
    }
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8], NesStateError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or(NesStateError::Truncated { what })?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    pub fn u8(&mut self, what: &'static str) -> Result<u8, NesStateError> {
        Ok(self.take(1, what)?[0])
    }
    pub fn bool(&mut self, what: &'static str) -> Result<bool, NesStateError> {
        match self.u8(what)? {
            0 => Ok(false),
            1 => Ok(true),
            v => Err(NesStateError::BadValue {
                what,
                value: u64::from(v),
            }),
        }
    }
    pub fn u16(&mut self, what: &'static str) -> Result<u16, NesStateError> {
        Ok(u16::from_le_bytes(
            self.take(2, what)?.try_into().expect("2 bytes"),
        ))
    }
    pub fn u32(&mut self, what: &'static str) -> Result<u32, NesStateError> {
        Ok(u32::from_le_bytes(
            self.take(4, what)?.try_into().expect("4 bytes"),
        ))
    }
    pub fn u64(&mut self, what: &'static str) -> Result<u64, NesStateError> {
        Ok(u64::from_le_bytes(
            self.take(8, what)?.try_into().expect("8 bytes"),
        ))
    }
    pub fn bytes(&mut self, what: &'static str) -> Result<&'a [u8], NesStateError> {
        let n = self.u32(what)? as usize;
        self.take(n, what)
    }
    /// A length-prefixed block that must be exactly `out.len()` bytes.
    pub fn fill(&mut self, out: &mut [u8], what: &'static str) -> Result<(), NesStateError> {
        let b = self.bytes(what)?;
        if b.len() != out.len() {
            return Err(NesStateError::Length {
                what,
                expected: out.len(),
                got: b.len(),
            });
        }
        out.copy_from_slice(b);
        Ok(())
    }
    pub fn opt_u8(&mut self, what: &'static str) -> Result<Option<u8>, NesStateError> {
        let some = self.bool(what)?;
        let v = self.u8(what)?;
        Ok(some.then_some(v))
    }
    /// A `u8` that must be `<= max`.
    pub fn small(&mut self, max: u8, what: &'static str) -> Result<u8, NesStateError> {
        let v = self.u8(what)?;
        if v > max {
            return Err(NesStateError::BadValue {
                what,
                value: u64::from(v),
            });
        }
        Ok(v)
    }
    pub fn fingerprint(&mut self, rom: &[u8], what: &'static str) -> Result<(), NesStateError> {
        if self.u64(what)? != fnv64(rom) {
            return Err(NesStateError::RomMismatch(what));
        }
        Ok(())
    }
}

fn mirroring_code(m: Mirroring) -> u8 {
    match m {
        Mirroring::Horizontal => 0,
        Mirroring::Vertical => 1,
        Mirroring::FourScreen => 2,
        Mirroring::SingleLow => 3,
        Mirroring::SingleHigh => 4,
    }
}

fn mirroring_from(v: u8) -> Result<Mirroring, NesStateError> {
    Ok(match v {
        0 => Mirroring::Horizontal,
        1 => Mirroring::Vertical,
        2 => Mirroring::FourScreen,
        3 => Mirroring::SingleLow,
        4 => Mirroring::SingleHigh,
        _ => {
            return Err(NesStateError::BadValue {
                what: "mirroring",
                value: u64::from(v),
            })
        }
    })
}

impl Nes2A03 {
    /// Save in the current format.
    pub fn save_state(&self) -> Vec<u8> {
        self.save_state_version(NES_STATE_VERSION)
    }

    /// Save in `version` (1 or 2); older versions drop what they did not store.
    pub fn save_state_version(&self, version: u16) -> Vec<u8> {
        assert!(
            (1..=NES_STATE_VERSION).contains(&version),
            "version {version}"
        );
        let mut w = StateWriter {
            buf: MAGIC.to_vec(),
            version,
        };
        w.u16(version);
        let c = &self.cpu;
        for v in [c.a, c.x, c.y, c.sp, c.p.raw()] {
            w.u8(v);
        }
        w.u16(c.pc);
        w.u64(c.cycles);
        w.u8(c.variant as u8);
        w.bool(c.halt_on_brk);
        w.bool(c.is_halted());
        w.bool(c.is_waiting());
        w.bytes(&c.cycle_state());
        w.u64(self.stalled);
        w.u32(self.dma_window);
        w.u64(self.oam_stall);
        let b = &c.bus;
        w.bytes(&b.ram);
        w.bool(b.cart.is_some());
        if b.cart.is_some() {
            w.u64(fnv64(&b.prg[..]));
        } else {
            // Without a cartridge `$8000`-`$FFFF` is writable memory.
            w.bytes(&b.prg[..]);
        }
        w.bool(b.chr_writable);
        if b.chr_writable {
            w.bytes(&b.chr);
        } else {
            w.u64(fnv64(&b.chr));
        }
        w.u8(mirroring_code(b.mirroring));
        w.opt_u8(b.dma_page);
        w.u64(b.oam_dma_cycles);
        if version >= 3 {
            w.bool(b.strobe);
            for pad in &b.pads {
                w.u8(pad.buttons);
                w.u8(pad.shift);
            }
        }
        if version >= 4 {
            w.bool(b.dmc_halted_read);
        }
        if version >= 5 {
            w.u8(b.ppu.region.code());
            w.u32(self.dot_phase);
            w.u8(b.dmc_stall_reads);
            w.u8(b.dmc_repeats);
        }
        if version >= 7 {
            w.bool(b.four_score.is_some());
            if let Some(f) = &b.four_score {
                for p in &f.extra {
                    w.u8(p.buttons);
                    w.u8(p.shift);
                }
                w.u8(f.count[0]);
                w.u8(f.count[1]);
            }
            w.bool(b.paddle.is_some());
            if let Some(p) = &b.paddle {
                w.u16(p.position);
                w.bool(p.fire);
                w.u16(p.shift);
                w.u8(p.shifted);
            }
            w.bool(b.power_pad.is_some());
            if let Some(p) = &b.power_pad {
                w.u16(p.buttons);
                w.u8(p.low);
                w.u8(p.high);
            }
        }
        b.ppu.save_into(&mut w);
        b.apu.put(&mut w);
        if let Some(cart) = &b.cart {
            w.u8(cart.state_kind());
            cart.save_state(&mut w);
        }
        w.buf
    }

    /// Restore a state saved from a machine with the same ROM and mapper. All or
    /// nothing: on any error the machine is exactly as it was before the call.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), NesStateError> {
        let backup = self.save_state();
        self.load_state_unchecked(data).inspect_err(|_| {
            self.load_state_unchecked(&backup)
                .expect("a state this machine just saved always reloads");
        })
    }

    fn load_state_unchecked(&mut self, data: &[u8]) -> Result<(), NesStateError> {
        if data.len() < 6 || data[..4] != MAGIC {
            return Err(NesStateError::BadMagic);
        }
        let version = u16::from_le_bytes([data[4], data[5]]);
        if !(1..=NES_STATE_VERSION).contains(&version) {
            return Err(NesStateError::UnsupportedVersion(version));
        }
        let mut r = StateReader::new(&data[6..], version);
        let (a, x, y, sp, p) = (r.u8("A")?, r.u8("X")?, r.u8("Y")?, r.u8("SP")?, r.u8("P")?);
        let pc = r.u16("PC")?;
        let cycles = r.u64("cycles")?;
        let variant = r.u8("variant")?;
        if variant != self.cpu.variant as u8 {
            return Err(NesStateError::BadValue {
                what: "variant",
                value: u64::from(variant),
            });
        }
        let halt_on_brk = r.bool("halt_on_brk")?;
        let halted = r.bool("halted")?;
        let waiting = r.bool("waiting")?;
        let cycle_state = r.bytes("cycle state")?;
        self.cpu
            .load_cycle_state(cycle_state)
            .map_err(NesStateError::Cpu)?;
        let cpu = &mut self.cpu;
        (cpu.a, cpu.x, cpu.y, cpu.sp, cpu.p, cpu.pc, cpu.cycles) =
            (a, x, y, sp, Status(p), pc, cycles);
        cpu.halt_on_brk = halt_on_brk;
        cpu.set_halted(halted);
        cpu.set_waiting(waiting);
        self.stalled = r.u64("stalled")?;
        self.dma_window = r.u32("dma window")?;
        self.oam_stall = r.u64("OAM stall")?;
        let b = &mut self.cpu.bus;
        r.fill(&mut b.ram, "RAM")?;
        let has_cart = r.bool("cartridge")?;
        if has_cart != b.cart.is_some() {
            return Err(NesStateError::RomMismatch("cartridge"));
        }
        if has_cart {
            r.fingerprint(&b.prg[..], "PRG")?;
        } else {
            r.fill(&mut b.prg[..], "PRG RAM")?;
        }
        let writable = r.bool("CHR writable")?;
        if writable != b.chr_writable {
            return Err(NesStateError::RomMismatch("CHR kind"));
        }
        if writable {
            r.fill(&mut b.chr, "CHR RAM")?;
        } else {
            r.fingerprint(&b.chr, "CHR ROM")?;
        }
        b.mirroring = mirroring_from(r.u8("mirroring")?)?;
        b.dma_page = r.opt_u8("DMA page")?;
        b.oam_dma_cycles = r.u64("OAM DMA cycles")?;
        if version >= 3 {
            b.strobe = r.bool("controller strobe")?;
            for pad in &mut b.pads {
                pad.buttons = r.u8("buttons")?;
                pad.shift = r.u8("controller shift")?;
            }
        } else {
            b.strobe = false;
            b.pads = Default::default();
        }
        b.dmc_halted_read = version >= 4 && r.bool("DMC halted read")?;
        let region = if version >= 5 {
            let code = r.u8("region")?;
            crate::ppu::Region::from_code(code).ok_or(NesStateError::BadValue {
                what: "region",
                value: u64::from(code),
            })?
        } else {
            crate::ppu::Region::Ntsc
        };
        if region != b.ppu.region {
            return Err(NesStateError::RomMismatch("region"));
        }
        if version >= 5 {
            let phase = r.u32("dot phase")?;
            if phase >= region.dots_per_cpu().1 {
                return Err(NesStateError::BadValue {
                    what: "dot phase",
                    value: u64::from(phase),
                });
            }
            self.dot_phase = phase;
            b.dmc_stall_reads = r.small(DMC_STALL_MAX, "DMC stall reads")?;
            b.dmc_repeats = r.small(DMC_STALL_MAX, "DMC repeats")?;
        } else {
            self.dot_phase = 0;
            b.dmc_stall_reads = 0;
            b.dmc_repeats = 0;
        }
        // Devices are configuration: a snapshot must name the same ones.
        let device = |r: &mut StateReader,
                      present: bool,
                      what: &'static str|
         -> Result<bool, NesStateError> {
            let saved = r.bool(what)?;
            if saved != present {
                return Err(NesStateError::RomMismatch(what));
            }
            Ok(saved)
        };
        if version >= 7 {
            if device(&mut r, b.four_score.is_some(), "Four Score")? {
                let f = b.four_score.as_mut().expect("present");
                for p in &mut f.extra {
                    p.buttons = r.u8("buttons")?;
                    p.shift = r.u8("controller shift")?;
                }
                f.count = [
                    r.small(24, "Four Score count")?,
                    r.small(24, "Four Score count")?,
                ];
            }
            if device(&mut r, b.paddle.is_some(), "Arkanoid controller")? {
                let p = b.paddle.as_mut().expect("present");
                p.position = r.u16("paddle position")?;
                p.fire = r.bool("paddle fire")?;
                p.shift = r.u16("paddle shift")?;
                p.shifted = r.small(9, "paddle bits")?;
                if p.position > 0x1FF || p.shift > 0x1FF {
                    return Err(NesStateError::BadValue {
                        what: "paddle",
                        value: u64::from(p.position.max(p.shift)),
                    });
                }
            }
            if device(&mut r, b.power_pad.is_some(), "Power Pad")? {
                let p = b.power_pad.as_mut().expect("present");
                p.buttons = r.u16("Power Pad buttons")?;
                p.low = r.u8("Power Pad D3")?;
                p.high = r.u8("Power Pad D4")?;
                if p.buttons > 0xFFF {
                    return Err(NesStateError::BadValue {
                        what: "Power Pad buttons",
                        value: u64::from(p.buttons),
                    });
                }
            }
        } else {
            if let Some(f) = b.four_score.as_mut() {
                *f = crate::input::FourScore {
                    extra: [Default::default(); 2],
                    count: [24; 2],
                };
            }
            if let Some(p) = b.paddle.as_mut() {
                p.shift = 0;
                p.shifted = 0;
            }
            if let Some(p) = b.power_pad.as_mut() {
                p.low = 0xFF;
                p.high = 0xFF;
            }
        }
        b.ppu.load_from(&mut r)?;
        b.apu.get(&mut r)?;
        if let Some(cart) = b.cart.as_mut() {
            if r.u8("mapper")? != cart.state_kind() {
                return Err(NesStateError::RomMismatch("mapper"));
            }
            cart.load_state(&mut r)?;
        }
        if r.pos != r.data.len() {
            return Err(NesStateError::Trailing(r.data.len() - r.pos));
        }
        Ok(())
    }
}
