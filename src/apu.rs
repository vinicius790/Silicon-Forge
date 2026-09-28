//! Original model of the 2A03 audio unit: two pulse channels, triangle, noise, frame
//! counter, length counters, envelopes, sweeps, registers `$4000`–`$4017`.
//!
//! Timing is in CPU cycles ([`Apu::tick`] is one). Pulse and noise timers count every
//! second CPU cycle, the triangle timer every cycle, so a pulse with timer period `t`
//! repeats every `16·(t+1)` CPU cycles and the triangle every `32·(t+1)`.
//!
//! Frame counter (`$4017`): in 4-step mode quarter frames (envelopes, triangle linear
//! counter) come at CPU cycles 7457, 14913, 22371 and 29829, half frames (length
//! counters, sweeps) at 14913 and 29829, and the frame IRQ flag rises at 29829 unless
//! bit 6 inhibits it; the sequence restarts at 29830. In 5-step mode (bit 7) the quarter
//! frames are 7457, 14913, 22371, 37281, half frames 14913 and 37281, the sequence is
//! 37282 cycles and never raises IRQ; writing `$4017` with bit 7 set also clocks a
//! quarter and a half frame at once. Writing `$4017` sets the mode and (bit 6) clears
//! the flag at once, and restarts the sequence 3 CPU cycles later when written on an
//! even cycle, 4 on an odd one ([`Apu::frame_reset_pending`]); the 5-step immediate
//! quarter/half clock happens at that restart.
//!
//! `$4015` write enables channels; a disabled channel's length counter is forced to 0.
//! `$4015` read returns bit 0–3 = length counter non-zero, bit 6 = frame IRQ, and the
//! read clears the frame IRQ flag. Bit 4 = DMC bytes remaining, bit 7 = DMC IRQ.
//!
//! DMC ([`Dmc`]): `$4010` IRQ enable (bit 7, clearing it clears the flag), loop (bit 6),
//! rate index (bits 3–0, [`DMC_RATES`] CPU cycles per output bit); `$4011` 7-bit level;
//! `$4012` start `$C000 + 64·v`; `$4013` length `16·v + 1`. The APU has no bus: when the
//! sample buffer is empty and bytes remain, [`Apu::dmc_fetch_address`] names the byte
//! and the machine reads it and hands it over with [`Apu::dmc_deliver`] at the end of a
//! [`DMC_STALL`]-cycle window in which the CPU is halted except on its write cycles.
//! The address wraps from `$FFFF` to `$8000`. After the last byte the sample restarts
//! when looping, otherwise the IRQ flag rises if enabled. Each output bit moves the
//! level by ±2 within `0..=127`; an empty buffer at the end of a byte silences the next
//! eight bits. Writing `$4015` clears the DMC IRQ; bit 4 clear stops the sample, set
//! restarts it only when no bytes remain.
//!
//! Mixer: the nonlinear formulas from the hardware documentation,
//! `pulse = 95.88 / (8128 / (p1 + p2) + 100)` and
//! `tnd = 159.79 / (1 / (t/8227 + n/12241 + d/22638) + 100)`, each 0 when its inputs are.
//! The sum lies in `0.0..≈1.0`.

use crate::bus::IoDevice;
use crate::cycles::BusPhase;

/// Length counter load values, indexed by bits 7–3 of the fourth channel register.
pub const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

/// Noise timer periods (NTSC), in CPU cycles, indexed by `$400E` bits 3–0.
pub const NOISE_PERIODS: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

/// Pulse duty sequences (12.5 %, 25 %, 50 %, 25 % inverted), one bit per step.
pub const DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

/// DMC timer periods (NTSC), in CPU cycles per output bit, indexed by `$4010` bits 3–0.
pub const DMC_RATES: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];

/// Length of the DMC DMA window in CPU cycles. The DMA may only halt the CPU on a read
/// cycle, so write cycles that fall inside the window still run: the CPU loses 4 cycles
/// on a read, 3 when the request meets one write, 2 or 1 inside the back-to-back writes
/// of RMW, JSR or interrupt pushes.
pub const DMC_STALL: u32 = 4;

/// RP2A07 (PAL) noise periods in CPU cycles.
pub const NOISE_PERIODS_PAL: [u16; 16] = [
    4, 8, 14, 30, 60, 88, 118, 148, 188, 236, 354, 472, 708, 944, 1890, 3778,
];
/// RP2A07 (PAL) DMC rates in CPU cycles per output bit.
pub const DMC_RATES_PAL: [u16; 16] = [
    398, 354, 316, 298, 276, 236, 210, 198, 176, 148, 132, 118, 98, 78, 66, 50,
];

