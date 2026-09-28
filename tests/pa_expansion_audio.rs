//! Expansion audio: Sunsoft 5B (inside FME-7, mapper 69), Namco 163 wavetable
//! (mapper 19) and VRC6 (mappers 24/26) with its board, plus the APU mixing hook
//! and version-6 snapshots. Every expected value is derived from the documented
//! register behaviour, not read back from the model.

use mos6502::apu::{mix, mix_expansion};
use mos6502::ines::MAGIC;
use mos6502::mapper::{
    Cartridge, Fme7, N163Audio, Namco163, PpuTarget, Sunsoft5B, Vrc6, APU_PULSE_FULL,
};
use mos6502::systems::nes2a03::Nes2A03;

fn prg(banks8: usize) -> Vec<u8> {
    (0..banks8 * 0x2000).map(|i| (i / 0x2000) as u8).collect()
}

fn reg(b: &mut Sunsoft5B, r: u8, v: u8) {
    b.select = r;
    b.write_register(v);
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

// ---------------------------------------------------------------- Sunsoft 5B

#[test]
fn b5_tone_flips_every_16_p_cycles() {
    for p in [0u16, 1, 2, 3, 7, 0x10, 0x123, 0xFFF] {
        let mut b = Sunsoft5B::default();
        reg(&mut b, 0, p as u8);
        reg(&mut b, 1, (p >> 8) as u8 | 0xF0); // high nibble only
        assert_eq!(b.tone_period(0), p.max(1));
        let half = 16 * u32::from(p.max(1));
        let mut flips = vec![];
        let mut last = b.tone_out[0];
        for c in 1..=half * 4 {
            b.clock();
            if b.tone_out[0] != last {
                flips.push(c);
                last = b.tone_out[0];
            }
        }
        let want: Vec<u32> = (1..=4).map(|k| k * half).collect();
        assert_eq!(flips, want, "P={p}");
    }
}

#[test]
fn b5_dac_volume_and_mixer_gates() {
    assert_eq!(Sunsoft5B::dac(0), 0.0);
    assert!(close(Sunsoft5B::dac(31), 1.0));
    for l in 1..31u8 {
        let ratio = Sunsoft5B::dac(l) / Sunsoft5B::dac(l + 1);
        assert!(
            close(ratio, 10f32.powf(-1.5 / 20.0)),
            "1.5 dB per level {l}"
        );
    }
    let mut b = Sunsoft5B::default();
    reg(&mut b, 7, 0x3F); // all tone and noise disabled: channels sound constantly
    for v in 0..16u8 {
        reg(&mut b, 8, v);
        assert_eq!(b.channel_level(0), if v == 0 { 0 } else { 2 * v + 1 });
    }
    reg(&mut b, 8, 15);
    for mixer in 0..64u8 {
        reg(&mut b, 7, mixer);
        for tone in [false, true] {
            b.tone_out[0] = tone;
            let noise = b.lfsr & 1 != 0;
            let on = (tone || mixer & 1 != 0) && (noise || mixer & 8 != 0);
            assert_eq!(
                b.channel_level(0),
                if on { 31 } else { 0 },
                "{mixer:02X} {tone}"
            );
        }
    }
    reg(&mut b, 7, 0x3F);
    for v in [8u8, 9, 10] {
        reg(&mut b, v, 15);
    }
    assert!(close(b.output(), 3.0 * APU_PULSE_FULL));
    b.gain = 0.5;
    assert!(close(b.output(), 1.5 * APU_PULSE_FULL));
}

#[test]
fn b5_noise_lfsr_shifts_every_32_n_cycles() {
    for n in [0u8, 1, 5, 31] {
        let mut b = Sunsoft5B::default();
        reg(&mut b, 6, n | 0xE0);
        let mut want = 1u32;
        let step = 32 * u32::from((n & 0x1F).max(1));
        for k in 1..=40u32 {
            for _ in 0..step {
                b.clock();
            }
            want = (want >> 1) | (((want ^ (want >> 3)) & 1) << 16);
            assert_eq!(b.lfsr, want, "N={n} shift {k}");
        }
    }
    // The 17-bit LFSR with taps 0 and 3 is maximal: period 2^17 - 1.
    let (mut x, mut len) = (1u32, 0u32);
    loop {
        x = (x >> 1) | (((x ^ (x >> 3)) & 1) << 16);
        len += 1;
        if x == 1 {
            break;
        }
    }
    assert_eq!(len, (1 << 17) - 1);
}

/// Level sequence (one value per envelope step) of each of the 16 AY shapes,
/// written from the datasheet's pictures for 32-step envelopes.
fn shape_reference(shape: u8, steps: usize) -> Vec<u8> {
    let down: Vec<u8> = (0..32).rev().collect();
    let up: Vec<u8> = (0..32).collect();
    let mut out = vec![];
    let seq: Box<dyn Fn(usize) -> u8> = match shape {
        0..=3 | 9 => Box::new(move |i| if i < 32 { down[i] } else { 0 }),
        4..=7 | 15 => Box::new(move |i| if i < 32 { up[i] } else { 0 }),
        8 => Box::new(move |i| down[i % 32]),
        10 => Box::new(move |i| {
            if (i / 32) % 2 == 0 {
                31 - (i % 32) as u8
            } else {
                (i % 32) as u8
            }
        }),
        11 => Box::new(move |i| if i < 32 { down[i] } else { 31 }),
        12 => Box::new(move |i| up[i % 32]),
        13 => Box::new(move |i| if i < 32 { up[i] } else { 31 }),
        14 => Box::new(move |i| {
            if (i / 32) % 2 == 0 {
                (i % 32) as u8
            } else {
                31 - (i % 32) as u8
            }
        }),
        _ => unreachable!(),
    };
    for i in 0..steps {
        out.push(seq(i));
    }
    out
}

#[test]
fn b5_envelope_all_sixteen_shapes() {
    for shape in 0..16u8 {
        for period in [1u16, 3] {
            let mut b = Sunsoft5B::default();
            reg(&mut b, 11, period as u8);
            reg(&mut b, 12, 0);
            reg(&mut b, 13, shape);
            let mut got = vec![b.envelope_level()];
            for _ in 1..100 {
                for _ in 0..16 * u32::from(period) {
                    b.clock();
                }
                got.push(b.envelope_level());
            }
            assert_eq!(got, shape_reference(shape, 100), "shape {shape} E={period}");
        }
    }
    // Envelope mode on a channel follows the envelope level; rewriting D restarts.
    let mut b = Sunsoft5B::default();
    reg(&mut b, 7, 0x3F);
    reg(&mut b, 8, 0x10);
    reg(&mut b, 13, 8);
    assert_eq!(b.channel_level(0), 31);
    reg(&mut b, 11, 1);
    for _ in 0..16 * 5 {
        b.clock();
    }
    assert_eq!(b.channel_level(0), 26);
    reg(&mut b, 13, 8);
    assert_eq!(b.channel_level(0), 31);
}

#[test]
fn fme7_ports_drive_the_5b() {
    let mut c = Fme7::new(prg(8)).unwrap();
    for r in 0..16u8 {
        c.write(0xC000 | (u16::from(r) * 0x101), r | 0xF0);
        c.write(0xE000 + u16::from(r), r.wrapping_mul(17));
    }
    for r in 0..16usize {
        assert_eq!(c.audio.regs[r], (r as u8).wrapping_mul(17));
    }
    let mut c = Fme7::new(prg(8)).unwrap();
    assert_eq!(c.audio(), 0.0);
    c.write(0xC000, 7);
    c.write(0xE000, 0x3F);
    c.write(0xC000, 9);
    c.write(0xE000, 15);
    assert!(close(c.audio(), APU_PULSE_FULL));
    // The 5B shares the FME-7 clock: CPU cycles advance it.
    c.write(0xC000, 7);
    c.write(0xE000, 0x3E);
    c.write(0xC000, 8);
    c.write(0xE000, 15);
    let before = c.audio.tone_out[0];
    for _ in 0..16 {
        c.clock();
    }
    assert_ne!(c.audio.tone_out[0], before);
}

// ---------------------------------------------------------------- Namco 163

fn set_channel(ram: &mut [u8; 128], c: usize, freq: u32, phase: u32, len: u32, addr: u8, vol: u8) {
    let b = 0x40 + 8 * c;
    ram[b] = freq as u8;
    ram[b + 2] = (freq >> 8) as u8;
    ram[b + 4] = ((256 - len) as u8 & 0xFC) | ((freq >> 16) as u8 & 3);
    ram[b + 1] = phase as u8;
    ram[b + 3] = (phase >> 8) as u8;
    ram[b + 5] = (phase >> 16) as u8;
    ram[b + 6] = addr;
    ram[b + 7] = (ram[b + 7] & 0xF0) | vol;
}

#[test]
fn n163_channel_update_matches_the_formula() {
    for (freq, len, addr, vol) in [
        (0x10000u32, 32u32, 0u8, 15u8),
        (0x3FFFF, 4, 0x10, 7),
        (0x08000, 256, 0x81, 1),
        (0x00001, 16, 0xFF, 15),
    ] {
        let mut ram = [0u8; 128];
        for (i, b) in ram.iter_mut().enumerate().take(0x40) {
            *b = (i as u8).wrapping_mul(37);
        }
        set_channel(&mut ram, 7, freq, 0, len, addr, vol);
        let mut phase = 0u32;
        for _ in 0..300 {
            let got = N163Audio::update(&mut ram, 7);
            phase = (phase + freq) % (len << 16);
            let a = ((phase >> 16) + u32::from(addr)) & 0xFF;
            let byte = ram[(a >> 1) as usize];
            let s = if a & 1 == 0 { byte & 15 } else { byte >> 4 };
            assert_eq!(got, (i16::from(s) - 8) * i16::from(vol));
            let stored =
                u32::from(ram[0x79]) | (u32::from(ram[0x7B]) << 8) | (u32::from(ram[0x7D]) << 16);
            assert_eq!(stored, phase, "phase written back");
        }
    }
}

#[test]
fn n163_time_multiplexes_one_channel_per_15_cycles() {
    for n in 1..=8u8 {
        let mut c = Namco163::new(prg(8)).unwrap();
        for ch in 0..8 {
            // Constant wave per channel: every nibble = channel number + 1, volume 1.
            set_channel(&mut c.sound_ram, ch, 0, 0, 4, 2 * ch as u8, 1);
            c.sound_ram[ch] = (ch as u8 + 1) * 0x11;
        }
        c.sound_ram[0x7F] = ((n - 1) << 4) | 1;
        let mut order = vec![];
        for k in 1..=15 * 2 * u32::from(n) {
            c.clock();
            if k % 15 == 0 {
                order.push(c.audio.current);
            } else {
                assert_eq!(c.audio.divider as u32, k % 15);
            }
        }
        let expect: Vec<i16> = (0..2)
            .flat_map(|_| (8 - n..8).rev())
            .map(|ch| i16::from(ch) + 1 - 8)
            .collect();
        assert_eq!(order, expect, "N={n}");
    }
}

#[test]
fn n163_disable_bit_gain_and_submappers() {
    let mut c = Namco163::new(prg(8)).unwrap();
    set_channel(&mut c.sound_ram, 7, 0, 0, 4, 0, 15);
    c.sound_ram[0] = 0xFF; // sample 15 -> +7 * 15
    for _ in 0..15 {
        c.clock();
    }
    assert_eq!(c.audio.current, 105);
    let want = 105.0 * APU_PULSE_FULL * 10f32.powf(12.0 / 20.0) / 120.0;
    assert!(close(c.audio(), want));
    c.write(0xE000, 0x40);
    c.clock();
    assert_eq!((c.audio.current, c.audio()), (0, 0.0), "sound disable");
    assert_eq!(N163Audio::db_for_submapper(2), None);
    assert_eq!(N163Audio::db_for_submapper(3), Some(12.0));
    assert_eq!(N163Audio::db_for_submapper(4), Some(16.5));
    assert_eq!(N163Audio::db_for_submapper(5), Some(18.75));
    assert_eq!(N163Audio::db_for_submapper(0), Some(12.0));
    c.audio.db = None;
    c.write(0xE000, 0);
    for _ in 0..15 {
        c.clock();
    }
    assert_eq!(c.audio.current, 105);
    assert_eq!(c.audio(), 0.0, "submapper 2 boards have no sound");
}

// ---------------------------------------------------------------- VRC6

fn vrc6(swapped: bool) -> Vrc6 {
    Vrc6::new(prg(16), swapped).unwrap()
}

/// CPU address of VRC6 register `$x000 + r` on the given board.
fn at(swapped: bool, base: u16, r: u16) -> u16 {
    if swapped {
        base | ((r & 1) << 1) | (r >> 1)
    } else {
        base | r
    }
}

#[test]
fn vrc6_pulse_duty_period_mode_and_enable() {
    for sw in [false, true] {
        for duty in 0..8u8 {
            for period in [0u16, 1, 5, 0x123] {
                let mut c = vrc6(sw);
                c.write(at(sw, 0x9000, 0), (duty << 4) | 9);
                c.write(at(sw, 0x9000, 1), period as u8);
                c.write(at(sw, 0x9000, 2), 0x80 | (period >> 8) as u8);
                let mut wave = vec![];
                for _ in 0..32 {
                    for _ in 0..=period {
                        c.clock();
                    }
                    wave.push(c.pulse[0].output());
                }
                let high = wave.iter().filter(|&&v| v == 9).count();
                assert_eq!(
                    high,
                    2 * (usize::from(duty) + 1),
                    "sw {sw} duty {duty} P {period}"
                );
                assert!(wave.iter().all(|&v| v == 0 || v == 9));
            }
        }
        let mut c = vrc6(sw);
        c.write(at(sw, 0xA000, 0), 0x80 | 6);
        c.write(at(sw, 0xA000, 2), 0x80);
        for _ in 0..40 {
            c.clock();
            assert_eq!(c.pulse[1].output(), 6, "mode bit: constant volume");
        }
        c.write(at(sw, 0xA000, 2), 0);
        assert_eq!(
            (c.pulse[1].output(), c.pulse[1].step),
            (0, 15),
            "disable resets duty"
        );
    }
}

#[test]
fn vrc6_sawtooth_accumulates_six_times_then_resets() {
    for rate in [0u8, 1, 7, 42, 0x3F] {
        let mut c = vrc6(false);
        c.write(0xB000, rate | 0xC0);
        c.write(0xB001, 2);
        c.write(0xB002, 0x80);
        let mut acc = 0u8;
        let mut step = 0u8;
        for _ in 0..60 {
            for _ in 0..3 {
                c.clock();
            }
            step += 1;
            if step == 14 {
                step = 0;
                acc = 0;
            } else if step % 2 == 0 {
                acc = acc.wrapping_add(rate);
            }
            assert_eq!(c.saw.acc, acc, "rate {rate}");
            assert_eq!(c.saw.output(), acc >> 3);
        }
    }
    let mut c = vrc6(false);
    c.write(0xB000, 42);
    c.write(0xB002, 0x80);
    for _ in 0..4 {
        c.clock();
    }
    assert!(c.saw.acc > 0);
    c.write(0xB002, 0);
    c.clock();
    assert_eq!(c.saw.acc, 0, "disabled saw is cleared");
}

#[test]
fn vrc6_frequency_control_halt_and_shifts() {
    for (ctrl, shift) in [(0u8, 0u8), (2, 4), (4, 8), (6, 8)] {
        let mut c = vrc6(false);
        c.write(0x9003, ctrl);
        c.write(0x9000, 0x80 | 1);
        c.write(0x9001, 0x00);
        c.write(0x9002, 0x81); // period 0x100
        let period = 0x100u32 >> shift;
        let mut steps = 0;
        for _ in 0..(period + 1) * 5 {
            let s = c.pulse[0].step;
            c.clock();
            if c.pulse[0].step != s {
                steps += 1;
            }
        }
        assert_eq!(steps, 5, "ctrl {ctrl}");
    }
    let mut c = vrc6(false);
    c.write(0x9003, 1);
    c.write(0x9002, 0x80);
    c.write(0xB002, 0x80);
    c.write(0xB000, 9);
    let before = (
        c.pulse[0].step,
        c.pulse[0].divider,
        c.saw.step,
        c.saw.divider,
        c.saw.acc,
    );
    for _ in 0..100 {
        c.clock();
    }
    let after = (
        c.pulse[0].step,
        c.pulse[0].divider,
        c.saw.step,
        c.saw.divider,
        c.saw.acc,
    );
    assert_eq!(after, before, "halt freezes all three");
    c.write(0x9003, 0);
    for _ in 0..100 {
        c.clock();
    }
    assert_ne!(c.pulse[0].step, before.0, "running again");
}

#[test]
fn vrc6_mixing_level() {
    let mut c = vrc6(false);
    c.write(0x9000, 0x8F);
    c.write(0x9002, 0x80);
    c.write(0xA000, 0x8F);
    c.write(0xA002, 0x80);
    c.saw.acc = 0xF8;
    c.write(0xB002, 0x80);
    assert_eq!(c.level(), 15 + 15 + 31);
    assert!(close(c.audio(), 61.0 * APU_PULSE_FULL / 15.0));
}

#[test]
fn vrc6_banking_mirroring_ram_and_wiring() {
    for sw in [false, true] {
        let mut c = vrc6(sw);
        for b in 0..16u8 {
            c.write(at(sw, 0x8000, 0), b | 0xF0);
            assert_eq!(c.peek(0x8000), (b * 2) % 16);
            assert_eq!(c.peek(0xBFFF), (b * 2 + 1) % 16);
        }
        for b in 0..32u8 {
            c.write(at(sw, 0xC000, 3), b | 0xE0);
            assert_eq!(c.peek(0xC000), b % 16);
        }
        assert_eq!((c.peek(0xE000), c.peek(0xFFFF)), (15, 15));
        c.write(0x6000, 0x55);
        assert_eq!(c.peek(0x6000), 0);
        c.write(at(sw, 0xB000, 3), 0x80);
        c.write(0x6000, 0x55);
        assert_eq!(c.peek(0x6000), 0x55);
        // $B003 bit 5 set (all commercial games): mode 0 mirroring from bits 3-2.
        for (bits, pages) in [
            (0u8, [0, 1, 0, 1]),
            (4, [0, 0, 1, 1]),
            (8, [0; 4]),
            (12, [1; 4]),
        ] {
            c.write(at(sw, 0xB000, 3), 0xA0 | bits);
            for (q, &page) in pages.iter().enumerate() {
                let addr = 0x2000 + 0x400 * q as u16 + 0x15;
                assert_eq!(
                    c.ppu_target(addr),
                    PpuTarget::Ciram(page * 0x400 + 0x15),
                    "{bits} {q}"
                );
            }
        }
        for r in 0..8u16 {
            c.write(
                at(sw, if r < 4 { 0xD000 } else { 0xE000 }, r & 3),
                0x10 + r as u8,
            );
        }
        assert_eq!(c.chr, [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17]);
        for a in (0..0x2000u16).step_by(0x155) {
            let slot = usize::from(a >> 10);
            let r = |i: usize| 0x10 + i;
            let off = usize::from(a & 0x3FF);
            for b5 in [0u8, 0x20] {
                // Registers count 1 KiB pages; 2 KiB banks take A10 from the PPU
                // with bit 5 set and from the register LSB with it clear.
                let two_k = |reg: usize| {
                    if b5 != 0 {
                        (r(reg) & !1) | (slot & 1)
                    } else {
                        r(reg)
                    }
                };
                c.write(at(sw, 0xB000, 3), 0x80 | b5);
                assert_eq!(c.chr_offset(a), r(slot) * 0x400 + off);
                c.write(at(sw, 0xB000, 3), 0x81 | b5);
                assert_eq!(c.chr_offset(a), two_k(slot / 2) * 0x400 + off);
                for mode in [0x82, 0x83] {
                    c.write(at(sw, 0xB000, 3), mode | b5);
                    let page = if slot < 4 {
                        r(slot)
                    } else {
                        two_k(4 + (slot - 4) / 2)
                    };
                    assert_eq!(c.chr_offset(a), page * 0x400 + off);
                }
            }
        }
    }
    // A0/A1 swap: $9001 on VRC6a is $9002 on VRC6b.
    let mut a = vrc6(false);
    let mut b = vrc6(true);
    a.write(0x9001, 0x34);
    b.write(0x9001, 0x34);
    assert_eq!((a.pulse[0].regs[1], b.pulse[0].regs[2]), (0x34, 0x34));
}

#[test]
fn vrc6_cycle_irq() {
    for latch in [0xFEu8, 0x80, 0x00] {
        let mut c = vrc6(false);
        c.write(0xF000, latch);
        c.write(0xF001, 0x06); // enable, cycle mode
        let mut k = 0u32;
        while !c.irq() {
            c.clock();
            k += 1;
            assert!(k < 1000);
        }
        assert_eq!(k, 256 - u32::from(latch));
        c.write(0xF002, 0);
        assert!(!c.irq());
    }
}

// ---------------------------------------------------------------- machine

fn image(mapper: u8, sub: u8, battery: bool) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([
        8,
        4,
        ((mapper & 15) << 4) | (u8::from(battery) << 1),
        (mapper & 0xF0) | 8,
        sub << 4,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]);
    let mut p = prg(16);
    let n = p.len();
    // Reset vector at $8000 and an endless JMP there.
    p[..3].copy_from_slice(&[0x4C, 0x00, 0x80]);
    p[n - 4..].copy_from_slice(&[0x00, 0x80, 0x00, 0x80]);
    f.extend(p);
    f.extend(vec![0; 4 * 0x2000]);
    f
}

