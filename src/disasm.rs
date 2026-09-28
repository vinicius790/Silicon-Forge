//! Disassembler for all 256 opcodes of the MOS 6502 and 65C02 architecture.
//!
//! Provides disassembly with relative branch target resolution, byte size computation,
//! hex bytes rendering, and symbol annotation support.

use crate::opcodes::{decode_variant, CpuVariant, Mode, OpType};
use std::collections::{BTreeSet, HashMap};

/// Disassembled line representing an instruction.
#[derive(Clone, Debug)]
pub struct Line {
    /// Number of bytes consumed by the instruction (1, 2, or 3).
    pub size: u8,
    /// Disassembled mnemonic and formatted operands.
    pub text: String,
    /// Raw hexadecimal bytes (e.g. "A9 42").
    pub bytes_hex: String,
}

/// Format instruction at PC with given variant.
pub fn format_at_variant(op: u8, b1: u8, b2: u8, pc: u16, variant: CpuVariant) -> Line {
    format_named(op, b1, b2, pc, variant, &|_| None)
}

/// Same as [`format_at_variant`], but every memory operand (zero page, absolute,
/// indirect, branch target) is printed as the name `name` returns for its address.
/// Immediate operands are values, not addresses, and are never named.
pub fn format_named(
    op: u8,
    b1: u8,
    b2: u8,
    pc: u16,
    variant: CpuVariant,
    name: &dyn Fn(u16) -> Option<String>,
) -> Line {
    let Some(info) = decode_variant(op, variant) else {
        return Line {
            size: 1,
            text: format!("DB ${op:02X}"),
            bytes_hex: format!("{op:02X}"),
        };
    };

    let address = u16::from_le_bytes([b1, b2]);
    let zp = name(u16::from(b1)).unwrap_or_else(|| format!("${b1:02X}"));
    let abs = name(address).unwrap_or_else(|| format!("${address:04X}"));
    let operand = match info.mode {
        Mode::Imp => String::new(),
        Mode::Acc => "A".into(),
        Mode::Imm => format!("#${b1:02X}"),
        Mode::Zp => zp,
        Mode::ZpX => format!("{zp},X"),
        Mode::ZpY => format!("{zp},Y"),
        Mode::Abs => abs,
        Mode::AbsX => format!("{abs},X"),
        Mode::AbsY => format!("{abs},Y"),
        Mode::Ind => format!("({abs})"),
        Mode::IndX => format!("({zp},X)"),
        Mode::IndY => format!("({zp}),Y"),
        Mode::Rel => {
            let target = pc.wrapping_add(2).wrapping_add_signed(b1 as i8 as i16);
            name(target).unwrap_or_else(|| format!("${target:04X}"))
        }
        Mode::ZpInd => format!("({zp})"),
        Mode::AbsIndX => format!("({abs},X)"),
        Mode::ZpRel => {
            let target = pc.wrapping_add(3).wrapping_add_signed(b2 as i8 as i16);
            let target = name(target).unwrap_or_else(|| format!("${target:04X}"));
            format!("{zp},{target}")
        }
    };

    let size = info.mode.size();
    let bytes_hex = match size {
        1 => format!("{op:02X}"),
        2 => format!("{op:02X} {b1:02X}"),
        _ => format!("{op:02X} {b1:02X} {b2:02X}"),
    };

    Line {
        size,
        text: format!("{} {}", info.mnemonic, operand)
            .trim_end()
            .to_string(),
        bytes_hex,
    }
}

/// Disassemble bytes using origin zero for relative branches (NMOS variant).
pub fn disasm(op: u8, b1: u8, b2: u8) -> Line {
    format_at_variant(op, b1, b2, 0, CpuVariant::NMOS)
}

/// Disassemble bytes with explicit CPU variant.
pub fn disasm_variant(op: u8, b1: u8, b2: u8, variant: CpuVariant) -> Line {
    format_at_variant(op, b1, b2, 0, variant)
}

/// Read instruction bytes from a 16-bit address space, including wrap at $FFFF.
pub fn disasm_at(mem: &[u8], pc: u16) -> Line {
    disasm_at_variant(mem, pc, CpuVariant::NMOS)
}

