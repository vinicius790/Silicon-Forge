//! Kansas City Standard (300 baud) as a bit-level mark stream, plus PCM rendering.
//!
//! A bit cell is a run of marks: a `0` bit is [`SPACE_MARKS`] marks of level 0 (four
//! cycles of 1200 Hz on tape), a `1` bit is [`MARK_MARKS`] marks of level 1 (eight
//! cycles of 2400 Hz). A frame is one start `0`, eight data bits LSB first and two stop
//! `1`s. Framing is asynchronous: any number of idle `1` bits (the leader tone, or
//! gaps between frames) may precede a start bit. [`to_pcm`] turns a mark stream into
//! square-wave cycles at the two KCS frequencies.

use std::fmt;

pub const SPACE_MARKS: usize = 4;
pub const MARK_MARKS: usize = 8;
/// Data bits per frame plus start and two stop bits.
pub const FRAME_BITS: usize = 11;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TapeError {
    /// A mark other than 0 or 1, at this mark index.
    BadLevel { at: usize, level: u8 },
    /// A bit cell whose marks are not all equal (or shorter than a cell).
    BrokenCell { at: usize },
    /// A frame whose stop bits are not both 1, at this bit index.
    StopBits { bit: usize },
    /// The stream ends inside a frame that started at this bit index.
    Truncated { bit: usize },
}

impl fmt::Display for TapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLevel { at, level } => write!(f, "marca {at}: nível {level} fora de 0/1"),
            Self::BrokenCell { at } => write!(f, "marca {at}: célula de bit quebrada"),
            Self::StopBits { bit } => write!(f, "bit {bit}: stop bits devem ser 1"),
            Self::Truncated { bit } => write!(f, "bit {bit}: quadro incompleto no fim"),
        }
    }
}

impl std::error::Error for TapeError {}

/// Encode bytes as KCS marks (`0` space, `1` mark), frames back to back.
pub fn encode(bytes: &[u8]) -> Vec<u8> {
    encode_with_leader(bytes, 0)
}

/// Like [`encode`] after `leader` idle `1` bits.
pub fn encode_with_leader(bytes: &[u8], leader: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(leader * MARK_MARKS + bytes.len() * FRAME_BITS * MARK_MARKS);
    for _ in 0..leader {
        push_bit(&mut out, true);
    }
    for &byte in bytes {
        push_bit(&mut out, false);
        for i in 0..8 {
            push_bit(&mut out, (byte >> i) & 1 != 0);
        }
        push_bit(&mut out, true);
        push_bit(&mut out, true);
    }
    out
}

fn push_bit(out: &mut Vec<u8>, one: bool) {
    let (n, v) = if one {
        (MARK_MARKS, 1)
    } else {
        (SPACE_MARKS, 0)
    };
    out.extend(std::iter::repeat_n(v, n));
}

/// Bit cells of a mark stream.
pub fn bits(marks: &[u8]) -> Result<Vec<bool>, TapeError> {
    let mut bits = Vec::with_capacity(marks.len() / SPACE_MARKS);
    let mut i = 0;
    while i < marks.len() {
        let level = marks[i];
        let need = match level {
            0 => SPACE_MARKS,
            1 => MARK_MARKS,
            _ => return Err(TapeError::BadLevel { at: i, level }),
        };
        let cell = marks
            .get(i..i + need)
            .ok_or(TapeError::BrokenCell { at: i })?;
        if let Some(bad) = cell.iter().position(|&m| m != level) {
            return Err(match marks[i + bad] {
                0 | 1 => TapeError::BrokenCell { at: i },
                other => TapeError::BadLevel {
                    at: i + bad,
                    level: other,
                },
            });
        }
        bits.push(level == 1);
        i += need;
    }
    Ok(bits)
}

/// Decode a mark stream, skipping idle `1` bits before each start bit.
pub fn decode(marks: &[u8]) -> Result<Vec<u8>, TapeError> {
    let bits = bits(marks)?;
    let mut bytes = Vec::new();
    let mut b = 0usize;
    while b < bits.len() {
        if bits[b] {
            b += 1;
            continue;
        }
        let frame = bits
            .get(b..b + FRAME_BITS)
            .ok_or(TapeError::Truncated { bit: b })?;
        if !frame[9] || !frame[10] {
            return Err(TapeError::StopBits { bit: b });
        }
        bytes.push((0..8).fold(0u8, |acc, i| acc | (u8::from(frame[1 + i]) << i)));
        b += FRAME_BITS;
    }
    Ok(bytes)
}

