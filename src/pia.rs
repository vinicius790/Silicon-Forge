//! Motorola 6820/6821 PIA, per the data sheet's control register map.
//!
//! Offsets: 0 = port A data or DDR A, 1 = CRA, 2 = port B data or DDR B, 3 = CRB.
//! Control register bits (same for A and B):
//!
//! | bit | meaning |
//! |---|---|
//! | 0 | C1 interrupt enable |
//! | 1 | C1 active edge: 1 rising, 0 falling |
//! | 2 | 1 = data register at offset 0/2, 0 = DDR |
//! | 3–5 | C2 control (below) |
//! | 6 | IRQx2 flag (C2 active edge, only while C2 is an input), read only |
//! | 7 | IRQx1 flag (C1 active edge), read only |
//!
//! C2 with bit 5 = 0 is an input: bit 3 enables its interrupt, bit 4 picks the edge
//! (1 rising). With bit 5 = 1 it is an output: bit 4 = 1 makes C2 follow bit 3; bit 4 = 0
//! is the strobe mode. On side A the strobe is armed by a *read* of port A, on side B
//! by a *write* of port B: C2 goes low, then returns high on the next C1 active edge
//! (bit 3 = 0, handshake) or after one E cycle (bit 3 = 1, pulse; [`Pia6821::tick`]).
//! Reading the data register clears both flags of that side. IRQx is
//! `(flag1 & bit0) | (flag2 & bit3 & !bit5)`. Port A reads the pins (outputs drive
//! them, inputs come from [`Pia6821::set_port_a_in`]); port B outputs read back the
//! output register. Reset (`new`) clears every register: all lines are inputs.

use crate::bus::IoDevice;

/// One side of the PIA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiaPort {
    pub or: u8,
    pub ddr: u8,
    pub cr: u8,
    pub pins: u8,
    c1: bool,
    c2_in: bool,
    /// C2 output level when bit 5 is set.
    pub c2_out: bool,
    pulse: bool,
}

impl PiaPort {
    fn new() -> Self {
        Self {
            or: 0,
            ddr: 0,
            cr: 0,
            pins: 0xFF,
            c1: true,
            c2_in: true,
            c2_out: true,
            pulse: false,
        }
    }

    fn irq(&self) -> bool {
        let c2_irq = self.cr & 0x40 != 0 && self.cr & 0x08 != 0 && self.cr & 0x20 == 0;
        (self.cr & 0x80 != 0 && self.cr & 0x01 != 0) || c2_irq
    }

    fn set_c1(&mut self, high: bool) {
        let active = high != self.c1 && high == (self.cr & 0x02 != 0);
        self.c1 = high;
        if active {
            self.cr |= 0x80;
            if self.cr & 0x38 == 0x20 {
                self.c2_out = true; // handshake completes
            }
        }
    }

    fn set_c2(&mut self, high: bool) {
        let input = self.cr & 0x20 == 0;
        if input && high != self.c2_in && high == (self.cr & 0x10 != 0) {
            self.cr |= 0x40;
        }
        self.c2_in = high;
    }

    fn write_cr(&mut self, value: u8) {
        self.cr = (value & 0x3F) | (self.cr & 0xC0);
        if value & 0x20 != 0 {
            // Output mode: no IRQ2 flag; manual mode drives C2 from bit 3 at once.
            self.cr &= !0x40;
            if value & 0x10 != 0 {
                self.c2_out = value & 0x08 != 0;
            }
        }
    }

    fn strobe(&mut self) {
        if self.cr & 0x30 == 0x20 {
            self.c2_out = false;
            self.pulse = self.cr & 0x08 != 0;
        }
    }

