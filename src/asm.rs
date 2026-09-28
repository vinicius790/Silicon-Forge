//! Assembler de duas passagens avançado 3.0 para MOS 6502 e 65C02.
//!
//! Suporta todos os 256 opcodes, labels locais e globais, diretivas (.org, .byte,
//! .word, .dbyt, .faraddr, .dword, .pstring, .lobytes, .hibytes, .assert, .text, .asciiz, .fill, .equ, .align, .rept/.endr, .if/.else/.endif,
//! .incbin, .error, .warn, .scope/.endscope), operadores de extração de bytes (< e >),
//! expressões (`+ - * << >> & | ^`, `hi`/`lo`, `<`/`>`), literais em hex/bin/dec/ASCII,
//! e geração de binário plano, formato Intel HEX e tabela de símbolos (.sym).
//! O pré-processador acrescenta `.define` (com parâmetros), `.ifdef`/`.ifndef`/
//! `.elseif`, `.strlen`, `.struct`/`.enum`/`.sizeof`, `A::b` e rótulos anônimos;
//! [`AsmOutput::listing_with_cycles`] mostra os ciclos de cada linha.
//! Ver `docs/ASSEMBLER.md`.

use std::fmt::Write as _;

use crate::opcodes::Mode;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmError(pub String);

impl std::fmt::Display for AsmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AsmError {}

/// Uma linha do listing: endereço, bytes emitidos e a linha-fonte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingLine {
    pub address: u16,
    pub bytes: Vec<u8>,
    pub source: String,
    /// Line of the original text this came from: `n`, or `file:n` for an
    /// `.include`d file. For a macro body line it is the line of the call.
    pub origin_line: String,
    /// The line was produced by a macro expansion.
    pub expanded: bool,
}

/// Cycles of one instruction line, see [`AsmOutput::line_cycles`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCycles {
    pub base: u8,
    /// One more cycle when an indexed access crosses a page.
    pub page_cross: bool,
    /// A branch: +1 when taken, +1 more when it lands on another page.
    pub branch: bool,
}

impl std::fmt::Display for LineCycles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.base)?;
        if self.page_cross {
            f.write_str("+p")?;
        }
        if self.branch {
            f.write_str("+t")?;
        }
        Ok(())
    }
}

/// Resultado detalhado de um processo de montagem.
#[derive(Debug, Clone)]
pub struct AsmOutput {
    pub bytes: Vec<u8>,
    pub origin: u16,
    pub symbols: HashMap<String, u16>,
    /// Uma entrada por linha-fonte não vazia, na ordem do texto.
    pub lines: Vec<ListingLine>,
    /// Mensagens de `.warn`. A montagem ainda sucede.
    pub warnings: Vec<String>,
}

impl AsmOutput {
    /// Listing file with macro expansions, for the text `src` this output was
    /// assembled from.
    ///
    /// Header `LINHA  END   BYTES        FONTE`, then one row per listed line:
    /// origin line (right-aligned to 7), a `+` when the row came from a macro
    /// expansion (space otherwise), the address, up to 4 bytes in hex and the
    /// source text. Longer byte runs continue on extra rows (address advanced, no
    /// text). Before the first expanded row of a macro call the call line itself is
    /// printed (origin line and the original text, no address), so the listing
    /// reads as the source with each expansion under its call. Rows for included
    /// files show `file:n`; their text is the included text.
    pub fn listing_file(&self, src: &str) -> String {
        let original: Vec<&str> = src.lines().collect();
        let mut out = String::from("  LINHA   END   BYTES        FONTE\n");
        let mut last_call: Option<&str> = None;
        for line in &self.lines {
            if line.expanded && last_call != Some(line.origin_line.as_str()) {
                let text = line
                    .origin_line
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| original.get(n.wrapping_sub(1)))
                    .map_or("", |t| t.trim());
                out += &format!("{:>7}               {:<12} {text}\n", line.origin_line, "");
            }
            last_call = line.expanded.then_some(line.origin_line.as_str());
            let mark = if line.expanded { '+' } else { ' ' };
            let mut chunks = line.bytes.chunks(4);
            let first = chunks.next().unwrap_or(&[]);
            let hex = |b: &[u8]| {
                b.iter()
                    .map(|x| format!("{x:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            out += &format!(
                "{:>7}{mark} {:04X}  {:<12} {}\n",
                line.origin_line,
                line.address,
                hex(first),
                line.source
            );
            for (i, chunk) in chunks.enumerate() {
                let at = line.address.wrapping_add(4 * (i as u16 + 1));
                out += &format!("{:>7}{mark} {at:04X}  {}\n", "", hex(chunk));
            }
        }
        out
    }

    /// Exporta o código montado no formato padrão Intel HEX.
    pub fn to_intel_hex(&self) -> String {
        to_intel_hex(&self.bytes, self.origin)
    }

    /// Listing `ADDR: BYTES  source`.
    pub fn listing(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            let encoded = line
                .bytes
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            out.push_str(&format!(
                "{:04X}: {:<12} {}\n",
                line.address, encoded, line.source
            ));
        }
        out
    }

    /// Cycle cost of one listing line: `None` for directives, data and empty lines.
    ///
    /// The opcode is looked up in the NMOS table, or in the 65C02 table when the
    /// NMOS mnemonic differs from the source (e.g. `BRA`, `STZ`). The result is the
    /// base count plus the conditional extras that can apply.
    pub fn line_cycles(line: &ListingLine) -> Option<LineCycles> {
        use crate::opcodes::{decode_variant, page_cross_extra, CpuVariant};
        let code = *line.bytes.first()?;
        let text = strip_comment(&line.source);
        let (_, rest) = split_label(text);
        let head = rest.split_whitespace().next()?.to_ascii_uppercase();
        if head.starts_with('.')
            || matches!(head.as_str(), "DB" | "DW")
            || parse_equ(rest).is_some()
        {
            return None;
        }
        let (op, variant) = [CpuVariant::NMOS, CpuVariant::CMOS]
            .into_iter()
            .find_map(|v| {
                decode_variant(code, v)
                    .filter(|op| op.mnemonic.eq_ignore_ascii_case(&head))
                    .map(|op| (op, v))
            })?;
        let branch = matches!(op.mode, Mode::Rel | Mode::ZpRel);
        Some(LineCycles {
            base: op.cycles,
            page_cross: !branch && page_cross_extra(code, variant),
            branch,
        })
    }

