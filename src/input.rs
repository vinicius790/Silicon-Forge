//! NES input devices on the controller ports `$4016` and `$4017`.
//!
//! * Writing `$4016` drives OUT0 (bit 0), the strobe of both ports. While it is
//!   high the pads' shift registers reload continuously; when it falls they hold.
//! * Reading `$4016` (port 1) or `$4017` (port 2) clocks that port once and returns
//!   its serial bit in D0. Only D0–D4 are driven by the ports; the rest is CPU open
//!   bus, which after `LDA $4016`/`$4017` holds the high address byte `$40`
//!   ([`OPEN_BUS_4016`]).
//! * [`Controller`]: the standard pad, a 4021 8-bit parallel-in shift register.
//! * [`Zapper`]: the light gun, which reports on D3 (light sense, 0 = light seen)
//!   and D4 (trigger, 1 = pulled) and drives D0 low.
//! * [`FourScore`]: four pads multiplexed on D0 with a signature.
//! * [`ArkanoidPaddle`]: knob (D4, serial, inverted) and fire (D3).
//! * [`PowerPad`]: twelve sensors on D3/D4.

use crate::ppu::{Ppu, DOTS, HEIGHT, WIDTH};

/// Upper bits of a controller read: open bus, which on the NES holds the high byte
/// of the address (`$40`). Only the low bits come from the device.
pub const OPEN_BUS_4016: u8 = 0x40;

/// Button bits in report order (bit 0 is read first).
pub mod button {
    pub const A: u8 = 0x01;
    pub const B: u8 = 0x02;
    pub const SELECT: u8 = 0x04;
    pub const START: u8 = 0x08;
    pub const UP: u8 = 0x10;
    pub const DOWN: u8 = 0x20;
    pub const LEFT: u8 = 0x40;
    pub const RIGHT: u8 = 0x80;
    /// All eight, in report order.
    pub const ALL: [u8; 8] = [A, B, SELECT, START, UP, DOWN, LEFT, RIGHT];
}

/// Standard NES controller: a 4021 8-bit parallel-in shift register. While the
/// strobe is high it keeps reloading, so every read returns A. After the strobe
/// falls each read shifts out the next button (A, B, Select, Start, Up, Down, Left,
/// Right; 1 = pressed); after all eight, reads return 1 (the serial input is tied
/// high on official pads).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Controller {
    /// Buttons currently held, see [`button`].
    pub buttons: u8,
    pub shift: u8,
}

impl Controller {
    pub fn with_buttons(buttons: u8) -> Self {
        Self { buttons, shift: 0 }
    }

    /// Parallel load (strobe high).
    pub fn latch(&mut self) {
        self.shift = self.buttons;
    }

    /// The bit the next read would return, without clocking.
    pub fn peek(&self, strobe: bool) -> u8 {
        if strobe {
            self.buttons & 1
        } else {
            self.shift & 1
        }
    }

    /// One port read: returns D0 and clocks the register (only with strobe low).
    pub fn read(&mut self, strobe: bool) -> u8 {
        if strobe {
            self.latch();
        }
        let bit = self.shift & 1;
        if !strobe {
            self.shift = (self.shift >> 1) | 0x80;
        }
        bit
    }
}

/// Lines the Zapper's photodiode keeps reporting light after the beam drew a bright
/// pixel under it (documented as roughly 19–26 scanlines; the upper bound is used).
pub const ZAPPER_HOLD_LINES: u32 = 26;

/// NES Zapper on port 2. It has no shift register: reads return D3 = 0 while the
/// sensor sees light and D4 = 1 while the trigger is pulled; D0 reads 0. The
/// sensor sees light when the pixel it is aimed at was drawn bright this frame and
/// the beam passed it less than [`ZAPPER_HOLD_LINES`] lines ago. "Bright" is a
/// palette entry in the two lightest rows (`$2x`, `$3x`) with a hue of `$0`–`$C`,
/// which is what games flash (white `$20`/`$30` boxes); darker rows and `$xD`–`$xF`
/// do not trigger the sensor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Zapper {
    pub trigger: bool,
    /// Aim point in framebuffer coordinates; `None` points off screen.
    pub aim: Option<(u16, u16)>,
}

impl Zapper {
    /// Palette index bright enough for the photodiode.
    pub fn bright(index: u8) -> bool {
        index & 0x30 >= 0x20 && index & 0x0F <= 0x0C
    }

    /// True while the sensor reports light at the PPU's current beam position.
    pub fn light(&self, ppu: &Ppu) -> bool {
        let Some((x, y)) = self.aim else {
            return false;
        };
        let (x, y) = (usize::from(x), usize::from(y));
        if x >= WIDTH || y >= HEIGHT {
            return false;
        }
        // Dot at which pixel (x, y) is drawn, and the beam now, in dots since line 0.
        let drawn = y as u32 * u32::from(DOTS) + x as u32 + 1;
        let now = u32::from(ppu.line) * u32::from(DOTS) + u32::from(ppu.dot);
        if now <= drawn || now - drawn > ZAPPER_HOLD_LINES * u32::from(DOTS) {
            return false;
        }
        Self::bright(ppu.framebuffer[y * WIDTH + x])
    }

    /// D4–D0 of a `$4017` read.
    pub fn report(&self, ppu: &Ppu) -> u8 {
        let light = if self.light(ppu) { 0 } else { 0x08 };
        let trigger = if self.trigger { 0x10 } else { 0 };
        light | trigger
    }
}

