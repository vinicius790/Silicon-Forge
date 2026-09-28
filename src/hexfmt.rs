//! Intel HEX and Motorola S-record (S0/S1/S5/S9) readers, and the S-record writer
//! (the Intel HEX writer is [`crate::asm::to_intel_hex`]).
//!
//! Intel HEX: `:CCAAAATT<data>KK`, checksum = two's complement of the byte sum.
//! Types: `00` data, `01` end (later lines are ignored), `02`/`04` extended segment or
//! linear address (accepted only when zero: this is a 16-bit machine), `03`/`05` start
//! address (the low 16 bits must hold the whole address). S-record: `STCC<addr><data>KK`,
//! checksum = one's complement of the sum of count, address and data. `S0` header is
//! skipped, `S1` data, `S5` record count (checked against the S1 lines seen), `S9`
//! start. Both readers reject a byte that would wrap past `$FFFF` and an address given
//! twice with different values; gaps inside the image read as 0.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexImage {
    pub origin: u16,
    pub bytes: Vec<u8>,
    pub start: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HexErrorKind {
    MissingMark(char),
    BadLength,
    BadDigit,
    Checksum {
        expected: u8,
        found: u8,
    },
    UnsupportedType(String),
    /// Extended or start address that does not fit 16 bits.
    WideAddress(u32),
    PastEnd(u16),
    Conflict {
        address: u16,
        first: u8,
        second: u8,
    },
    RecordCount {
        declared: u16,
        seen: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexError {
    /// 1-based line number.
    pub line: usize,
    pub kind: HexErrorKind,
}

impl fmt::Display for HexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "linha {}: ", self.line)?;
        match &self.kind {
            HexErrorKind::MissingMark(c) => write!(f, "falta '{c}' no início"),
            HexErrorKind::BadLength => f.write_str("comprimento do registro inválido"),
            HexErrorKind::BadDigit => f.write_str("dígito hexadecimal inválido"),
            HexErrorKind::Checksum { expected, found } => {
                write!(f, "checksum ${found:02X}, esperado ${expected:02X}")
            }
            HexErrorKind::UnsupportedType(t) => write!(f, "tipo de registro {t} não suportado"),
            HexErrorKind::WideAddress(a) => write!(f, "endereço ${a:X} não cabe em 16 bits"),
            HexErrorKind::PastEnd(a) => write!(f, "dados a partir de ${a:04X} passam de $FFFF"),
            HexErrorKind::Conflict {
                address,
                first,
                second,
            } => write!(
                f,
                "${address:04X} recebe ${first:02X} e depois ${second:02X}"
            ),
            HexErrorKind::RecordCount { declared, seen } => {
                write!(f, "S5 declara {declared} registros, vistos {seen}")
            }
        }
    }
}

impl std::error::Error for HexError {}

/// Sparse 64 KiB image shared by both readers.
struct Sparse {
    cells: Vec<Option<u8>>,
    low: u16,
    high: u16,
    any: bool,
}

impl Sparse {
    fn new() -> Self {
        Self {
            cells: vec![None; 0x10000],
            low: 0xFFFF,
            high: 0,
            any: false,
        }
    }

    fn put(&mut self, addr: u16, data: &[u8]) -> Result<(), HexErrorKind> {
        if usize::from(addr) + data.len() > 0x10000 {
            return Err(HexErrorKind::PastEnd(addr));
        }
        for (i, &b) in data.iter().enumerate() {
            let a = addr + i as u16;
            match self.cells[usize::from(a)] {
                Some(first) if first != b => {
                    return Err(HexErrorKind::Conflict {
                        address: a,
                        first,
                        second: b,
                    })
                }
                _ => self.cells[usize::from(a)] = Some(b),
            }
            self.low = self.low.min(a);
            self.high = self.high.max(a);
            self.any = true;
        }
        Ok(())
    }

    fn finish(self, start: Option<u16>) -> HexImage {
        if !self.any {
            return HexImage {
                origin: 0,
                bytes: Vec::new(),
                start,
            };
        }
        let bytes = (self.low..=self.high)
            .map(|a| self.cells[usize::from(a)].unwrap_or(0))
            .collect();
        HexImage {
            origin: self.low,
            bytes,
            start,
        }
    }
}

