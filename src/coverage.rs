//! Coverage map of executed bytes.
//!
//! [`Coverage::step`] runs one [`Cpu::step`] and marks the opcode byte and its operand
//! bytes (length from the decoder for the CPU's variant; an undecodable byte counts
//! as one). Interrupt sequences mark nothing: no instruction bytes are fetched.
//! Addresses wrap at `$FFFF`, so an instruction at `$FFFE` marks `$FFFE`, `$FFFF`, `$0000`.

use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::opcodes::decode_variant;

/// How a byte was executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Never,
    Opcode,
    Operand,
}

#[derive(Clone)]
pub struct Coverage {
    /// Bit 0: executed as an opcode. Bit 1: executed as an operand byte.
    marks: Box<[u8; 65536]>,
    /// Times each address was executed as an opcode (saturating).
    hits: Box<[u32; 65536]>,
}

/// One executable source line: its 1-based number and the address of its first byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLine {
    pub line: u32,
    pub address: u16,
}

impl Default for Coverage {
    fn default() -> Self {
        Self::new()
    }
}

impl Coverage {
    pub fn new() -> Self {
        Self {
            marks: Box::new([0; 65536]),
            hits: Box::new([0; 65536]),
        }
    }

    /// Mark the instruction at `cpu.pc`, then run it.
    pub fn step<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> Result<u32, String> {
        let pc = cpu.pc;
        let size = decode_variant(cpu.bus.peek(pc), cpu.variant).map_or(1, |op| op.mode.size());
        let cycles = cpu.step()?;
        self.marks[usize::from(pc)] |= 1;
        let h = &mut self.hits[usize::from(pc)];
        *h = h.saturating_add(1);
        for offset in 1..u16::from(size) {
            self.marks[usize::from(pc.wrapping_add(offset))] |= 2;
        }
        Ok(cycles)
    }

    /// The strongest mark: a byte run both ways is reported as an opcode.
    pub fn mark(&self, addr: u16) -> Mark {
        match self.marks[usize::from(addr)] {
            0 => Mark::Never,
            m if m & 1 != 0 => Mark::Opcode,
            _ => Mark::Operand,
        }
    }

    pub fn executed(&self, addr: u16) -> bool {
        self.marks[usize::from(addr)] != 0
    }

    /// Executed bytes in the inclusive range.
    pub fn count(&self, start: u16, end: u16) -> usize {
        (start..=end).filter(|&a| self.executed(a)).count()
    }

    /// Executed share of the inclusive range in tenths of a percent (`1000` = all).
    pub fn per_mille(&self, start: u16, end: u16) -> u32 {
        let len = u32::from(end.saturating_sub(start)) + 1;
        if end < start {
            return 0;
        }
        (self.count(start, end) as u32 * 1000) / len
    }

    /// Maximal runs of never-executed bytes inside the inclusive range.
    pub fn gaps(&self, start: u16, end: u16) -> Vec<(u16, u16)> {
        let mut gaps = Vec::new();
        let mut open: Option<u16> = None;
        for addr in start..=end {
            match (self.executed(addr), open) {
                (false, None) => open = Some(addr),
                (true, Some(from)) => {
                    gaps.push((from, addr - 1));
                    open = None;
                }
                _ => {}
            }
        }
        if let Some(from) = open {
            gaps.push((from, end));
        }
        gaps
    }

    /// One character per byte of the inclusive range, 64 per line, each line led by
    /// its address: `O` opcode, `o` operand, `.` never.
    pub fn render(&self, start: u16, end: u16) -> String {
        let mut out = String::new();
        for (i, addr) in (start..=end).enumerate() {
            if i % 64 == 0 {
                if i > 0 {
                    out.push('\n');
                }
                out.push_str(&format!("{addr:04X} "));
            }
            out.push(match self.mark(addr) {
                Mark::Never => '.',
                Mark::Opcode => 'O',
                Mark::Operand => 'o',
            });
        }
        out.push('\n');
        out
    }

    /// Union with another map (several runs of one program).
    pub fn merge(&mut self, other: &Coverage) {
        for (mine, theirs) in self.marks.iter_mut().zip(other.marks.iter()) {
            *mine |= theirs;
        }
        for (mine, theirs) in self.hits.iter_mut().zip(other.hits.iter()) {
            *mine = mine.saturating_add(*theirs);
        }
    }

    /// Times the instruction at `addr` was executed.
    pub fn hits(&self, addr: u16) -> u32 {
        self.hits[usize::from(addr)]
    }

