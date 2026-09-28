//! A tune written in the project's assembler, played by the 2A03 machine and
//! exported to a WAV file.

use mos6502::audio::{parse_wav, pcm_from_levels, wav_bytes};
use mos6502::systems::nes2a03::Nes2A03;

/// Three notes on pulse 1 (timer periods for A4, C#5, E5 at NTSC), each held for
/// a busy-wait loop; then the channel is silenced.
const TUNE: &str = "
.org $8000
reset:
    lda #$01
    sta $4015       ; pulse 1 on
    lda #$BF        ; 50 % duty, halt, constant volume 15
    sta $4000
    ldx #0
note:
    lda periods_lo,x
    sta $4002
    lda periods_hi,x
    sta $4003
    ldy #40
hold_y:
    lda #0
hold_a:
    sec
    sbc #1
    bne hold_a
    dey
    bne hold_y
    inx
    cpx #3
    bne note
    lda #$00
    sta $4015       ; silence
done:
    jmp done
periods_lo: .byte $FD, $93, $52
periods_hi: .byte $00, $00, $00
.org $FFFA
    .word reset, reset, reset
";

const RATE: u32 = 44_100;
const CPU_HZ: u64 = 1_789_773;

fn render(seconds: f64) -> Vec<f32> {
    render_with_periods(seconds).0
}

/// Mixer level per output sample and the pulse 1 period in force at that sample.
fn render_with_periods(seconds: f64) -> (Vec<f32>, Vec<u16>) {
    let mut nes = Nes2A03::from_source(TUNE).unwrap();
    let count = (seconds * f64::from(RATE)) as usize;
    let mut out = Vec::with_capacity(count);
    let mut periods = Vec::with_capacity(count);
    let mut done = 0u64;
    for i in 1..=count as u64 {
        let target = i * CPU_HZ / u64::from(RATE);
        nes.run_cycles(target - done).unwrap();
        done = target;
        out.push(nes.cpu.bus.apu.sample());
        let p = &nes.cpu.bus.apu.pulse[0];
        // 0 marks a silenced channel (length counter cleared by $4015).
        periods.push(if p.length.value > 0 {
            p.timer_period
        } else {
            0
        });
    }
    (out, periods)
}

/// Pitch from the mean spacing of upward mean crossings (first to last).
fn pitch_hz(pcm: &[i16]) -> f64 {
    let mean = pcm.iter().map(|&s| f64::from(s)).sum::<f64>() / pcm.len() as f64;
    let ups: Vec<usize> = pcm
        .windows(2)
        .enumerate()
        .filter(|(_, w)| f64::from(w[0]) < mean && f64::from(w[1]) >= mean)
        .map(|(i, _)| i)
        .collect();
    assert!(ups.len() >= 3, "too few cycles to measure");
    let span = (ups[ups.len() - 1] - ups[0]) as f64 / (ups.len() - 1) as f64;
    f64::from(RATE) / span
}

#[test]
fn tune_exports_to_a_valid_wav_with_the_three_pitches_then_silence() {
    let levels = render(0.5);
    let pcm = pcm_from_levels(&levels);
    let wav = wav_bytes(&pcm, RATE).unwrap();
    assert_eq!(wav.len(), 44 + pcm.len() * 2);
    let (rate, back) = parse_wav(&wav).unwrap();
    assert_eq!((rate, back.as_slice()), (RATE, pcm.as_slice()));

    // Segment by the period the program wrote, then measure each segment's pitch.
    let (_, written) = render_with_periods(0.5);
    let mut segments: Vec<(u16, usize, usize)> = Vec::new();
    for (i, &t) in written.iter().enumerate() {
        match segments.last_mut() {
            Some(seg) if seg.0 == t => seg.2 = i + 1,
            _ => segments.push((t, i, i + 1)),
        }
    }
    let notes: Vec<_> = segments.iter().filter(|s| s.0 != 0).collect();
    assert_eq!(
        notes.iter().map(|s| s.0).collect::<Vec<_>>(),
        [0x0FD, 0x093, 0x052],
        "the three notes, in order"
    );
    let mut previous = 0.0;
    for (k, &&(t, start, end)) in notes.iter().enumerate() {
        let len = end - start;
        let window = &pcm[start + len / 4..start + 3 * len / 4];
        let hz = pitch_hz(window);
        let want = CPU_HZ as f64 / (16.0 * (f64::from(t) + 1.0));
        assert!(
            (hz - want).abs() < want * 0.02,
            "note {k}: measured {hz:.1} Hz, expected {want:.1}"
        );
        assert!(hz > previous, "rising tune");
        previous = hz;
    }
    // After the last note: silence (constant level).
    let tail = &pcm[pcm.len() - 2000..];
    assert!(tail.iter().all(|&s| s == tail[0]), "silenced at the end");
}

#[test]
fn rendering_is_deterministic() {
    assert_eq!(render(0.05), render(0.05));
}

#[test]
fn parse_wav_rejects_bad_input() {
    assert!(parse_wav(b"RIFX").is_err());
    let mut wav = wav_bytes(&[1, 2, 3], 8000).unwrap();
    wav.truncate(wav.len() - 1);
    assert!(parse_wav(&wav).is_err(), "truncated data");
    let mut stereo = wav_bytes(&[0], 8000).unwrap();
    stereo[22] = 2;
    assert!(parse_wav(&stereo).is_err());
}