    /// Listing with a cycle column: `ADDR: BYTES  CYC  source`. `CYC` is the base
    /// count; `+p` marks a possible page-crossing cycle, `+t` a branch (one more
    /// when taken, another when the target is on a different page).
    pub fn listing_with_cycles(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            let encoded = line
                .bytes
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let cycles = Self::line_cycles(line).map_or(String::new(), |c| c.to_string());
            let _ = writeln!(
                out,
                "{:04X}: {:<12} {:<5} {}",
                line.address, encoded, cycles, line.source
            );
        }
        out
    }

    /// Sum of the base cycles of every instruction line (one pass through the
    /// source, branches not taken, no page crossings).
    pub fn base_cycles(&self) -> u32 {
        self.lines
            .iter()
            .filter_map(Self::line_cycles)
            .map(|c| u32::from(c.base))
            .sum()
    }

    /// Exporta a tabela de símbolos no formato clássico de depuração.
    /// Same text as [`crate::symbols::SymbolTable::to_text`], which loads it back.
    pub fn to_symbol_table(&self) -> String {
        let mut sorted: Vec<_> = self.symbols.iter().collect();
        sorted.sort_by_key(|(k, _)| (*k).clone());
        sorted
            .into_iter()
            .fold(String::new(), |mut out, (sym, addr)| {
                let _ = writeln!(out, "{sym:<20} = ${addr:04X}");
                out
            })
    }
}

/// Monta o código-fonte assembly para bytes na memória a partir da origem especificada.
pub fn assemble(src: &str, origin: u16) -> Result<Vec<u8>, AsmError> {
    assemble_with_symbols(src, origin).map(|res| res.bytes)
}

/// Monta o código-fonte assembly retornando os bytes e o mapa completo de símbolos.
pub fn assemble_with_symbols(src: &str, default_origin: u16) -> Result<AsmOutput, AsmError> {
    assemble_with_files(src, default_origin, &HashMap::new())
}

/// Like [`assemble_with_symbols`], but `.include "name"` is loaded from `files`.
///
/// The map is in-memory only (no filesystem and no network). Included text may itself
/// include further names. A cycle is an error.
pub fn assemble_with_files(
    src: &str,
    default_origin: u16,
    files: &HashMap<String, String>,
) -> Result<AsmOutput, AsmError> {
    assemble_with_blobs(src, default_origin, files, &HashMap::new())
}

/// Like [`assemble_with_files`], and `.incbin "name"` reads `blobs` first, then the
/// UTF-8 bytes of a text entry in `files`. Both maps are in memory only.
pub fn assemble_with_blobs(
    src: &str,
    default_origin: u16,
    files: &HashMap<String, String>,
    blobs: &HashMap<String, Vec<u8>>,
) -> Result<AsmOutput, AsmError> {
    let (expanded, warnings) = preprocess(src, files, blobs)?;
    assemble_preprocessed(&expanded, default_origin, warnings)
}