/// Hex pairs of `digits` as bytes.
fn decode_pairs(digits: &str) -> Result<Vec<u8>, HexErrorKind> {
    if digits.len() % 2 != 0 {
        return Err(HexErrorKind::BadLength);
    }
    digits
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .ok_or(HexErrorKind::BadDigit)
        })
        .collect()
}

fn sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |a, &b| a.wrapping_add(b))
}

pub fn from_intel_hex(text: &str) -> Result<HexImage, HexError> {
    let mut image = Sparse::new();
    let mut start = None;
    for (index, raw) in text.lines().enumerate() {
        let at = |kind| HexError {
            line: index + 1,
            kind,
        };
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let digits = line
            .strip_prefix(':')
            .ok_or(at(HexErrorKind::MissingMark(':')))?;
        let rec = decode_pairs(digits).map_err(at)?;
        if rec.len() < 5 || rec.len() != usize::from(rec[0]) + 5 {
            return Err(at(HexErrorKind::BadLength));
        }
        let (body, check) = rec.split_at(rec.len() - 1);
        let expected = sum(body).wrapping_neg();
        if expected != check[0] {
            return Err(at(HexErrorKind::Checksum {
                expected,
                found: check[0],
            }));
        }
        let addr = u16::from_be_bytes([body[1], body[2]]);
        let data = &body[4..];
        let wide = |data: &[u8]| data.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b));
        match body[3] {
            0x00 => image.put(addr, data).map_err(at)?,
            0x01 => break,
            0x02 | 0x04 if data.len() == 2 => {
                if wide(data) != 0 {
                    return Err(at(HexErrorKind::WideAddress(wide(data) << 4)));
                }
            }
            0x03 | 0x05 if data.len() == 4 => {
                let value = if body[3] == 0x03 {
                    (u32::from(u16::from_be_bytes([data[0], data[1]])) << 4)
                        + u32::from(u16::from_be_bytes([data[2], data[3]]))
                } else {
                    wide(data)
                };
                start =
                    Some(u16::try_from(value).map_err(|_| at(HexErrorKind::WideAddress(value)))?);
            }
            t => return Err(at(HexErrorKind::UnsupportedType(format!("{t:02X}")))),
        }
    }
    Ok(image.finish(start))
}

fn srec_checksum(bytes: &[u8]) -> u8 {
    !sum(bytes)
}

/// `S0` header, S1 records of at most 16 bytes, `S5` count, `S9` with `start`.
pub fn to_srec(bytes: &[u8], load: u16, start: u16) -> String {
    use std::fmt::Write as _;
    let record = |out: &mut String, kind: char, body: &[u8]| {
        let _ = write!(out, "S{kind}");
        for b in body {
            let _ = write!(out, "{b:02X}");
        }
        let _ = writeln!(out, "{:02X}", srec_checksum(body));
    };
    let mut out = String::new();
    record(&mut out, '0', &[3, 0, 0]);
    let mut addr = load;
    let mut count = 0u16;
    for chunk in bytes.chunks(16) {
        let mut body = vec![(chunk.len() + 3) as u8];
        body.extend(addr.to_be_bytes());
        body.extend_from_slice(chunk);
        record(&mut out, '1', &body);
        addr = addr.wrapping_add(chunk.len() as u16);
        count = count.wrapping_add(1);
    }
    let [hi, lo] = count.to_be_bytes();
    record(&mut out, '5', &[3, hi, lo]);
    let [hi, lo] = start.to_be_bytes();
    record(&mut out, '9', &[3, hi, lo]);
    out
}