/// Read instruction bytes from memory taking CPU variant into account.
pub fn disasm_at_variant(mem: &[u8], pc: u16, variant: CpuVariant) -> Line {
    let read = |offset| {
        mem.get(pc.wrapping_add(offset) as usize)
            .copied()
            .unwrap_or(0)
    };
    format_at_variant(read(0), read(1), read(2), pc, variant)
}

/// Disassemble a range of instructions starting from `start_pc` up to `count` instructions.
pub fn disasm_range(
    mem: &[u8],
    start_pc: u16,
    count: usize,
    symbols: Option<&HashMap<u16, String>>,
) -> Vec<(u16, Line, Option<String>)> {
    let mut results = Vec::with_capacity(count);
    let mut pc = start_pc;
    for _ in 0..count {
        let line = disasm_at(mem, pc);
        let sym = symbols.and_then(|map| map.get(&pc).cloned());
        results.push((pc, line.clone(), sym));
        pc = pc.wrapping_add(line.size as u16);
    }
    results
}

/// Listing of `buf` as instructions beginning at `origin`.
///
/// Each line is `ADDR  BYTES  MNEMONIC`. A partial opcode at the end of the
/// buffer is printed as `DB`.
pub fn write_listing(buf: &[u8], origin: u16) -> String {
    let mut out = String::new();
    let mut index = 0usize;
    while index < buf.len() {
        let address = origin.wrapping_add(index as u16);
        let op = buf[index];
        let b1 = buf.get(index + 1).copied().unwrap_or(0);
        let b2 = buf.get(index + 2).copied().unwrap_or(0);
        let decoded = format_at_variant(op, b1, b2, address, CpuVariant::NMOS);
        let size = (decoded.size as usize).min(buf.len() - index).max(1);
        let bytes = buf[index..index + size]
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let text = if size == decoded.size as usize {
            decoded.text
        } else {
            format!("DB ${op:02X}")
        };
        out.push_str(&format!("{address:04X}  {bytes:<8} {text}\n"));
        index += size;
    }
    out
}

/// Mnemonic, addressing mode, and byte length of an official NMOS opcode.
///
/// Returns `None` for unofficial and CMOS-only bytes. This does not say those
/// bytes are unimplemented; [`disasm`] still formats the ones the core executes.
pub fn official_layout(opcode: u8) -> Option<(&'static str, Mode, u8)> {
    let info = decode_variant(opcode, CpuVariant::NMOS)?;
    if info.op_type != OpType::Official {
        return None;
    }
    Some((info.mnemonic, info.mode, info.mode.size()))
}

/// How [`to_source`] classified one address of the range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteKind {
    /// First byte of an instruction written as a mnemonic.
    Code,
    /// Operand byte of a [`ByteKind::Code`] instruction.
    Operand,
    /// Written inside a `.byte` line.
    Data,
}

/// Reassemblable source for `mem[start..=end]` (addresses `start` to `end`, no wrap).
#[derive(Debug, Clone)]
pub struct Source {
    pub text: String,
    /// One entry per address from `start` to `end`.
    pub kinds: Vec<ByteKind>,
    /// Whether operands and labels carry symbol names. `false` when no table was
    /// given or when the named text did not reassemble to the same bytes, in which
    /// case the plain numeric text is returned instead.
    pub named: bool,
}

fn valid_label(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(name.to_ascii_lowercase().as_str(), "a" | "x" | "y")
        && crate::asm::assemble(&format!("{name} = $1234\n"), 0).is_ok()
}