fn assemble_preprocessed(
    src: &str,
    default_origin: u16,
    warnings: Vec<String>,
) -> Result<AsmOutput, AsmError> {
    let mut labels = HashMap::new();
    let mut origin = default_origin;
    let mut origin_selected = false;
    let mut emitted_before_origin = false;

    // Pré-passo: Coleta símbolos estáticos (.equ / =) e detecta .org inicial se presente
    for raw in src.lines() {
        let line = strip_comment(raw);
        if line.is_empty() || cpu_directive(line).is_some() {
            continue;
        }
        if let Some((name, val_expr)) = parse_equ(line) {
            let val = resolve(val_expr, &labels)?;
            labels.insert(name.to_ascii_lowercase(), val);
            continue;
        }
        if let Some(target) = parse_org(line) {
            if !origin_selected && !emitted_before_origin {
                origin = resolve(target, &labels)?;
                origin_selected = true;
            }
            continue;
        }
        let (label, rest) = split_label(line);
        if !rest.is_empty() {
            emitted_before_origin = true;
        }
        if let Some(label) = label {
            let mut chars = label.chars();
            if !chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(AsmError(format!("label inválido: {label}")));
            }
            if labels.insert(label.to_ascii_lowercase(), 0).is_some() {
                return Err(AsmError(format!("label duplicado: {label}")));
            }
        }
    }

    // Passo 1: Calcula o endereço exato de cada label e o avanço do PC.
    // `equ_seen` holds the constants defined on earlier lines: only those may pick a
    // zero-page operand, in both passes alike, so a forward use stays absolute.
    let mut equ_seen = HashSet::new();
    let mut pc = origin as u32;
    let mut cpu = AsmCpu::Any;
    for raw in src.lines() {
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(selected) = cpu_directive(line) {
            cpu = selected?;
            continue;
        }
        if let Some((name, expr)) = parse_equ(line) {
            // Recompute with this pass's label addresses, so `b = a` after `a:`
            // gets `a`'s real address rather than the pre-pass placeholder.
            let key = name.to_ascii_lowercase();
            if let Ok(value) = resolve(expr, &labels) {
                labels.insert(key.clone(), value);
            }
            equ_seen.insert(key);
            continue;
        }
        if let Some(target) = parse_org(line) {
            let target = resolve(target, &labels)? as u32;
            if target < pc {
                return Err(AsmError("org para trás".into()));
            }
            pc = target;
            continue;
        }
        let (label, rest) = split_label(line);
        if let Some(l) = label {
            let address = u16::try_from(pc)
                .map_err(|_| AsmError("label fora do espaço de endereçamento de 16 bits".into()))?;
            labels.insert(l.to_ascii_lowercase(), address);
        }
        if rest.is_empty() {
            continue;
        }
        if let Some(expr) = parse_align(rest) {
            let pad = align_pad(pc, resolve(expr, &labels)?)?;
            pc = pc
                .checked_add(pad)
                .filter(|next| *next <= u16::MAX as u32 + 1)
                .ok_or_else(|| {
                    AsmError("programa excede o espaço de endereçamento de 16 bits".into())
                })?;
            continue;
        }
        let size = u32::try_from(size_of_instr(rest, &labels, &equ_seen, cpu)?)
            .map_err(|_| AsmError("linha assembly excede o espaço de endereçamento".into()))?;
        pc = pc
            .checked_add(size)
            .filter(|next_pc| *next_pc <= u16::MAX as u32 + 1)
            .ok_or_else(|| {
                AsmError("programa excede o espaço de endereçamento de 16 bits".into())
            })?;
    }

    // Passo 2: Emissão final dos opcodes e operandos
    let mut out = Vec::new();
    let mut listing = Vec::new();
    pc = origin as u32;
    equ_seen.clear();
    let mut cpu = AsmCpu::Any;
    for raw in src.lines() {
        let (text, tags) = raw.split_once('\u{1}').unwrap_or((raw, ""));
        let source = text.trim().to_string();
        let origin_line = tags.rsplit('\u{1}').next().unwrap_or("").to_string();
        let expanded = tags.contains('\u{1}');
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(selected) = cpu_directive(line) {
            cpu = selected.map_err(|e| at_line(&origin_line, e))?;
            listing.push(ListingLine {
                address: u16::try_from(pc).unwrap_or(0xFFFF),
                bytes: Vec::new(),
                source,
                origin_line: origin_line.clone(),
                expanded,
            });
            continue;
        }
        if let Some((name, expr)) = parse_equ(line) {
            equ_seen.insert(name.to_ascii_lowercase());
            let value = resolve(expr, &labels)?;
            labels.insert(name.to_ascii_lowercase(), value);
            listing.push(ListingLine {
                address: value,
                bytes: Vec::new(),
                source,
                origin_line: origin_line.clone(),
                expanded,
            });
            let _ = name;
            continue;
        }
        if let Some(target) = parse_org(line) {
            let target = resolve(target, &labels)?;
            let offset = target as usize - origin as usize;
            if offset < out.len() {
                return Err(AsmError("org para trás".into()));
            }
            out.resize(offset, 0);
            pc = u32::from(target);
            listing.push(ListingLine {
                address: target,
                bytes: Vec::new(),
                source,
                origin_line: origin_line.clone(),
                expanded,
            });
            continue;
        }
        let (_, rest) = split_label(line);
        let pc16 = u16::try_from(pc)
            .map_err(|_| AsmError("contador de programa fora do espaço de 16 bits".into()))?;
        if rest.is_empty() {
            listing.push(ListingLine {
                address: pc16,
                bytes: Vec::new(),
                source,
                origin_line: origin_line.clone(),
                expanded,
            });
            continue;
        }
        if let Some(expr) = parse_align(rest) {
            let pad = align_pad(pc, resolve(expr, &labels)?)?;
            let bytes = vec![0u8; pad as usize];
            listing.push(ListingLine {
                address: pc16,
                bytes: bytes.clone(),
                source,
                origin_line: origin_line.clone(),
                expanded,
            });
            pc = pc.wrapping_add(pad);
            out.extend(bytes);
            continue;
        }
        let bytes = emit(rest, pc16, &labels, &equ_seen, cpu)?;
        check_cpu(cpu, rest, &bytes).map_err(|e| at_line(&origin_line, e))?;
        listing.push(ListingLine {
            address: pc16,
            bytes: bytes.clone(),
            source,
            origin_line: origin_line.clone(),
            expanded,
        });
        pc = pc.wrapping_add(bytes.len() as u32);
        out.extend(bytes);
        if out.len() > 65536 - origin as usize {
            return Err(AsmError(
                "programa excede o espaço de endereçamento de 16 bits".into(),
            ));
        }
    }

    Ok(AsmOutput {
        bytes: out,
        origin,
        symbols: labels,
        lines: listing,
        warnings,
    })
}

/// Instruction set accepted by the assembler, chosen with `.setcpu` or the
/// `.p02`-family directives (ca65 names). Until one is given every opcode the
/// assembler knows is accepted ([`AsmCpu::Any`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsmCpu {
    /// No selection yet: everything, including undocumented NMOS opcodes.
    Any,
    /// NMOS 6502, documented opcodes only (`.p02`, `.setcpu "6502"`).
    Nmos6502,
    /// NMOS 6502 plus the undocumented/"illegal" opcodes (`.setcpu "6502X"`).
    /// Mnemonics follow the common docs: `SLO RLA SRE RRA SAX LAX DCP ISC ANC
    /// ALR ARR XAA AXS` and the rest of [`crate::opcodes::OPCODES_TABLE`]'s
    /// unofficial set, with the aliases listed under [`unofficial_alias`].
    Nmos6502X,
    /// 65SC02: 6502 + `BRA PHX PHY PLX PLY STZ TRB TSB`, `(zp)`, `(abs,X)`,
    /// `BIT #`/`zp,X`/`abs,X`, `INC A`/`DEC A` (`.psc02`, `.setcpu "65SC02"`).
    Sc65c02,
    /// 65C02 = 65SC02 + Rockwell `RMB0-7 SMB0-7 BBR0-7 BBS0-7` (`.pc02`,
    /// `.setcpu "65C02"` / `"R65C02"`).
    R65c02,
    /// WDC 65C02 = 65C02 + `WAI STP` (`.pwc02`, `.setcpu "W65C02"`).
    W65c02,
}

impl AsmCpu {
    /// Whether this selection encodes undocumented NMOS opcodes.
    pub fn allows_unofficial(self) -> bool {
        matches!(self, AsmCpu::Any | AsmCpu::Nmos6502X)
    }

