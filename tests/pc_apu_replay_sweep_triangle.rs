//! APU: exact replay with the DMC playing (sample bytes in the log), pulse sweep
//! negate per channel, and the triangle's ultrasonic periods.

use mos6502::apu::{Apu, LogError, RegisterLog, DMC_SAMPLE_REG};

fn lcg(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

fn sample_memory(addr: u16) -> u8 {
    (addr as u8).wrapping_mul(37) ^ (addr >> 8) as u8
}

#[test]
fn random_sessions_with_dmc_on_replay_identically() {
    const REGS: [u16; 21] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 0xA, 0xB, 0xC, 0xE, 0xF, 0x10, 0x11, 0x12, 0x13, 0x15, 0x17,
        0x15,
    ];
    let mut total_bytes = 0;
    for s in 0..30u64 {
        let mut seed = s ^ 0xD3C;
        let mut apu = Apu::new();
        let mut log = RegisterLog::default();
        let mut live = Vec::new();
        for _ in 0..50 {
            for _ in 0..lcg(&mut seed) % 4 {
                let reg = REGS[(lcg(&mut seed) % REGS.len() as u64) as usize];
                let mut value = lcg(&mut seed) as u8;
                if reg == 0x15 {
                    value |= 0x10; // keep the DMC running
                }
                if reg == 0x10 {
                    value = (value & 0xC0) | 0x0F; // fastest rate: many fetches
                }
                log.write(&mut apu, reg, value);
            }
            for _ in 0..lcg(&mut seed) % 1200 {
                log.tick_with_dmc(&mut apu, sample_memory);
                live.push(apu.sample());
            }
        }
        total_bytes += log.dmc_bytes();
        let mut fresh = Apu::new();
        let replay = log.play(&mut fresh, live.len() as u64);
        assert_eq!(replay, live, "seed {s}");
        assert_eq!(
            format!("{:?}", fresh.dmc),
            format!("{:?}", apu.dmc),
            "DMC state after replay, seed {s}"
        );
        assert_eq!(RegisterLog::parse(&log.to_text()).unwrap(), log);
    }
    assert!(total_bytes > 100, "the sessions really fetched samples");
}

#[test]
fn without_the_sample_bytes_dmc_replay_diverges() {
    let mut apu = Apu::new();
    let mut log = RegisterLog::default();
    log.write(&mut apu, 0x10, 0x0F);
    log.write(&mut apu, 0x12, 0x00);
    log.write(&mut apu, 0x13, 0x04);
    log.write(&mut apu, 0x15, 0x10);
    let mut live = Vec::new();
    for _ in 0..4000 {
        log.tick_with_dmc(&mut apu, |_| 0xFF);
        live.push(apu.sample());
    }
    assert!(log.dmc_bytes() >= 2);
    let mut stripped = log.clone();
    stripped.writes.retain(|w| w.reg != DMC_SAMPLE_REG);
    assert_ne!(stripped.play(&mut Apu::new(), 4000), live);
    assert_eq!(log.play(&mut Apu::new(), 4000), live);
}

#[test]
fn dmc_entry_text_form_and_register_limit() {
    let log = RegisterLog::parse("10 15 10\n12 1F A5 # sample byte\n").unwrap();
    assert_eq!(log.writes[1].reg, DMC_SAMPLE_REG);
    assert_eq!(log.dmc_bytes(), 1);
    assert_eq!(
        RegisterLog::parse("1 19 00"),
        Err(LogError::BadRegister { line: 1, reg: 0x19 })
    );
}

#[test]
fn tick_with_dmc_reads_only_when_a_fetch_is_due() {
    let mut apu = Apu::new();
    let mut log = RegisterLog::default();
    // Silent channel, DMC off: no read ever happens.
    for _ in 0..5000 {
        assert_eq!(log.tick_with_dmc(&mut apu, |_| panic!("no fetch")), None);
    }
    log.write(&mut apu, 0x12, 0x10); // $C400
    log.write(&mut apu, 0x13, 0x00); // one byte
    log.write(&mut apu, 0x15, 0x10);
    let mut read = Vec::new();
    for _ in 0..5000 {
        if let Some(b) = log.tick_with_dmc(&mut apu, |a| (a >> 4) as u8) {
            read.push(b);
        }
    }
    assert_eq!(read, [0x40], "length 1 at $C400, no loop");
}