    /// LCOV tracefile (`genhtml`, Coveralls, Codecov) for one source file: `DA:` per
    /// line with the opcode executions at its address, then `LF`/`LH`.
    pub fn to_lcov(&self, test_name: &str, source_file: &str, lines: &[SourceLine]) -> String {
        let mut out = format!("TN:{test_name}\nSF:{source_file}\n");
        let mut hit = 0;
        for l in lines {
            let n = self.hits(l.address);
            if n > 0 {
                hit += 1;
            }
            out.push_str(&format!("DA:{},{n}\n", l.line));
        }
        out.push_str(&format!("LF:{}\nLH:{hit}\nend_of_record\n", lines.len()));
        out
    }
}

/// Executable lines of `src` as assembled into `output`. Each
/// [`crate::asm::ListingLine`] carries its trimmed source text, so entries are matched
/// in order to the next source line with the same text (comment-only and blank lines
/// have no entry). Kept: lines that emitted bytes and are instructions, not `.`
/// directives, equates or `*=` (data would otherwise count as uncovered code). `None`
/// when an entry has no matching line (macros or `.include` expanded the text).
pub fn source_lines(src: &str, output: &crate::asm::AsmOutput) -> Option<Vec<SourceLine>> {
    let mut text = src.lines().enumerate();
    let mut out = Vec::new();
    for entry in &output.lines {
        let (index, _) = text
            .find(|(_, raw)| raw.trim() == entry.source || without_comment(raw) == entry.source)?;
        if !entry.bytes.is_empty() && !is_directive(&entry.source) {
            out.push(SourceLine {
                line: index as u32 + 1,
                address: entry.address,
            });
        }
    }
    Some(out)
}

/// `raw` up to a `;` outside quotes, trimmed (the preprocessor drops comments).
fn without_comment(raw: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, c) in raw.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ';') => return raw[..i].trim(),
            _ => {}
        }
    }
    raw.trim()
}

fn is_directive(source: &str) -> bool {
    let code = source.split(';').next().unwrap_or("").trim();
    let code = match code.split_once(':') {
        Some((_, rest)) => rest.trim(),
        None => code,
    };
    code.starts_with('.') || code.starts_with('*') || code.contains('=')
}

/// Executable lines of one source file, each with every address assembled from it
/// (a macro call line owns all the instructions of its expansion).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLines {
    pub file: String,
    /// `(line, addresses)` sorted by line.
    pub lines: Vec<(u32, Vec<u16>)>,
}

/// Map a listing to source files through each entry's `origin_line` (`n` in the main
/// file, `name:n` in an `.include`d one; a macro body line carries the call line).
/// Same filter as [`source_lines`]: only entries that emitted bytes and are not
/// directives. The main file is named `main_file`; files come out sorted with the
/// main file first.
pub fn line_map(output: &crate::asm::AsmOutput, main_file: &str) -> Vec<FileLines> {
    use std::collections::BTreeMap;
    let mut files: BTreeMap<(bool, String), BTreeMap<u32, Vec<u16>>> = BTreeMap::new();
    for entry in &output.lines {
        if entry.bytes.is_empty() || is_directive(&entry.source) {
            continue;
        }
        let (file, line) = match entry.origin_line.rsplit_once(':') {
            Some((f, n)) => (f.to_string(), n),
            None => (main_file.to_string(), entry.origin_line.as_str()),
        };
        let Ok(line) = line.trim().parse::<u32>() else {
            continue;
        };
        let addrs = files
            .entry((file != main_file, file))
            .or_default()
            .entry(line)
            .or_default();
        if !addrs.contains(&entry.address) {
            addrs.push(entry.address);
        }
    }
    files
        .into_iter()
        .map(|((_, file), lines)| FileLines {
            file,
            lines: lines.into_iter().collect(),
        })
        .collect()
}

impl Coverage {
    /// Executions credited to a line: the most-executed of its addresses (a macro
    /// call line counts as run as often as its busiest instruction).
    pub fn line_hits(&self, addresses: &[u16]) -> u32 {
        addresses.iter().map(|&a| self.hits(a)).max().unwrap_or(0)
    }

    /// LCOV tracefile with one record per file of [`line_map`].
    pub fn to_lcov_files(&self, test_name: &str, files: &[FileLines]) -> String {
        let mut out = String::new();
        for f in files {
            out.push_str(&format!("TN:{test_name}\nSF:{}\n", f.file));
            let mut hit = 0;
            for (line, addrs) in &f.lines {
                let n = self.line_hits(addrs);
                hit += usize::from(n > 0);
                out.push_str(&format!("DA:{line},{n}\n"));
            }
            out.push_str(&format!("LF:{}\nLH:{hit}\nend_of_record\n", f.lines.len()));
        }
        out
    }
}