    /// Minimum CPU an assembled opcode needs. Undocumented NMOS opcodes need
    /// [`AsmCpu::Nmos6502X`].
    pub fn required_for(opcode: u8) -> AsmCpu {
        use crate::opcodes::{decode_variant, CpuVariant, OpType};
        // CMOS extensions share bytes with NMOS illegals; prefer the CMOS reading.
        if let Some(info) = decode_variant(opcode, CpuVariant::CMOS) {
            if info.op_type == OpType::CmosExtension {
                let name = info.mnemonic;
                if ["RMB", "SMB", "BBR", "BBS"]
                    .iter()
                    .any(|p| name.starts_with(p))
                {
                    return AsmCpu::R65c02;
                }
                if matches!(name, "WAI" | "STP") {
                    return AsmCpu::W65c02;
                }
                return AsmCpu::Sc65c02;
            }
        }
        if let Some(info) = decode_variant(opcode, CpuVariant::NMOS) {
            return if info.op_type == OpType::Official {
                AsmCpu::Nmos6502
            } else {
                AsmCpu::Nmos6502X
            };
        }
        AsmCpu::Sc65c02
    }

    fn rank(self) -> u8 {
        match self {
            AsmCpu::Nmos6502 => 0,
            AsmCpu::Nmos6502X => 0,
            AsmCpu::Sc65c02 => 1,
            AsmCpu::R65c02 => 2,
            AsmCpu::W65c02 => 3,
            AsmCpu::Any => 4,
        }
    }

    /// Whether this selection accepts `opcode`.
    pub fn accepts(self, opcode: u8) -> bool {
        let need = Self::required_for(opcode);
        if need == AsmCpu::Nmos6502X {
            return self.allows_unofficial();
        }
        need.rank() <= self.rank()
    }

    pub fn name(self) -> &'static str {
        match self {
            AsmCpu::Any => "qualquer",
            AsmCpu::Nmos6502 => "6502",
            AsmCpu::Nmos6502X => "6502X",
            AsmCpu::Sc65c02 => "65SC02",
            AsmCpu::R65c02 => "65C02",
            AsmCpu::W65c02 => "W65C02",
        }
    }
}

/// Canonical table mnemonic for an undocumented-opcode alias, or `mnemonic`
/// itself when it is already the table name (or not an alias).
pub fn unofficial_alias(mnemonic: &str) -> &'static str {
    match mnemonic.to_ascii_uppercase().as_str() {
        "ASO" => "SLO",
        "LSE" => "SRE",
        "AXS" => "SBX",
        "XAA" => "ANE",
        "AHX" => "SHA",
        "TAS" => "SHS",
        "LXA" => "LAX",
        "DCM" => "DCP",
        "INS" | "ISB" => "ISC",
        "ASR" => "ALR",
        "USBC" => "SBC",
        "KIL" | "HLT" => "JAM",
        "SLO" => "SLO",
        "RLA" => "RLA",
        "SRE" => "SRE",
        "RRA" => "RRA",
        "SAX" => "SAX",
        "LAX" => "LAX",
        "DCP" => "DCP",
        "ISC" => "ISC",
        "ANC" => "ANC",
        "ALR" => "ALR",
        "ARR" => "ARR",
        "ANE" => "ANE",
        "SBX" => "SBX",
        "SHA" => "SHA",
        "SHS" => "SHS",
        "SHX" => "SHX",
        "SHY" => "SHY",
        "LAS" => "LAS",
        "JAM" => "JAM",
        _ => "",
    }
}

/// Encode `mnemonic`/`mode` for a CPU selection.
///
/// Search order: official NMOS, then (if the CPU is CMOS-class) the 65C02
/// extensions, then (if [`AsmCpu::allows_unofficial`]) undocumented NMOS
/// opcodes and their [`unofficial_alias`] names. Unlike
/// [`crate::opcodes::encode`], this never returns a CMOS encoding for a
/// 6502 / 6502X selection.
pub fn encode_for(
    mnemonic: &str,
    mode: Mode,
    cpu: AsmCpu,
) -> Option<&'static crate::opcodes::Opcode> {
    use crate::opcodes::{OpType, CMOS_BIT_OPS, CMOS_OVERRIDES, OPCODES_TABLE};
    for op in OPCODES_TABLE.iter() {
        if op.op_type == OpType::Official
            && op.mnemonic.eq_ignore_ascii_case(mnemonic)
            && op.mode == mode
        {
            return Some(op);
        }
    }
    let cmos = matches!(
        cpu,
        AsmCpu::Any | AsmCpu::Sc65c02 | AsmCpu::R65c02 | AsmCpu::W65c02
    );
    if cmos {
        if let Some(op) = CMOS_OVERRIDES
            .iter()
            .chain(CMOS_BIT_OPS.iter())
            .find(|op| op.mnemonic.eq_ignore_ascii_case(mnemonic) && op.mode == mode)
        {
            // Rockwell / WDC gated by level.
            let need = required_for_mnemonic(&mnemonic.to_ascii_uppercase());
            if need.rank() <= cpu.rank() || cpu == AsmCpu::Any {
                return Some(op);
            }
            return None;
        }
    }
    if !cpu.allows_unofficial() {
        return None;
    }
    let alias = unofficial_alias(mnemonic);
    let canon = if alias.is_empty() { mnemonic } else { alias };
    OPCODES_TABLE.iter().find(|op| {
        op.op_type == OpType::Unofficial
            && op.mnemonic.eq_ignore_ascii_case(canon)
            && op.mode == mode
    })
}

/// `Some(cpu)` for a CPU selection line, `None` for anything else.
fn cpu_directive(line: &str) -> Option<Result<AsmCpu, AsmError>> {
    let mut words = line.split_whitespace();
    let head = words.next()?.to_ascii_lowercase();
    let fixed = match head.as_str() {
        ".p02" => Some(AsmCpu::Nmos6502),
        ".psc02" => Some(AsmCpu::Sc65c02),
        ".pc02" => Some(AsmCpu::R65c02),
        ".pwc02" => Some(AsmCpu::W65c02),
        ".setcpu" => None,
        _ => return None,
    };
    let rest: Vec<&str> = words.collect();
    if let Some(cpu) = fixed {
        return Some(if rest.is_empty() {
            Ok(cpu)
        } else {
            Err(AsmError(format!("{head} não leva operando")))
        });
    }
    let arg = rest.join(" ");
    let name = arg.trim().trim_matches('"').to_ascii_uppercase();
    Some(match name.as_str() {
        "6502" => Ok(AsmCpu::Nmos6502),
        "6502X" => Ok(AsmCpu::Nmos6502X),
        "65SC02" => Ok(AsmCpu::Sc65c02),
        "65C02" | "R65C02" => Ok(AsmCpu::R65c02),
        "W65C02" => Ok(AsmCpu::W65c02),
        "" => Err(AsmError(".setcpu exige o nome da CPU".into())),
        _ => Err(AsmError(format!("CPU desconhecida em .setcpu: {arg}"))),
    })
}