/// Delta modulation channel.
#[derive(Debug, Clone, Default)]
pub struct Dmc {
    pub irq_enabled: bool,
    pub looping: bool,
    pub rate: u8,
    pub level: u8,
    pub start: u16,
    pub length: u16,
    pub address: u16,
    pub remaining: u16,
    pub buffer: Option<u8>,
    pub irq: bool,
    timer: u16,
    shift: u8,
    bits: u8,
    silent: bool,
    /// RP2A07 rate table ([`DMC_RATES_PAL`]).
    pub pal: bool,
}

impl Dmc {
    fn write(&mut self, reg: u16, value: u8) {
        match reg {
            0x10 => {
                self.irq_enabled = value & 0x80 != 0;
                self.looping = value & 0x40 != 0;
                self.rate = value & 0x0F;
                if !self.irq_enabled {
                    self.irq = false;
                }
            }
            0x11 => self.level = value & 0x7F,
            0x12 => self.start = 0xC000 | (u16::from(value) << 6),
            _ => self.length = (u16::from(value) << 4) | 1,
        }
    }

    fn restart(&mut self) {
        self.address = self.start;
        self.remaining = self.length;
    }

    fn set_enabled(&mut self, on: bool) {
        if !on {
            self.remaining = 0;
        } else if self.remaining == 0 {
            self.restart();
        }
    }

    pub fn fetch_address(&self) -> Option<u16> {
        (self.buffer.is_none() && self.remaining > 0).then_some(self.address)
    }

    fn deliver(&mut self, byte: u8) {
        if self.fetch_address().is_none() {
            return;
        }
        self.buffer = Some(byte);
        self.address = self.address.checked_add(1).unwrap_or(0x8000);
        self.remaining -= 1;
        if self.remaining == 0 {
            if self.looping {
                self.restart();
            } else if self.irq_enabled {
                self.irq = true;
            }
        }
    }

    fn clock_timer(&mut self) {
        if self.timer > 0 {
            self.timer -= 1;
            return;
        }
        let rates = if self.pal { DMC_RATES_PAL } else { DMC_RATES };
        self.timer = rates[usize::from(self.rate)] - 1;
        if !self.silent {
            if self.shift & 1 != 0 {
                if self.level <= 125 {
                    self.level += 2;
                }
            } else if self.level >= 2 {
                self.level -= 2;
            }
        }
        self.shift >>= 1;
        self.bits = self.bits.saturating_sub(1);
        if self.bits == 0 {
            self.bits = 8;
            match self.buffer.take() {
                Some(byte) => {
                    self.shift = byte;
                    self.silent = false;
                }
                None => self.silent = true,
            }
        }
    }
}

const FOUR_STEP: [u32; 4] = [7457, 14913, 22371, 29829];
const FIVE_STEP: [u32; 4] = [7457, 14913, 22371, 37281];
/// RP2A07 frame sequencer (APU half-cycles 4156.5, 8313.5, 12469.5, 16626.5 and
/// 20782.5 in CPU cycles), wrapping at 33254 / 41566.
const FOUR_STEP_PAL: [u32; 4] = [8313, 16627, 24939, 33253];
const FIVE_STEP_PAL: [u32; 4] = [8313, 16627, 24939, 41565];

/// Volume envelope: constant volume or a 15→0 decay clocked by quarter frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Envelope {
    pub start: bool,
    pub looping: bool,
    pub constant: bool,
    /// Constant volume, or the divider period.
    pub period: u8,
    divider: u8,
    pub decay: u8,
}

impl Envelope {
    fn write(&mut self, value: u8) {
        self.looping = value & 0x20 != 0;
        self.constant = value & 0x10 != 0;
        self.period = value & 0x0F;
    }

    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.period;
        } else if self.divider == 0 {
            self.divider = self.period;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.looping {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }

    pub fn volume(&self) -> u8 {
        if self.constant {
            self.period
        } else {
            self.decay
        }
    }
}

/// Length counter: silences a channel when it reaches 0; halted by the loop bit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Length {
    pub value: u8,
    pub halt: bool,
    pub enabled: bool,
}

impl Length {
    fn load(&mut self, index: u8) {
        if self.enabled {
            self.value = LENGTH_TABLE[usize::from(index >> 3)];
        }
    }

    fn clock(&mut self) {
        if !self.halt && self.value > 0 {
            self.value -= 1;
        }
    }

    fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.value = 0;
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pulse {
    /// Pulse 1 negates with one's complement (`-c-1`), pulse 2 with two's (`-c`).
    second: bool,
    pub duty: u8,
    pub envelope: Envelope,
    pub length: Length,
    pub timer_period: u16,
    timer: u16,
    pub step: u8,
    sweep_enabled: bool,
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_divider: u8,
    sweep_reload: bool,
}

impl Pulse {
    fn write(&mut self, reg: u16, value: u8) {
        match reg & 3 {
            0 => {
                self.duty = value >> 6;
                self.length.halt = value & 0x20 != 0;
                self.envelope.write(value);
            }
            1 => {
                self.sweep_enabled = value & 0x80 != 0;
                self.sweep_period = (value >> 4) & 7;
                self.sweep_negate = value & 0x08 != 0;
                self.sweep_shift = value & 7;
                self.sweep_reload = true;
            }
            2 => self.timer_period = (self.timer_period & 0x700) | u16::from(value),
            _ => {
                self.timer_period = (self.timer_period & 0xFF) | (u16::from(value & 7) << 8);
                self.length.load(value);
                self.step = 0;
                self.envelope.start = true;
            }
        }
    }