/// Disassemble `start..=end` into source for this project's assembler such that
/// assembling the text at `start` gives back exactly `mem[start..=end]`.
///
/// Classification:
/// - with `coverage`, a byte starts an instruction only if it ran as an opcode,
///   the instruction fits in the range and none of its operand bytes also ran as
///   an opcode (overlapping code); every other byte is data;
/// - without coverage, the range is swept linearly as code.
///
/// An instruction is written as a mnemonic only if that single line reassembles at
/// its address to the same bytes (this rejects opcodes the assembler does not
/// accept for the variant and encodings it would shorten); otherwise its bytes go
/// to `.byte`. Data runs hold up to 8 bytes and break at labels.
///
/// With `symbols`, valid names inside the range become `name:` lines at the start
/// of a line, the rest become `name = $XXXX` equates at the top, and memory
/// operands and branch targets are named. The whole named text is reassembled and
/// compared; if it differs the numeric text is returned (`named == false`).
pub fn to_source(
    mem: &[u8],
    start: u16,
    end: u16,
    variant: CpuVariant,
    symbols: Option<&crate::symbols::SymbolTable>,
    coverage: Option<&crate::coverage::Coverage>,
) -> Source {
    to_source_with(mem, start, end, variant, symbols, coverage, false)
}

/// Every address inside `start..=end` that a [`ByteKind::Code`] instruction of
/// `kinds` branches, jumps or calls to (relative branches, `BBR`/`BBS`, `JMP abs`,
/// `JSR`) and that is itself the start of a code line. Indirect jumps are not
/// followed: their target is in memory, not in the instruction.
pub fn code_targets(
    mem: &[u8],
    start: u16,
    kinds: &[ByteKind],
    variant: CpuVariant,
) -> BTreeSet<u16> {
    let mut out = BTreeSet::new();
    let end = usize::from(start) + kinds.len();
    for (i, kind) in kinds.iter().enumerate() {
        if *kind != ByteKind::Code {
            continue;
        }
        let at = usize::from(start) + i;
        let pc = at as u16;
        let Some(info) = decode_variant(mem[at], variant) else {
            continue;
        };
        let b = |k: usize| mem.get(at + k).copied().unwrap_or(0);
        let target = match info.mode {
            Mode::Rel => Some(pc.wrapping_add(2).wrapping_add_signed(b(1) as i8 as i16)),
            Mode::ZpRel => Some(pc.wrapping_add(3).wrapping_add_signed(b(2) as i8 as i16)),
            Mode::Abs if matches!(info.mnemonic, "JMP" | "JSR") => {
                Some(u16::from_le_bytes([b(1), b(2)]))
            }
            _ => None,
        };
        if let Some(t) = target {
            let ti = usize::from(t);
            if ti >= usize::from(start)
                && ti < end
                && kinds[ti - usize::from(start)] == ByteKind::Code
            {
                out.insert(t);
            }
        }
    }
    out
}

/// [`to_source`], and with `auto_labels` every [`code_targets`] address that has
/// no valid symbol gets a generated label `L_XXXX` (its address in hex), used both
/// as a `L_XXXX:` line and as the operand of the instructions that reach it. A
/// generated name that already exists in `symbols` is not reused for another
/// address. The text is still verified by reassembly (`named` tells whether names,
/// generated or given, made it into the result).
pub fn to_source_with(
    mem: &[u8],
    start: u16,
    end: u16,
    variant: CpuVariant,
    symbols: Option<&crate::symbols::SymbolTable>,
    coverage: Option<&crate::coverage::Coverage>,
    auto_labels: bool,
) -> Source {
    if !auto_labels {
        return to_source_plain(mem, start, end, variant, symbols, coverage);
    }
    let (_, kinds) = render(mem, start, end, variant, None, coverage);
    let mut table = symbols.cloned().unwrap_or_default();
    for t in code_targets(mem, start, &kinds, variant) {
        let has_name = table.names_at(t).iter().any(|n| valid_label(n));
        let name = format!("L_{t:04X}");
        if !has_name && table.address_of(&name).is_none() {
            let _ = table.insert(&name, t);
        }
    }
    to_source_plain(mem, start, end, variant, Some(&table), coverage)
}