/// Reject an instruction line whose mnemonic the selected CPU lacks.
///
/// Classification is by mnemonic (an opcode byte is shared by NMOS illegal
/// `NOP zp` and CMOS `STZ zp`), so a CMOS program is never told it needs 6502X.
fn check_cpu(cpu: AsmCpu, rest: &str, bytes: &[u8]) -> Result<(), AsmError> {
    let head = rest.split_whitespace().next().unwrap_or("");
    if cpu == AsmCpu::Any || head.starts_with('.') || bytes.is_empty() {
        return Ok(());
    }
    let upper = head.to_ascii_uppercase();
    if matches!(upper.as_str(), "DB" | "DW") {
        return Ok(());
    }
    let need = required_for_mnemonic(&upper);
    let ok = match need {
        AsmCpu::Nmos6502X => cpu.allows_unofficial(),
        other => other.rank() <= cpu.rank(),
    };
    if !ok {
        return Err(AsmError(format!(
            "{upper} (opcode ${:02X}) exige {}, CPU atual {}",
            bytes[0],
            need.name(),
            cpu.name()
        )));
    }
    Ok(())
}

/// CPU required by a mnemonic (uppercase). Official NMOS names need 6502;
/// undocumented names/aliases need 6502X; Rockwell bit ops need 65C02; `WAI`/
/// `STP` need W65C02; other CMOS-only names need 65SC02.
fn required_for_mnemonic(upper: &str) -> AsmCpu {
    use crate::opcodes::{OpType, OPCODES_TABLE};
    if upper.starts_with("RMB")
        || upper.starts_with("SMB")
        || upper.starts_with("BBR")
        || upper.starts_with("BBS")
    {
        return AsmCpu::R65c02;
    }
    if matches!(upper, "WAI" | "STP") {
        return AsmCpu::W65c02;
    }
    if matches!(
        upper,
        "BRA" | "PHX" | "PHY" | "PLX" | "PLY" | "STZ" | "TRB" | "TSB"
    ) {
        return AsmCpu::Sc65c02;
    }
    let alias = unofficial_alias(upper);
    let name = if alias.is_empty() { upper } else { alias };
    let unofficial = OPCODES_TABLE
        .iter()
        .any(|op| op.op_type == OpType::Unofficial && op.mnemonic.eq_ignore_ascii_case(name));
    // `NOP`/`SBC` exist as both official and unofficial; the official form wins
    // unless the only encodings are unofficial (handled by encode_for elsewhere).
    if unofficial
        && !OPCODES_TABLE
            .iter()
            .any(|op| op.op_type == OpType::Official && op.mnemonic.eq_ignore_ascii_case(name))
    {
        return AsmCpu::Nmos6502X;
    }
    AsmCpu::Nmos6502
}

fn at_line(origin: &str, e: AsmError) -> AsmError {
    if origin.is_empty() {
        e
    } else {
        AsmError(format!("linha {origin}: {}", e.0))
    }
}

pub(crate) fn strip_comment(s: &str) -> &str {
    // The preprocessor's line tag (U+0001) ends every line, strings included.
    let s = s.split('\u{1}').next().unwrap_or("");
    match find_unquoted(s, ';') {
        Some(index) => s[..index].trim(),
        None => s.trim(),
    }
}

/// Operand text with whitespace removed and letters uppercased, except inside
/// `"strings"` and `'c'` literals, which stay verbatim.
pub(crate) fn normalize_operand(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while let Some(c) = s[i..].chars().next() {
        let verbatim = match c {
            '"' => crate::escape::string_end(&s[i..]).map_or(s.len() - i, |e| e + 1),
            '\'' => crate::escape::char_literal_len(&s[i..]).unwrap_or(1),
            _ => 0,
        };
        if verbatim > 0 {
            out.push_str(&s[i..i + verbatim]);
            i += verbatim;
            continue;
        }
        if c.is_whitespace() {
            // A run of spaces between two word characters stays as one space, so
            // `a .lt 2` keeps its word operator; every other space is dropped.
            let word =
                |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$' | '%' | '@');
            let next = s[i..].trim_start().chars().next();
            if out.chars().last().is_some_and(word) && next.is_some_and(word) && !out.ends_with(' ')
            {
                out.push(' ');
            }
        } else {
            out.push(c.to_ascii_uppercase());
        }
        i += c.len_utf8();
    }
    out
}

/// Split at every `sep` outside strings and character literals.
pub(crate) fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = s;
    while let Some(i) = find_unquoted(rest, sep) {
        parts.push(&rest[..i]);
        rest = &rest[i + sep.len_utf8()..];
    }
    parts.push(rest);
    parts
}

/// First `target` outside `"strings"` and `'c'` character literals.
pub(crate) fn find_unquoted(s: &str, target: char) -> Option<usize> {
    let mut i = 0;
    while let Some(c) = s[i..].chars().next() {
        match c {
            '"' => i += crate::escape::string_end(&s[i..]).map_or(s.len() - i, |e| e + 1),
            '\'' => i += crate::escape::char_literal_len(&s[i..]).unwrap_or(1),
            _ if c == target => return Some(i),
            _ => i += c.len_utf8(),
        }
    }
    None
}

pub(crate) fn split_label(s: &str) -> (Option<&str>, &str) {
    // `:-` / `:+` is an anonymous-label reference, not a label separator.
    if let Some(i) = find_unquoted(s, ':').filter(|&i| !s[i + 1..].starts_with(['-', '+'])) {
        (Some(s[..i].trim()), s[i + 1..].trim())
    } else {
        (None, s)
    }
}

