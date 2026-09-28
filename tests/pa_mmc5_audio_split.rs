//! MMC5 expansion audio (two sweepless pulses on a fixed 240 Hz frame clock,
//! 8-bit PCM with write/read modes and its IRQ) and vertical split mode rendered
//! through the PPU.

use mos6502::mapper::{
    Cartridge, Mmc5, Mmc5Pulse, APU_PULSE_FULL, MMC5_FRAME_CYCLES, MMC5_LENGTHS,
};
use mos6502::ppu::{Mirroring, Ppu, PpuMemory, HEIGHT, WIDTH};

fn mmc5() -> Mmc5 {
    Mmc5::new((0..0x40000).map(|i| (i >> 13) as u8 | 1).collect()).unwrap()
}

const APU_LENGTHS: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

#[test]
fn length_table_differs_from_apu_only_at_1c() {
    // The APU table (NESdev) with $1C = $10; MMC5 lacks bit 0 and that's the only
    // entry that changes.
    for (i, (&m, &a)) in MMC5_LENGTHS.iter().zip(APU_LENGTHS.iter()).enumerate() {
        assert_eq!(m, a & !1 | if i == 0x1C { 0 } else { a & 1 }, "{i:02X}");
    }
    assert_eq!(MMC5_LENGTHS[0x1C], 0x10);
    assert_eq!(MMC5_FRAME_CYCLES, 7457);
}

fn period_of(c: &mut Mmc5, ch: usize) -> u32 {
    // Cycles between rising edges of the output.
    let mut last = c.pulse[ch].output();
    let mut edges = vec![];
    for t in 0..20_000u32 {
        c.clock();
        let o = c.pulse[ch].output();
        if o > last {
            edges.push(t);
        }
        last = o;
        if edges.len() == 3 {
            break;
        }
    }
    edges[2] - edges[1]
}

#[test]
fn pulse_period_duty_and_no_sweep_or_low_period_mute() {
    for ch in 0..2usize {
        let base = 0x5000 + 4 * ch as u16;
        for period in [0u16, 3, 7, 8, 100, 0x7FF] {
            let mut c = mmc5();
            c.write(0x5015, 1 << ch);
            c.write(base, 0xBF); // duty 2, halt, constant 15
            c.write(base + 1, 0xFF); // no sweep unit: ignored
            c.write(base + 2, period as u8);
            c.write(base + 3, (period >> 8) as u8);
            assert_eq!(c.pulse[ch].period, period);
            if period < 100 {
                assert_eq!(
                    period_of(&mut c, ch),
                    16 * (u32::from(period) + 1),
                    "ch {ch} p {period}"
                );
            }
            // Periods under 8 still sound.
            let high: u32 = (0..70_000)
                .map(|_| {
                    c.clock();
                    u32::from(c.pulse[ch].output() > 0)
                })
                .sum();
            assert!(high > 0, "p {period}");
        }
        for (duty, want) in [(0u8, 1u32), (1, 2), (2, 4), (3, 6)] {
            let mut c = mmc5();
            c.write(0x5015, 3);
            c.write(base, (duty << 6) | 0x3F);
            c.write(base + 2, 0);
            c.write(base + 3, 0);
            let high: u32 = (0..16 * 8)
                .map(|_| {
                    c.clock();
                    u32::from(c.pulse[ch].output() > 0)
                })
                .sum();
            assert_eq!(high, want * 16, "duty {duty}");
        }
    }
}

#[test]
fn envelope_and_length_at_240_hz_and_status() {
    let mut c = mmc5();
    c.write(0x5015, 3);
    c.write(0x5000, 0x02); // envelope period 3, length counting
    c.write(0x5003, 0x08); // length index 1 = 254
    c.write(0x5004, 0x10);
    c.write(0x5007, 0x18); // index 3 = 2
    assert_eq!(c.peek(0x5015), 3);
    let p = |c: &Mmc5| (c.pulse[0].env_decay, c.pulse[0].length, c.pulse[1].length);
    for _ in 0..MMC5_FRAME_CYCLES - 1 {
        c.clock();
    }
    assert_eq!(p(&c), (0, 254, 2));
    c.clock();
    assert_eq!(
        p(&c),
        (15, 253, 1),
        "first frame clock: start flag, length -1"
    );
    for _ in 0..MMC5_FRAME_CYCLES {
        c.clock();
    }
    assert_eq!(p(&c).2, 0);
    assert_eq!(c.peek(0x5015), 1);
    for _ in 0..12u32 {
        for _ in 0..MMC5_FRAME_CYCLES {
            c.clock();
        }
    }
    assert_eq!(
        c.pulse[0].env_decay, 11,
        "volume 2: decays on clocks 4, 7, 10, 13"
    );
    c.write(0x5015, 0);
    assert_eq!(c.peek(0x5015), 0);
    c.write(0x5003, 0xF8);
    assert_eq!(
        c.pulse[0].length, 0,
        "disabled channel ignores length loads"
    );
    assert_eq!(Mmc5Pulse::default().output(), 0);
}