pub fn from_srec(text: &str) -> Result<HexImage, HexError> {
    let mut image = Sparse::new();
    let mut start = None;
    let mut data_records = 0u16;
    for (index, raw) in text.lines().enumerate() {
        let at = |kind| HexError {
            line: index + 1,
            kind,
        };
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let rest = line
            .strip_prefix('S')
            .ok_or(at(HexErrorKind::MissingMark('S')))?;
        let mut chars = rest.chars();
        let kind = chars.next().ok_or(at(HexErrorKind::BadLength))?;
        let rec = decode_pairs(chars.as_str()).map_err(at)?;
        if rec.len() < 4 || rec.len() != usize::from(rec[0]) + 1 {
            return Err(at(HexErrorKind::BadLength));
        }
        let (body, check) = rec.split_at(rec.len() - 1);
        let expected = srec_checksum(body);
        if expected != check[0] {
            return Err(at(HexErrorKind::Checksum {
                expected,
                found: check[0],
            }));
        }
        let addr = u16::from_be_bytes([body[1], body[2]]);
        match kind {
            '0' => {}
            '1' => {
                image.put(addr, &body[3..]).map_err(at)?;
                data_records = data_records.wrapping_add(1);
            }
            '5' if body.len() == 3 => {
                if addr != data_records {
                    return Err(at(HexErrorKind::RecordCount {
                        declared: addr,
                        seen: data_records,
                    }));
                }
            }
            '9' if body.len() == 3 => start = Some(addr),
            other => return Err(at(HexErrorKind::UnsupportedType(format!("S{other}")))),
        }
    }
    Ok(image.finish(start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_name_the_line() {
        let err = from_srec("S0030000FC\nS1030000FC\nS1040000AA52\n").unwrap_err();
        assert_eq!(err.line, 3);
        assert!(matches!(err.kind, HexErrorKind::Checksum { .. }), "{err}");
    }
}

// ---------------------------------------------------------------------------
// MOS Technology paper-tape format (KIM-1 `Q` dump / `L` load).
// ---------------------------------------------------------------------------

/// Data bytes per MOS record: the KIM-1 punches 24 (`$18`).
pub const MOS_RECORD_BYTES: usize = 0x18;

/// MOS Technology hex: `;LLAAAA<data>CCCC`, `CCCC` the 16-bit sum of the count, both
/// address bytes and the data. The last record is `;00NNNN CCCC` with `NNNN` the number
/// of data records and `CCCC` the sum of its two bytes. Empty input yields only it.
pub fn to_mos_hex(bytes: &[u8], origin: u16) -> String {
    let mut out = String::new();
    let mut addr = origin;
    let mut records: u16 = 0;
    for chunk in bytes.chunks(MOS_RECORD_BYTES) {
        let mut check = chunk.len() as u16;
        out.push_str(&format!(";{:02X}{addr:04X}", chunk.len()));
        check = check.wrapping_add(addr >> 8).wrapping_add(addr & 0xFF);
        for &b in chunk {
            out.push_str(&format!("{b:02X}"));
            check = check.wrapping_add(u16::from(b));
        }
        out.push_str(&format!("{check:04X}\n"));
        addr = addr.wrapping_add(chunk.len() as u16);
        records = records.wrapping_add(1);
    }
    let check = (records >> 8).wrapping_add(records & 0xFF);
    out.push_str(&format!(";00{records:04X}{check:04X}\n"));
    out
}

/// Read MOS Technology hex. Checks every 16-bit checksum and the record count of the
/// final `;00` record; lines after it are ignored. A missing final record is
/// [`HexErrorKind::RecordCount`] with `declared` 0. Reuses the conflict and wrap rules.
pub fn from_mos_hex(text: &str) -> Result<HexImage, HexError> {
    let mut image = Sparse::new();
    let mut seen: u16 = 0;
    let mut last_line = 0;
    for (index, raw) in text.lines().enumerate() {
        let at = |kind| HexError {
            line: index + 1,
            kind,
        };
        last_line = index + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let digits = line
            .strip_prefix(';')
            .ok_or(at(HexErrorKind::MissingMark(';')))?;
        let rec = decode_pairs(digits).map_err(at)?;
        if rec.len() < 5 || rec.len() != usize::from(rec[0]) + 5 {
            return Err(at(HexErrorKind::BadLength));
        }
        let (body, check) = rec.split_at(rec.len() - 2);
        let expected = body.iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
        let found = u16::from_be_bytes([check[0], check[1]]);
        if expected != found {
            return Err(at(HexErrorKind::Checksum {
                expected: expected as u8,
                found: found as u8,
            }));
        }
        let addr = u16::from_be_bytes([body[1], body[2]]);
        if body[0] == 0 {
            if addr != seen {
                return Err(at(HexErrorKind::RecordCount {
                    declared: addr,
                    seen,
                }));
            }
            return Ok(image.finish(None));
        }
        image.put(addr, &body[3..]).map_err(at)?;
        seen = seen.wrapping_add(1);
    }
    Err(HexError {
        line: last_line,
        kind: HexErrorKind::RecordCount { declared: 0, seen },
    })
}

// ---------------------------------------------------------------------------
// Intel HEX with 32-bit addressing (type 04 extended linear, type 05 start).
// ---------------------------------------------------------------------------

/// One contiguous run of bytes at a 32-bit address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment32 {
    pub address: u32,
    pub bytes: Vec<u8>,
}

/// A 32-bit Intel HEX image: runs sorted by address and merged when adjacent.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HexImage32 {
    pub segments: Vec<Segment32>,
    pub start: Option<u32>,
}

