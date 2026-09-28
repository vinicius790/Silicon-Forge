//! Tiny PCM square-wave generator. No audio device and no SDL.
//!
//! Samples are `i16`. A cycle starts high for the first half of the period and then
//! goes low. Period is `sample_rate / frequency` in samples; the tests check that
//! spacing, not a quality score.

/// Square wave at `frequency_hz` for `duration_secs`, sampled at `sample_rate`.
///
/// Amplitude is ±12000. Returns an error for non-positive frequency or duration,
/// a zero sample rate, or a length that would not produce at least one sample.
pub fn square_wave(
    frequency_hz: f32,
    sample_rate: u32,
    duration_secs: f32,
) -> Result<Vec<i16>, String> {
    if !(frequency_hz.is_finite() && frequency_hz > 0.0) {
        return Err("frequency must be finite and > 0".into());
    }
    if sample_rate == 0 {
        return Err("sample rate must be > 0".into());
    }
    if !(duration_secs.is_finite() && duration_secs > 0.0) {
        return Err("duration must be finite and > 0".into());
    }
    let count = (f64::from(sample_rate) * f64::from(duration_secs)).round();
    if !(count.is_finite() && (1.0..=16_000_000.0).contains(&count)) {
        return Err("duration produces no samples or an unreasonable buffer".into());
    }
    let count = count as usize;
    let period = f64::from(sample_rate) / f64::from(frequency_hz);
    let mut samples = Vec::with_capacity(count);
    for index in 0..count {
        let phase = (index as f64 % period) / period;
        samples.push(if phase < 0.5 { 12_000 } else { -12_000 });
    }
    Ok(samples)
}

/// Distance in samples between rising edges, for tests. `None` if fewer than two edges.
pub fn rising_edge_period(samples: &[i16]) -> Option<usize> {
    let mut edges = Vec::new();
    for (index, pair) in samples.windows(2).enumerate() {
        if pair[0] < 0 && pair[1] > 0 {
            edges.push(index + 1);
        }
    }
    edges.windows(2).map(|pair| pair[1] - pair[0]).next()
}

/// Mixer levels in `0.0..=1.0` (for example [`crate::apu::Apu::render`]) to signed
/// PCM, centred: 0.0 is -32767, 1.0 is +32767; out-of-range and NaN clamp.
pub fn pcm_from_levels(levels: &[f32]) -> Vec<i16> {
    levels
        .iter()
        .map(|&v| {
            let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
            ((v * 2.0 - 1.0) * 32767.0).round() as i16
        })
        .collect()
}

/// Mono 16-bit PCM RIFF/WAVE file in memory.
pub fn wav_bytes(samples: &[i16], sample_rate: u32) -> Result<Vec<u8>, String> {
    if sample_rate == 0 {
        return Err("sample rate must be > 0".into());
    }
    let data_len = u32::try_from(samples.len() * 2)
        .ok()
        .filter(|&n| n <= u32::MAX - 36)
        .ok_or("too many samples for a WAV file")?;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(out)
}

/// Parse a WAV made by [`wav_bytes`] (mono, 16-bit PCM): sample rate and samples.
pub fn parse_wav(bytes: &[u8]) -> Result<(u32, Vec<i16>), String> {
    let field = |at: usize, n: usize| bytes.get(at..at + n).ok_or("WAV truncated");
    if field(0, 4)? != b"RIFF" || field(8, 8)? != b"WAVEfmt " {
        return Err("not a RIFF WAVE file".into());
    }
    let u16_at = |at| field(at, 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at| field(at, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u16_at(20)? != 1 || u16_at(22)? != 1 || u16_at(34)? != 16 {
        return Err("only mono 16-bit PCM is supported".into());
    }
    if field(36, 4)? != b"data" {
        return Err("data chunk missing".into());
    }
    let rate = u32_at(24)?;
    let len = u32_at(40)? as usize;
    let data = field(44, len)?;
    Ok((
        rate,
        data.chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect(),
    ))
}

/// Layout of a PCM WAV file: 1 or 2 channels, 8-bit (unsigned, centred on 128) or
/// 16-bit (signed little-endian) samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
}

impl WavFormat {
    pub fn block_align(self) -> u16 {
        self.channels * (self.bits / 8)
    }

    fn validate(self) -> Result<(), String> {
        if self.sample_rate == 0 {
            return Err("sample rate must be > 0".into());
        }
        if !matches!(self.channels, 1 | 2) {
            return Err(format!("{} channels: only mono or stereo", self.channels));
        }
        if !matches!(self.bits, 8 | 16) {
            return Err(format!("{}-bit samples: only 8 or 16", self.bits));
        }
        Ok(())
    }
}

/// 16-bit sample to the 8-bit WAV encoding: the high byte, rounded, offset by 128.
pub fn to_u8_sample(s: i16) -> u8 {
    let rounded = (i32::from(s) + 128).clamp(-32768, 32767) >> 8;
    (rounded + 128) as u8
}

/// 8-bit WAV sample back to 16 bits (`128` is silence).
pub fn from_u8_sample(b: u8) -> i16 {
    (i16::from(b) - 128) << 8
}

/// Interleave two equal-length channels as L, R, L, R...
pub fn interleave(left: &[i16], right: &[i16]) -> Result<Vec<i16>, String> {
    if left.len() != right.len() {
        return Err(format!(
            "channel lengths differ: {} and {}",
            left.len(),
            right.len()
        ));
    }
    Ok(left.iter().zip(right).flat_map(|(&l, &r)| [l, r]).collect())
}

/// WAV with any [`WavFormat`]. `samples` are interleaved frames; their count must
/// be a multiple of the channel count.
pub fn wav_bytes_with(samples: &[i16], format: WavFormat) -> Result<Vec<u8>, String> {
    format.validate()?;
    if samples.len() % usize::from(format.channels) != 0 {
        return Err("sample count is not a whole number of frames".into());
    }
    let width = usize::from(format.bits / 8);
    let data_len = u32::try_from(samples.len() * width)
        .ok()
        .filter(|&n| n <= u32::MAX - 37)
        .ok_or("too many samples for a WAV file")?;
    let pad = data_len & 1;
    let mut out = Vec::with_capacity(44 + data_len as usize + pad as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len + pad).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&format.channels.to_le_bytes());
    out.extend_from_slice(&format.sample_rate.to_le_bytes());
    let byte_rate = format.sample_rate * u32::from(format.block_align());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&format.block_align().to_le_bytes());
    out.extend_from_slice(&format.bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        if format.bits == 8 {
            out.push(to_u8_sample(s));
        } else {
            out.extend_from_slice(&s.to_le_bytes());
        }
    }
    if pad == 1 {
        out.push(0); // RIFF chunks are word aligned.
    }
    Ok(out)
}