#[test]
fn machine_mixes_expansion_audio_on_top_of_the_apu() {
    for m in [69u8, 19, 24, 26] {
        let mut nes = Nes2A03::from_ines(&image(m, 0, false)).unwrap();
        {
            let c = nes.cpu.bus.cart.as_mut().unwrap();
            match m {
                69 => {
                    c.write(0xC000, 7);
                    c.write(0xE000, 0x3F);
                    c.write(0xC000, 8);
                    c.write(0xE000, 12);
                }
                19 => {
                    c.write(0xF800, 0x80 | 0x78);
                    for v in [0, 0, 0, 0, 0xFC, 0, 0, 15] {
                        c.write(0x4800, v);
                    }
                    c.write(0xF800, 0x80);
                    c.write(0x4800, 0xFF);
                }
                _ => {
                    c.write(0x9000, 0x8F);
                    c.write(at(m == 26, 0x9000, 2), 0x80);
                }
            }
        }
        let samples = nes.render_audio(30, 50).unwrap();
        assert_eq!(samples.len(), 50);
        let ext = nes.cpu.bus.cart.as_ref().unwrap().audio();
        assert!(ext > 0.0, "mapper {m}");
        assert!(close(
            nes.audio_sample(),
            mix_expansion(nes.cpu.bus.apu.sample(), ext)
        ));
        assert!(close(
            nes.audio_sample(),
            mix(nes.cpu.bus.apu.channel_outputs()) + ext
        ));
    }
    let mut nes = Nes2A03::from_ines(&image(2, 0, false)).unwrap();
    nes.run_cycles(100).unwrap();
    assert_eq!(
        nes.audio_sample(),
        nes.cpu.bus.apu.sample(),
        "UxROM adds nothing"
    );
    for (sub, db) in [
        (0u8, Some(12.0)),
        (2, None),
        (3, Some(12.0)),
        (4, Some(16.5)),
        (5, Some(18.75)),
    ] {
        let mut nes = Nes2A03::from_ines(&image(19, sub, false)).unwrap();
        let c = nes.cpu.bus.cart.as_mut().unwrap();
        c.write(0xF800, 0x80 | 0x78);
        for v in [0, 0, 0, 0, 0xFC, 0, 0, 15] {
            c.write(0x4800, v);
        }
        c.write(0xF800, 0x80);
        c.write(0x4800, 0xFF);
        nes.run_cycles(40).unwrap();
        let ext = nes.cpu.bus.cart.as_ref().unwrap().audio();
        match db {
            None => assert_eq!(ext, 0.0, "submapper 2: no expansion sound"),
            Some(db) => assert!(
                close(ext, 105.0 * APU_PULSE_FULL * 10f32.powf(db / 20.0) / 120.0),
                "sub {sub}"
            ),
        }
    }
}