/// Four Score signature bits, reads 16–23 of each port: `$4016` reads 1 on its
/// 20th read (index 19), `$4017` on its 19th (index 18).
pub const FOUR_SCORE_SIGNATURE: [u8; 2] = [0x08, 0x04];

/// NES Four Score in 4-player mode. Each port reports 24 bits on D0: its own pad
/// (1 on `$4016`, 2 on `$4017`), then the extra pad (3 or 4), then the signature
/// ([`FOUR_SCORE_SIGNATURE`]); every read after the 24th returns 1. The strobe
/// reloads all four pads and restarts both sequences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FourScore {
    /// Controllers 3 and 4.
    pub extra: [Controller; 2],
    /// Reads taken on each port since the strobe (saturates at 24).
    pub count: [u8; 2],
}

impl FourScore {
    pub fn latch(&mut self) {
        for p in &mut self.extra {
            p.latch();
        }
        self.count = [0; 2];
    }

    fn bit(&self, port: usize, pads: &[Controller; 2], strobe: bool) -> u8 {
        if strobe {
            return pads[port].buttons & 1;
        }
        match self.count[port] {
            0..=7 => pads[port].shift & 1,
            8..=15 => self.extra[port].shift & 1,
            n @ 16..=23 => (FOUR_SCORE_SIGNATURE[port] >> (n - 16)) & 1,
            _ => 1,
        }
    }

    pub fn peek(&self, port: usize, pads: &[Controller; 2], strobe: bool) -> u8 {
        self.bit(port, pads, strobe)
    }

    /// One read of `port` (0 = `$4016`): D0, then clock that port's sequence.
    pub fn read(&mut self, port: usize, pads: &mut [Controller; 2], strobe: bool) -> u8 {
        if strobe {
            pads[port].latch();
            self.latch();
            return pads[port].buttons & 1;
        }
        let bit = self.bit(port, pads, false);
        match self.count[port] {
            0..=7 => {
                pads[port].read(false);
            }
            8..=15 => {
                self.extra[port].read(false);
            }
            _ => {}
        }
        self.count[port] = (self.count[port] + 1).min(24);
        bit
    }
}

/// NES Arkanoid (Vaus) controller in port 2. D3 is the fire button (1 = pressed),
/// straight from the switch. D4 is the knob position, 9 bits MSB first, each bit
/// inverted on the wire. The conversion starts on the strobe's rising edge; the
/// model completes it immediately (the real one takes up to ~7 ms, proportional
/// to the value), latching `position` when the strobe falls. Past the ninth bit the
/// ninth repeats (it is the shift register's serial input).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArkanoidPaddle {
    /// Raw (non-inverted) 9-bit knob reading.
    pub position: u16,
    pub fire: bool,
    /// Latched bits, MSB in bit 8.
    pub shift: u16,
    /// Bits shifted out since the latch (saturates at 9).
    pub shifted: u8,
}

impl ArkanoidPaddle {
    pub fn latch(&mut self) {
        self.shift = self.position & 0x1FF;
        self.shifted = 0;
    }

    pub fn peek(&self) -> u8 {
        let bit = ((self.shift >> 8) & 1) as u8;
        ((bit ^ 1) << 4) | (u8::from(self.fire) << 3)
    }

    pub fn read(&mut self, strobe: bool) -> u8 {
        let v = self.peek();
        if !strobe {
            if self.shifted < 8 {
                self.shift = (self.shift << 1) & 0x1FF;
                self.shifted += 1;
            } else {
                // Ninth bit reached: it repeats from here on.
                self.shift = if self.position & 1 != 0 { 0x100 } else { 0 };
                self.shifted = 9;
            }
        }
        v
    }
}

/// Power Pad buttons in shift order: D3 carries 2, 1, 5, 9, 6, 10, 11, 7; D4
/// carries 4, 3, 12, 8 and then 1s. After eight reads both lines read 1.
pub const POWER_PAD_D3: [u8; 8] = [2, 1, 5, 9, 6, 10, 11, 7];
pub const POWER_PAD_D4: [u8; 4] = [4, 3, 12, 8];

/// NES Power Pad (port 2): twelve sensors, bit `n - 1` of `buttons` for sensor
/// `n` (1 = stepped on), read two at a time on D3/D4 after a strobe.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PowerPad {
    pub buttons: u16,
    pub low: u8,
    pub high: u8,
}

impl PowerPad {
    pub fn latch(&mut self) {
        let b = |n: u8| u8::from((self.buttons >> (n - 1)) & 1 != 0);
        self.low = POWER_PAD_D3
            .iter()
            .enumerate()
            .fold(0, |a, (i, &n)| a | (b(n) << i));
        self.high = POWER_PAD_D4
            .iter()
            .enumerate()
            .fold(0xF0, |a, (i, &n)| a | (b(n) << i));
    }

    pub fn peek(&self) -> u8 {
        ((self.low & 1) << 3) | ((self.high & 1) << 4)
    }

    pub fn read(&mut self, strobe: bool) -> u8 {
        if strobe {
            self.latch();
        }
        let v = self.peek();
        if !strobe {
            self.low = (self.low >> 1) | 0x80;
            self.high = (self.high >> 1) | 0x80;
        }
        v
    }
}