fn parse_org(s: &str) -> Option<&str> {
    let s = s.trim_start();
    if s.get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(".org"))
    {
        Some(s[4..].trim())
    } else if let Some(rest) = s.strip_prefix('*') {
        rest.trim_start().strip_prefix('=').map(str::trim)
    } else {
        None
    }
}

fn parse_align(s: &str) -> Option<&str> {
    let s = s.trim_start();
    let rest = s
        .strip_prefix(".align")
        .or_else(|| s.strip_prefix(".ALIGN"))?;
    if !rest.is_empty() && !rest.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    Some(rest)
}

fn align_pad(pc: u32, n: u16) -> Result<u32, AsmError> {
    if n == 0 {
        return Err(AsmError(".align exige um valor diferente de zero".into()));
    }
    let n = u32::from(n);
    Ok((n - (pc % n)) % n)
}

fn parse_equ(s: &str) -> Option<(&str, &str)> {
    // `name = expr`: one name before a lone `=` (not `==`, `<=`, `>=`, `!=`), so a
    // comparison in an operand such as `.byte a == 1` is not an assignment.
    if let Some(i) = find_unquoted(s, '=') {
        let name = s[..i].trim();
        let lone = !s[i + 1..].starts_with('=');
        let plain =
            !name.is_empty() && !name.starts_with('.') && !name.contains(char::is_whitespace);
        if !s.starts_with('*') && lone && plain {
            return Some((name, s[i + 1..].trim()));
        }
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() == 3 && parts[1].eq_ignore_ascii_case(".equ") {
        return Some((parts[0], parts[2]));
    }
    None
}

fn size_of_instr(
    s: &str,
    labels: &HashMap<String, u16>,
    equ: &HashSet<String>,
    cpu: AsmCpu,
) -> Result<usize, AsmError> {
    emit_impl(s, 0, labels, equ, true, cpu).map(|v| v.len())
}

fn emit(
    s: &str,
    pc: u16,
    labels: &HashMap<String, u16>,
    equ: &HashSet<String>,
    cpu: AsmCpu,
) -> Result<Vec<u8>, AsmError> {
    emit_impl(s, pc, labels, equ, false, cpu)
}

fn emit_byte_string(raw: &str, labels: &HashMap<String, u16>) -> Result<Vec<u8>, AsmError> {
    let mut out = Vec::new();
    let mut rest = raw.trim();
    while !rest.is_empty() {
        if let Some(body) = rest.strip_prefix('"') {
            let end = match crate::escape::string_end(rest) {
                Some(end) => end - 1,
                None => return Err(AsmError(format!("string sem aspas correspondentes: {raw}"))),
            };
            out.extend(crate::escape::decode_string(&body[..end]).map_err(AsmError)?);
            rest = body[end + 1..].trim_start();
            if rest.is_empty() {
                break;
            }
            if !rest.starts_with(',') {
                return Err(AsmError(format!("vírgula ausente entre elementos: {raw}")));
            }
            rest = rest[1..].trim_start();
            if rest.is_empty() {
                return Err(AsmError(format!("string sem aspas correspondentes: {raw}")));
            }
        } else {
            let (part, next) = match find_unquoted(rest, ',') {
                Some(i) => (rest[..i].trim(), rest[i + 1..].trim_start()),
                None => (rest.trim(), ""),
            };
            if part.is_empty()
                || (part.contains('"') && crate::escape::char_literal_len(part).is_none())
            {
                return Err(AsmError(format!("vírgula ausente entre elementos: {raw}")));
            }
            let value = resolve(&normalize_operand(part), labels)?;
            let byte =
                u8::try_from(value).map_err(|_| AsmError(format!("byte fora da faixa: {part}")))?;
            out.push(byte);
            rest = next;
        }
    }
    Ok(out)
}

fn emit_impl(
    s: &str,
    pc: u16,
    labels: &HashMap<String, u16>,
    equ: &HashSet<String>,
    layout: bool,
    cpu: AsmCpu,
) -> Result<Vec<u8>, AsmError> {
    let head = s.split_whitespace().next().unwrap_or("");
    let mnemonic = head.to_ascii_uppercase();
    let arg = normalize_operand(s.trim_start().get(head.len()..).unwrap_or(""));

    // Diretivas de dados (.BYTE / .DB / DB)
    if mnemonic == "DB" || mnemonic == ".DB" || mnemonic == ".BYTE" {
        let raw = s.trim_start().get(head.len()..).unwrap_or("").trim();
        if raw.contains('"') {
            return emit_byte_string(raw, labels);
        }
        return split_unquoted(&arg, ',')
            .into_iter()
            .map(|part| {
                u8::try_from(resolve(part, labels)?)
                    .map_err(|_| AsmError(format!("byte fora da faixa: {part}")))
            })
            .collect();
    }

    // Diretivas de palavra de 16 bits (.WORD / DW)
    if mnemonic == "DW" || mnemonic == ".WORD" {
        if arg.contains('"') {
            return Err(AsmError(".word não aceita string".into()));
        }
        let mut bytes = Vec::new();
        for part in split_unquoted(&arg, ',') {
            let val = resolve(part, labels)?;
            bytes.extend(val.to_le_bytes());
        }
        return Ok(bytes);
    }

    // Tabelas divididas: .LOBYTES / .HIBYTES emitem o byte baixo/alto de cada valor;
    // .DBYT emite palavras big-endian.
    if mnemonic == ".LOBYTES" || mnemonic == ".HIBYTES" || mnemonic == ".DBYT" {
        if arg.is_empty() || arg.contains('"') {
            return Err(AsmError(format!(
                "{} exige valores numéricos",
                head.to_ascii_lowercase()
            )));
        }
        let mut bytes = Vec::new();
        for part in split_unquoted(&arg, ',') {
            let [lo, hi] = resolve(part, labels)?.to_le_bytes();
            match mnemonic.as_str() {
                ".LOBYTES" => bytes.push(lo),
                ".HIBYTES" => bytes.push(hi),
                _ => bytes.extend([hi, lo]),
            }
        }
        return Ok(bytes);
    }

    // .ASSERT expr[, "mensagem"]: falha a montagem quando expr é zero (checada na
    // passada final, quando todos os símbolos existem). Não emite bytes.
    if mnemonic == ".ASSERT" {
        let raw = s.trim_start().get(head.len()..).unwrap_or("").trim();
        let (expr, message) = match raw.split_once(',') {
            Some((e, m)) => (e.trim(), m.trim().trim_matches('"')),
            None => (raw, ""),
        };
        if expr.is_empty() {
            return Err(AsmError(".assert exige uma expressão".into()));
        }
        if !layout && resolve(&expr.to_ascii_uppercase().replace(' ', ""), labels)? == 0 {
            return Err(AsmError(if message.is_empty() {
                format!(".assert falhou: {expr}")
            } else {
                format!(".assert falhou: {message}")
            }));
        }
        return Ok(Vec::new());
    }

    // .FARADDR (24 bits) e .DWORD (32 bits), little-endian; os valores do montador
    // têm 16 bits, então os bytes altos são zero.
    if mnemonic == ".FARADDR" || mnemonic == ".DWORD" {
        if arg.is_empty() || arg.contains('"') {
            return Err(AsmError(format!(
                "{} exige valores numéricos",
                head.to_ascii_lowercase()
            )));
        }
        let width = if mnemonic == ".FARADDR" { 3 } else { 4 };
        let mut bytes = Vec::new();
        for part in split_unquoted(&arg, ',') {
            let [lo, hi] = resolve(part, labels)?.to_le_bytes();
            bytes.extend([lo, hi, 0, 0].iter().take(width));
        }
        return Ok(bytes);
    }

    // .PSTRING "texto": string Pascal, um byte de tamanho (até 255) e o texto.
    if mnemonic == ".PSTRING" {
        let raw = s.trim_start()[head.len()..].trim();
        let text = whole_string(raw, s)?;
        let len = u8::try_from(text.len())
            .map_err(|_| AsmError(format!(".pstring com mais de 255 bytes: {}", text.len())))?;
        let mut bytes = vec![len];
        bytes.extend(text);
        return Ok(bytes);
    }

    // Diretivas de texto (.TEXT / .ASCII / .ASCIIZ)
    if mnemonic == ".TEXT" || mnemonic == ".ASCII" || mnemonic == ".ASCIIZ" {
        let is_null_terminated = mnemonic == ".ASCIIZ";
        let raw_text = s.trim_start()[mnemonic.len()..].trim();
        let mut bytes = whole_string(raw_text, s)?;
        if is_null_terminated {
            bytes.push(0);
        }
        return Ok(bytes);
    }

    // Diretiva de preenchimento (.FILL <count>, [val])
    if mnemonic == ".FILL" || mnemonic == ".RES" {
        let subparts: Vec<&str> = split_unquoted(&arg, ',');
        if subparts.len() > 2 {
            return Err(AsmError(
                ".fill aceita quantidade e, opcionalmente, um byte".into(),
            ));
        }
        let count = resolve(subparts[0], labels)? as usize;
        let val = if subparts.len() > 1 {
            u8::try_from(resolve(subparts[1], labels)?)
                .map_err(|_| AsmError("valor de .fill fora da faixa de byte".into()))?
        } else {
            0
        };
        return Ok(vec![val; count]);
    }

    let (mode, expr) = match operand_syntax_for(&mnemonic, &arg, cpu)
        .map_err(|e| AsmError(format!("{e}: {s}")))?
    {
        Syntax::ZpRel(zp, target) => {
            let op = encode_for(&mnemonic, Mode::ZpRel, cpu).expect("syntax checked the mnemonic");
            let zp = u8::try_from(resolve(zp, labels)?)
                .map_err(|_| AsmError(format!("operando fora da página zero: {zp}")))?;
            let offset = resolve(target, labels)?.wrapping_sub(pc.wrapping_add(3)) as i16;
            if !layout && !(-128..=127).contains(&offset) {
                return Err(AsmError(format!("branch fora de alcance: {s}")));
            }
            return Ok(vec![op.code, zp, offset as u8]);
        }
        Syntax::Fixed(mode, expr) => (mode, expr),
        Syntax::Sized { expr, short, long } => {
            if bare_char_const(expr) {
                return Err(AsmError("caractere não é endereço".into()));
            }
            let value = resolve(expr, labels)?;
            // Any label inside the expression (`select+2`, `<table`) forces the
            // absolute form, so both passes pick the same size for forward references.
            let wide = expr
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
                .any(|word| {
                    let word = word.to_ascii_lowercase();
                    labels.contains_key(&word) && !equ.contains(&word)
                })
                || expr.strip_prefix('$').is_some_and(|v| v.len() > 2)
                || expr.strip_prefix("0X").is_some_and(|v| v.len() > 2);
            (
                pick_size_for(&mnemonic, short, long, value <= 255 && !wide, cpu),
                expr,
            )
        }
    };

    let op = encode_for(&mnemonic, mode, cpu)
        .ok_or_else(|| AsmError(format!("instrução ou modo inválido: {s}")))?;
    let mut bytes = vec![op.code];
    if matches!(mode, Mode::Imp | Mode::Acc) {
        return Ok(bytes);
    }
    let value = resolve(expr, labels)?;
    if mode == Mode::Rel {
        let offset = value.wrapping_sub(pc.wrapping_add(2)) as i16;
        if !layout && !(-128..=127).contains(&offset) {
            return Err(AsmError(format!("branch fora de alcance: {s}")));
        }
        bytes.push(offset as u8);
    } else if mode.size() == 2 {
        bytes.push(
            u8::try_from(value)
                .map_err(|_| AsmError(format!("operando fora da faixa de 8 bits: {expr}")))?,
        );
    } else {
        bytes.extend(value.to_le_bytes());
    }
    Ok(bytes)
}

/// Operand syntax of one instruction, shared by the assembler and the object assembler
/// (`link`), which differ only in how they resolve the expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Syntax<'a> {
    /// Mode fixed by the text: implied, accumulator, immediate, relative, indirect forms.
    Fixed(Mode, &'a str),
    /// `expr`, `expr,X`, `expr,Y`: zero page or absolute, chosen by the caller.
    Sized {
        expr: &'a str,
        short: Mode,
        long: Mode,
    },
    /// `BBRn`/`BBSn zp,target`.
    ZpRel(&'a str, &'a str),
}

/// Classify `arg` (upper case, no spaces) for `mnemonic`.
pub(crate) fn operand_syntax<'a>(mnemonic: &str, arg: &'a str) -> Result<Syntax<'a>, String> {
    operand_syntax_for(mnemonic, arg, AsmCpu::Any)
}