    fn data_access(&self) -> bool {
        self.cr & 0x04 != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pia6821 {
    pub a: PiaPort,
    pub b: PiaPort,
}

impl Default for Pia6821 {
    fn default() -> Self {
        Self::new()
    }
}

impl Pia6821 {
    pub fn new() -> Self {
        Self {
            a: PiaPort::new(),
            b: PiaPort::new(),
        }
    }

    pub fn irq_a(&self) -> bool {
        self.a.irq()
    }

    pub fn irq_b(&self) -> bool {
        self.b.irq()
    }

    pub fn irq_asserted(&self) -> bool {
        self.irq_a() || self.irq_b()
    }

    pub fn set_port_a_in(&mut self, pins: u8) {
        self.a.pins = pins;
    }

    pub fn set_port_b_in(&mut self, pins: u8) {
        self.b.pins = pins;
    }

    pub fn port_a_out(&self) -> u8 {
        self.a.or & self.a.ddr
    }

    pub fn port_b_out(&self) -> u8 {
        self.b.or & self.b.ddr
    }

    pub fn set_ca1(&mut self, high: bool) {
        self.a.set_c1(high);
    }

    pub fn set_cb1(&mut self, high: bool) {
        self.b.set_c1(high);
    }

    pub fn set_ca2(&mut self, high: bool) {
        self.a.set_c2(high);
    }

    pub fn set_cb2(&mut self, high: bool) {
        self.b.set_c2(high);
    }

    /// CA2/CB2 level seen on the pin (input level or driven output).
    pub fn ca2(&self) -> bool {
        if self.a.cr & 0x20 != 0 {
            self.a.c2_out
        } else {
            self.a.c2_in
        }
    }

    pub fn cb2(&self) -> bool {
        if self.b.cr & 0x20 != 0 {
            self.b.c2_out
        } else {
            self.b.c2_in
        }
    }

    /// One E cycle: ends a pending C2 pulse.
    pub fn tick(&mut self) {
        for port in [&mut self.a, &mut self.b] {
            if std::mem::take(&mut port.pulse) {
                port.c2_out = true;
            }
        }
    }

    fn port_a_pins(&self) -> u8 {
        (self.a.or & self.a.ddr) | (self.a.pins & !self.a.ddr)
    }

    fn port_b_value(&self) -> u8 {
        (self.b.or & self.b.ddr) | (self.b.pins & !self.b.ddr)
    }
}

impl IoDevice for Pia6821 {
    fn read(&mut self, offset: u16) -> u8 {
        let value = self.peek(offset);
        match offset & 3 {
            0 if self.a.data_access() => {
                self.a.cr &= 0x3F;
                self.a.strobe();
            }
            2 if self.b.data_access() => self.b.cr &= 0x3F,
            _ => {}
        }
        value
    }

    fn write(&mut self, offset: u16, value: u8) {
        match offset & 3 {
            0 if self.a.data_access() => self.a.or = value,
            0 => self.a.ddr = value,
            1 => self.a.write_cr(value),
            2 if self.b.data_access() => {
                self.b.or = value;
                self.b.strobe();
            }
            2 => self.b.ddr = value,
            _ => self.b.write_cr(value),
        }
    }

    fn peek(&self, offset: u16) -> u8 {
        match offset & 3 {
            0 if self.a.data_access() => self.port_a_pins(),
            0 => self.a.ddr,
            1 => self.a.cr,
            2 if self.b.data_access() => self.port_b_value(),
            2 => self.b.ddr,
            _ => self.b.cr,
        }
    }
}

/// MOS 6532 RIOT (RAM, I/O, timer), per the MCS6532 data sheet.
///
/// Offsets as the chip decodes them, with bit 7 standing for the RS pin:
///
/// | RS | A4 | A2 | A1 A0 | read | write |
/// |---|---|---|---|---|---|
/// | 0 | – | – | – | RAM byte `A6–A0` | RAM byte |
/// | 1 | – | 0 | 00 | port A pins | ORA |
/// | 1 | – | 0 | 01 | DDRA | DDRA |
/// | 1 | – | 0 | 10 | port B | ORB |
/// | 1 | – | 0 | 11 | DDRB | DDRB |
/// | 1 | 1 | 1 | 00–11 | – | timer, interval ÷1 ÷8 ÷64 ÷1024; A3 = timer IRQ enable |
/// | 1 | 0 | 1 | – | – | PA7 edge control: A0 = 1 rising, A1 = PA7 IRQ enable |
/// | 1 | – | 1 | A0 = 0 | timer; A3 = timer IRQ enable | – |
/// | 1 | – | 1 | A0 = 1 | interrupt flags: bit 7 timer, bit 6 PA7 | – |
///
/// The timer decrements one clock after it is written and then once per interval.
/// Passing from `$00` to `$FF` sets the timer flag, after which it decrements on
/// every clock until it is written again. Reading or writing the timer clears the
/// timer flag; reading the flag register clears only the PA7 flag. Port A reads the
/// pins; port B outputs read back the output register. `/IRQ` is
/// `(timer flag & timer enable) | (PA7 flag & PA7 enable)`. Reset clears the I/O
/// registers and the interrupt enables; RAM and the timer are not cleared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Riot6532 {
    pub ram: [u8; 128],
    pub ora: u8,
    pub ddra: u8,
    pub orb: u8,
    pub ddrb: u8,
    /// Levels driven into port A / port B by the outside world.
    pub pa_in: u8,
    pub pb_in: u8,
    timer: u8,
    interval: u16,
    prescale: u16,
    expired: bool,
    timer_flag: bool,
    timer_irq: bool,
    pa7_flag: bool,
    pa7_irq: bool,
    pa7_rising: bool,
    pa7_level: bool,
}

impl Default for Riot6532 {
    fn default() -> Self {
        Self::new()
    }
}

impl Riot6532 {
    pub fn new() -> Self {
        Self {
            ram: [0; 128],
            ora: 0,
            ddra: 0,
            orb: 0,
            ddrb: 0,
            pa_in: 0xFF,
            pb_in: 0xFF,
            timer: 0,
            interval: 1,
            prescale: 1,
            expired: false,
            timer_flag: false,
            timer_irq: false,
            pa7_flag: false,
            pa7_irq: false,
            pa7_rising: false,
            pa7_level: true,
        }
    }