fn to_source_plain(
    mem: &[u8],
    start: u16,
    end: u16,
    variant: CpuVariant,
    symbols: Option<&crate::symbols::SymbolTable>,
    coverage: Option<&crate::coverage::Coverage>,
) -> Source {
    let (plain, kinds) = render(mem, start, end, variant, None, coverage);
    let want = &mem[usize::from(start)..=usize::from(end).min(mem.len().saturating_sub(1))];
    if let Some(table) = symbols {
        let (named, _) = render(mem, start, end, variant, Some(table), coverage);
        if crate::asm::assemble(&named, start).is_ok_and(|b| b == want) {
            return Source {
                text: named,
                kinds,
                named: true,
            };
        }
    }
    Source {
        text: plain,
        kinds,
        named: false,
    }
}

fn render(
    mem: &[u8],
    start: u16,
    end: u16,
    variant: CpuVariant,
    symbols: Option<&crate::symbols::SymbolTable>,
    coverage: Option<&crate::coverage::Coverage>,
) -> (String, Vec<ByteKind>) {
    use crate::coverage::Mark;
    let end = usize::from(end).min(mem.len().saturating_sub(1));
    let start = usize::from(start);
    let mut kinds = vec![ByteKind::Data; end.saturating_sub(start) + 1];
    let mut out = format!(".org ${start:04X}\n");
    if start > end {
        return (out, Vec::new());
    }
    let good = |name: &&String| valid_label(name);
    let mut inside: BTreeSet<u16> = BTreeSet::new();
    if let Some(table) = symbols {
        let mut equates = String::new();
        for (name, addr) in table.iter() {
            if !valid_label(name) {
                continue;
            }
            let a = usize::from(addr);
            if a < start || a > end {
                equates += &format!("{name} = ${addr:04X}\n");
            } else {
                inside.insert(addr);
            }
        }
        out += &equates;
    }
    let name_of = |addr: u16| -> Option<String> {
        let table = symbols?;
        table.names_at(addr).iter().find(good).cloned()
    };
    let labelled = |out: &mut String, at: u16, inside: &mut BTreeSet<u16>| {
        if !inside.remove(&at) {
            return;
        }
        if let Some(table) = symbols {
            for name in table.names_at(at).iter().filter(good) {
                *out += &format!("{name}:\n");
            }
        }
    };
    let mut data: Vec<u8> = Vec::new();
    let flush = |out: &mut String, data: &mut Vec<u8>| {
        if !data.is_empty() {
            let list: Vec<String> = data.iter().map(|b| format!("${b:02X}")).collect();
            *out += &format!("  .byte {}\n", list.join(","));
            data.clear();
        }
    };
    let mut at = start;
    while at <= end {
        let pc = at as u16;
        let op = mem[at];
        let size = decode_variant(op, variant).map_or(1, |i| usize::from(i.mode.size()));
        let fits = at + size - 1 <= end;
        let runnable = fits
            && decode_variant(op, variant).is_some()
            && coverage.is_none_or(|c| {
                c.mark(pc) == Mark::Opcode
                    && (1..size).all(|k| c.mark((at + k) as u16) != Mark::Opcode)
            })
            && (1..size).all(|k| !inside.contains(&((at + k) as u16)) || symbols.is_none());
        let line = runnable.then(|| {
            let b = |k: usize| if k < size { mem[at + k] } else { 0 };
            let numeric = format_at_variant(op, b(1), b(2), pc, variant);
            let ok = crate::asm::assemble(&format!("{}\n", numeric.text), pc)
                .is_ok_and(|bytes| bytes == mem[at..at + size]);
            ok.then(|| match symbols {
                Some(_) => format_named(op, b(1), b(2), pc, variant, &name_of).text,
                None => numeric.text,
            })
        });
        if let Some(Some(text)) = line {
            flush(&mut out, &mut data);
            labelled(&mut out, pc, &mut inside);
            out += &format!("  {text}\n");
            kinds[at - start] = ByteKind::Code;
            for k in 1..size {
                kinds[at - start + k] = ByteKind::Operand;
            }
            at += size;
        } else {
            if inside.contains(&pc) || data.len() == 8 {
                flush(&mut out, &mut data);
            }
            labelled(&mut out, pc, &mut inside);
            data.push(op);
            at += 1;
        }
    }
    flush(&mut out, &mut data);
    (out, kinds)
}
