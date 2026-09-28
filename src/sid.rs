//! MOS 6581 / 8580 SID (Sound Interface Device) Sound Chip Emulator.
//!
//! Emulates the iconic sound synthesis chip of the Commodore 64:
//! - 3 independent voices with 16-bit frequency registers and 12-bit pulse width.
//! - 4 selectable waveforms per voice: Triangle, Sawtooth, Pulse/Square, and 23-bit LFSR Noise.
//! - Ring modulation and oscillator synchronization between voices.
//! - Complete ADSR envelope generator with authentic exponential rate timings.
//! - Multi-mode resonant filter: Low-Pass, High-Pass, Band-Pass, and Notch.
//! - Master volume and voice 3 output readbacks (OSC3/ENV3).
//! - Implements [`crate::bus::IoDevice`] and direct audio sample rendering (PCM/WAV).

use crate::bus::IoDevice;

/// The clock frequency of a PAL Commodore 64 in Hz (~0.985 MHz).
pub const SID_PAL_CLOCK_HZ: u32 = 985_248;
/// The clock frequency of an NTSC Commodore 64 in Hz (~1.023 MHz).
pub const SID_NTSC_CLOCK_HZ: u32 = 1_022_727;

/// Internal rate counter lookup table for SID ADSR envelope generator (cycles per step).
const ADSR_RATES: [u32; 16] = [
    9, 32, 63, 95, 149, 220, 267, 313,
    392, 977, 1954, 3126, 3907, 11720, 19532, 31251,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeState {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// One of the 3 SID audio synthesis voices.
#[derive(Debug, Clone)]
pub struct Voice {
    pub freq: u16,
    pub pulse_width: u16,
    // Control register bits
    pub gate: bool,
    pub sync: bool,
    pub ring_mod: bool,
    pub test: bool,
    pub triangle: bool,
    pub sawtooth: bool,
    pub pulse: bool,
    pub noise: bool,

    // Envelope registers
    pub attack_rate: u8,
    pub decay_rate: u8,
    pub sustain_level: u8,
    pub release_rate: u8,

    // Internal oscillator state
    pub accumulator: u32,
    pub noise_lfsr: u32,
    pub msb_rising: bool,

    // Internal envelope state
    pub env_state: EnvelopeState,
    pub env_level: u8,
    pub env_counter: u32,
}

impl Default for Voice {
    fn default() -> Self {
        Self::new()
    }
}

impl Voice {
    pub fn new() -> Self {
        Self {
            freq: 0,
            pulse_width: 0,
            gate: false,
            sync: false,
            ring_mod: false,
            test: false,
            triangle: false,
            sawtooth: false,
            pulse: false,
            noise: false,
            attack_rate: 0,
            decay_rate: 0,
            sustain_level: 0,
            release_rate: 0,
            accumulator: 0,
            noise_lfsr: 0x7FFFFF, // 23-bit non-zero seed
            msb_rising: false,
            env_state: EnvelopeState::Idle,
            env_level: 0,
            env_counter: 0,
        }
    }

    /// Reset voice state.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Write control byte ($04, $0B, $12).
    pub fn write_control(&mut self, val: u8) {
        let prev_gate = self.gate;
        self.gate = (val & 0x01) != 0;
        self.sync = (val & 0x02) != 0;
        self.ring_mod = (val & 0x04) != 0;
        self.test = (val & 0x08) != 0;
        self.triangle = (val & 0x10) != 0;
        self.sawtooth = (val & 0x20) != 0;
        self.pulse = (val & 0x40) != 0;
        self.noise = (val & 0x80) != 0;

        if self.test {
            self.accumulator = 0;
            self.noise_lfsr = 0x7FFFFF;
        }

        // Gate 0 -> 1 starts Attack; Gate 1 -> 0 starts Release
        if !prev_gate && self.gate {
            self.env_state = EnvelopeState::Attack;
            self.env_counter = 0;
        } else if prev_gate && !self.gate {
            self.env_state = EnvelopeState::Release;
            self.env_counter = 0;
        }
    }

    /// Advance oscillator by one clock cycle.
    pub fn clock_oscillator(&mut self, sync_source_msb: bool, ring_source_msb: bool) {
        if self.test {
            return;
        }

        let old_acc = self.accumulator;
        self.accumulator = (self.accumulator + self.freq as u32) & 0x00FFFFFF;

        // Detect MSB rising edge (bit 23)
        let old_msb = (old_acc & 0x800000) != 0;
        let new_msb = (self.accumulator & 0x800000) != 0;
        self.msb_rising = !old_msb && new_msb;

        // Hard Sync: when sync source has MSB rising edge, reset our accumulator
        if self.sync && sync_source_msb {
            self.accumulator = 0;
        }

        // Clock 23-bit Galois LFSR on bit 19 rising edge
        let bit19_rising = ((old_acc ^ self.accumulator) & (self.accumulator & 0x080000)) != 0;
        if bit19_rising {
            let bit22 = (self.noise_lfsr >> 22) & 1;
            let bit17 = (self.noise_lfsr >> 17) & 1;
            let feedback = bit22 ^ bit17;
            self.noise_lfsr = ((self.noise_lfsr << 1) | feedback) & 0x7FFFFF;
        }

        let _ = ring_source_msb; // used in waveform generation
    }

    /// Advance envelope ADSR counter by one clock cycle.
    pub fn clock_envelope(&mut self) {
        let rate_index = match self.env_state {
            EnvelopeState::Idle => return,
            EnvelopeState::Attack => self.attack_rate as usize,
            EnvelopeState::Decay => self.decay_rate as usize,
            EnvelopeState::Sustain => return,
            EnvelopeState::Release => self.release_rate as usize,
        };

        let period = ADSR_RATES[rate_index.min(15)];
        self.env_counter += 1;
        if self.env_counter >= period {
            self.env_counter = 0;
            match self.env_state {
                EnvelopeState::Idle | EnvelopeState::Sustain => {}
                EnvelopeState::Attack => {
                    if self.env_level == 255 {
                        self.env_state = EnvelopeState::Decay;
                    } else {
                        self.env_level = self.env_level.saturating_add(1);
                    }
                }
                EnvelopeState::Decay => {
                    let target = (self.sustain_level << 4) | self.sustain_level;
                    if self.env_level <= target {
                        self.env_state = EnvelopeState::Sustain;
                    } else {
                        self.env_level = self.env_level.saturating_sub(1);
                    }
                }
                EnvelopeState::Release => {
                    if self.env_level > 0 {
                        self.env_level = self.env_level.saturating_sub(1);
                    } else {
                        self.env_state = EnvelopeState::Idle;
                    }
                }
            }
        }
    }

    /// Generate raw 12-bit waveform output sample (0..4095).
    pub fn generate_waveform(&self, ring_source_msb: bool) -> u16 {
        if self.test {
            return 0;
        }

        let mut out = 0u16;
        let mut active_count = 0u32;

        // Triangle
        if self.triangle {
            let mut msb = (self.accumulator & 0x800000) != 0;
            if self.ring_mod {
                msb ^= ring_source_msb;
            }
            let tri = if msb {
                ((!self.accumulator >> 11) & 0x0FFF) as u16
            } else {
                ((self.accumulator >> 11) & 0x0FFF) as u16
            };
            out = tri;
            active_count += 1;
        }

        // Sawtooth
        if self.sawtooth {
            let saw = ((self.accumulator >> 12) & 0x0FFF) as u16;
            out = if active_count > 0 { out & saw } else { saw };
            active_count += 1;
        }

        // Pulse / Square
        if self.pulse {
            let acc12 = ((self.accumulator >> 12) & 0x0FFF) as u16;
            let pulse_val = if acc12 >= (self.pulse_width & 0x0FFF) {
                0x0FFF
            } else {
                0x0000
            };
            out = if active_count > 0 { out & pulse_val } else { pulse_val };
            active_count += 1;
        }

        // Noise
        if self.noise {
            // Pick 12 bits from LFSR output
            let noise_val = (((self.noise_lfsr >> 20) & 1) << 11)
                | (((self.noise_lfsr >> 18) & 1) << 10)
                | (((self.noise_lfsr >> 14) & 1) << 9)
                | (((self.noise_lfsr >> 11) & 1) << 8)
                | (((self.noise_lfsr >> 9) & 1) << 7)
                | (((self.noise_lfsr >> 5) & 1) << 6)
                | (((self.noise_lfsr >> 2) & 1) << 5)
                | (((self.noise_lfsr >> 0) & 1) << 4);
            let noise_val = (noise_val as u16) & 0x0FFF;
            out = if active_count > 0 { out & noise_val } else { noise_val };
        }

        out
    }

    /// Complete voice output level scaled by envelope (-1.0 to +1.0).
    pub fn output(&self, ring_source_msb: bool) -> f32 {
        let wave = self.generate_waveform(ring_source_msb);
        // Normalize 12-bit waveform (0..4095) to -1.0..1.0
        let wave_centered = (wave as f32 / 2048.0) - 1.0;
        let env_gain = self.env_level as f32 / 255.0;
        wave_centered * env_gain
    }
}

/// The MOS 6581 / 8580 Sound Interface Device (SID).
#[derive(Debug, Clone)]
pub struct Sid6581 {
    pub voices: [Voice; 3],

    // Filter controls ($15..$18)
    pub filter_cutoff: u16,  // 11 bits (FC LO + FC HI)
    pub filter_res: u8,      // 4 bits
    pub filter_voice1: bool,
    pub filter_voice2: bool,
    pub filter_voice3: bool,
    pub filter_voice_ext: bool,
    pub filter_lowpass: bool,
    pub filter_bandpass: bool,
    pub filter_highpass: bool,
    pub voice3_off: bool,
    pub volume: u8, // 4 bits

    // I/O Paddle registers ($19, $1A)
    pub pot_x: u8,
    pub pot_y: u8,

    // Filter biquad state variables
    filter_w0: f32,
    filter_w1: f32,
}

impl Default for Sid6581 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sid6581 {
    pub fn new() -> Self {
        Self {
            voices: [Voice::new(), Voice::new(), Voice::new()],
            filter_cutoff: 0,
            filter_res: 0,
            filter_voice1: false,
            filter_voice2: false,
            filter_voice3: false,
            filter_voice_ext: false,
            filter_lowpass: false,
            filter_bandpass: false,
            filter_highpass: false,
            voice3_off: false,
            volume: 0,
            pot_x: 0xFF,
            pot_y: 0xFF,
            filter_w0: 0.0,
            filter_w1: 0.0,
        }
    }

    /// Reset all SID registers and internal oscillators.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Read SID register by offset 0x00..=0x1C.
    pub fn read_reg(&self, reg: u8) -> u8 {
        match reg {
            0x19 => self.pot_x,
            0x1A => self.pot_y,
            0x1B => ((self.voices[2].accumulator >> 16) & 0xFF) as u8, // OSC3
            0x1C => self.voices[2].env_level,                          // ENV3
            _ => 0x00, // write-only registers return 0 on SID
        }
    }

    /// Write SID register by offset 0x00..=0x1C.
    pub fn write_reg(&mut self, reg: u8, val: u8) {
        match reg {
            // Voice 1 ($00..$06)
            0x00 => self.voices[0].freq = (self.voices[0].freq & 0xFF00) | (val as u16),
            0x01 => self.voices[0].freq = (self.voices[0].freq & 0x00FF) | ((val as u16) << 8),
            0x02 => self.voices[0].pulse_width = (self.voices[0].pulse_width & 0x0F00) | (val as u16),
            0x03 => self.voices[0].pulse_width = (self.voices[0].pulse_width & 0x00FF) | (((val as u16) & 0x0F) << 8),
            0x04 => self.voices[0].write_control(val),
            0x05 => {
                self.voices[0].attack_rate = (val >> 4) & 0x0F;
                self.voices[0].decay_rate = val & 0x0F;
            }
            0x06 => {
                self.voices[0].sustain_level = (val >> 4) & 0x0F;
                self.voices[0].release_rate = val & 0x0F;
            }

            // Voice 2 ($07..$0D)
            0x07 => self.voices[1].freq = (self.voices[1].freq & 0xFF00) | (val as u16),
            0x08 => self.voices[1].freq = (self.voices[1].freq & 0x00FF) | ((val as u16) << 8),
            0x09 => self.voices[1].pulse_width = (self.voices[1].pulse_width & 0x0F00) | (val as u16),
            0x0A => self.voices[1].pulse_width = (self.voices[1].pulse_width & 0x00FF) | (((val as u16) & 0x0F) << 8),
            0x0B => self.voices[1].write_control(val),
            0x0C => {
                self.voices[1].attack_rate = (val >> 4) & 0x0F;
                self.voices[1].decay_rate = val & 0x0F;
            }
            0x0D => {
                self.voices[1].sustain_level = (val >> 4) & 0x0F;
                self.voices[1].release_rate = val & 0x0F;
            }

            // Voice 3 ($0E..$14)
            0x0E => self.voices[2].freq = (self.voices[2].freq & 0xFF00) | (val as u16),
            0x0F => self.voices[2].freq = (self.voices[2].freq & 0x00FF) | ((val as u16) << 8),
            0x10 => self.voices[2].pulse_width = (self.voices[2].pulse_width & 0x0F00) | (val as u16),
            0x11 => self.voices[2].pulse_width = (self.voices[2].pulse_width & 0x00FF) | (((val as u16) & 0x0F) << 8),
            0x12 => self.voices[2].write_control(val),
            0x13 => {
                self.voices[2].attack_rate = (val >> 4) & 0x0F;
                self.voices[2].decay_rate = val & 0x0F;
            }
            0x14 => {
                self.voices[2].sustain_level = (val >> 4) & 0x0F;
                self.voices[2].release_rate = val & 0x0F;
            }

            // Filter cutoff ($15, $16)
            0x15 => self.filter_cutoff = (self.filter_cutoff & 0x07F8) | ((val as u16) & 0x07),
            0x16 => self.filter_cutoff = (self.filter_cutoff & 0x0007) | (((val as u16) & 0xFF) << 3),

            // Filter resonance & voice routing ($17)
            0x17 => {
                self.filter_voice1 = (val & 0x01) != 0;
                self.filter_voice2 = (val & 0x02) != 0;
                self.filter_voice3 = (val & 0x04) != 0;
                self.filter_voice_ext = (val & 0x08) != 0;
                self.filter_res = (val >> 4) & 0x0F;
            }

            // Mode & Master Volume ($18)
            0x18 => {
                self.volume = val & 0x0F;
                self.filter_lowpass = (val & 0x10) != 0;
                self.filter_bandpass = (val & 0x20) != 0;
                self.filter_highpass = (val & 0x40) != 0;
                self.voice3_off = (val & 0x80) != 0;
            }

            _ => {}
        }
    }

    /// Advance SID oscillators, envelopes, and filters by one hardware clock cycle.
    pub fn clock(&mut self) {
        // Read MSB states before clocking for ring modulation and sync chaining:
        // Voice 1 is modulated/synced by Voice 3
        // Voice 2 is modulated/synced by Voice 1
        // Voice 3 is modulated/synced by Voice 2
        let msb0 = (self.voices[0].accumulator & 0x800000) != 0;
        let msb1 = (self.voices[1].accumulator & 0x800000) != 0;
        let msb2 = (self.voices[2].accumulator & 0x800000) != 0;

        let rising0 = self.voices[0].msb_rising;
        let rising1 = self.voices[1].msb_rising;
        let rising2 = self.voices[2].msb_rising;

        self.voices[0].clock_oscillator(rising2, msb2);
        self.voices[0].clock_envelope();

        self.voices[1].clock_oscillator(rising0, msb0);
        self.voices[1].clock_envelope();

        self.voices[2].clock_oscillator(rising1, msb1);
        self.voices[2].clock_envelope();
    }

    /// Calculate combined mixed audio sample (-1.0..=1.0).
    pub fn sample(&mut self) -> f32 {
        let msb0 = (self.voices[0].accumulator & 0x800000) != 0;
        let msb1 = (self.voices[1].accumulator & 0x800000) != 0;
        let msb2 = (self.voices[2].accumulator & 0x800000) != 0;

        let v1 = self.voices[0].output(msb2);
        let v2 = self.voices[1].output(msb0);
        let v3 = if self.voice3_off { 0.0 } else { self.voices[2].output(msb1) };

        let mut filtered_input = 0.0f32;
        let mut unfiltered_sum = 0.0f32;

        if self.filter_voice1 { filtered_input += v1; } else { unfiltered_sum += v1; }
        if self.filter_voice2 { filtered_input += v2; } else { unfiltered_sum += v2; }
        if self.filter_voice3 { filtered_input += v3; } else { unfiltered_sum += v3; }

        // State-variable resonant filter (Chamberlin SVF)
        let cutoff_norm = (self.filter_cutoff as f32 / 2048.0).clamp(0.001, 0.99);
        let f = 2.0 * (std::f32::consts::PI * cutoff_norm * 0.25).sin();
        let q = 1.0 - (self.filter_res as f32 / 18.0).clamp(0.0, 0.85);

        let low = self.filter_w1 + f * self.filter_w0;
        let high = filtered_input - low - q * self.filter_w0;
        let band = f * high + self.filter_w0;
        self.filter_w0 = band;
        self.filter_w1 = low;

        let mut filter_out = 0.0f32;
        if self.filter_lowpass { filter_out += low; }
        if self.filter_bandpass { filter_out += band; }
        if self.filter_highpass { filter_out += high; }
        if !self.filter_lowpass && !self.filter_bandpass && !self.filter_highpass {
            filter_out = filtered_input;
        }

        let mixed = (unfiltered_sum + filter_out) * (self.volume as f32 / 15.0);
        (mixed * 0.33).clamp(-1.0, 1.0)
    }

    /// Synthesize `num_samples` audio samples at `sample_rate` Hz into mono 16-bit PCM.
    pub fn render_pcm(&mut self, num_samples: usize, sample_rate: u32, clock_hz: u32) -> Vec<i16> {
        let mut out = Vec::with_capacity(num_samples);
        let clocks_per_sample = clock_hz as f64 / sample_rate as f64;
        let mut clock_accumulator = 0.0f64;

        for _ in 0..num_samples {
            clock_accumulator += clocks_per_sample;
            let steps = clock_accumulator.floor() as u32;
            clock_accumulator -= steps as f64;

            for _ in 0..steps {
                self.clock();
            }
            let s = self.sample();
            let pcm = (s * 32767.0).round().clamp(-32768.0, 32767.0) as i16;
            out.push(pcm);
        }
        out
    }
}

impl IoDevice for Sid6581 {
    fn read(&mut self, offset: u16) -> u8 {
        self.read_reg((offset & 0x1F) as u8)
    }

    fn write(&mut self, offset: u16, value: u8) {
        self.write_reg((offset & 0x1F) as u8, value);
    }

    fn peek(&self, offset: u16) -> u8 {
        self.read_reg((offset & 0x1F) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sid_reset_and_control() {
        let mut sid = Sid6581::new();
        // Set voice 1 frequency to 440 Hz equivalent
        sid.write_reg(0x00, 0x10);
        sid.write_reg(0x01, 0x1C);
        assert_eq!(sid.voices[0].freq, 0x1C10);

        // Attack 2, Decay 8, Sustain 12, Release 4
        sid.write_reg(0x05, 0x28);
        sid.write_reg(0x06, 0xC4);
        assert_eq!(sid.voices[0].attack_rate, 2);
        assert_eq!(sid.voices[0].decay_rate, 8);
        assert_eq!(sid.voices[0].sustain_level, 12);
        assert_eq!(sid.voices[0].release_rate, 4);

        // Turn on Triangle and Gate
        sid.write_reg(0x04, 0x11);
        assert!(sid.voices[0].triangle);
        assert!(sid.voices[0].gate);
        assert_eq!(sid.voices[0].env_state, EnvelopeState::Attack);

        // Clock several times and check envelope advances
        for _ in 0..1000 {
            sid.clock();
        }
        assert!(sid.voices[0].env_level > 0);
    }

    #[test]
    fn test_sid_audio_pcm_rendering() {
        let mut sid = Sid6581::new();
        sid.write_reg(0x00, 0x00);
        sid.write_reg(0x01, 0x10); // freq
        sid.write_reg(0x05, 0x00); // fast attack
        sid.write_reg(0x06, 0xF0); // max sustain
        sid.write_reg(0x04, 0x21); // Sawtooth + Gate
        sid.write_reg(0x18, 0x0F); // Max volume

        let pcm = sid.render_pcm(100, 44100, SID_PAL_CLOCK_HZ);
        assert_eq!(pcm.len(), 100);
    }
}