#[test]
fn audio_state_round_trips_and_v5_still_loads() {
    for m in [69u8, 19, 24, 26] {
        let data = image(m, 0, false);
        let mut a = Nes2A03::from_ines(&data).unwrap();
        {
            let c = a.cpu.bus.cart.as_mut().unwrap();
            let writes: &[(u16, u8)] = match m {
                69 => &[
                    (0xC000, 7),
                    (0xE000, 0x30),
                    (0xC000, 13),
                    (0xE000, 14),
                    (0xC000, 11),
                    (0xE000, 2),
                    (0xC000, 8),
                    (0xE000, 0x1F),
                ],
                19 => &[
                    (0xF800, 0xFF),
                    (0x4800, 0x70),
                    (0xF800, 0xF8),
                    (0x4800, 0x40),
                    (0x4800, 0),
                    (0x4800, 1),
                ],
                _ => &[
                    (0x9000, 0x35),
                    (0xA000, 0x47),
                    (0xB000, 5),
                    (0xB001, 3),
                    (0xB002, 0x80),
                    (0xB001, 3),
                ],
            };
            for &(addr, v) in writes {
                c.write(addr, v);
            }
            c.write(at(m == 26, 0x9000, 2), 0x81);
            c.write(at(m == 26, 0xA000, 2), 0x80);
        }
        a.run_cycles(12_345).unwrap();
        let s6 = a.save_state();
        let mut b = Nes2A03::from_ines(&data).unwrap();
        b.load_state(&s6).unwrap();
        a.run_cycles(20_000).unwrap();
        b.run_cycles(20_000).unwrap();
        assert_eq!(a.save_state(), b.save_state(), "mapper {m}");
        assert_eq!(a.audio_sample(), b.audio_sample());
        if m == 69 || m == 19 {
            let s5 = a.save_state_version(5);
            assert!(s5.len() < a.save_state().len());
            let mut c = Nes2A03::from_ines(&data).unwrap();
            c.load_state(&s5).unwrap();
        }
    }
    // A zero LFSR is impossible and refused.
    let data = image(69, 0, false);
    let a = Nes2A03::from_ines(&data).unwrap();
    let good = a.save_state();
    let mut found = false;
    for i in 0..good.len() - 4 {
        if good[i..i + 4] == [1, 0, 0, 0] {
            let mut bad = good.clone();
            bad[i] = 0;
            let mut b = Nes2A03::from_ines(&data).unwrap();
            if b.load_state(&bad).is_err() {
                found = true;
                assert_eq!(b.save_state(), good, "refused atomically");
            }
        }
    }
    assert!(found);
}