pub(crate) fn operand_syntax_for<'a>(
    mnemonic: &str,
    arg: &'a str,
    cpu: AsmCpu,
) -> Result<Syntax<'a>, String> {
    if encode_for(mnemonic, Mode::ZpRel, cpu).is_some() {
        let (zp, target) = arg
            .split_once(',')
            .ok_or_else(|| format!("{mnemonic} precisa de zp,destino"))?;
        return Ok(Syntax::ZpRel(zp, target));
    }
    Ok(Syntax::Fixed(
        if encode_for(mnemonic, Mode::Rel, cpu).is_some() {
            return Ok(Syntax::Fixed(Mode::Rel, arg));
        } else if arg.is_empty() {
            if encode_for(mnemonic, Mode::Imp, cpu).is_some() {
                Mode::Imp
            } else {
                Mode::Acc
            }
        } else if arg == "A" && encode_for(mnemonic, Mode::Acc, AsmCpu::Any).is_some() {
            // Bare `A` is the accumulator, never the hex value $0A, whenever the
            // mnemonic has an accumulator form on any CPU (ca65 rule).
            Mode::Acc
        } else if let Some(rest) = arg.strip_prefix('#') {
            return Ok(Syntax::Fixed(Mode::Imm, rest));
        } else if arg.starts_with('(') && arg.ends_with(",X)") {
            return Ok(Syntax::Fixed(
                if encode_for(mnemonic, Mode::AbsIndX, cpu).is_some() {
                    Mode::AbsIndX
                } else {
                    Mode::IndX
                },
                &arg[1..arg.len() - 3],
            ));
        } else if arg.starts_with('(') && arg.ends_with("),Y") {
            return Ok(Syntax::Fixed(Mode::IndY, &arg[1..arg.len() - 3]));
        } else if arg.starts_with('(') && arg.ends_with(')') {
            let inner = &arg[1..arg.len() - 1];
            let mode = if encode_for(mnemonic, Mode::Ind, cpu).is_some() {
                Mode::Ind
            } else {
                Mode::ZpInd
            };
            return Ok(Syntax::Fixed(mode, inner));
        } else {
            let (expr, short, long) = if let Some(rest) = arg.strip_suffix(",X") {
                (rest, Mode::ZpX, Mode::AbsX)
            } else if let Some(rest) = arg.strip_suffix(",Y") {
                (rest, Mode::ZpY, Mode::AbsY)
            } else {
                (arg, Mode::Zp, Mode::Abs)
            };
            if expr.contains('(') || expr.contains(')') {
                return Err("modo de endereçamento inválido".into());
            }
            return Ok(Syntax::Sized { expr, short, long });
        },
        "",
    ))
}

