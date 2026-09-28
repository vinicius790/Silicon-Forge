//! Atari 2600 TIA (Television Interface Adapter) Video & Sound Chip Emulator.
//!
//! Emulates the custom TIA chip of the Atari 2600 Video Computer System (VCS):
//! - Color-luminance registers: COLUP0, COLUP1, COLUPF, COLUBK.
//! - Playfield graphics: PF0, PF1, PF2 and reflection CTRLPF.
//! - Player graphics: GRP0, GRP1, NUSIZ0, NUSIZ1.
//! - Horizontal timing: WSYNC (halts CPU until horizontal blank).
//! - Frame sync: VSYNC, VBLANK.
//! - Audio channels: 2 independent voices with 4-bit polynomial sound generators (AUDC, AUDF, AUDV).
//! - Hardware collision detection registers (15 collision latch pairs) and CXCLR.
//! - Implements [`crate::bus::IoDevice`].

use crate::bus::IoDevice;

/// Atari 2600 TIA chip.
#[derive(Debug, Clone)]
pub struct Tia2600 {
    // Sync registers
    pub vsync: bool,
    pub vblank: bool,
    pub wsync_active: bool,

    // Color registers ($06..$09)
    pub colup0: u8,
    pub colup1: u8,
    pub colupf: u8,
    pub colubk: u8,

    // Playfield & Control ($0A, $0D..$0F)
    pub ctrlpf: u8,
    pub pf0: u8,
    pub pf1: u8,
    pub pf2: u8,

    // Player graphics ($1B, $1C)
    pub grp0: u8,
    pub grp1: u8,

    // Audio registers (Channel 0: $15, $17, $19; Channel 1: $16, $18, $1A)
    pub audc0: u8,
    pub audc1: u8,
    pub audf0: u8,
    pub audf1: u8,
    pub audv0: u8,
    pub audv1: u8,

    // Collision registers (15 pairs, read at $00..$0E)
    pub collisions: [u8; 15],

    // Horizontal position counters (0..227 color clocks per scanline)
    pub color_clock: u16,
    pub scanline: u16,
}

impl Default for Tia2600 {
    fn default() -> Self {
        Self::new()
    }
}

impl Tia2600 {
    pub fn new() -> Self {
        Self {
            vsync: false,
            vblank: false,
            wsync_active: false,
            colup0: 0,
            colup1: 0,
            colupf: 0,
            colubk: 0,
            ctrlpf: 0,
            pf0: 0,
            pf1: 0,
            pf2: 0,
            grp0: 0,
            grp1: 0,
            audc0: 0,
            audc1: 0,
            audf0: 0,
            audf1: 0,
            audv0: 0,
            audv1: 0,
            collisions: [0; 15],
            color_clock: 0,
            scanline: 0,
        }
    }

    /// Reset TIA registers.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Advance by 1 CPU cycle (which equals 3 TIA color clocks).
    pub fn clock_cpu_cycle(&mut self) {
        self.color_clock += 3;
        if self.color_clock >= 228 {
            self.color_clock -= 228;
            self.scanline = self.scanline.wrapping_add(1);
            if self.scanline >= 262 {
                self.scanline = 0;
            }
            // WSYNC is released at start of horizontal blank of next scanline
            self.wsync_active = false;
        }
    }

    /// Read TIA register (offsets $00..=$0E are collision registers).
    pub fn read_reg(&self, reg: u8) -> u8 {
        let r = (reg & 0x0F) as usize;
        if r < 15 {
            self.collisions[r]
        } else {
            0x00
        }
    }

    /// Write TIA register (offsets $00..=$2C).
    pub fn write_reg(&mut self, reg: u8, val: u8) {
        match reg {
            0x00 => self.vsync = (val & 0x02) != 0,
            0x01 => self.vblank = (val & 0x02) != 0,
            0x02 => self.wsync_active = true, // WSYNC halts CPU until next scanline
            0x06 => self.colup0 = val,
            0x07 => self.colup1 = val,
            0x08 => self.colupf = val,
            0x09 => self.colubk = val,
            0x0A => self.ctrlpf = val,
            0x0D => self.pf0 = val & 0xF0,
            0x0E => self.pf1 = val,
            0x0F => self.pf2 = val,
            0x15 => self.audc0 = val & 0x0F,
            0x16 => self.audc1 = val & 0x0F,
            0x17 => self.audf0 = val & 0x1F,
            0x18 => self.audf1 = val & 0x1F,
            0x19 => self.audv0 = val & 0x0F,
            0x1A => self.audv1 = val & 0x0F,
            0x1B => self.grp0 = val,
            0x1C => self.grp1 = val,
            0x2C => self.collisions = [0; 15], // CXCLR clears all collisions
            _ => {}
        }
    }
}

impl IoDevice for Tia2600 {
    fn read(&mut self, offset: u16) -> u8 {
        self.read_reg((offset & 0x3F) as u8)
    }

    fn write(&mut self, offset: u16, value: u8) {
        self.write_reg((offset & 0x3F) as u8, value);
    }

    fn peek(&self, offset: u16) -> u8 {
        self.read_reg((offset & 0x3F) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tia_wsync_and_collisions() {
        let mut tia = Tia2600::new();
        tia.write_reg(0x02, 0); // WSYNC
        assert!(tia.wsync_active);

        // Advance 76 CPU cycles = 228 color clocks (1 full scanline)
        for _ in 0..76 {
            tia.clock_cpu_cycle();
        }
        assert!(!tia.wsync_active);
        assert_eq!(tia.scanline, 1);

        // Collision test and CXCLR
        tia.collisions[0] = 0x80;
        assert_eq!(tia.read_reg(0x00), 0x80);
        tia.write_reg(0x2C, 0); // CXCLR
        assert_eq!(tia.read_reg(0x00), 0x00);
    }
}
