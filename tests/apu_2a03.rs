//! APU channels, frame counter and the 2A03 machine.

use mos6502::apu::{mix, Apu};
use mos6502::systems::nes2a03::Nes2A03;

fn ticks(apu: &mut Apu, n: u32) {
    for _ in 0..n {
        apu.tick();
    }
}

#[test]
fn pulse_period_is_sixteen_timer_periods_plus_one_and_duty_is_exact() {
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x01);
    apu.write_register(0x00, 0xBF); // 50 %, halt, constant volume 15
    apu.write_register(0x02, 0x10);
    apu.write_register(0x03, 0x00);
    let trace = apu.trace_channel(0, 272 * 4);
    assert!(trace.iter().all(|&v| v == 0 || v == 15));
    assert_eq!(trace.iter().filter(|&&v| v == 15).count(), 136 * 4);
    let rises: Vec<usize> = (1..trace.len())
        .filter(|&i| trace[i] == 15 && trace[i - 1] == 0)
        .collect();
    assert!(rises.len() >= 3);
    assert!(rises.windows(2).all(|w| w[1] - w[0] == 272), "{rises:?}");

    let mut eighth = Apu::new();
    eighth.write_register(0x15, 0x01);
    eighth.write_register(0x00, 0x3F); // 12.5 %
    eighth.write_register(0x02, 0x10);
    eighth.write_register(0x03, 0x00);
    let trace = eighth.trace_channel(0, 272 * 2);
    assert_eq!(trace.iter().filter(|&&v| v == 15).count(), 34 * 2);
}

#[test]
fn envelope_decays_one_step_per_period_plus_one_quarter_frames() {
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x01);
    apu.write_register(0x00, 0x82); // envelope, divider period 2
    apu.write_register(0x02, 0x40);
    apu.write_register(0x03, 0x08); // length 254
    let mut decays = Vec::new();
    let mut at = 0;
    for target in [7457, 14913, 22371, 29829, 37287, 44743, 52201] {
        ticks(&mut apu, target - at);
        at = target;
        decays.push(apu.pulse[0].envelope.decay);
    }
    assert_eq!(decays, [15, 15, 15, 14, 14, 14, 13]);

    let mut looping = Apu::new();
    looping.write_register(0x0C, 0x20); // noise: loop + envelope period 0
    looping.write_register(0x0F, 0x00);
    let mut seen = Vec::new();
    let mut at = 0;
    for target in
        (1..=17).map(|q| [7457, 14913, 22371, 29829][(q - 1) % 4] + 29830 * ((q - 1) / 4) as u32)
    {
        ticks(&mut looping, target - at);
        at = target;
        seen.push(looping.noise.envelope.decay);
    }
    assert_eq!(&seen[..3], [15, 14, 13]);
    assert_eq!(seen[15], 0, "16th quarter frame reaches 0");
    assert_eq!(seen[16], 15, "loop restarts the decay");
}

#[test]
fn length_counter_silences_after_its_half_frames_unless_halted() {
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x03);
    apu.write_register(0x00, 0x9F);
    apu.write_register(0x04, 0xBF); // pulse 2 halted
    for reg in [0x02, 0x06] {
        apu.write_register(reg, 0x40);
    }
    apu.write_register(0x03, 0x00); // length index 0: 10
    apu.write_register(0x07, 0x00);
    ticks(&mut apu, 4 * 29830 + 29828);
    assert_eq!(apu.peek_status() & 3, 3);
    apu.tick();
    assert_eq!(apu.peek_status() & 3, 2, "tenth half frame");
    assert_eq!(apu.channel_outputs()[0], 0);
    apu.write_register(0x15, 0x00);
    assert_eq!(apu.peek_status() & 3, 0, "disable clears the length");
    apu.write_register(0x07, 0x00);
    assert_eq!(
        apu.pulse[1].length.value, 0,
        "a disabled channel does not load"
    );
}

#[test]
fn frame_irq_four_step_only_inhibit_and_status_read_clear_it() {
    let mut apu = Apu::new();
    ticks(&mut apu, 29828);
    assert!(!apu.irq_asserted());
    apu.tick();
    assert!(apu.irq_asserted());
    assert_eq!(apu.read_status() & 0x40, 0x40);
    assert!(!apu.irq_asserted(), "read clears");

    let mut inhibited = Apu::new();
    inhibited.write_register(0x17, 0x40);
    ticks(&mut inhibited, 3 * 29830);
    assert!(!inhibited.irq_asserted());

    let mut five = Apu::new();
    five.write_register(0x17, 0x80);
    ticks(&mut five, 3 * 37282);
    assert!(!five.irq_asserted());
}

#[test]
fn five_step_write_clocks_immediately_and_half_frames_follow_its_table() {
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x01);
    apu.write_register(0x00, 0x1F);
    apu.write_register(0x03, 0x18); // index 3: length 2
    apu.write_register(0x17, 0x80);
    assert_eq!(apu.pulse[0].length.value, 2, "the restart is 3 cycles away");
    ticks(&mut apu, 3);
    assert_eq!(apu.pulse[0].length.value, 1, "half frame at the restart");
    ticks(&mut apu, 14912);
    assert_eq!(apu.pulse[0].length.value, 1);
    apu.tick();
    assert_eq!(apu.pulse[0].length.value, 0);
}