    /// `/RES`: I/O registers and interrupt enables clear; RAM and timer keep.
    pub fn reset(&mut self) {
        self.ora = 0;
        self.ddra = 0;
        self.orb = 0;
        self.ddrb = 0;
        self.timer_irq = false;
        self.pa7_irq = false;
        self.pa7_rising = false;
        self.sample_pa7();
    }

    /// Port A pin levels: outputs from ORA, inputs from [`Riot6532::pa_in`].
    pub fn port_a(&self) -> u8 {
        (self.ora & self.ddra) | (self.pa_in & !self.ddra)
    }

    /// Port B as read: outputs from ORB, inputs from [`Riot6532::pb_in`].
    pub fn port_b(&self) -> u8 {
        (self.orb & self.ddrb) | (self.pb_in & !self.ddrb)
    }

    /// Drive the port A input lines; a PA7 edge may set the PA7 flag.
    pub fn set_port_a_in(&mut self, pins: u8) {
        self.pa_in = pins;
        self.sample_pa7();
    }

    pub fn set_port_b_in(&mut self, pins: u8) {
        self.pb_in = pins;
    }

    fn sample_pa7(&mut self) {
        let level = self.port_a() & 0x80 != 0;
        if level != self.pa7_level && level == self.pa7_rising {
            self.pa7_flag = true;
        }
        self.pa7_level = level;
    }

    pub fn timer(&self) -> u8 {
        self.timer
    }

    /// Clocks per decrement now: the programmed interval, or 1 after a timeout.
    pub fn interval(&self) -> u16 {
        if self.expired {
            1
        } else {
            self.interval
        }
    }

    pub fn timer_flag(&self) -> bool {
        self.timer_flag
    }

    pub fn pa7_flag(&self) -> bool {
        self.pa7_flag
    }

    pub fn irq_asserted(&self) -> bool {
        (self.timer_flag && self.timer_irq) || (self.pa7_flag && self.pa7_irq)
    }

    /// One PHI2 clock.
    pub fn tick(&mut self) {
        self.prescale -= 1;
        if self.prescale != 0 {
            return;
        }
        if self.timer == 0 {
            self.timer_flag = true;
            self.expired = true;
        }
        self.timer = self.timer.wrapping_sub(1);
        self.prescale = self.interval();
    }

    fn flags(&self) -> u8 {
        (u8::from(self.timer_flag) << 7) | (u8::from(self.pa7_flag) << 6)
    }
}

impl IoDevice for Riot6532 {
    fn read(&mut self, offset: u16) -> u8 {
        let value = self.peek(offset);
        if offset & 0x80 != 0 && offset & 0x04 != 0 {
            if offset & 0x01 == 0 {
                self.timer_irq = offset & 0x08 != 0;
                self.timer_flag = false;
            } else {
                self.pa7_flag = false;
            }
        }
        value
    }

    fn write(&mut self, offset: u16, value: u8) {
        if offset & 0x80 == 0 {
            self.ram[usize::from(offset & 0x7F)] = value;
            return;
        }
        if offset & 0x04 == 0 {
            match offset & 3 {
                0 => self.ora = value,
                1 => self.ddra = value,
                2 => self.orb = value,
                _ => self.ddrb = value,
            }
            self.sample_pa7();
        } else if offset & 0x10 != 0 {
            self.timer = value;
            self.interval = [1, 8, 64, 1024][usize::from(offset & 3)];
            self.prescale = 1;
            self.expired = false;
            self.timer_flag = false;
            self.timer_irq = offset & 0x08 != 0;
        } else {
            self.pa7_rising = offset & 0x01 != 0;
            self.pa7_irq = offset & 0x02 != 0;
        }
    }

    fn peek(&self, offset: u16) -> u8 {
        if offset & 0x80 == 0 {
            return self.ram[usize::from(offset & 0x7F)];
        }
        if offset & 0x04 == 0 {
            return match offset & 3 {
                0 => self.port_a(),
                1 => self.ddra,
                2 => self.port_b(),
                _ => self.ddrb,
            };
        }
        if offset & 0x01 == 0 {
            self.timer
        } else {
            self.flags()
        }
    }
}