fn pulse_with(channel: usize, period: u16, sweep: u8) -> Apu {
    let mut apu = Apu::new();
    let base = channel as u16 * 4;
    apu.write_register(0x15, 0x03);
    apu.write_register(base, 0xBF);
    apu.write_register(base + 2, period as u8);
    apu.write_register(base + 3, 0x08 | (period >> 8) as u8);
    apu.write_register(base + 1, sweep);
    apu
}

#[test]
fn sweep_negate_target_differs_by_one_between_channels_for_every_shift_and_period() {
    for period in (8u16..=0x7FF).step_by(7) {
        for shift in 0..8u8 {
            let c = period >> shift;
            let p1 = pulse_with(0, period, 0x08 | shift);
            let p2 = pulse_with(1, period, 0x08 | shift);
            assert_eq!(p1.pulse[0].target_period(), period.saturating_sub(c + 1));
            assert_eq!(p2.pulse[1].target_period(), period - c);
            let up = pulse_with(0, period, shift);
            assert_eq!(up.pulse[0].target_period(), period + c);
            assert_eq!(up.pulse[0].sweep_muted(), period + c > 0x7FF);
            // Negating never mutes by overflow.
            assert!(!p1.pulse[0].sweep_muted() && !p2.pulse[1].sweep_muted());
        }
    }
}

#[test]
fn sweep_negate_moves_each_channel_by_its_own_amount_on_the_half_frame() {
    // Enabled, period 0 (every half frame), negate, shift 1, from $200.
    let mut a = pulse_with(0, 0x200, 0x89);
    let mut b = pulse_with(1, 0x200, 0x89);
    for _ in 0..7458 * 2 + 4 {
        a.tick();
        b.tick();
    }
    assert_eq!(a.pulse[0].timer_period, 0x200 - 0x100 - 1);
    assert_eq!(b.pulse[1].timer_period, 0x200 - 0x100);
}

#[test]
fn period_below_eight_mutes_and_sweep_never_updates_it() {
    for period in 0..8u16 {
        let mut apu = pulse_with(0, period, 0x81);
        assert!(apu.pulse[0].sweep_muted());
        for _ in 0..40000 {
            apu.tick();
        }
        assert_eq!(apu.pulse[0].timer_period, period);
        assert_eq!(apu.channel_outputs()[0], 0);
    }
}

fn triangle(period: u16) -> Apu {
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x04);
    apu.write_register(0x08, 0xFF);
    apu.write_register(0x0A, period as u8);
    apu.write_register(0x0B, 0x08 | (period >> 8) as u8);
    for _ in 0..7458 {
        apu.tick(); // first quarter frame loads the linear counter
    }
    apu
}

#[test]
fn ultrasonic_triangle_keeps_stepping_but_reports_no_pitch() {
    for period in [0u16, 1] {
        let mut apu = triangle(period);
        assert!(apu.triangle.linear > 0);
        assert_eq!(apu.tone_hz(2, 1_789_773), None, "period {period}");
        let mut seen = [false; 16];
        for _ in 0..64 {
            apu.tick();
            seen[usize::from(apu.channel_outputs()[2])] = true;
        }
        // The hardware sequencer still runs every period+1 cycles, so all 16 levels
        // appear (an average near 7.5, the "pop" games avoid by muting instead).
        assert!(seen.iter().all(|&s| s), "period {period}");
    }
    assert!(triangle(2).tone_hz(2, 1_789_773).is_some());
}

#[test]
fn triangle_step_rate_is_period_plus_one_for_every_small_period() {
    for period in 0u16..64 {
        let mut apu = triangle(period);
        let start = apu.triangle.step;
        let ticks = 32 * (u32::from(period) + 1);
        for _ in 0..ticks {
            apu.tick();
        }
        assert_eq!(
            apu.triangle.step, start,
            "a whole 32-step cycle, period {period}"
        );
    }
}

#[test]
fn silenced_triangle_holds_its_level() {
    let mut apu = triangle(5);
    for _ in 0..37 {
        apu.tick();
    }
    let held = apu.channel_outputs()[2];
    apu.write_register(0x15, 0x00);
    for _ in 0..2000 {
        apu.tick();
        assert_eq!(apu.channel_outputs()[2], held);
    }
}