#[test]
fn triangle_steps_thirty_two_levels_only_with_both_counters() {
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x04);
    apu.write_register(0x08, 0xFF);
    apu.write_register(0x0A, 0x03);
    apu.write_register(0x0B, 0x08);
    let held = apu.trace_channel(2, 7456);
    assert!(held.iter().all(|&v| v == 15), "linear counter still 0");
    ticks(&mut apu, 1);
    let trace = apu.trace_channel(2, 256);
    assert!((0..128).all(|i| trace[i] == trace[i + 128]));
    assert_eq!(trace[..128].iter().map(|&v| u32::from(v)).sum::<u32>(), 960);
    assert_eq!(*trace.iter().max().unwrap(), 15);
    assert_eq!(*trace.iter().min().unwrap(), 0);
}

#[test]
fn noise_register_repeats_after_32767_or_93_clocks() {
    for (mode, period) in [(0x00u8, 32767usize), (0x80, 93)] {
        let mut apu = Apu::new();
        apu.write_register(0x0E, mode);
        let mut states = Vec::new();
        for _ in 0..period {
            ticks(&mut apu, 4);
            states.push(apu.noise.shift);
        }
        assert_eq!(states[period - 1], 1, "mode {mode:02X}");
        assert!(
            states[..period - 1].iter().all(|&s| s != 1),
            "mode {mode:02X}"
        );
    }
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x08);
    apu.write_register(0x0C, 0x3F);
    apu.write_register(0x0F, 0x08);
    let trace = apu.trace_channel(3, 4000);
    let loud = trace.iter().filter(|&&v| v == 15).count();
    assert!(trace.iter().all(|&v| v == 0 || v == 15));
    assert!(loud > 1500 && loud < 2500, "{loud}");
}

#[test]
fn sweep_targets_differ_between_pulses_and_overflow_mutes() {
    let mut apu = Apu::new();
    for base in [0x00, 0x04] {
        apu.write_register(base + 1, 0x89); // enabled, period 0, negate, shift 1
        apu.write_register(base + 2, 0x00);
        apu.write_register(base + 3, 0x01);
    }
    assert_eq!(apu.pulse[0].target_period(), 0x7F);
    assert_eq!(apu.pulse[1].target_period(), 0x80);
    apu.write_register(0x01, 0x01); // disabled, add, shift 1
    apu.write_register(0x02, 0xFF);
    apu.write_register(0x03, 0x07);
    assert!(
        apu.pulse[0].sweep_muted(),
        "target past $7FF mutes even when disabled"
    );
    apu.write_register(0x06, 0x07);
    apu.write_register(0x07, 0x00);
    assert!(apu.pulse[1].sweep_muted(), "period below 8 mutes");

    let mut sweeping = Apu::new();
    sweeping.write_register(0x15, 1);
    sweeping.write_register(0x01, 0x81); // enabled, period 0, add, shift 1
    sweeping.write_register(0x02, 0x00);
    sweeping.write_register(0x03, 0x01);
    ticks(&mut sweeping, 14913);
    assert_eq!(sweeping.pulse[0].timer_period, 0x180);
}

#[test]
fn mixer_is_nonlinear_and_render_ticks_per_sample() {
    let pulse = |s: f32| 95.88 / (8128.0 / s + 100.0);
    let tnd = |t: f32| 159.79 / (1.0 / (t / 8227.0) + 100.0);
    assert_eq!(mix([0; 5]), 0.0);
    assert!((mix([15, 15, 0, 0, 0]) - pulse(30.0)).abs() < 1e-6);
    assert!(
        mix([15, 15, 0, 0, 0]) < 2.0 * mix([15, 0, 0, 0, 0]),
        "compresses"
    );
    let full = mix([15, 15, 15, 15, 127]);
    assert!(full > 0.99 && full < 1.0, "{full}");
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x01);
    apu.write_register(0x00, 0xFF); // 25 % inverted
    apu.write_register(0x02, 0x10);
    apu.write_register(0x03, 0x00);
    let samples = apu.render(34, 8);
    let high = pulse(15.0) + tnd(15.0);
    let low = tnd(15.0);
    assert!(samples
        .iter()
        .all(|&s| (s - high).abs() < 1e-6 || (s - low).abs() < 1e-6));
    assert_eq!(
        samples.iter().filter(|&&s| (s - high).abs() < 1e-6).count(),
        6
    );
}

#[test]
fn nes_machine_has_no_decimal_mirrors_ram_and_takes_the_frame_irq() {
    let mut nes = Nes2A03::from_source(
        "
.org $8000
reset:
    sed
    clc
    lda #$09
    adc #$01
    sta $00
    lda #$55
    sta $0801
    lda #$01
    sta $4015
    lda #$BF
    sta $4000
    lda #$10
    sta $4002
    lda #$00
    sta $4003
    cli
idle:
    jmp idle
irq:
    lda $4015
    inc $02
    rti
.org $FFFA
    .word reset
    .word reset
    .word irq
",
    )
    .unwrap();
    nes.run_cycles(29830 * 3 + 100).unwrap();
    assert_eq!(nes.cpu.bus.ram[0], 0x0A, "binary ADC with D set");
    assert_eq!(nes.cpu.bus.ram[1], 0x55, "$0801 mirrors $0001");
    assert_eq!(nes.cpu.bus.ram[2], 3, "one frame IRQ per sequence");
    assert!(!nes.cpu.bus.apu.irq_asserted());
    assert!(nes.cpu.bus.apu.peek_status() & 1 == 1);
    assert!(Nes2A03::from_source(".org $0200\n nop\n").is_err());
}
