//! Channel pitch, exact-rate resampling and the NES output filter chain.

use mos6502::apu::{Apu, OutputFilter, Resampler, CPU_HZ_NTSC, DMC_RATES};

fn pulse(apu: &mut Apu, base: u16, t: u16) {
    apu.write_register(base, 0xBF);
    apu.write_register(base + 1, 0);
    apu.write_register(base + 2, t as u8);
    apu.write_register(base + 3, 0x08 | (t >> 8) as u8);
}

#[test]
fn pitch_over_every_period() {
    let cpu = f64::from(CPU_HZ_NTSC);
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x0F);
    for t in 0..=0x7FFu16 {
        pulse(&mut apu, 0x00, t);
        pulse(&mut apu, 0x04, t);
        // Sweep shift 0 targets 2t: the mute applies even with the sweep disabled.
        let want = (t >= 8 && 2 * t <= 0x7FF).then(|| cpu / (16.0 * (f64::from(t) + 1.0)));
        assert_eq!(apu.tone_hz(0, CPU_HZ_NTSC), want, "{t}");
        assert_eq!(apu.tone_hz(1, CPU_HZ_NTSC), want, "{t}");
        apu.write_register(0x08, 0xFF);
        apu.write_register(0x0A, t as u8);
        apu.write_register(0x0B, 0x08 | (t >> 8) as u8);
        apu.tick(); // quarter-frame not needed: the linear reload happens on the next clock
        for _ in 0..7500 {
            apu.tick();
        }
        let want = (t >= 2).then(|| cpu / (32.0 * (f64::from(t) + 1.0)));
        assert_eq!(apu.tone_hz(2, CPU_HZ_NTSC), want, "tri {t}");
    }
    // A pulse with length 0 is silent; disabling the channel clears the length.
    apu.write_register(0x15, 0x00);
    assert_eq!(apu.tone_hz(0, CPU_HZ_NTSC), None);
    assert_eq!(apu.tone_hz(2, CPU_HZ_NTSC), None);
    for (i, &rate) in DMC_RATES.iter().enumerate() {
        apu.write_register(0x10, i as u8);
        assert_eq!(apu.tone_hz(4, CPU_HZ_NTSC), Some(cpu / f64::from(rate)));
    }
    apu.write_register(0x15, 0x08);
    apu.write_register(0x0C, 0x3F);
    apu.write_register(0x0E, 0x00);
    apu.write_register(0x0F, 0x08);
    assert_eq!(apu.tone_hz(3, CPU_HZ_NTSC), Some(cpu / 4.0));
    assert_eq!(apu.tone_hz(5, CPU_HZ_NTSC), None);
}

#[test]
fn resampler_is_exact_and_box_filters() {
    for rate in [8_000u32, 22_050, 44_100, 48_000, 96_000, CPU_HZ_NTSC] {
        let mut res = Resampler::new(CPU_HZ_NTSC, rate).unwrap();
        let mut apu = Apu::new();
        let n = u64::from(rate) / 20;
        apu.render_resampled(&mut res, n as usize);
        assert_eq!(
            res.cycles,
            n * u64::from(CPU_HZ_NTSC) / u64::from(rate),
            "{rate}"
        );
    }
    assert!(Resampler::new(CPU_HZ_NTSC, 0).is_none());
    assert!(Resampler::new(100, 101).is_none());
    // Each sample is the mean of the per-cycle mixer output it covers.
    let mut a = Apu::new();
    a.write_register(0x15, 0x01);
    pulse(&mut a, 0x00, 50);
    let mut b = a.clone();
    let mut res = Resampler::new(CPU_HZ_NTSC, 44_100).unwrap();
    let got = a.render_resampled(&mut res, 500);
    let mut done = 0u64;
    for (k, &g) in got.iter().enumerate() {
        let end = (k as u64 + 1) * u64::from(CPU_HZ_NTSC) / 44_100;
        let mut sum = 0.0f64;
        for _ in done..end {
            b.tick();
            sum += f64::from(b.sample());
        }
        let want = (sum / (end - done) as f64) as f32;
        done = end;
        assert_eq!(g, want, "{k}");
    }
}

#[test]
fn filter_chain_removes_dc_keeps_midband_and_cuts_highs() {
    let rate = 44_100;
    let mut f = OutputFilter::nes(rate);
    let out: Vec<f32> = (0..rate).map(|_| f.process(0.5)).collect();
    assert!(out[0] > 0.2, "step passes at first");
    assert!(out.last().unwrap().abs() < 1e-4, "DC removed");
    let amp = |hz: f32| {
        let mut f = OutputFilter::nes(rate);
        let mut peak = 0.0f32;
        for i in 0..rate {
            let y = f.process((2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin());
            if i > rate / 2 {
                peak = peak.max(y.abs());
            }
        }
        peak
    };
    let mid = amp(3_000.0);
    assert!((0.85..=1.0).contains(&mid), "{mid}");
    assert!(amp(20.0) < 0.1, "below 90 Hz");
    assert!(amp(20_000.0) < 0.75 * mid, "above 14 kHz");
    let mut f = OutputFilter::nes(rate);
    assert_eq!(f.process(0.0), 0.0);
}