/// Parse any PCM WAV with 1–2 channels and 8/16-bit samples, walking the chunk list
/// (so `LIST`, `fact` and other chunks before or after `data` are skipped). Samples
/// come back interleaved as `i16` (8-bit expanded with [`from_u8_sample`]).
pub fn parse_wav_any(bytes: &[u8]) -> Result<(WavFormat, Vec<i16>), String> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("not a RIFF WAVE file".into());
    }
    let mut pos = 12;
    let mut format = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len =
            u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap_or_default()) as usize;
        let body = bytes.get(pos + 8..pos + 8 + len).ok_or("WAV truncated")?;
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err("fmt chunk too short".into());
                }
                let u16_at = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
                if u16_at(0) != 1 {
                    return Err(format!("format tag {}: only PCM", u16_at(0)));
                }
                let f = WavFormat {
                    channels: u16_at(2),
                    sample_rate: u32::from_le_bytes(body[4..8].try_into().unwrap_or_default()),
                    bits: u16_at(14),
                };
                f.validate()?;
                if u16_at(12) != f.block_align() {
                    return Err("block align does not match channels and bits".into());
                }
                format = Some(f);
            }
            b"data" => {
                let f = format.ok_or("data chunk before fmt")?;
                let samples = if f.bits == 8 {
                    body.iter().map(|&b| from_u8_sample(b)).collect()
                } else {
                    body.chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]]))
                        .collect()
                };
                return Ok((f, samples));
            }
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    Err("data chunk missing".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_and_pcm_mapping() {
        let pcm = pcm_from_levels(&[0.0, 0.5, 1.0, 2.0, f32::NAN]);
        assert_eq!(pcm, [-32767, 0, 32767, 32767, -32767]);
        let wav = wav_bytes(&pcm[..2], 8000).unwrap();
        assert_eq!(wav.len(), 48);
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 40);
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 8000);
        assert_eq!(u32::from_le_bytes(wav[28..32].try_into().unwrap()), 16000);
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(&wav[44..], [0x01, 0x80, 0x00, 0x00]);
        assert!(wav_bytes(&[], 0).is_err());
    }

    #[test]
    fn exact_period_for_a_divisor_of_the_sample_rate() {
        // 1000 Hz at 8000 Hz is exactly 8 samples, 4 high then 4 low.
        let samples = square_wave(1000.0, 8000, 0.004).unwrap();
        assert_eq!(samples.len(), 32);
        assert_eq!(
            &samples[..8],
            &[12000, 12000, 12000, 12000, -12000, -12000, -12000, -12000]
        );
        assert_eq!(rising_edge_period(&samples), Some(8));
    }

    #[test]
    fn approximate_period_tracks_sample_rate_over_frequency() {
        let frequency = 440.0f32;
        let rate = 44100u32;
        let samples = square_wave(frequency, rate, 0.02).unwrap();
        let measured = rising_edge_period(&samples).unwrap();
        let expected = f64::from(rate) / f64::from(frequency);
        assert!(
            (measured as f64 - expected).abs() < 1.5,
            "measured period {measured} samples, expected about {expected}"
        );
    }

    #[test]
    fn rejects_non_positive_inputs() {
        assert!(square_wave(0.0, 8000, 0.1).is_err());
        assert!(square_wave(440.0, 0, 0.1).is_err());
        assert!(square_wave(440.0, 8000, 0.0).is_err());
    }
}