/// Square-wave PCM at `sample_rate`: every mark is one cycle, 1200 Hz for level 0 and
/// 2400 Hz for level 1, so one bit lasts 1/300 s either way. Needs at least 4800 Hz.
pub fn to_pcm(marks: &[u8], sample_rate: u32) -> Result<Vec<i16>, String> {
    if sample_rate < 4800 {
        return Err("taxa de amostragem abaixo de 4800 Hz".into());
    }
    let mut out = Vec::new();
    let mut clock = 0.0f64;
    for (i, &level) in marks.iter().enumerate() {
        let hz = match level {
            0 => 1200.0,
            1 => 2400.0,
            _ => return Err(TapeError::BadLevel { at: i, level }.to_string()),
        };
        clock += f64::from(sample_rate) / hz;
        let samples = clock.round() as usize - out.len();
        out.extend((0..samples).map(|s| if s < samples / 2 { 12_000 } else { -12_000 }));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Kansas City Standard from PCM.
// ---------------------------------------------------------------------------

/// Lengths in samples of the half-cycles of `pcm`, split at sign changes (zero counts
/// as positive). The first run starts at sample 0 and the last ends at the buffer end.
pub fn half_cycles(pcm: &[i16]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut run = 0usize;
    let mut positive = pcm.first().is_some_and(|&s| s >= 0);
    for &s in pcm {
        let p = s >= 0;
        if p != positive {
            out.push(run);
            run = 0;
            positive = p;
        }
        run += 1;
    }
    if run > 0 {
        out.push(run);
    }
    out
}

/// Recover KCS marks from square or sine PCM at `sample_rate`: half-cycles shorter than
/// the 1800 Hz midpoint are 2400 Hz (level 1), longer are 1200 Hz (level 0); each run
/// of one class becomes half as many marks, rounded. Needs at least 4800 Hz.
pub fn kcs_marks_from_pcm(pcm: &[i16], sample_rate: u32) -> Result<Vec<u8>, String> {
    if sample_rate < 4800 {
        return Err("taxa de amostragem abaixo de 4800 Hz".into());
    }
    let threshold = f64::from(sample_rate) / 3600.0;
    let mut marks = Vec::new();
    let mut run_level: Option<u8> = None;
    let mut run_halves = 0usize;
    let flush = |level: Option<u8>, halves: usize, marks: &mut Vec<u8>| {
        if let Some(l) = level {
            marks.extend(std::iter::repeat_n(l, halves.div_ceil(2)));
        }
    };
    for h in half_cycles(pcm) {
        let level = u8::from((h as f64) < threshold);
        if run_level != Some(level) {
            flush(run_level, run_halves, &mut marks);
            run_level = Some(level);
            run_halves = 0;
        }
        run_halves += 1;
    }
    flush(run_level, run_halves, &mut marks);
    Ok(marks)
}

/// Bytes encoded as KCS PCM by [`to_pcm`] (or a recording of a real tape that keeps
/// the two tones apart).
pub fn kcs_decode_pcm(pcm: &[i16], sample_rate: u32) -> Result<Vec<u8>, String> {
    let marks = kcs_marks_from_pcm(pcm, sample_rate)?;
    decode(&marks).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Apple II cassette (Monitor `W`/`R`, Integer BASIC SAVE/LOAD).
// ---------------------------------------------------------------------------

/// Header tone half-cycle: 770 Hz.
pub const APPLE_HEADER_US: f64 = 650.0;
/// Sync: one 2500 Hz half-cycle, then one 2000 Hz half-cycle.
pub const APPLE_SYNC1_US: f64 = 200.0;
pub const APPLE_SYNC2_US: f64 = 250.0;
/// A `0` bit is one 2000 Hz cycle, a `1` bit one 1000 Hz cycle.
pub const APPLE_ZERO_HALF_US: f64 = 250.0;
pub const APPLE_ONE_HALF_US: f64 = 500.0;

/// XOR of all bytes seeded with `$FF`, as the Monitor computes it.
pub fn apple_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0xFF, |a, &b| a ^ b)
}

struct HalfWriter {
    rate: f64,
    clock: f64,
    level: bool,
    out: Vec<i16>,
}

impl HalfWriter {
    fn half(&mut self, micros: f64) {
        self.clock += micros * self.rate / 1_000_000.0;
        let n = (self.clock.round() as usize).saturating_sub(self.out.len());
        let v = if self.level { 12_000 } else { -12_000 };
        self.out.extend(std::iter::repeat_n(v, n));
        self.level = !self.level;
    }
}

/// Apple II cassette PCM: `header_halves` half-cycles of 770 Hz (the ROM writes about
/// ten seconds, ~15 400), sync, the data MSB first, and the checksum byte.
/// Needs at least 8000 Hz so the 200 µs sync half is a whole sample.
pub fn apple_encode_pcm(
    bytes: &[u8],
    sample_rate: u32,
    header_halves: usize,
) -> Result<Vec<i16>, String> {
    if sample_rate < 8000 {
        return Err("taxa de amostragem abaixo de 8000 Hz".into());
    }
    let mut w = HalfWriter {
        rate: f64::from(sample_rate),
        clock: 0.0,
        level: true,
        out: Vec::new(),
    };
    for _ in 0..header_halves {
        w.half(APPLE_HEADER_US);
    }
    w.half(APPLE_SYNC1_US);
    w.half(APPLE_SYNC2_US);
    let check = apple_checksum(bytes);
    for &byte in bytes.iter().chain(std::iter::once(&check)) {
        for bit in (0..8).rev() {
            let us = if (byte >> bit) & 1 != 0 {
                APPLE_ONE_HALF_US
            } else {
                APPLE_ZERO_HALF_US
            };
            w.half(us);
            w.half(us);
        }
    }
    Ok(w.out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppleTapeError {
    /// No header tone followed by a sync half-cycle.
    NoSync,
    /// Bits left over after the last whole byte.
    PartialByte {
        bits: usize,
    },
    /// No bytes at all after sync (the checksum byte is required).
    Empty,
    Checksum {
        expected: u8,
        found: u8,
    },
}

impl fmt::Display for AppleTapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSync => f.write_str("sem tom de cabeçalho seguido de sincronismo"),
            Self::PartialByte { bits } => write!(f, "{bits} bits sobrando após o último byte"),
            Self::Empty => f.write_str("nenhum byte após o sincronismo"),
            Self::Checksum { expected, found } => {
                write!(f, "checksum ${found:02X}, esperado ${expected:02X}")
            }
        }
    }
}