#[test]
fn pcm_write_mode_read_mode_and_irq() {
    let mut c = mmc5();
    assert_eq!(c.peek(0x5010), 0x01, "MMC5A revision bit");
    c.write(0x5011, 0x80);
    assert_eq!(c.pcm, 0x80);
    c.write(0x5011, 0);
    assert_eq!(c.pcm, 0x80, "$00 leaves the DAC alone");
    assert!(!c.irq(), "IRQ needs $5010 bit 7");
    c.write(0x5010, 0x80);
    assert!(c.irq());
    assert_eq!(c.read(0x5010), 0x81);
    assert!(!c.irq(), "read acknowledges");
    assert_eq!(c.read(0x5010), 0x01);
    // Read mode: $8000-$BFFF reads feed the DAC, direct writes are ignored.
    c.write(0x5114, 0x83);
    c.write(0x5010, 0x81);
    c.write(0x5011, 0x22);
    assert_eq!(c.pcm, 0x80);
    let v = c.read(0x8000);
    assert_eq!(c.pcm, v);
    c.write(0x5115, 0x86);
    let before = c.pcm;
    c.read(0xC000);
    assert_eq!(c.pcm, before, "$C000+ does not feed the DAC");
    // A zero read trips the IRQ.
    c.write(0x5100, 3);
    c.write(0x5114, 0x00); // RAM bank with zeros at $8000
    c.read(0x8000);
    assert!(c.irq());
    // The trip flag clears on the next non-zero value too.
    c.write(0x5010, 0x80);
    c.write(0x5011, 5);
    assert!(!c.irq());
}

#[test]
fn audio_is_inverted_and_scaled_like_the_apu() {
    let mut c = mmc5();
    assert_eq!(c.audio(), 0.0);
    c.write(0x5015, 3);
    for base in [0x5000u16, 0x5004] {
        c.write(base, 0xFF);
        c.write(base + 3, 0x08);
    }
    let mut peak = 0f32;
    for _ in 0..64 {
        c.clock();
        peak = peak.min(c.audio());
    }
    assert!(
        (peak + 2.0 * APU_PULSE_FULL).abs() < 1e-6,
        "two full pulses, negative"
    );
    let mut c = mmc5();
    c.write(0x5011, 127);
    let dmc = mos6502::apu::mix([0, 0, 0, 0, 127]);
    assert!((c.audio() + dmc).abs() < 1e-6);
    c.write(0x5011, 254);
    assert!(
        (c.audio() + 2.0 * dmc).abs() < 1e-5,
        "the 8th bit doubles it"
    );
}

/// Render `frames` frames with the split configured and return the framebuffer.
fn render_split(ctrl: u8, scroll: u8) -> Vec<u8> {
    let mut c = mmc5();
    let mut ppu = Ppu::new();
    let mut chr = vec![0u8; 0x2000];
    for tile in 0..32usize {
        for k in 0..8usize {
            chr[0x1000 + tile * 16 + k] = if (tile * 8 + k) % 2 == 1 { 0xFF } else { 0 };
        }
    }
    c.write(0x5104, 0);
    for r in 0..30usize {
        for col in 0..32usize {
            c.exram[r * 32 + col] = r as u8;
        }
    }
    c.write(0x5105, 0x00);
    c.write(0x5200, ctrl);
    c.write(0x5201, scroll);
    c.write(0x5202, 1);
    {
        let mut m = PpuMemory {
            cart: Some(&mut c),
            chr: &mut chr,
            chr_writable: true,
            mirroring: Mirroring::Vertical,
        };
        ppu.mem_write(&mut m, 0x3F00, 0x0F);
        ppu.mem_write(&mut m, 0x3F01, 0x30);
    }
    ppu.mask = 0x1E;
    ppu.oam = [0xFF; 256];
    c.snoop_ppu_register(0x2001, 0x1E);
    for d in 0..341 * 262 * 3 {
        {
            let mut m = PpuMemory {
                cart: Some(&mut c),
                chr: &mut chr,
                chr_writable: true,
                mirroring: Mirroring::Vertical,
            };
            ppu.tick(&mut m);
        }
        if d % 3 == 2 {
            c.clock();
        }
    }
    ppu.framebuffer.clone()
}

#[test]
fn vertical_split_left_and_right_with_scroll() {
    for (ctrl, scroll) in [
        (0x84u8, 0u8),
        (0x84, 13),
        (0x80 | 0x40 | 20, 0),
        (0xC0 | 9, 100),
        (0x80 | 31, 239),
        (0x04, 0),
        (0x80, 0),
    ] {
        let fb = render_split(ctrl, scroll);
        let t = usize::from(ctrl & 0x1F);
        let right = ctrl & 0x40 != 0;
        for y in 1..HEIGHT {
            for x in 0..WIDTH {
                let col = x / 8;
                let in_split = ctrl & 0x80 != 0 && (col < t) != right;
                let yy = (usize::from(scroll) + y) % 240;
                let want = if in_split && yy % 2 == 1 { 0x30 } else { 0x0F };
                assert_eq!(
                    fb[y * WIDTH + x],
                    want,
                    "ctrl {ctrl:02X} scroll {scroll} ({x},{y})"
                );
            }
        }
    }
}

#[test]
fn split_is_off_in_exram_modes_2_and_3() {
    let mut c = mmc5();
    c.write(0x5200, 0x9F);
    c.in_frame = true;
    c.rendering = true;
    assert!(c.split_fetch().is_some());
    for mode in [2u8, 3] {
        c.write(0x5104, mode);
        assert_eq!(c.split_fetch(), None);
    }
    c.write(0x5104, 1);
    assert!(c.split_fetch().is_some());
    c.in_frame = false;
    assert_eq!(c.split_fetch(), None);
}