impl HexImage32 {
    /// Byte at `address`, if some record defined it.
    pub fn byte(&self, address: u32) -> Option<u8> {
        self.segments.iter().find_map(|s| {
            let off = address.checked_sub(s.address)? as usize;
            s.bytes.get(off).copied()
        })
    }

    /// Total data bytes.
    pub fn len(&self) -> usize {
        self.segments.iter().map(|s| s.bytes.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }
}

fn intel_line(kind: u8, addr: u16, data: &[u8]) -> String {
    let mut rec = vec![data.len() as u8, (addr >> 8) as u8, addr as u8, kind];
    rec.extend_from_slice(data);
    let check = sum(&rec).wrapping_neg();
    let mut s = String::from(":");
    for b in rec.iter().chain(std::iter::once(&check)) {
        s.push_str(&format!("{b:02X}"));
    }
    s.push('\n');
    s
}

/// Intel HEX with type `04` records whenever the upper 16 address bits change
/// (a record never crosses a 64 KiB boundary), type `05` for `start`, and `01` last.
pub fn to_intel_hex32(segments: &[Segment32], start: Option<u32>) -> String {
    let mut out = String::new();
    let mut upper: Option<u16> = None;
    for seg in segments {
        let mut addr = seg.address;
        let mut rest = &seg.bytes[..];
        while !rest.is_empty() {
            let hi = (addr >> 16) as u16;
            if upper != Some(hi) {
                out.push_str(&intel_line(0x04, 0, &hi.to_be_bytes()));
                upper = Some(hi);
            }
            let room = 0x1_0000 - (addr & 0xFFFF) as usize;
            let n = rest.len().min(16).min(room);
            out.push_str(&intel_line(0x00, addr as u16, &rest[..n]));
            rest = &rest[n..];
            addr = addr.wrapping_add(n as u32);
        }
    }
    if let Some(s) = start {
        out.push_str(&intel_line(0x05, 0, &s.to_be_bytes()));
    }
    out.push_str(&intel_line(0x01, 0, &[]));
    out
}

/// Read Intel HEX with full 32-bit addressing: type `02` (segment, address << 4) and
/// `04` (linear, address << 16) set the base, `03`/`05` the start. A data record
/// overlapping an earlier one with different bytes is a [`HexErrorKind::Conflict`]
/// (the address reported is the low 16 bits). A missing `01` record is accepted.
pub fn from_intel_hex32(text: &str) -> Result<HexImage32, HexError> {
    let mut cells: std::collections::BTreeMap<u32, u8> = std::collections::BTreeMap::new();
    let mut base: u32 = 0;
    let mut start = None;
    for (index, raw) in text.lines().enumerate() {
        let at = |kind| HexError {
            line: index + 1,
            kind,
        };
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let digits = line
            .strip_prefix(':')
            .ok_or(at(HexErrorKind::MissingMark(':')))?;
        let rec = decode_pairs(digits).map_err(at)?;
        if rec.len() < 5 || rec.len() != usize::from(rec[0]) + 5 {
            return Err(at(HexErrorKind::BadLength));
        }
        let (body, check) = rec.split_at(rec.len() - 1);
        let expected = sum(body).wrapping_neg();
        if expected != check[0] {
            return Err(at(HexErrorKind::Checksum {
                expected,
                found: check[0],
            }));
        }
        let offset = u32::from(u16::from_be_bytes([body[1], body[2]]));
        let data = &body[4..];
        let wide = data.iter().fold(0u32, |a, &b| (a << 8) | u32::from(b));
        match body[3] {
            0x00 => {
                for (i, &b) in data.iter().enumerate() {
                    // Within a segment the 16-bit offset wraps (Intel spec).
                    let a = base.wrapping_add((offset + i as u32) & 0xFFFF);
                    if let Some(&first) = cells.get(&a) {
                        if first != b {
                            return Err(at(HexErrorKind::Conflict {
                                address: a as u16,
                                first,
                                second: b,
                            }));
                        }
                    }
                    cells.insert(a, b);
                }
            }
            0x01 => break,
            0x02 if data.len() == 2 => base = wide << 4,
            0x04 if data.len() == 2 => base = wide << 16,
            0x03 if data.len() == 4 => {
                let cs = u32::from(u16::from_be_bytes([data[0], data[1]]));
                let ip = u32::from(u16::from_be_bytes([data[2], data[3]]));
                start = Some((cs << 4).wrapping_add(ip));
            }
            0x05 if data.len() == 4 => start = Some(wide),
            0x02..=0x05 => return Err(at(HexErrorKind::BadLength)),
            t => return Err(at(HexErrorKind::UnsupportedType(format!("{t:02X}")))),
        }
    }
    let mut segments: Vec<Segment32> = Vec::new();
    for (a, b) in cells {
        match segments.last_mut() {
            Some(s) if s.address.wrapping_add(s.bytes.len() as u32) == a => s.bytes.push(b),
            _ => segments.push(Segment32 {
                address: a,
                bytes: vec![b],
            }),
        }
    }
    Ok(HexImage32 { segments, start })
}

/// Text format detected from the first non-blank character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexFormat {
    Intel,
    SRecord,
    Mos,
}