impl std::error::Error for AppleTapeError {}

/// Decode Apple II cassette PCM, like the ROM `READ` routine: wait for at least 16
/// header half-cycles (> 575 µs), then a half shorter than 400 µs (the 200 µs sync, with slack for coarse sample
/// rates; a 500 µs `1` half cannot follow the header directly), skip its
/// partner, and read bits as full cycles (< 750 µs is `0`). A trailing lone half (the
/// tape end) is ignored. Returns the data without the verified checksum byte.
pub fn apple_decode_pcm(pcm: &[i16], sample_rate: u32) -> Result<Vec<u8>, AppleTapeError> {
    let us = 1_000_000.0 / f64::from(sample_rate);
    let halves: Vec<f64> = half_cycles(pcm).iter().map(|&n| n as f64 * us).collect();
    let mut header = 0usize;
    let mut i = 0;
    let mut sync = None;
    while i < halves.len() {
        let h = halves[i];
        if h > 575.0 {
            header += 1;
        } else if h < 400.0 && header >= 16 {
            sync = Some(i);
            break;
        } else {
            header = 0;
        }
        i += 1;
    }
    let first_bit = sync.ok_or(AppleTapeError::NoSync)? + 2;
    let mut bits = Vec::new();
    let mut j = first_bit;
    while j + 1 < halves.len() {
        bits.push(halves[j] + halves[j + 1] >= 750.0);
        j += 2;
    }
    if bits.len() % 8 != 0 {
        return Err(AppleTapeError::PartialByte {
            bits: bits.len() % 8,
        });
    }
    let mut bytes: Vec<u8> = bits
        .chunks(8)
        .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
        .collect();
    let found = bytes.pop().ok_or(AppleTapeError::Empty)?;
    let expected = apple_checksum(&bytes);
    if expected != found {
        return Err(AppleTapeError::Checksum { expected, found });
    }
    Ok(bytes)
}