    /// Sweep target period.
    pub fn target_period(&self) -> u16 {
        let change = self.timer_period >> self.sweep_shift;
        if self.sweep_negate {
            let sub = change + u16::from(!self.second);
            self.timer_period.saturating_sub(sub)
        } else {
            self.timer_period + change
        }
    }

    /// Muted by the sweep unit even when the sweep is disabled.
    pub fn sweep_muted(&self) -> bool {
        self.timer_period < 8 || self.target_period() > 0x7FF
    }

    fn clock_sweep(&mut self) {
        if self.sweep_divider == 0
            && self.sweep_enabled
            && self.sweep_shift > 0
            && !self.sweep_muted()
        {
            self.timer_period = self.target_period();
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.step = (self.step + 1) & 7;
        } else {
            self.timer -= 1;
        }
    }

    pub fn output(&self) -> u8 {
        if self.length.value == 0
            || self.sweep_muted()
            || DUTY[usize::from(self.duty)][usize::from(self.step)] == 0
        {
            0
        } else {
            self.envelope.volume()
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Triangle {
    pub length: Length,
    /// `$4008` bit 7: length halt and linear counter control.
    pub control: bool,
    pub linear_reload_value: u8,
    pub linear: u8,
    linear_reload: bool,
    pub timer_period: u16,
    timer: u16,
    pub step: u8,
}

impl Triangle {
    fn write(&mut self, reg: u16, value: u8) {
        match reg & 3 {
            0 => {
                self.control = value & 0x80 != 0;
                self.length.halt = self.control;
                self.linear_reload_value = value & 0x7F;
            }
            1 => {}
            2 => self.timer_period = (self.timer_period & 0x700) | u16::from(value),
            _ => {
                self.timer_period = (self.timer_period & 0xFF) | (u16::from(value & 7) << 8);
                self.length.load(value);
                self.linear_reload = true;
            }
        }
    }

    fn clock_linear(&mut self) {
        if self.linear_reload {
            self.linear = self.linear_reload_value;
        } else if self.linear > 0 {
            self.linear -= 1;
        }
        if !self.control {
            self.linear_reload = false;
        }
    }

    /// The sequencer only moves while both counters are non-zero.
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            if self.length.value > 0 && self.linear > 0 {
                self.step = (self.step + 1) & 31;
            }
        } else {
            self.timer -= 1;
        }
    }

    /// 15 down to 0, then 0 up to 15. A silenced triangle holds its last level.
    pub fn output(&self) -> u8 {
        if self.step < 16 {
            15 - self.step
        } else {
            self.step - 16
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Noise {
    pub envelope: Envelope,
    pub length: Length,
    /// `$400E` bit 7: feedback from bit 6 instead of bit 1 (93-step sequences).
    pub short_mode: bool,
    pub timer_period: u16,
    timer: u16,
    /// 15-bit shift register; 1 after power-up.
    pub shift: u16,
    /// RP2A07 period table ([`NOISE_PERIODS_PAL`]).
    pub pal: bool,
}

impl Default for Noise {
    fn default() -> Self {
        Self {
            envelope: Envelope::default(),
            length: Length::default(),
            short_mode: false,
            timer_period: NOISE_PERIODS[0],
            timer: 0,
            shift: 1,
            pal: false,
        }
    }
}

impl Noise {
    fn write(&mut self, reg: u16, value: u8) {
        match reg & 3 {
            0 => {
                self.length.halt = value & 0x20 != 0;
                self.envelope.write(value);
            }
            1 => {}
            2 => {
                self.short_mode = value & 0x80 != 0;
                let periods = if self.pal {
                    NOISE_PERIODS_PAL
                } else {
                    NOISE_PERIODS
                };
                self.timer_period = periods[usize::from(value & 0x0F)];
            }
            _ => {
                self.length.load(value);
                self.envelope.start = true;
            }
        }
    }

    /// Timer period counts CPU cycles here: the table already is in CPU cycles.
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period - 1;
            let tap = if self.short_mode { 6 } else { 1 };
            let feedback = (self.shift ^ (self.shift >> tap)) & 1;
            self.shift = (self.shift >> 1) | (feedback << 14);
        } else {
            self.timer -= 1;
        }
    }

    pub fn output(&self) -> u8 {
        if self.length.value == 0 || self.shift & 1 != 0 {
            0
        } else {
            self.envelope.volume()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Apu {
    pub pulse: [Pulse; 2],
    pub triangle: Triangle,
    pub noise: Noise,
    pub dmc: Dmc,
    five_step: bool,
    irq_inhibit: bool,
    frame_irq: bool,
    frame_cycle: u32,
    cycle: u64,
    /// Ticks until a `$4017` write resets the sequencer (0: none pending).
    reset_delay: u8,
    /// RP2A07 tables (PAL); Dendy uses the NTSC ones.
    pub pal: bool,
}

impl Apu {
    pub fn new() -> Self {
        let mut apu = Self::default();
        apu.pulse[1].second = true;
        apu.dmc.silent = true;
        // Power-on registers are 0: start $C000, length 16·0 + 1.
        apu.dmc.start = 0xC000;
        apu.dmc.length = 1;
        apu
    }

    /// Select the RP2A07 (PAL) noise, DMC and frame-sequencer tables.
    pub fn set_pal(&mut self, pal: bool) {
        self.pal = pal;
        self.noise.pal = pal;
        self.dmc.pal = pal;
    }

    /// CPU-visible write at `$4000 + reg`.
    pub fn write_register(&mut self, reg: u16, value: u8) {
        match reg {
            0x00..=0x03 => self.pulse[0].write(reg, value),
            0x04..=0x07 => self.pulse[1].write(reg, value),
            0x08..=0x0B => self.triangle.write(reg, value),
            0x0C..=0x0F => self.noise.write(reg, value),
            0x10..=0x13 => self.dmc.write(reg, value),
            0x15 => {
                self.dmc.set_enabled(value & 0x10 != 0);
                self.dmc.irq = false;
                self.pulse[0].length.set_enabled(value & 1 != 0);
                self.pulse[1].length.set_enabled(value & 2 != 0);
                self.triangle.length.set_enabled(value & 4 != 0);
                self.noise.length.set_enabled(value & 8 != 0);
            }
            0x17 => {
                self.five_step = value & 0x80 != 0;
                self.irq_inhibit = value & 0x40 != 0;
                if self.irq_inhibit {
                    self.frame_irq = false;
                }
                // The sequencer restarts 3 CPU cycles after a write on an even cycle
                // (an APU cycle boundary), 4 after an odd one.
                self.reset_delay = if self.cycle % 2 == 0 { 3 } else { 4 };
            }
            _ => {}
        }
    }

    /// CPU cycles until a pending `$4017` restart, or 0.
    pub fn frame_reset_pending(&self) -> u8 {
        self.reset_delay
    }

    /// Status without the read side effect.
    pub fn peek_status(&self) -> u8 {
        u8::from(self.pulse[0].length.value > 0)
            | (u8::from(self.pulse[1].length.value > 0) << 1)
            | (u8::from(self.triangle.length.value > 0) << 2)
            | (u8::from(self.noise.length.value > 0) << 3)
            | (u8::from(self.dmc.remaining > 0) << 4)
            | (u8::from(self.frame_irq) << 6)
            | (u8::from(self.dmc.irq) << 7)
    }

    /// `$4015` read: status, then the frame IRQ flag clears.
    pub fn read_status(&mut self) -> u8 {
        let status = self.peek_status();
        self.frame_irq = false;
        status
    }

    pub fn irq_asserted(&self) -> bool {
        self.frame_irq || self.dmc.irq
    }

    /// Byte the DMC wants read now, if any.
    pub fn dmc_fetch_address(&self) -> Option<u16> {
        self.dmc.fetch_address()
    }

    /// Hand over the byte read at [`Apu::dmc_fetch_address`]; ignored when none is due.
    pub fn dmc_deliver(&mut self, byte: u8) {
        self.dmc.deliver(byte);
    }

    fn quarter_frame(&mut self) {
        self.pulse[0].envelope.clock();
        self.pulse[1].envelope.clock();
        self.noise.envelope.clock();
        self.triangle.clock_linear();
    }

    fn half_frame(&mut self) {
        for pulse in &mut self.pulse {
            pulse.length.clock();
            pulse.clock_sweep();
        }
        self.triangle.length.clock();
        self.noise.length.clock();
    }

    /// One CPU cycle.
    pub fn tick(&mut self) {
        if self.reset_delay > 0 {
            self.reset_delay -= 1;
            if self.reset_delay == 0 {
                self.frame_cycle = 0;
                if self.five_step {
                    self.quarter_frame();
                    self.half_frame();
                }
                self.clock_channels();
                return;
            }
        }
        self.frame_cycle += 1;
        let steps = match (self.pal, self.five_step) {
            (false, false) => FOUR_STEP,
            (false, true) => FIVE_STEP,
            (true, false) => FOUR_STEP_PAL,
            (true, true) => FIVE_STEP_PAL,
        };
        if let Some(i) = steps.iter().position(|&c| c == self.frame_cycle) {
            self.quarter_frame();
            if i == 1 || i == 3 {
                self.half_frame();
            }
            if i == 3 && !self.five_step && !self.irq_inhibit {
                self.frame_irq = true;
            }
        }
        if self.frame_cycle > steps[3] {
            self.frame_cycle = 0;
        }
        self.clock_channels();
    }

    fn clock_channels(&mut self) {
        self.triangle.clock_timer();
        self.noise.clock_timer();
        self.dmc.clock_timer();
        if self.cycle % 2 == 1 {
            self.pulse[0].clock_timer();
            self.pulse[1].clock_timer();
        }
        self.cycle += 1;
    }

    /// Channel levels: pulse 1, pulse 2, triangle, noise (`0..=15`), DMC (`0..=127`).
    pub fn channel_outputs(&self) -> [u8; 5] {
        [
            self.pulse[0].output(),
            self.pulse[1].output(),
            self.triangle.output(),
            self.noise.output(),
            self.dmc.level,
        ]
    }

    /// Mixed level from the nonlinear mixer.
    pub fn sample(&self) -> f32 {
        mix(self.channel_outputs())
    }

    /// `count` samples, ticking `cycles_per_sample` CPU cycles before each.
    pub fn render(&mut self, cycles_per_sample: u32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|_| {
                for _ in 0..cycles_per_sample {
                    self.tick();
                }
                self.sample()
            })
            .collect()
    }

    /// `count` samples of one channel's level, one per CPU cycle.
    pub fn trace_channel(&mut self, channel: usize, count: usize) -> Vec<u8> {
        (0..count)
            .map(|_| {
                self.tick();
                self.channel_outputs()[channel]
            })
            .collect()
    }
}

/// NTSC 2A03 CPU clock.
pub const CPU_HZ_NTSC: u32 = 1_789_773;

impl Apu {
    /// Pitch a channel is sounding, in Hz at `cpu_hz`; `None` when it is silent by
    /// its counters (length 0, sweep mute, triangle linear 0) or, for the triangle,
    /// ultrasonic (period < 2, which games use to stop it). Channels 0–1 pulse
    /// (`cpu / (16 (t + 1))`), 2 triangle (`cpu / (32 (t + 1))`), 3 noise (shift
    /// clock `cpu / period`), 4 DMC (output-bit rate `cpu / period`).
    pub fn tone_hz(&self, channel: usize, cpu_hz: u32) -> Option<f64> {
        let cpu = f64::from(cpu_hz);
        match channel {
            0 | 1 => {
                let p = &self.pulse[channel];
                (p.length.value > 0 && !p.sweep_muted())
                    .then(|| cpu / (16.0 * (f64::from(p.timer_period) + 1.0)))
            }
            2 => {
                let t = &self.triangle;
                (t.length.value > 0 && t.linear > 0 && t.timer_period >= 2)
                    .then(|| cpu / (32.0 * (f64::from(t.timer_period) + 1.0)))
            }
            3 => (self.noise.length.value > 0)
                .then(|| cpu / f64::from(self.noise.timer_period.max(1))),
            4 => Some(cpu / f64::from(DMC_RATES[usize::from(self.dmc.rate)])),
            _ => None,
        }
    }

    /// Next `count` samples at the resampler's rate: each is the box-filtered mean of
    /// the mixer over the CPU cycles it covers. Cycle counts per sample vary by one
    /// so that `n` samples always take `floor(n * cpu_hz / rate)` cycles in total.
    pub fn render_resampled(&mut self, res: &mut Resampler, count: usize) -> Vec<f32> {
        (0..count)
            .map(|_| {
                res.samples += 1;
                let target = res.samples * u64::from(res.cpu_hz) / u64::from(res.rate);
                let n = target - res.cycles;
                let mut sum = 0.0f64;
                for _ in 0..n {
                    self.tick();
                    sum += f64::from(self.sample());
                }
                res.cycles = target;
                if n == 0 {
                    self.sample()
                } else {
                    (sum / n as f64) as f32
                }
            })
            .collect()
    }
}

/// Fractional CPU-cycle to host-sample bookkeeping for [`Apu::render_resampled`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resampler {
    pub cpu_hz: u32,
    pub rate: u32,
    /// Samples produced and CPU cycles consumed so far.
    pub samples: u64,
    pub cycles: u64,
}

impl Resampler {
    /// `rate` must be non-zero and not above `cpu_hz`.
    pub fn new(cpu_hz: u32, rate: u32) -> Option<Self> {
        (rate > 0 && rate <= cpu_hz).then_some(Self {
            cpu_hz,
            rate,
            samples: 0,
            cycles: 0,
        })
    }
}

/// The console's analogue output stage as documented for the NES: first-order
/// high-pass at 90 Hz, high-pass at 440 Hz, low-pass at 14 kHz, applied in that
/// order at a fixed sample rate. Removes the mixer's DC offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputFilter {
    hp: [(f32, f32, f32); 2],
    lp_a: f32,
    lp_prev: f32,
}

impl OutputFilter {
    pub fn nes(rate: u32) -> Self {
        let dt = 1.0 / rate as f32;
        let hp = |hz: f32| {
            let rc = 1.0 / (2.0 * std::f32::consts::PI * hz);
            (rc / (rc + dt), 0.0, 0.0)
        };
        let rc = 1.0 / (2.0 * std::f32::consts::PI * 14_000.0);
        Self {
            hp: [hp(90.0), hp(440.0)],
            lp_a: dt / (rc + dt),
            lp_prev: 0.0,
        }
    }

    pub fn process(&mut self, x: f32) -> f32 {
        let mut v = x;
        for (a, prev_in, prev_out) in &mut self.hp {
            let y = *a * (*prev_out + v - *prev_in);
            *prev_in = v;
            *prev_out = y;
            v = y;
        }
        self.lp_prev += self.lp_a * (v - self.lp_prev);
        self.lp_prev
    }
}

/// Expansion-audio hook: cartridge sound enters the console's audio path after the
/// APU's own mixer (the cartridge audio-in pin is summed with the 2A03 output), so
/// it adds linearly to [`mix`]'s result. `expansion` is in the same units.
pub fn mix_expansion(apu: f32, expansion: f32) -> f32 {
    apu + expansion
}

/// Nonlinear mixer for `[p1, p2, triangle, noise, dmc]`.
pub fn mix(levels: [u8; 5]) -> f32 {
    let [p1, p2, t, n, d] = levels.map(f32::from);
    let pulse = if p1 + p2 == 0.0 {
        0.0
    } else {
        95.88 / (8128.0 / (p1 + p2) + 100.0)
    };
    let sum = t / 8227.0 + n / 12241.0 + d / 22638.0;
    let tnd = if sum == 0.0 {
        0.0
    } else {
        159.79 / (1.0 / sum + 100.0)
    };
    pulse + tnd
}

/// `$4000`–`$4017` window: offsets are registers; only `$4015` reads back.
impl IoDevice for Apu {
    fn read(&mut self, offset: u16) -> u8 {
        if offset == 0x15 {
            self.read_status()
        } else {
            0
        }
    }

    fn write(&mut self, offset: u16, value: u8) {
        self.write_register(offset, value);
    }

    fn peek(&self, offset: u16) -> u8 {
        if offset == 0x15 {
            self.peek_status()
        } else {
            0
        }
    }

    /// One APU cycle per CPU cycle, on PHI2.
    fn phase(&mut self, phase: BusPhase) {
        if phase == BusPhase::Phi2 {
            self.tick();
        }
    }
}

mod state {
    use super::*;
    use crate::nes_state::{NesStateError, StateReader, StateWriter};
    type R<'a, 'b> = &'a mut StateReader<'b>;

    impl Envelope {
        fn put(&self, w: &mut StateWriter) {
            w.bool(self.start);
            w.bool(self.looping);
            w.bool(self.constant);
            w.u8(self.period);
            w.u8(self.divider);
            w.u8(self.decay);
        }
        fn get(&mut self, r: R) -> Result<(), NesStateError> {
            self.start = r.bool("envelope start")?;
            self.looping = r.bool("envelope loop")?;
            self.constant = r.bool("envelope constant")?;
            self.period = r.small(15, "envelope period")?;
            self.divider = r.small(15, "envelope divider")?;
            self.decay = r.small(15, "envelope decay")?;
            Ok(())
        }
    }

    impl Length {
        fn put(&self, w: &mut StateWriter) {
            w.u8(self.value);
            w.bool(self.halt);
            w.bool(self.enabled);
        }
        fn get(&mut self, r: R) -> Result<(), NesStateError> {
            self.value = r.u8("length")?;
            self.halt = r.bool("length halt")?;
            self.enabled = r.bool("length enabled")?;
            Ok(())
        }
    }

    impl Pulse {
        fn put(&self, w: &mut StateWriter) {
            w.u8(self.duty);
            self.envelope.put(w);
            self.length.put(w);
            w.u16(self.timer_period);
            w.u16(self.timer);
            w.u8(self.step);
            w.bool(self.sweep_enabled);
            w.u8(self.sweep_period);
            w.bool(self.sweep_negate);
            w.u8(self.sweep_shift);
            w.u8(self.sweep_divider);
            w.bool(self.sweep_reload);
        }
        fn get(&mut self, r: R) -> Result<(), NesStateError> {
            self.duty = r.small(3, "duty")?;
            self.envelope.get(r)?;
            self.length.get(r)?;
            self.timer_period = r.u16("pulse period")?;
            self.timer = r.u16("pulse timer")?;
            self.step = r.small(7, "pulse step")?;
            self.sweep_enabled = r.bool("sweep enabled")?;
            self.sweep_period = r.small(7, "sweep period")?;
            self.sweep_negate = r.bool("sweep negate")?;
            self.sweep_shift = r.small(7, "sweep shift")?;
            self.sweep_divider = r.small(7, "sweep divider")?;
            self.sweep_reload = r.bool("sweep reload")?;
            Ok(())
        }
    }

    impl Triangle {
        fn put(&self, w: &mut StateWriter) {
            self.length.put(w);
            w.bool(self.control);
            w.u8(self.linear_reload_value);
            w.u8(self.linear);
            w.bool(self.linear_reload);
            w.u16(self.timer_period);
            w.u16(self.timer);
            w.u8(self.step);
        }
        fn get(&mut self, r: R) -> Result<(), NesStateError> {
            self.length.get(r)?;
            self.control = r.bool("triangle control")?;
            self.linear_reload_value = r.small(127, "linear reload")?;
            self.linear = r.small(127, "linear counter")?;
            self.linear_reload = r.bool("linear reload flag")?;
            self.timer_period = r.u16("triangle period")?;
            self.timer = r.u16("triangle timer")?;
            self.step = r.small(31, "triangle step")?;
            Ok(())
        }
    }

    impl Noise {
        fn put(&self, w: &mut StateWriter) {
            self.envelope.put(w);
            self.length.put(w);
            w.bool(self.short_mode);
            w.u16(self.timer_period);
            w.u16(self.timer);
            w.u16(self.shift);
        }
        fn get(&mut self, r: R) -> Result<(), NesStateError> {
            self.envelope.get(r)?;
            self.length.get(r)?;
            self.short_mode = r.bool("noise mode")?;
            self.timer_period = r.u16("noise period")?;
            self.timer = r.u16("noise timer")?;
            self.shift = r.u16("noise shift")?;
            if self.shift > 0x7FFF {
                return Err(NesStateError::BadValue {
                    what: "noise shift",
                    value: u64::from(self.shift),
                });
            }
            Ok(())
        }
    }

    impl Dmc {
        fn put(&self, w: &mut StateWriter) {
            w.bool(self.irq_enabled);
            w.bool(self.looping);
            w.u8(self.rate);
            w.u8(self.level);
            for v in [
                self.start,
                self.length,
                self.address,
                self.remaining,
                self.timer,
            ] {
                w.u16(v);
            }
            w.opt_u8(self.buffer);
            w.bool(self.irq);
            w.u8(self.shift);
            w.u8(self.bits);
            w.bool(self.silent);
        }
        fn get(&mut self, r: R) -> Result<(), NesStateError> {
            self.irq_enabled = r.bool("DMC IRQ enable")?;
            self.looping = r.bool("DMC loop")?;
            self.rate = r.small(15, "DMC rate")?;
            self.level = r.small(127, "DMC level")?;
            self.start = r.u16("DMC start")?;
            self.length = r.u16("DMC length")?;
            self.address = r.u16("DMC address")?;
            self.remaining = r.u16("DMC remaining")?;
            self.timer = r.u16("DMC timer")?;
            self.buffer = r.opt_u8("DMC buffer")?;
            self.irq = r.bool("DMC IRQ")?;
            self.shift = r.u8("DMC shift")?;
            self.bits = r.small(8, "DMC bits")?;
            self.silent = r.bool("DMC silent")?;
            Ok(())
        }
    }

    impl Apu {
        pub(crate) fn put(&self, w: &mut StateWriter) {
            self.pulse[0].put(w);
            self.pulse[1].put(w);
            self.triangle.put(w);
            self.noise.put(w);
            self.dmc.put(w);
            w.bool(self.five_step);
            w.bool(self.irq_inhibit);
            w.bool(self.frame_irq);
            w.u32(self.frame_cycle);
            w.u64(self.cycle);
            w.u8(self.reset_delay);
        }
        pub(crate) fn get(&mut self, r: R) -> Result<(), NesStateError> {
            self.pulse[0].get(r)?;
            self.pulse[1].get(r)?;
            self.triangle.get(r)?;
            self.noise.get(r)?;
            self.dmc.get(r)?;
            self.five_step = r.bool("five-step")?;
            self.irq_inhibit = r.bool("IRQ inhibit")?;
            self.frame_irq = r.bool("frame IRQ")?;
            self.frame_cycle = r.u32("frame cycle")?;
            self.cycle = r.u64("APU cycle")?;
            self.reset_delay = r.small(4, "frame reset delay")?;
            Ok(())
        }
    }
}

/// One register write at an APU cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoggedWrite {
    pub cycle: u64,
    /// Register offset `0x00..=0x17` (as [`Apu::write_register`] takes it), or
    /// [`DMC_SAMPLE_REG`] for a DMC sample byte.
    pub reg: u16,
    pub value: u8,
}

/// Pseudo-register of a [`RegisterLog`] entry that is a DMC sample byte delivered
/// by DMA, not a CPU write.
pub const DMC_SAMPLE_REG: u16 = 0x1F;

/// Typed errors of [`RegisterLog::parse`] (1-based line numbers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogError {
    Syntax {
        line: usize,
        text: String,
    },
    BadRegister {
        line: usize,
        reg: u16,
    },
    /// Cycles must not go backwards.
    OutOfOrder {
        line: usize,
        cycle: u64,
        previous: u64,
    },
}

impl std::fmt::Display for LogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Syntax { line, text } => write!(f, "linha {line}: sintaxe: {text}"),
            Self::BadRegister { line, reg } => {
                write!(
                    f,
                    "linha {line}: registrador ${reg:02X} fora de $00-$17 e $1F"
                )
            }
            Self::OutOfOrder {
                line,
                cycle,
                previous,
            } => {
                write!(f, "linha {line}: ciclo {cycle} antes de {previous}")
            }
        }
    }
}