/// `:` Intel, `S` S-record, `;` MOS. `None` for anything else (raw binary).
pub fn detect_format(text: &str) -> Option<HexFormat> {
    match text.trim_start().chars().next()? {
        ':' => Some(HexFormat::Intel),
        'S' | 's' => Some(HexFormat::SRecord),
        ';' => Some(HexFormat::Mos),
        _ => None,
    }
}

/// Read any of the three 16-bit formats, chosen by [`detect_format`].
pub fn read_any(text: &str) -> Result<HexImage, HexError> {
    match detect_format(text) {
        Some(HexFormat::Intel) => from_intel_hex(text),
        Some(HexFormat::SRecord) => from_srec(text),
        Some(HexFormat::Mos) => from_mos_hex(text),
        None => Err(HexError {
            line: 1,
            kind: HexErrorKind::MissingMark(':'),
        }),
    }
}

// ---------------------------------------------------------------------------
// Load-address binary formats: Commodore PRG and Atari XEX.
// ---------------------------------------------------------------------------

/// Commodore `.prg`: two-byte little-endian load address, then the bytes.
pub fn to_prg(bytes: &[u8], load: u16) -> Result<Vec<u8>, String> {
    if usize::from(load) + bytes.len() > 0x10000 {
        return Err(format!(
            "{} bytes em ${load:04X} passam de $FFFF",
            bytes.len()
        ));
    }
    let mut out = load.to_le_bytes().to_vec();
    out.extend_from_slice(bytes);
    Ok(out)
}

/// Read a `.prg`. The image has no start address of its own (`start` is `None`).
pub fn from_prg(file: &[u8]) -> Result<HexImage, String> {
    let (head, body) = file
        .split_first_chunk::<2>()
        .ok_or("PRG sem endereço de carga")?;
    let load = u16::from_le_bytes(*head);
    if usize::from(load) + body.len() > 0x10000 {
        return Err(format!(
            "{} bytes em ${load:04X} passam de $FFFF",
            body.len()
        ));
    }
    Ok(HexImage {
        origin: load,
        bytes: body.to_vec(),
        start: None,
    })
}

/// C64 BASIC stub `10 SYS <addr>` at `$0801`, the usual way a PRG starts machine
/// code with `RUN`. Returns the stub bytes; code may follow right after.
pub fn c64_sys_stub(address: u16) -> Vec<u8> {
    let digits = address.to_string();
    let next = 0x0801 + 2 + 2 + 1 + digits.len() as u16 + 1;
    let mut out = next.to_le_bytes().to_vec();
    out.extend_from_slice(&10u16.to_le_bytes());
    out.push(0x9E); // SYS token
    out.extend_from_slice(digits.as_bytes());
    out.push(0); // end of line
    out.extend_from_slice(&[0, 0]); // end of program
    out
}