/// Zero page when allowed and the mnemonic has it, else absolute (or zero page when
/// the mnemonic has no absolute form, e.g. `STX zp,Y`).
pub(crate) fn pick_size(mnemonic: &str, short: Mode, long: Mode, small: bool) -> Mode {
    pick_size_for(mnemonic, short, long, small, AsmCpu::Any)
}

pub(crate) fn pick_size_for(
    mnemonic: &str,
    short: Mode,
    long: Mode,
    small: bool,
    cpu: AsmCpu,
) -> Mode {
    let has_short = encode_for(mnemonic, short, cpu).is_some();
    if has_short && (small || encode_for(mnemonic, long, cpu).is_none()) {
        short
    } else {
        long
    }
}

fn bare_char_const(expr: &str) -> bool {
    crate::escape::char_literal_len(expr) == Some(expr.len())
}

/// The whole operand is one `"string"`; its bytes with escapes decoded.
fn whole_string(raw: &str, line: &str) -> Result<Vec<u8>, AsmError> {
    let raw = raw.trim();
    match crate::escape::string_end(raw) {
        Some(end) if raw.starts_with('"') && end == raw.len() - 1 => {
            crate::escape::decode_string(&raw[1..end]).map_err(|e| AsmError(format!("{e}: {line}")))
        }
        _ => Err(AsmError(format!(
            "string sem aspas correspondentes: {line}"
        ))),
    }
}

fn resolve(s: &str, labels: &HashMap<String, u16>) -> Result<u16, AsmError> {
    parse_expr(s, labels)
}

fn parse_expr(s: &str, labels: &HashMap<String, u16>) -> Result<u16, AsmError> {
    crate::expr::eval(s, &|name| labels.get(&name.to_ascii_lowercase()).copied())
        .map_err(|err| AsmError(err.to_string()))
}

/// Converte um bloco binário para o formato Intel HEX (.hex).
pub fn to_intel_hex(bytes: &[u8], start_addr: u16) -> String {
    let mut out = String::new();
    let mut addr = start_addr;
    for chunk in bytes.chunks(16) {
        let count = chunk.len() as u8;
        let mut checksum = count
            .wrapping_add((addr >> 8) as u8)
            .wrapping_add((addr & 0xFF) as u8);
        out.push_str(&format!(":{:02X}{:04X}00", count, addr));
        for &b in chunk {
            out.push_str(&format!("{:02X}", b));
            checksum = checksum.wrapping_add(b);
        }
        let final_chk = (!checksum).wrapping_add(1);
        out.push_str(&format!("{:02X}\n", final_chk));
        addr = addr.wrapping_add(count as u16);
    }
    out.push_str(":00000001FF\n"); // End of File Record
    out
}

include!("asm_pre.rs");