impl std::error::Error for LogError {}

/// A recording of APU register writes: playing it into a fresh [`Apu`] reproduces
/// the original output cycle for cycle (the APU is deterministic given its writes;
/// DMC sample bytes are not part of the log, so DMC playback is exact only for the
/// direct-load level `$11`) unless the DMC bytes are recorded too: pseudo-register
/// [`DMC_SAMPLE_REG`] (`$1F`, outside the `$4000`–`$4017` map) holds a sample byte handed to [`Apu::dmc_deliver`] at
/// that cycle, so replay with the DMC playing is exact without the memory it read.
/// Text form: one `cycle reg value` line per write, hex register and value
/// (`1234 15 0F`); `#` starts a comment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegisterLog {
    pub writes: Vec<LoggedWrite>,
}

impl RegisterLog {
    /// Write through to `apu` and record the write at its current cycle.
    pub fn write(&mut self, apu: &mut Apu, reg: u16, value: u8) {
        self.writes.push(LoggedWrite {
            cycle: apu.cycle,
            reg,
            value,
        });
        apu.write_register(reg, value);
    }

    /// One APU tick that also serves a due DMC fetch from `memory` and records the
    /// byte as a [`DMC_SAMPLE_REG`] entry. Returns the byte read, if any.
    pub fn tick_with_dmc(&mut self, apu: &mut Apu, memory: impl Fn(u16) -> u8) -> Option<u8> {
        apu.tick();
        let byte = memory(apu.dmc_fetch_address()?);
        self.writes.push(LoggedWrite {
            cycle: apu.cycle,
            reg: DMC_SAMPLE_REG,
            value: byte,
        });
        apu.dmc_deliver(byte);
        Some(byte)
    }