/// One segment of an Atari DOS binary load file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XexSegment {
    pub start: u16,
    pub bytes: Vec<u8>,
}

/// An Atari `.xex`: segments in file order, the run address (`RUNAD`, `$02E0`) and
/// every init address (`INITAD`, `$02E2`) in the order DOS would call them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Xex {
    pub segments: Vec<XexSegment>,
    pub run: Option<u16>,
    pub inits: Vec<u16>,
}

pub const XEX_RUNAD: u16 = 0x02E0;
pub const XEX_INITAD: u16 = 0x02E2;

/// Parse an Atari binary load file: `$FFFF` header (required first, optional
/// between segments), then `start`, `end` (inclusive, both LE) and the bytes. A
/// segment covering `$02E0-$02E1` sets the run address; one covering `$02E2-$02E3`
/// records an init call. `end < start` and a short segment are errors.
pub fn from_xex(file: &[u8]) -> Result<Xex, String> {
    if file.get(0..2) != Some(&[0xFF, 0xFF]) {
        return Err("XEX sem cabeçalho $FFFF".into());
    }
    let mut xex = Xex::default();
    let mut pos = 2;
    while pos < file.len() {
        let word = |at: usize| {
            file.get(at..at + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .ok_or_else(|| format!("XEX truncado no byte {at}"))
        };
        let mut start = word(pos)?;
        if start == 0xFFFF {
            pos += 2;
            start = word(pos)?;
        }
        let end = word(pos + 2)?;
        if end < start {
            return Err(format!("segmento ${start:04X}-${end:04X} invertido"));
        }
        let len = usize::from(end - start) + 1;
        let body = file
            .get(pos + 4..pos + 4 + len)
            .ok_or_else(|| format!("segmento ${start:04X}-${end:04X} truncado"))?;
        let word_in = |addr: u16| {
            if start <= addr && addr < end {
                let i = usize::from(addr - start);
                Some(u16::from_le_bytes([body[i], body[i + 1]]))
            } else {
                None
            }
        };
        if let Some(r) = word_in(XEX_RUNAD) {
            xex.run = Some(r);
        }
        if let Some(i) = word_in(XEX_INITAD) {
            xex.inits.push(i);
        }
        xex.segments.push(XexSegment {
            start,
            bytes: body.to_vec(),
        });
        pos += 4 + len;
    }
    if xex.segments.is_empty() {
        return Err("XEX sem segmentos".into());
    }
    Ok(xex)
}

/// Write an Atari binary load file: header, the segments, then a `RUNAD` segment
/// when `run` is set.
pub fn to_xex(segments: &[XexSegment], run: Option<u16>) -> Result<Vec<u8>, String> {
    let mut out = vec![0xFF, 0xFF];
    let mut push = |start: u16, bytes: &[u8]| -> Result<(), String> {
        if bytes.is_empty() || usize::from(start) + bytes.len() > 0x10000 {
            return Err(format!("segmento em ${start:04X} vazio ou além de $FFFF"));
        }
        out.extend_from_slice(&start.to_le_bytes());
        out.extend_from_slice(&(start + (bytes.len() - 1) as u16).to_le_bytes());
        out.extend_from_slice(bytes);
        Ok(())
    };
    for s in segments {
        push(s.start, &s.bytes)?;
    }
    if let Some(r) = run {
        push(XEX_RUNAD, &r.to_le_bytes())?;
    }
    Ok(out)
}

impl Xex {
    /// Flatten into one image (later segments overwrite earlier ones, as DOS loads
    /// them). `start` is the run address.
    pub fn image(&self) -> HexImage {
        let low = self.segments.iter().map(|s| s.start).min().unwrap_or(0);
        let high = self
            .segments
            .iter()
            .map(|s| usize::from(s.start) + s.bytes.len())
            .max()
            .unwrap_or(0);
        let mut bytes = vec![0u8; high - usize::from(low)];
        for s in &self.segments {
            let at = usize::from(s.start - low);
            bytes[at..at + s.bytes.len()].copy_from_slice(&s.bytes);
        }
        HexImage {
            origin: low,
            bytes,
            start: self.run,
        }
    }
}