    /// Number of recorded DMC sample bytes.
    pub fn dmc_bytes(&self) -> usize {
        self.writes
            .iter()
            .filter(|w| w.reg == DMC_SAMPLE_REG)
            .count()
    }

    /// Play into `apu` (normally fresh) for `cycles` ticks from its current cycle,
    /// applying each write before the tick of its cycle. Returns the mixer output
    /// after every tick.
    pub fn play(&self, apu: &mut Apu, cycles: u64) -> Vec<f32> {
        let mut next = self.writes.partition_point(|w| w.cycle < apu.cycle);
        let mut out = Vec::with_capacity(cycles as usize);
        for _ in 0..cycles {
            while let Some(w) = self.writes.get(next).filter(|w| w.cycle == apu.cycle) {
                if w.reg == DMC_SAMPLE_REG {
                    apu.dmc_deliver(w.value);
                } else {
                    apu.write_register(w.reg, w.value);
                }
                next += 1;
            }
            apu.tick();
            out.push(apu.sample());
        }
        out
    }

    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for w in &self.writes {
            let _ = writeln!(out, "{} {:02X} {:02X}", w.cycle, w.reg, w.value);
        }
        out
    }

    pub fn parse(text: &str) -> Result<Self, LogError> {
        let mut writes: Vec<LoggedWrite> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = i + 1;
            let body = raw.split('#').next().unwrap_or("").trim();
            if body.is_empty() {
                continue;
            }
            let syntax = || LogError::Syntax {
                line,
                text: body.to_string(),
            };
            let parts: Vec<&str> = body.split_whitespace().collect();
            let [c, r, v] = parts[..] else {
                return Err(syntax());
            };
            let cycle: u64 = c.parse().map_err(|_| syntax())?;
            let reg = u16::from_str_radix(r, 16).map_err(|_| syntax())?;
            let value = u8::from_str_radix(v, 16).map_err(|_| syntax())?;
            if reg > 0x17 && reg != DMC_SAMPLE_REG {
                return Err(LogError::BadRegister { line, reg });
            }
            if let Some(prev) = writes.last().filter(|p| p.cycle > cycle) {
                return Err(LogError::OutOfOrder {
                    line,
                    cycle,
                    previous: prev.cycle,
                });
            }
            writes.push(LoggedWrite { cycle, reg, value });
        }
        Ok(Self { writes })
    }
}

impl Apu {
    /// CPU cycles ticked since power-on.
    pub fn cycles(&self) -> u64 {
        self.cycle
    }
}
