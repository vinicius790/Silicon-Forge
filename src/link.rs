//! Two-pass linker for relocatable 6502 objects.
//!
//! Pass 1 assigns each named section an address (fixed `.org`, or `reloc_base`
//! for a section without one) and builds the symbol table. Pass 2 patches
//! absolute little-endian relocations, absolute byte relocations, and
//! PC-relative branch relocations.
//! An undefined symbol, a duplicate global, or overlapping sections is an error.
//!
//! [`link_with_options`] adds linker-defined symbols: strong `define`s, weak
//! `provide`s that objects override, and per-section `__name_start/_end/_size`.
//! [`Linked::map`] prints a map file and [`Linked::image`] a flat binary.
//! [`link_banks`] links overlays: banks that share addresses, each seeing the fixed
//! code and itself only.
//!
//! [`write_object`] / [`read_object`] store the same module, including the
//! source notes [`Linked::line_at`] turns into addresses after placement.

use crate::asm::Syntax;
use crate::asm::{split_label, strip_comment};
use crate::expr;
use crate::opcodes::{encode, Mode};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    pub message: String,
    /// Sections already patched when an absolute byte was `fora de faixa`
    /// or a relative branch was out of range. In-range sites stay patched.
    /// Empty for every other error.
    pub patched: Vec<Segment>,
}

impl LinkError {
    fn plain(message: String) -> Self {
        Self {
            message,
            patched: Vec::new(),
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LinkError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocKind {
    Absolute,
    Relative,
    /// One absolute byte. A defined value above 255 is `fora de faixa`.
    AbsoluteByte,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reloc {
    pub offset: u16,
    pub symbol: String,
    pub kind: RelocKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub origin: Option<u16>,
    pub bytes: Vec<u8>,
    pub relocs: Vec<Reloc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub section: usize,
    pub offset: u16,
    pub global: bool,
    /// Weak global (`.weak` / library member): overridden by a strong global of
    /// the same name, and not an error when several weak definitions exist.
    pub weak: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceNote {
    pub section: usize,
    pub offset: u16,
    pub line: u32,
    pub text: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    pub name: String,
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
    pub notes: Vec<SourceNote>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub address: u16,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappedLine {
    pub address: u16,
    pub line: u32,
    pub text: String,
    pub file: String,
}

/// One line of a linker script: `section <nome> <endereço>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionAssign {
    pub name: String,
    pub address: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linked {
    pub symbols: BTreeMap<String, u16>,
    pub segments: Vec<Segment>,
    pub lines: Vec<MappedLine>,
    /// Placement of every section: name, module name, address, length.
    pub placements: Vec<Placement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub section: String,
    pub module: String,
    pub address: u16,
    pub len: usize,
    /// Fill bytes appended after the section by a [`SectionFill`].
    pub padding: usize,
}

/// Everything [`link_with_options`] takes besides the objects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkOptions {
    pub script: Vec<SectionAssign>,
    pub reloc_base: u16,
    /// Weak definitions (`PROVIDE`): used only when no object defines the name.
    pub provide: BTreeMap<String, u16>,
    /// Strong linker definitions (`--defsym`): an object global of the same name is
    /// a duplicate.
    pub define: BTreeMap<String, u16>,
    /// Define `__<section>_start`, `__<section>_end` (one past the last byte) and
    /// `__<section>_size` for each section name, weakly. Sections sharing a name
    /// span from the lowest start to the highest end; the size is their byte total.
    pub section_symbols: bool,
    /// Memory regions (`region <nome> <início> <fim>`, inclusive). When any are
    /// given every non-empty section must lie wholly inside one of them (ROM/RAM
    /// range checks); overlapping regions are an error.
    pub regions: Vec<Region>,
    /// Size limits per section name (`limit <seção> <máx>`): the bytes of every
    /// section with that name, summed, must not exceed `max`.
    pub limits: Vec<SectionLimit>,
    /// Padding per section name (`fill <seção> <fronteira> <byte>`): the last
    /// placed section of that name is extended with `byte` up to the next multiple
    /// of `boundary` (a bank boundary). The padding must not overlap anything.
    pub fills: Vec<SectionFill>,
    /// Garbage-collect sections (`gc` in the script): keep only sections reachable
    /// from [`LinkOptions::keep`] through relocations, drop the rest before
    /// placement. See [`collect_garbage`].
    pub gc: bool,
    /// GC roots (`keep <nome>`): a symbol name (the section that defines it is kept)
    /// or a section name (every section with that name is kept).
    pub keep: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionLimit {
    pub name: String,
    pub max: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionFill {
    pub name: String,
    pub boundary: u32,
    pub byte: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub name: String,
    pub start: u16,
    pub end: u16,
}

impl Linked {
    /// Human-readable map: sections by address (start, last byte, size, section,
    /// module), then every symbol by address then name.
    /// Human-readable map (Portuguese headers). Prefer [`Linked::map_ld65`] for
    /// the ld65 layout.
    pub fn map(&self) -> String {
        let mut out = String::from("SEÇÕES\n");
        let mut ps = self.placements.clone();
        ps.sort_by_key(|p| (p.address, p.section.clone()));
        for p in &ps {
            let last = if p.len == 0 {
                p.address
            } else {
                p.address.wrapping_add(p.len as u16 - 1)
            };
            out += &format!(
                "  ${:04X}-${last:04X} {:>5}  {} ({})\n",
                p.address, p.len, p.section, p.module
            );
        }
        out += "SÍMBOLOS\n";
        let mut syms: Vec<(&u16, &String)> = self.symbols.iter().map(|(n, a)| (a, n)).collect();
        syms.sort();
        for (a, n) in syms {
            out += &format!("  ${a:04X} {n}\n");
        }
        out
    }

    /// Flat image of `start..=end`, unused bytes set to `fill`.
    /// Map file in the ld65 layout: `Modules list`, `Segment list`,
    /// `Exports list by name`, `Exports list by value`, `Imports list`.
    ///
    /// Addresses are 6-digit hex (`$00XXXX`). The imports list is empty: objects
    /// in this linker do not retain which module imported a symbol after the
    /// link (relocations are already patched).
    pub fn map_ld65(&self) -> String {
        let mut out = String::from("Modules list:\n------------\n");
        let mut modules: BTreeMap<&str, Vec<&Placement>> = BTreeMap::new();
        for p in &self.placements {
            modules.entry(p.module.as_str()).or_default().push(p);
        }
        for (i, (name, ps)) in modules.iter().enumerate() {
            out += &format!("{i}. {name}:\n");
            let mut segs: Vec<&Placement> = ps.to_vec();
            segs.sort_by_key(|p| p.address);
            for p in segs {
                let size = p.len + p.padding;
                out += &format!(
                    "    {:16} Offs={:06X} Size={:06X} Align=00001  Base={:06X}\n",
                    p.section,
                    0u32,
                    size,
                    u32::from(p.address)
                );
            }
            out.push('\n');
        }
        out += "Segment list:\n-------------\n";
        out += "Name                   Start     End    Size  Align\n";
        out += "----------------------------------------------\n";
        let mut segs = self.placements.clone();
        segs.sort_by_key(|p| (p.address, p.section.clone()));
        for p in &segs {
            let size = p.len + p.padding;
            let end = if size == 0 {
                u32::from(p.address)
            } else {
                u32::from(p.address) + size as u32 - 1
            };
            out += &format!(
                "{:22} {:06X}  {:06X}  {:06X}  00001\n",
                p.section,
                u32::from(p.address),
                end,
                size
            );
        }
        out += "\nExports list by name:\n---------------------\n";
        let mut by_name: Vec<_> = self.symbols.iter().collect();
        by_name.sort_by(|a, b| a.0.cmp(b.0));
        for (name, val) in &by_name {
            out += &format!("{name:24} {:06X}\n", u32::from(**val));
        }
        out += "\nExports list by value:\n----------------------\n";
        let mut by_val = by_name;
        by_val.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));
        for (name, val) in &by_val {
            out += &format!("{:06X}  {name}\n", u32::from(**val));
        }
        out += "\nImports list:\n-------------\n";
        out += "(nenhum: relocações já aplicadas)\n";
        out
    }

    pub fn image(&self, start: u16, end: u16, fill: u8) -> Vec<u8> {
        let mut mem = vec![fill; 0x10000];
        self.store(&mut mem);
        mem[usize::from(start)..=usize::from(end)].to_vec()
    }

    pub fn store(&self, mem: &mut [u8]) {
        for segment in &self.segments {
            for (index, byte) in segment.bytes.iter().enumerate() {
                let addr = segment.address.wrapping_add(index as u16) as usize;
                if let Some(slot) = mem.get_mut(addr) {
                    *slot = *byte;
                }
            }
        }
    }

    pub fn line_at(&self, address: u16) -> Option<&MappedLine> {
        self.lines.iter().find(|line| line.address == address)
    }
}

pub fn link(objects: &[Object], reloc_base: u16) -> Result<Linked, LinkError> {
    link_with_script(objects, &[], reloc_base)
}

/// Two-pass link. `script` fixes the start address of each named section.
/// A section absent from the script keeps `.org`, or `reloc_base` when it has none.
/// A script name with no section, a duplicated script name, or a script address
/// that disagrees with `.org` is an error.
pub fn link_with_script(
    objects: &[Object],
    script: &[SectionAssign],
    reloc_base: u16,
) -> Result<Linked, LinkError> {
    link_with_options(
        objects,
        &LinkOptions {
            script: script.to_vec(),
            reloc_base,
            ..LinkOptions::default()
        },
    )
}

/// [`link_with_script`] plus linker-defined symbols (see [`LinkOptions`]).
/// Resolution order: the referencing module's own symbols, object globals, strong
/// defines, then weak provides and section symbols.
pub fn link_with_options(objects: &[Object], options: &LinkOptions) -> Result<Linked, LinkError> {
    if options.gc {
        let kept = collect_garbage(objects, &options.keep)?;
        let rest = LinkOptions {
            gc: false,
            ..options.clone()
        };
        return link_with_options(&kept, &rest);
    }
    let script = &options.script[..];
    let reloc_base = options.reloc_base;
    if objects.iter().all(|object| object.sections.is_empty()) {
        return Err(LinkError::plain("lista de seções vazia".into()));
    }
    let mut scripted = BTreeMap::new();
    for item in script {
        if scripted.insert(item.name.clone(), item.address).is_some() {
            return Err(LinkError::plain(format!(
                "script duplica a seção {}",
                item.name
            )));
        }
    }
    for name in scripted.keys() {
        let found = objects
            .iter()
            .any(|object| object.sections.iter().any(|section| &section.name == name));
        if !found {
            return Err(LinkError::plain(format!("script sem seção: {name}")));
        }
    }
    let mut placed: Vec<(usize, usize, u16)> = Vec::new();
    let mut cursor = reloc_base;
    for (module, object) in objects.iter().enumerate() {
        for (index, section) in object.sections.iter().enumerate() {
            let len = section.bytes.len();
            let address = if let Some(at) = scripted.get(&section.name).copied() {
                if let Some(origin) = section.origin {
                    if origin != at {
                        return Err(LinkError::plain(format!(
                            "script e .org discordam em {}",
                            section.name
                        )));
                    }
                }
                if !fits(at, len) {
                    return Err(LinkError::plain(format!(
                        "seção {} não cabe em ${at:04X}",
                        section.name
                    )));
                }
                at
            } else if let Some(origin) = section.origin {
                if !fits(origin, len) {
                    return Err(LinkError::plain(format!(
                        "seção {} não cabe em ${origin:04X}",
                        section.name
                    )));
                }
                origin
            } else {
                loop {
                    if !fits(cursor, len) {
                        return Err(LinkError::plain("seções relocáveis excedem 64 KiB".into()));
                    }
                    if let Some(end) = conflict_end(&placed, objects, cursor, len) {
                        cursor = end;
                        continue;
                    }
                    break;
                }
                let address = cursor;
                let next = u32::from(address) + len as u32;
                cursor = u16::try_from(next).unwrap_or(0xFFFF);
                address
            };
            if conflict_end(&placed, objects, address, len).is_some() {
                return Err(LinkError::plain(format!(
                    "seções sobrepostas em ${address:04X}"
                )));
            }
            placed.push((module, index, address));
        }
    }

    // (module, section, offset, weak)
    let mut globals: BTreeMap<String, (usize, usize, u16, bool)> = BTreeMap::new();
    for (module, object) in objects.iter().enumerate() {
        for symbol in &object.symbols {
            if symbol.offset as usize > object.sections[symbol.section].bytes.len() {
                return Err(LinkError::plain(format!(
                    "símbolo {} fora da seção",
                    symbol.name
                )));
            }
            if !symbol.global {
                continue;
            }
            match globals.get(&symbol.name) {
                None => {
                    globals.insert(
                        symbol.name.clone(),
                        (module, symbol.section, symbol.offset, symbol.weak),
                    );
                }
                Some(&(_, _, _, prev_weak)) => {
                    if !symbol.weak && !prev_weak {
                        return Err(LinkError::plain(format!(
                            "símbolo global duplicado: {}",
                            symbol.name
                        )));
                    }
                    if !symbol.weak && prev_weak {
                        globals.insert(
                            symbol.name.clone(),
                            (module, symbol.section, symbol.offset, false),
                        );
                    }
                }
            }
        }
    }

    let mut addresses = BTreeMap::new();
    for (name, (module, section, offset, _)) in &globals {
        let base = placed_addr(&placed, *module, *section)?;
        addresses.insert(name.clone(), base.wrapping_add(*offset));
    }
    let mut extra: BTreeMap<String, u16> = BTreeMap::new();
    for (name, &value) in &options.define {
        if addresses.contains_key(name) {
            return Err(LinkError::plain(format!(
                "símbolo global duplicado: {name}"
            )));
        }
        extra.insert(name.clone(), value);
    }
    let mut weak: BTreeMap<String, u16> = options.provide.clone();
    if options.section_symbols {
        let mut spans: BTreeMap<&str, (u32, u32, usize)> = BTreeMap::new();
        for (module, index, address) in &placed {
            let sec = &objects[*module].sections[*index];
            let (a, e) = (
                u32::from(*address),
                u32::from(*address) + sec.bytes.len() as u32,
            );
            let span = spans.entry(&sec.name).or_insert((a, e, 0));
            span.0 = span.0.min(a);
            span.1 = span.1.max(e);
            span.2 += sec.bytes.len();
        }
        for (name, (a, e, size)) in spans {
            weak.entry(format!("__{name}_start")).or_insert(a as u16);
            weak.entry(format!("__{name}_end")).or_insert(e as u16);
            weak.entry(format!("__{name}_size")).or_insert(size as u16);
        }
    }
    for (name, value) in weak {
        if !addresses.contains_key(&name) {
            extra.entry(name).or_insert(value);
        }
    }
    for (name, &value) in &extra {
        addresses.insert(name.clone(), value);
    }

    let mut segments = Vec::new();
    let mut lines = Vec::new();
    let mut byte_range: Option<String> = None;
    for (module, index, address) in &placed {
        let section = &objects[*module].sections[*index];
        let mut bytes = section.bytes.clone();
        for reloc in &section.relocs {
            let target = resolve_symbol(objects, &globals, &placed, *module, &reloc.symbol)
                .or_else(|e| extra.get(&reloc.symbol).copied().ok_or(e))?;
            match reloc.kind {
                RelocKind::Absolute => {
                    let at = reloc.offset as usize;
                    if at + 1 >= bytes.len() {
                        return Err(LinkError::plain(format!(
                            "relocação absoluta fora da seção {}",
                            section.name
                        )));
                    }
                    bytes[at] = target as u8;
                    bytes[at + 1] = (target >> 8) as u8;
                }
                RelocKind::Relative => {
                    let at = reloc.offset as usize;
                    if at >= bytes.len() {
                        return Err(LinkError::plain(format!(
                            "relocação relativa fora da seção {}",
                            section.name
                        )));
                    }
                    let next = address.wrapping_add(reloc.offset).wrapping_add(1) as i32;
                    let disp = target as i32 - next;
                    if !(-128..=127).contains(&disp) {
                        if byte_range.is_none() {
                            byte_range = Some(format!("desvio fora de alcance: {}", reloc.symbol));
                        }
                    } else {
                        bytes[at] = disp as u8;
                    }
                }
                RelocKind::AbsoluteByte => {
                    let at = reloc.offset as usize;
                    if at >= bytes.len() {
                        return Err(LinkError::plain(format!(
                            "relocação absoluta fora da seção {}",
                            section.name
                        )));
                    }
                    if target > 255 {
                        if byte_range.is_none() {
                            byte_range = Some(format!("fora de faixa: {}", reloc.symbol));
                        }
                    } else {
                        bytes[at] = target as u8;
                    }
                }
            }
        }
        segments.push(Segment {
            address: *address,
            bytes,
        });
    }
    if let Some(message) = byte_range {
        return Err(LinkError {
            message,
            patched: segments,
        });
    }

    for (module, object) in objects.iter().enumerate() {
        for note in &object.notes {
            let base = placed_addr(&placed, module, note.section)?;
            lines.push(MappedLine {
                address: base.wrapping_add(note.offset),
                line: note.line,
                text: note.text.clone(),
                file: note.file.clone(),
            });
        }
    }

    check_regions(&options.regions, &placed, objects)?;
    let placements = placed
        .iter()
        .map(|(module, index, address)| Placement {
            section: objects[*module].sections[*index].name.clone(),
            module: objects[*module].name.clone(),
            address: *address,
            len: objects[*module].sections[*index].bytes.len(),
            padding: 0,
        })
        .collect::<Vec<Placement>>();
    let mut placements = placements;
    check_limits(&options.limits, &placements)?;
    apply_fills(
        &options.fills,
        &options.regions,
        &mut placements,
        &mut segments,
    )?;
    Ok(Linked {
        symbols: addresses,
        segments,
        lines,
        placements,
    })
}

fn fits(addr: u16, len: usize) -> bool {
    u32::from(addr) + len as u32 <= 65536
}

fn conflict_end(
    placed: &[(usize, usize, u16)],
    objects: &[Object],
    addr: u16,
    len: usize,
) -> Option<u16> {
    for (module, index, at) in placed {
        let other = objects[*module].sections[*index].bytes.len();
        if ranges_overlap(addr, len, *at, other) {
            return Some(at.wrapping_add(other as u16));
        }
    }
    None
}

fn ranges_overlap(a: u16, a_len: usize, b: u16, b_len: usize) -> bool {
    if a_len == 0 || b_len == 0 {
        return false;
    }
    let a0 = u32::from(a);
    let b0 = u32::from(b);
    a0 < b0 + b_len as u32 && b0 < a0 + a_len as u32
}

fn placed_addr(
    placed: &[(usize, usize, u16)],
    module: usize,
    section: usize,
) -> Result<u16, LinkError> {
    placed
        .iter()
        .find(|(m, s, _)| *m == module && *s == section)
        .map(|(_, _, addr)| *addr)
        .ok_or_else(|| LinkError::plain("seção sem endereço".into()))
}

fn resolve_symbol(
    objects: &[Object],
    globals: &BTreeMap<String, (usize, usize, u16, bool)>,
    placed: &[(usize, usize, u16)],
    module: usize,
    name: &str,
) -> Result<u16, LinkError> {
    if let Some(symbol) = objects[module]
        .symbols
        .iter()
        .find(|symbol| symbol.name == name)
    {
        let base = placed_addr(placed, module, symbol.section)?;
        return Ok(base.wrapping_add(symbol.offset));
    }
    if let Some((owner, section, offset, _)) = globals.get(name) {
        let base = placed_addr(placed, *owner, *section)?;
        return Ok(base.wrapping_add(*offset));
    }
    Err(LinkError::plain(format!("símbolo indefinido: {name}")))
}

pub fn assemble_object(name: &str, src: &str) -> Result<Object, LinkError> {
    let mut object = Object {
        name: name.to_string(),
        sections: Vec::new(),
        symbols: Vec::new(),
        notes: Vec::new(),
    };
    let mut current: Option<usize> = None;
    let mut pending: HashSet<String> = HashSet::new();
    let mut weak_pending: HashSet<String> = HashSet::new();
    let mut imports: BTreeSet<String> = BTreeSet::new();
    for (index, raw) in src.lines().enumerate() {
        let line_no = (index + 1) as u32;
        let line = strip_comment(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = directive(line, ".section") {
            let section_name = one_ident(rest)?;
            if object
                .sections
                .iter()
                .any(|section| section.name == section_name)
            {
                return Err(LinkError::plain(format!("seção duplicada: {section_name}")));
            }
            object.sections.push(Section {
                name: section_name,
                origin: None,
                bytes: Vec::new(),
                relocs: Vec::new(),
            });
            current = Some(object.sections.len() - 1);
            continue;
        }
        if let Some(rest) = directive(line, ".segment") {
            let section_name = one_ident(rest.trim().trim_matches('"'))?;
            current = Some(
                match object.sections.iter().position(|s| s.name == section_name) {
                    Some(found) => found,
                    None => {
                        object.sections.push(Section {
                            name: section_name,
                            origin: None,
                            bytes: Vec::new(),
                            relocs: Vec::new(),
                        });
                        object.sections.len() - 1
                    }
                },
            );
            continue;
        }
        let section = current.ok_or_else(|| LinkError::plain("código antes de .section".into()))?;
        if let Some(rest) = directive(line, ".org") {
            if !object.sections[section].bytes.is_empty() {
                return Err(LinkError::plain(".org depois de bytes emitidos".into()));
            }
            object.sections[section].origin = Some(eval_const(rest)?);
            continue;
        }
        if let Some(rest) = directive(line, ".global").or_else(|| directive(line, ".export")) {
            for name in rest.split(',') {
                let symbol = one_ident(name)?;
                if let Some(found) = object.symbols.iter_mut().find(|item| item.name == symbol) {
                    found.global = true;
                    found.weak = false;
                } else {
                    weak_pending.remove(&symbol);
                    pending.insert(symbol);
                }
            }
            continue;
        }
        if let Some(rest) = directive(line, ".weak") {
            for name in rest.split(',') {
                let symbol = one_ident(name)?;
                if let Some(found) = object.symbols.iter_mut().find(|item| item.name == symbol) {
                    found.global = true;
                    found.weak = true;
                } else {
                    weak_pending.insert(symbol.clone());
                    pending.insert(symbol);
                }
            }
            continue;
        }
        if let Some(rest) = directive(line, ".import") {
            for name in rest.split(',') {
                let symbol = one_ident(name)?;
                if !imports.insert(symbol.clone()) {
                    return Err(LinkError::plain(format!(".import repetido: {symbol}")));
                }
            }
            continue;
        }
        let (label, rest) = split_label(line);
        if let Some(label) = label {
            define(&mut object, &mut pending, &mut weak_pending, section, label)?;
        }
        if rest.is_empty() {
            continue;
        }
        if let Some(args) = directive(rest, ".res") {
            let (count, fill) = match args.split_once(',') {
                Some((c, f)) => (c.trim(), Some(f.trim())),
                None => (args, None),
            };
            let count = usize::from(eval_const(count)?);
            let fill = match fill {
                Some(f) => u8::try_from(eval_const(f)?)
                    .map_err(|_| LinkError::plain(format!("byte fora da faixa: {f}")))?,
                None => 0,
            };
            let sec = &mut object.sections[section];
            if sec.bytes.len() + count > 0x10000 {
                return Err(LinkError::plain(format!(
                    ".res excede 64 KiB em {}",
                    sec.name
                )));
            }
            sec.bytes.resize(sec.bytes.len() + count, fill);
            continue;
        }
        if let Some(expr) = directive(rest, ".byte") {
            let offset = object.sections[section].bytes.len() as u16;
            if is_ident(expr) {
                object.sections[section].bytes.push(0);
                object.sections[section].relocs.push(Reloc {
                    offset,
                    symbol: expr.to_ascii_lowercase(),
                    kind: RelocKind::AbsoluteByte,
                });
            } else {
                let value = eval_const(expr)?;
                let byte = u8::try_from(value)
                    .map_err(|_| LinkError::plain(format!("byte fora da faixa: {expr}")))?;
                object.sections[section].bytes.push(byte);
            }
            object.notes.push(SourceNote {
                section,
                offset,
                line: line_no,
                text: rest.to_string(),
                file: object.name.clone(),
            });
            continue;
        }
        emit_instruction(&mut object, section, rest, line_no)?;
    }
    if let Some(name) = pending.iter().next() {
        return Err(LinkError::plain(format!(".global sem definição: {name}")));
    }
    if !imports.is_empty() {
        check_imports(&object, &imports)?;
    }
    Ok(object)
}

fn define(
    object: &mut Object,
    pending: &mut HashSet<String>,
    weak_pending: &mut HashSet<String>,
    section: usize,
    label: &str,
) -> Result<(), LinkError> {
    let name = one_ident(label)?;
    if object.symbols.iter().any(|symbol| symbol.name == name) {
        return Err(LinkError::plain(format!("label duplicado: {name}")));
    }
    let offset = object.sections[section].bytes.len() as u16;
    let global = pending.remove(&name);
    let weak = weak_pending.remove(&name);
    object.symbols.push(Symbol {
        global: global || weak,
        weak,
        name,
        section,
        offset,
    });
    Ok(())
}

fn emit_instruction(
    object: &mut Object,
    section: usize,
    line: &str,
    line_no: u32,
) -> Result<(), LinkError> {
    let mut parts = line.split_whitespace();
    let mnemonic = parts.next().unwrap_or("").to_ascii_uppercase();
    let arg = parts.collect::<Vec<_>>().join("").to_ascii_uppercase();
    let offset = object.sections[section].bytes.len() as u16;
    let bad = |what: &str| LinkError::plain(format!("{what}: {line}"));
    let syntax = crate::asm::operand_syntax(&mnemonic, &arg).map_err(|e| bad(&e))?;
    let sec = &mut object.sections[section];
    let (bytes, relocs) = (&mut sec.bytes, &mut sec.relocs);
    // A bare symbol is relocated; any other operand must be a constant expression.
    let symbol = |expr: &str| is_ident(expr).then(|| expr.to_ascii_lowercase());
    match syntax {
        Syntax::ZpRel(zp, target) => {
            let op = encode(&mnemonic, Mode::ZpRel).ok_or_else(|| bad("instrução inválida"))?;
            let zp = u8::try_from(eval_const(zp)?).map_err(|_| bad("zp fora da faixa"))?;
            bytes.extend([op.code, zp, 0]);
            relocs.push(Reloc {
                offset: offset + 2,
                symbol: one_ident(target)?,
                kind: RelocKind::Relative,
            });
        }
        Syntax::Fixed(Mode::Rel, target) => {
            let op = encode(&mnemonic, Mode::Rel).ok_or_else(|| bad("instrução inválida"))?;
            bytes.extend([op.code, 0]);
            relocs.push(Reloc {
                offset: offset + 1,
                symbol: one_ident(target)?,
                kind: RelocKind::Relative,
            });
        }
        Syntax::Fixed(mode, _) if mode.size() == 1 => {
            bytes.push(
                encode(&mnemonic, mode)
                    .ok_or_else(|| bad("instrução inválida"))?
                    .code,
            );
        }
        Syntax::Fixed(mode, expr) => {
            let op = encode(&mnemonic, mode).ok_or_else(|| bad("modo inválido"))?;
            push_operand(
                bytes,
                relocs,
                offset,
                op.code,
                mode.size(),
                symbol(expr),
                expr,
            )?;
        }
        Syntax::Sized { expr, short, long } => {
            let small = match symbol(expr) {
                Some(_) => false,
                None => eval_const(expr)? <= 0xFF,
            };
            let mode = crate::asm::pick_size(&mnemonic, short, long, small);
            let op = encode(&mnemonic, mode).ok_or_else(|| bad("modo inválido"))?;
            push_operand(
                bytes,
                relocs,
                offset,
                op.code,
                mode.size(),
                symbol(expr),
                expr,
            )?;
        }
    }
    object.notes.push(SourceNote {
        section,
        offset,
        line: line_no,
        text: line.to_string(),
        file: object.name.clone(),
    });
    Ok(())
}

/// Opcode plus a 1- or 2-byte operand: a constant, or a symbol relocated as
/// [`RelocKind::Absolute`] (2 bytes) or [`RelocKind::AbsoluteByte`] (zero page).
fn push_operand(
    bytes: &mut Vec<u8>,
    relocs: &mut Vec<Reloc>,
    offset: u16,
    code: u8,
    size: u8,
    symbol: Option<String>,
    expr: &str,
) -> Result<(), LinkError> {
    bytes.push(code);
    match (symbol, size) {
        (Some(symbol), 3) => {
            bytes.extend([0, 0]);
            relocs.push(Reloc {
                offset: offset + 1,
                symbol,
                kind: RelocKind::Absolute,
            });
        }
        (Some(symbol), _) => {
            bytes.push(0);
            relocs.push(Reloc {
                offset: offset + 1,
                symbol,
                kind: RelocKind::AbsoluteByte,
            });
        }
        (None, 3) => bytes.extend(eval_const(expr)?.to_le_bytes()),
        (None, _) => {
            bytes.push(u8::try_from(eval_const(expr)?).map_err(|_| {
                LinkError::plain(format!("operando fora da faixa de 8 bits: {expr}"))
            })?)
        }
    }
    Ok(())
}

fn directive<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name).or_else(|| {
        let (head, tail) = line.split_at(
            line.find(|ch: char| ch.is_ascii_whitespace())
                .unwrap_or(line.len()),
        );
        head.eq_ignore_ascii_case(name).then_some(tail)
    })?;
    if rest.is_empty() || rest.starts_with(|ch: char| ch.is_ascii_whitespace()) {
        Some(rest.trim())
    } else {
        None
    }
}

fn one_ident(text: &str) -> Result<String, LinkError> {
    let text = text.trim();
    if is_ident(text) {
        Ok(text.to_ascii_lowercase())
    } else {
        Err(LinkError::plain(format!("nome inválido: {text}")))
    }
}

fn is_ident(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn eval_const(expr: &str) -> Result<u16, LinkError> {
    expr::eval(expr, &|_| None).map_err(|err| LinkError::plain(err.to_string()))
}

/// A bare decimal integer in a linker script is base 10. `$10` stays hex via [`eval_const`].
fn script_address(addr: &str) -> Result<u16, LinkError> {
    if !addr.is_empty() && addr.bytes().all(|b| b.is_ascii_digit()) {
        return addr
            .parse::<u16>()
            .map_err(|_| LinkError::plain(format!("script: endereço inválido: {addr}")));
    }
    eval_const(addr)
}

const MAGIC: &[u8; 4] = b"M65O";

/// `section <nome> <endereço>` per line. `$10` is hex and `16` is decimal.
/// `;` starts a comment. Names are lowercased.
pub fn parse_link_script(text: &str) -> Result<Vec<SectionAssign>, LinkError> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let keyword = parts.next().unwrap_or("");
        if !keyword.eq_ignore_ascii_case("section") {
            return Err(LinkError::plain(format!("script: linha inválida: {line}")));
        }
        let name = parts
            .next()
            .ok_or_else(|| LinkError::plain("script: falta o nome".into()))?;
        let addr = parts
            .next()
            .ok_or_else(|| LinkError::plain("script: falta o endereço".into()))?;
        if parts.next().is_some() {
            return Err(LinkError::plain(format!("script: sobra em {line}")));
        }
        out.push(SectionAssign {
            name: one_ident(name)?,
            address: script_address(addr)?,
        });
    }
    Ok(out)
}

pub fn write_object(object: &Object) -> Result<Vec<u8>, LinkError> {
    let mut names = HashSet::new();
    for section in &object.sections {
        if section.name.len() > 16 {
            return Err(LinkError::plain(format!(
                "nome de seção maior que 16 bytes: {}",
                section.name
            )));
        }
        if !names.insert(section.name.clone()) {
            return Err(LinkError::plain(format!(
                "seção duplicada: {}",
                section.name
            )));
        }
        for reloc in &section.relocs {
            if reloc.kind == RelocKind::Relative && usize::from(reloc.offset) == section.bytes.len()
            {
                return Err(LinkError::plain(
                    "relocação relativa no fim da seção".into(),
                ));
            }
        }
    }
    Ok(write_object_bytes(object))
}

/// Writes the object only after the section checks. A rejected name, or a relative
/// relocation whose offset equals the section length, leaves the path untouched.
pub fn write_object_file(object: &Object, path: &std::path::Path) -> Result<(), LinkError> {
    let bytes = write_object(object)?;
    std::fs::write(path, bytes).map_err(|err| LinkError::plain(err.to_string()))
}

fn write_object_bytes(object: &Object) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(MAGIC);
    out.push(3);
    write_str(&mut out, &object.name);
    write_u16(&mut out, object.sections.len() as u16);
    for section in &object.sections {
        write_str(&mut out, &section.name);
        match section.origin {
            Some(origin) => {
                out.push(1);
                write_u16(&mut out, origin);
            }
            None => {
                out.push(0);
                write_u16(&mut out, 0);
            }
        }
        write_u16(&mut out, section.bytes.len() as u16);
        out.extend(&section.bytes);
        write_u16(&mut out, section.bytes.len() as u16);
        write_u16(&mut out, section.relocs.len() as u16);
        for reloc in &section.relocs {
            write_u16(&mut out, reloc.offset);
            write_str(&mut out, &reloc.symbol);
            out.push(match reloc.kind {
                RelocKind::Absolute => 0,
                RelocKind::Relative => 1,
                RelocKind::AbsoluteByte => 2,
            });
        }
    }
    write_u16(&mut out, object.symbols.len() as u16);
    for symbol in &object.symbols {
        write_str(&mut out, &symbol.name);
        write_u16(&mut out, symbol.section as u16);
        write_u16(&mut out, symbol.offset);
        // bit0 = global, bit1 = weak (old files only used 0/1).
        out.push(u8::from(symbol.global) | (u8::from(symbol.weak) << 1));
    }
    write_u16(&mut out, object.notes.len() as u16);
    for note in &object.notes {
        write_u16(&mut out, note.section as u16);
        write_u16(&mut out, note.offset);
        out.extend(note.line.to_le_bytes());
        write_str(&mut out, &note.text);
        write_str(&mut out, &note.file);
    }
    let sum = checksum(&out);
    out.extend(sum.to_le_bytes());
    out
}

pub fn read_object(bytes: &[u8]) -> Result<Object, LinkError> {
    if bytes.len() < 4 || &bytes[..4] != MAGIC {
        return Err(LinkError::plain("objeto inválido".into()));
    }
    if bytes.len() < 8 {
        return Err(LinkError::plain("objeto truncado".into()));
    }
    let mut cur = Cursor {
        data: bytes,
        index: 4,
    };
    let version = cur.u8()?;
    if version != 1 && version != 2 && version != 3 {
        return Err(LinkError::plain(format!("versão de objeto {version}")));
    }
    let name = cur.name()?;
    let section_count = cur.u16()? as usize;
    let mut sections = Vec::with_capacity(section_count);
    for _ in 0..section_count {
        let section_name = cur.name()?;
        let has_origin = cur.u8()?;
        let origin_raw = cur.u16()?;
        let origin = if has_origin == 1 {
            Some(origin_raw)
        } else {
            None
        };
        let nbytes = cur.u16()? as usize;
        let data = cur.bytes(nbytes)?.to_vec();
        if version >= 3 {
            let declared = cur.u16()? as usize;
            if declared != data.len() {
                return Err(LinkError::plain("comprimento da seção não confere".into()));
            }
        }
        let reloc_count = cur.u16()? as usize;
        let mut relocs = Vec::with_capacity(reloc_count);
        for _ in 0..reloc_count {
            let offset = cur.u16()?;
            let symbol = cur.name()?;
            let kind = match cur.u8()? {
                0 => RelocKind::Absolute,
                1 => RelocKind::Relative,
                2 => RelocKind::AbsoluteByte,
                other => return Err(LinkError::plain(format!("tipo de relocação {other}"))),
            };
            let end = offset as usize + if kind == RelocKind::Absolute { 2 } else { 1 };
            if end > data.len() {
                return Err(LinkError::plain("relocação fora da seção".into()));
            }
            relocs.push(Reloc {
                offset,
                symbol,
                kind,
            });
        }
        sections.push(Section {
            name: section_name,
            origin,
            bytes: data,
            relocs,
        });
    }
    let symbol_count = cur.u16()? as usize;
    let mut symbols = Vec::with_capacity(symbol_count);
    for _ in 0..symbol_count {
        let symbol_name = cur.name()?;
        let section = cur.u16()? as usize;
        let offset = cur.u16()?;
        let flags = cur.u8()?;
        let global = flags & 1 != 0;
        let weak = flags & 2 != 0;
        if section >= sections.len() {
            return Err(LinkError::plain(format!("seção inválida em {symbol_name}")));
        }
        symbols.push(Symbol {
            name: symbol_name,
            section,
            offset,
            global,
            weak,
        });
    }
    let note_count = cur.u16()? as usize;
    let mut notes = Vec::with_capacity(note_count);
    for _ in 0..note_count {
        let section = cur.u16()? as usize;
        let offset = cur.u16()?;
        let line = cur.u32()?;
        let text = cur.str()?;
        let file = if version >= 2 {
            cur.str()?
        } else {
            String::new()
        };
        if section >= sections.len() {
            return Err(LinkError::plain("nota de origem em seção inválida".into()));
        }
        notes.push(SourceNote {
            section,
            offset,
            line,
            text,
            file,
        });
    }
    let left = bytes.len().saturating_sub(cur.index);
    if left < 4 {
        return Err(LinkError::plain("objeto truncado".into()));
    }
    if left != 4 {
        return Err(LinkError::plain("bytes sobrando no objeto".into()));
    }
    let stored = u32::from_le_bytes(bytes[cur.index..].try_into().unwrap());
    if stored == 0 && !bytes[..cur.index].is_empty() {
        return Err(LinkError::plain("checksum zero".into()));
    }
    if checksum(&bytes[..cur.index]) != stored {
        return Err(LinkError::plain("checksum do objeto não confere".into()));
    }
    Ok(Object {
        name,
        sections,
        symbols,
        notes,
    })
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |sum, byte| sum.wrapping_add(u32::from(*byte)))
}

fn write_u16(out: &mut Vec<u8>, value: u16) {
    out.extend(value.to_le_bytes());
}

fn write_str(out: &mut Vec<u8>, text: &str) {
    write_u16(out, text.len() as u16);
    out.extend(text.as_bytes());
}

struct Cursor<'a> {
    data: &'a [u8],
    index: usize,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, LinkError> {
        let byte = *self
            .data
            .get(self.index)
            .ok_or_else(|| LinkError::plain("objeto truncado".into()))?;
        self.index += 1;
        Ok(byte)
    }

    fn u16(&mut self) -> Result<u16, LinkError> {
        let bytes = self.bytes(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, LinkError> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], LinkError> {
        let end = self
            .index
            .checked_add(len)
            .ok_or_else(|| LinkError::plain("objeto truncado".into()))?;
        let slice = self
            .data
            .get(self.index..end)
            .ok_or_else(|| LinkError::plain("objeto truncado".into()))?;
        self.index = end;
        Ok(slice)
    }

    fn str(&mut self) -> Result<String, LinkError> {
        let len = self.u16()? as usize;
        let bytes = self.bytes(len)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| LinkError::plain("nome do objeto não é utf-8".into()))
    }

    fn name(&mut self) -> Result<String, LinkError> {
        let text = self.str()?;
        if text.is_empty() {
            return Err(LinkError::plain("nome de comprimento 0".into()));
        }
        Ok(text)
    }
}

/// One switchable bank of an overlay link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bank {
    pub name: String,
    pub objects: Vec<Object>,
}

/// Result of [`link_banks`]: the fixed part and each bank on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankedLink {
    /// Fixed sections and their symbols (identical in every bank's view).
    pub fixed: Linked,
    /// Per bank: its own segments and every symbol visible inside it.
    pub banks: Vec<(String, Linked)>,
}

/// Overlay / bank-switched link. Each bank is linked together with the `fixed`
/// objects (placed first, so their layout is the same in every bank), so a bank sees
/// the fixed symbols and its own, never another bank's. Banks may occupy the same
/// addresses (they are switched in one at a time); within one bank the usual
/// overlap check applies. Fixed code may reference a symbol only every bank defines
/// (a common entry point), and it must have the same address in all of them,
/// otherwise the fixed bytes would differ between banks; that is an error.
pub fn link_banks(
    fixed: &[Object],
    banks: &[Bank],
    options: &LinkOptions,
) -> Result<BankedLink, LinkError> {
    if banks.is_empty() {
        return Err(LinkError::plain("nenhum banco".into()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for b in banks {
        if !seen.insert(b.name.as_str()) {
            return Err(LinkError::plain(format!("banco duplicado: {}", b.name)));
        }
    }
    let fixed_sections: usize = fixed.iter().map(|o| o.sections.len()).sum();
    let mut views = Vec::new();
    for bank in banks {
        let mut all = fixed.to_vec();
        all.extend(bank.objects.iter().cloned());
        let linked = link_with_options(&all, options).map_err(|e| LinkError {
            message: format!("banco {}: {}", bank.name, e.message),
            patched: e.patched,
        })?;
        views.push((bank.name.clone(), linked));
    }
    let first = &views[0].1;
    let fixed_segments: Vec<Segment> = first.segments[..fixed_sections].to_vec();
    for (name, view) in &views[1..] {
        if view.segments[..fixed_sections] != fixed_segments[..] {
            return Err(LinkError::plain(format!(
                "a parte fixa difere no banco {name}: símbolo de banco com endereços diferentes"
            )));
        }
    }
    let fixed_names: std::collections::BTreeSet<&str> = fixed
        .iter()
        .flat_map(|o| {
            o.symbols
                .iter()
                .filter(|s| s.global)
                .map(|s| s.name.as_str())
        })
        .collect();
    let fixed_link = Linked {
        symbols: first
            .symbols
            .iter()
            .filter(|(n, _)| fixed_names.contains(n.as_str()))
            .map(|(n, a)| (n.clone(), *a))
            .collect(),
        segments: fixed_segments,
        lines: first
            .lines
            .iter()
            .filter(|l| {
                first.placements[..fixed_sections]
                    .iter()
                    .any(|p| (p.address..p.address.wrapping_add(p.len as u16)).contains(&l.address))
            })
            .cloned()
            .collect(),
        placements: first.placements[..fixed_sections].to_vec(),
    };
    let banks_out = views
        .into_iter()
        .map(|(name, mut view)| {
            view.segments.drain(..fixed_sections);
            view.placements.drain(..fixed_sections);
            (name, view)
        })
        .collect();
    Ok(BankedLink {
        fixed: fixed_link,
        banks: banks_out,
    })
}

/// [`parse_link_script`] plus `provide <nome> <valor>` (weak) and
/// `define <nome> <valor>` (strong) lines, returned as [`LinkOptions`].
pub fn parse_link_options(text: &str, reloc_base: u16) -> Result<LinkOptions, LinkError> {
    let mut options = LinkOptions {
        reloc_base,
        ..LinkOptions::default()
    };
    let mut sections = String::new();
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        let mut parts = line.split_whitespace();
        let keyword = parts.next().unwrap_or("").to_ascii_lowercase();
        if keyword == "provide" || keyword == "define" {
            let name = parts
                .next()
                .ok_or_else(|| LinkError::plain(format!("script: falta o nome em {line}")))?;
            let value = parts
                .next()
                .ok_or_else(|| LinkError::plain(format!("script: falta o valor em {line}")))?;
            if parts.next().is_some() {
                return Err(LinkError::plain(format!("script: sobra em {line}")));
            }
            let map = if keyword == "provide" {
                &mut options.provide
            } else {
                &mut options.define
            };
            if map
                .insert(one_ident(name)?, script_address(value)?)
                .is_some()
            {
                return Err(LinkError::plain(format!(
                    "script: {name} definido duas vezes"
                )));
            }
        } else if keyword == "region" {
            let name = parts
                .next()
                .ok_or_else(|| LinkError::plain(format!("script: falta o nome em {line}")))?;
            let start = parts
                .next()
                .ok_or_else(|| LinkError::plain(format!("script: falta o início em {line}")))?;
            let end = parts
                .next()
                .ok_or_else(|| LinkError::plain(format!("script: falta o fim em {line}")))?;
            if parts.next().is_some() {
                return Err(LinkError::plain(format!("script: sobra em {line}")));
            }
            options.regions.push(Region {
                name: one_ident(name)?,
                start: script_address(start)?,
                end: script_address(end)?,
            });
        } else if keyword == "limit" {
            let (name, max) = match (parts.next(), parts.next(), parts.next()) {
                (Some(n), Some(m), None) => (n, m),
                _ => {
                    return Err(LinkError::plain(format!(
                        "script: use limit <seção> <máx>: {line}"
                    )))
                }
            };
            options.limits.push(SectionLimit {
                name: one_ident(name)?,
                max: script_size(max)?,
            });
        } else if keyword == "fill" {
            let (name, boundary, byte) =
                match (parts.next(), parts.next(), parts.next(), parts.next()) {
                    (Some(n), Some(b), Some(v), None) => (n, b, v),
                    _ => {
                        return Err(LinkError::plain(format!(
                            "script: use fill <seção> <fronteira> <byte>: {line}"
                        )))
                    }
                };
            let boundary = script_size(boundary)?;
            if boundary == 0 {
                return Err(LinkError::plain(format!(
                    "script: fronteira zero em {line}"
                )));
            }
            options.fills.push(SectionFill {
                name: one_ident(name)?,
                boundary: boundary as u32,
                byte: u8::try_from(script_address(byte)?).map_err(|_| {
                    LinkError::plain(format!("script: byte fora da faixa em {line}"))
                })?,
            });
        } else if keyword == "symbols" {
            options.section_symbols = true;
        } else if keyword == "gc" {
            if parts.next().is_some() {
                return Err(LinkError::plain(format!("script: sobra em {line}")));
            }
            options.gc = true;
        } else if keyword == "keep" {
            let name = parts
                .next()
                .ok_or_else(|| LinkError::plain(format!("script: falta o nome em {line}")))?;
            if parts.next().is_some() {
                return Err(LinkError::plain(format!("script: sobra em {line}")));
            }
            options.keep.push(one_ident(name)?);
        } else {
            sections.push_str(raw);
            sections.push('\n');
        }
    }
    options.script = parse_link_script(&sections)?;
    Ok(options)
}

fn check_regions(
    regions: &[Region],
    placed: &[(usize, usize, u16)],
    objects: &[Object],
) -> Result<(), LinkError> {
    for (i, r) in regions.iter().enumerate() {
        if r.start > r.end {
            return Err(LinkError::plain(format!(
                "região {} vazia: ${:04X} > ${:04X}",
                r.name, r.start, r.end
            )));
        }
        for o in &regions[..i] {
            if o.name == r.name {
                return Err(LinkError::plain(format!("região duplicada: {}", r.name)));
            }
            if r.start <= o.end && o.start <= r.end {
                return Err(LinkError::plain(format!(
                    "regiões sobrepostas: {} e {}",
                    o.name, r.name
                )));
            }
        }
    }
    if regions.is_empty() {
        return Ok(());
    }
    for &(module, index, address) in placed {
        let section = &objects[module].sections[index];
        let len = section.bytes.len() as u32;
        if len == 0 {
            continue;
        }
        let last = u32::from(address) + len - 1;
        if !regions
            .iter()
            .any(|r| u32::from(r.start) <= u32::from(address) && last <= u32::from(r.end))
        {
            return Err(LinkError::plain(format!(
                "seção {} (${address:04X}-${last:04X}) fora das regiões de memória",
                section.name
            )));
        }
    }
    Ok(())
}

impl Linked {
    /// Bytes used by placed sections inside each region, in region order.
    pub fn region_usage(&self, regions: &[Region]) -> Vec<(String, usize, usize)> {
        regions
            .iter()
            .map(|r| {
                let used = self
                    .placements
                    .iter()
                    .filter(|p| p.len > 0 && r.start <= p.address && p.address <= r.end)
                    .map(|p| p.len)
                    .sum();
                (r.name.clone(), used, usize::from(r.end - r.start) + 1)
            })
            .collect()
    }
}

/// A size in a linker script: like [`script_address`], plus `$10000` / `65536`
/// (a whole 64 KiB bank).
fn script_size(text: &str) -> Result<usize, LinkError> {
    if text == "65536" || text.eq_ignore_ascii_case("$10000") {
        return Ok(0x10000);
    }
    script_address(text).map(usize::from)
}

fn check_limits(limits: &[SectionLimit], placements: &[Placement]) -> Result<(), LinkError> {
    for limit in limits {
        let used: usize = placements
            .iter()
            .filter(|p| p.section == limit.name)
            .map(|p| p.len)
            .sum();
        if used > limit.max {
            return Err(LinkError::plain(format!(
                "seção {} excede o limite: {used} > {} bytes",
                limit.name, limit.max
            )));
        }
    }
    Ok(())
}

fn apply_fills(
    fills: &[SectionFill],
    regions: &[Region],
    placements: &mut [Placement],
    segments: &mut [Segment],
) -> Result<(), LinkError> {
    for fill in fills {
        let Some(last) = (0..placements.len())
            .filter(|&i| placements[i].section == fill.name)
            .max_by_key(|&i| u32::from(placements[i].address) + placements[i].len as u32)
        else {
            return Err(LinkError::plain(format!("fill sem seção: {}", fill.name)));
        };
        let start = u32::from(placements[last].address);
        let end = start + placements[last].len as u32 + placements[last].padding as u32;
        let target = end.div_ceil(fill.boundary) * fill.boundary;
        if target > 0x10000 {
            return Err(LinkError::plain(format!(
                "fill de {} passa de $FFFF",
                fill.name
            )));
        }
        if target == end {
            continue;
        }
        for (i, other) in placements.iter().enumerate() {
            let o_start = u32::from(other.address);
            let o_end = o_start + (other.len + other.padding) as u32;
            if i != last && other.len + other.padding > 0 && o_start < target && end < o_end {
                return Err(LinkError::plain(format!(
                    "fill de {} (${end:04X}-${:04X}) sobrepõe {} em ${o_start:04X}",
                    fill.name,
                    target - 1,
                    other.section
                )));
            }
        }
        if !regions.is_empty()
            && !regions
                .iter()
                .any(|r| u32::from(r.start) <= start && target - 1 <= u32::from(r.end))
        {
            return Err(LinkError::plain(format!(
                "fill de {} sai da região de memória",
                fill.name
            )));
        }
        let extra = (target - end) as usize;
        segments[last]
            .bytes
            .resize(segments[last].bytes.len() + extra, fill.byte);
        placements[last].padding += extra;
    }
    Ok(())
}

/// One relocation after linking, for [`relocation_report`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocEntry {
    pub module: String,
    pub section: String,
    /// Absolute address of the patched byte(s).
    pub site: u16,
    pub kind: RelocKind,
    pub symbol: String,
    /// The symbol's final address.
    pub target: u16,
    /// The bytes now at `site` (2 for absolute, 1 otherwise).
    pub patched: Vec<u8>,
}

/// Every relocation of `objects` as `linked` resolved it, in placement order.
/// `linked` must come from linking exactly these objects. Module names should be
/// unique: a placement is matched to the first object with its module and
/// section name.
pub fn relocation_report(objects: &[Object], linked: &Linked) -> Vec<RelocEntry> {
    let mut out = Vec::new();
    for (pi, p) in linked.placements.iter().enumerate() {
        let Some(object) = objects.iter().find(|o| o.name == p.module) else {
            continue;
        };
        let Some(si) = object.sections.iter().position(|s| s.name == p.section) else {
            continue;
        };
        let section = &object.sections[si];
        for reloc in &section.relocs {
            let local = object
                .symbols
                .iter()
                .find(|sym| sym.name == reloc.symbol)
                .and_then(|sym| {
                    let base = linked.placements.iter().find(|q| {
                        q.module == object.name && q.section == object.sections[sym.section].name
                    })?;
                    Some(base.address.wrapping_add(sym.offset))
                });
            let Some(target) = local.or_else(|| linked.symbols.get(&reloc.symbol).copied()) else {
                continue;
            };
            let width = if reloc.kind == RelocKind::Absolute {
                2
            } else {
                1
            };
            let at = usize::from(reloc.offset);
            let patched = linked.segments[pi].bytes[at..at + width].to_vec();
            out.push(RelocEntry {
                module: p.module.clone(),
                section: p.section.clone(),
                site: p.address.wrapping_add(reloc.offset),
                kind: reloc.kind,
                symbol: reloc.symbol.clone(),
                target,
                patched,
            });
        }
    }
    out
}

/// Text table of [`relocation_report`]: `$SITE KIND module:section symbol -> $TARGET [bytes]`.
pub fn format_relocation_report(entries: &[RelocEntry]) -> String {
    let mut out = String::from("RELOCAÇÕES\n");
    for e in entries {
        let kind = match e.kind {
            RelocKind::Absolute => "ABS16",
            RelocKind::AbsoluteByte => "ABS8 ",
            RelocKind::Relative => "REL8 ",
        };
        let bytes = e
            .patched
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        out += &format!(
            "  ${:04X} {kind} {}:{} {} -> ${:04X} [{bytes}]\n",
            e.site, e.module, e.section, e.symbol, e.target
        );
    }
    out += &format!("  total: {}\n", entries.len());
    out
}

impl Linked {
    /// Debug info in the line format of ca65/ld65 `--dbgfile` (version 2.0): a
    /// `version` line, an `info` line with the counts, one `seg` line per placed
    /// section and one `sym` line per symbol (`type=lab`, `seg` = the section that
    /// contains it, or no `seg` for linker-defined values outside every section).
    /// Only these record types are written; [`Linked::to_dbg_full`] adds `file`,
    /// `mod`, `scope`, `span` and `line`.
    pub fn to_dbg(&self) -> String {
        let mut out = String::from("version\tmajor=2,minor=0\n");
        let mut segs = self.placements.clone();
        segs.sort_by_key(|p| (p.address, p.section.clone()));
        out += &format!("info\tseg={},sym={}\n", segs.len(), self.symbols.len());
        for (id, p) in segs.iter().enumerate() {
            let size = p.len + p.padding;
            let addrsize = if u32::from(p.address) + size as u32 <= 0x100 {
                "zeropage"
            } else {
                "absolute"
            };
            out += &format!(
                "seg\tid={id},name=\"{}\",start=0x{:06X},size=0x{size:04X},addrsize={addrsize},type=rw\n",
                p.section, p.address
            );
        }
        let mut syms: Vec<(&String, &u16)> = self.symbols.iter().collect();
        syms.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));
        for (id, (name, value)) in syms.into_iter().enumerate() {
            let addrsize = if *value < 0x100 {
                "zeropage"
            } else {
                "absolute"
            };
            let seg = segs.iter().position(|p| {
                let a = u32::from(p.address);
                let v = u32::from(*value);
                a <= v && v < a + (p.len + p.padding) as u32
            });
            out += &format!("sym\tid={id},name=\"{name}\",addrsize={addrsize},val=0x{value:04X}");
            if let Some(seg) = seg {
                out += &format!(",seg={seg}");
            }
            out += ",type=lab\n";
        }
        out
    }

    /// Complete debug file in the ld65 `--dbgfile` 2.0 line format, records in the
    /// order ld65 writes them:
    ///
    /// - `version`, then `info` with the count of every record kind below;
    /// - `file`: one per source file named by the line notes (`size=0` and
    ///   `mtime=0x00000000`, because objects keep neither), `mod` = the module;
    /// - `mod`: one per module that placed a section, `file` = its source;
    /// - `seg`: as in [`Linked::to_dbg`];
    /// - `span`: one per byte-producing source line; `start` is the offset in the
    ///   segment, `size` runs to the next line of the same placement or the end of
    ///   the placement's bytes (padding excluded); zero-size lines get no span;
    /// - `scope`: one top-level scope (`name=""`) per module, `size` = bytes it
    ///   placed, `span` = its spans joined with `+`;
    /// - `line`: `file`, `line` number and `span` of each mapped line, `type=0`;
    /// - `sym`: as in [`Linked::to_dbg`], plus `scope` (module of the placement that
    ///   holds the value) and `def` (the line at that address in that module).
    ///
    /// Ids are dense, starting at 0, in the order written.
    pub fn to_dbg_full(&self) -> String {
        let mut segs = self.placements.clone();
        segs.sort_by_key(|p| (p.address, p.section.clone()));
        let in_seg = |p: &Placement, v: u16| {
            let a = u32::from(p.address);
            a <= u32::from(v) && u32::from(v) < a + (p.len + p.padding) as u32
        };
        let mut modules: Vec<String> = Vec::new();
        for p in &segs {
            if !modules.contains(&p.module) {
                modules.push(p.module.clone());
            }
        }
        let mut files: Vec<String> = Vec::new();
        for l in &self.lines {
            if !files.contains(&l.file) {
                files.push(l.file.clone());
            }
        }
        // One `span` record: segment id, module id, offset in the segment, size.
        struct SpanRec {
            seg: usize,
            module: usize,
            start: usize,
            size: usize,
        }
        let mut spans: Vec<SpanRec> = Vec::new();
        // (file id, line number, span id, module id, address) per `line` record.
        type LineRec = (usize, u32, Option<usize>, Option<usize>, u16);
        let mut line_recs: Vec<LineRec> = Vec::new();
        for l in &self.lines {
            let seg = segs
                .iter()
                .position(|p| p.module == l.file && in_seg(p, l.address));
            let mut span = None;
            let module = seg.map(|s| {
                modules
                    .iter()
                    .position(|m| *m == segs[s].module)
                    .unwrap_or(0)
            });
            if let Some(si) = seg {
                let p = &segs[si];
                let start = usize::from(l.address.wrapping_sub(p.address));
                let next = self
                    .lines
                    .iter()
                    .filter(|o| o.file == l.file && in_seg(p, o.address))
                    .map(|o| usize::from(o.address.wrapping_sub(p.address)))
                    .filter(|&o| o > start)
                    .min()
                    .unwrap_or(p.len)
                    .min(p.len);
                if next > start {
                    span = Some(spans.len());
                    spans.push(SpanRec {
                        seg: si,
                        module: module.unwrap_or(0),
                        start,
                        size: next - start,
                    });
                }
            }
            let file = files.iter().position(|f| *f == l.file).unwrap_or(0);
            line_recs.push((file, l.line, span, module, l.address));
        }
        let mut syms: Vec<(&String, &u16)> = self.symbols.iter().collect();
        syms.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));
        let mut out = String::from("version\tmajor=2,minor=0\n");
        out += &format!(
            "info\tcsym=0,file={},lib=0,line={},mod={},scope={},seg={},span={},sym={},type=0\n",
            files.len(),
            line_recs.len(),
            modules.len(),
            modules.len(),
            segs.len(),
            spans.len(),
            syms.len()
        );
        for (id, f) in files.iter().enumerate() {
            let module = modules.iter().position(|m| m == f);
            out += &format!("file\tid={id},name=\"{f}\",size=0,mtime=0x00000000");
            if let Some(m) = module {
                out += &format!(",mod={m}");
            }
            out += "\n";
        }
        for (id, m) in modules.iter().enumerate() {
            out += &format!("mod\tid={id},name=\"{m}\"");
            if let Some(f) = files.iter().position(|f| f == m) {
                out += &format!(",file={f}");
            }
            out += "\n";
        }
        for (id, p) in segs.iter().enumerate() {
            let size = p.len + p.padding;
            let addrsize = if u32::from(p.address) + size as u32 <= 0x100 {
                "zeropage"
            } else {
                "absolute"
            };
            out += &format!(
                "seg\tid={id},name=\"{}\",start=0x{:06X},size=0x{size:04X},addrsize={addrsize},type=rw\n",
                p.section, p.address
            );
        }
        for (id, s) in spans.iter().enumerate() {
            out += &format!(
                "span\tid={id},seg={},start={},size={}\n",
                s.seg, s.start, s.size
            );
        }
        for (id, m) in modules.iter().enumerate() {
            let size: usize = segs.iter().filter(|p| p.module == *m).map(|p| p.len).sum();
            let list: Vec<String> = spans
                .iter()
                .enumerate()
                .filter(|(_, s)| s.module == id)
                .map(|(i, _)| i.to_string())
                .collect();
            out += &format!("scope\tid={id},name=\"\",mod={id},size={size}");
            if !list.is_empty() {
                out += &format!(",span={}", list.join("+"));
            }
            out += "\n";
        }
        for (id, (file, line, span, _, _)) in line_recs.iter().enumerate() {
            out += &format!("line\tid={id},file={file},line={line},type=0");
            if let Some(span) = span {
                out += &format!(",span={span}");
            }
            out += "\n";
        }
        for (id, (name, value)) in syms.into_iter().enumerate() {
            let addrsize = if *value < 0x100 {
                "zeropage"
            } else {
                "absolute"
            };
            let seg = segs.iter().position(|p| in_seg(p, *value));
            out += &format!("sym\tid={id},name=\"{name}\",addrsize={addrsize}");
            if let Some(si) = seg {
                let module = modules
                    .iter()
                    .position(|m| *m == segs[si].module)
                    .unwrap_or(0);
                out += &format!(",scope={module}");
                let def = line_recs
                    .iter()
                    .position(|(_, _, _, m, a)| *m == Some(module) && *a == *value);
                if let Some(def) = def {
                    out += &format!(",def={def}");
                }
            }
            out += &format!(",val=0x{value:04X}");
            if let Some(seg) = seg {
                out += &format!(",seg={seg}");
            }
            out += ",type=lab\n";
        }
        out
    }
}

/// Strict name checking, enabled when an object declares any `.import`: an
/// imported name must not also be defined in the object, and every symbol a
/// relocation names must be defined in the object or imported. (Objects without
/// `.import` keep the permissive rule: any unknown name is left to the linker.)
fn check_imports(object: &Object, imports: &BTreeSet<String>) -> Result<(), LinkError> {
    for name in imports {
        if object.symbols.iter().any(|s| &s.name == name) {
            return Err(LinkError::plain(format!(
                ".import de nome definido aqui: {name}"
            )));
        }
    }
    for section in &object.sections {
        for reloc in &section.relocs {
            let local = object.symbols.iter().any(|s| s.name == reloc.symbol);
            if !local && !imports.contains(&reloc.symbol) {
                return Err(LinkError::plain(format!(
                    "símbolo sem definição nem .import: {}",
                    reloc.symbol
                )));
            }
        }
    }
    Ok(())
}

/// Section garbage collection (like GNU ld `--gc-sections`).
///
/// Roots are the sections named in `keep` and the sections defining a symbol
/// named in `keep` (local or global, in any object). From a kept section, a
/// relocation against a name keeps the section that defines it: a symbol of the
/// same object first (local or global), else the global of that name in any
/// object. Linker definitions (`define`/`provide`) keep nothing. Unreached
/// sections are removed with their symbols and line notes; an object can end with
/// no sections. Empty `keep`, a root that names nothing, or nothing kept at all is
/// an error, so a typo cannot silently drop the whole program.
pub fn collect_garbage(objects: &[Object], keep: &[String]) -> Result<Vec<Object>, LinkError> {
    if keep.is_empty() {
        return Err(LinkError::plain("gc sem raiz: use keep <nome>".into()));
    }
    let global_home = |name: &str| -> Option<(usize, usize)> {
        objects.iter().enumerate().find_map(|(m, o)| {
            o.symbols
                .iter()
                .find(|s| s.global && s.name == name)
                .map(|s| (m, s.section))
        })
    };
    let mut live: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut work: Vec<(usize, usize)> = Vec::new();
    for root in keep {
        let mut found = false;
        for (m, o) in objects.iter().enumerate() {
            for (i, sec) in o.sections.iter().enumerate() {
                if &sec.name == root {
                    found = true;
                    work.push((m, i));
                }
            }
            for sym in &o.symbols {
                if &sym.name == root {
                    found = true;
                    work.push((m, sym.section));
                }
            }
        }
        if !found {
            return Err(LinkError::plain(format!(
                "keep sem seção nem símbolo: {root}"
            )));
        }
    }
    while let Some((m, i)) = work.pop() {
        if !live.insert((m, i)) {
            continue;
        }
        let object = &objects[m];
        for reloc in &object.sections[i].relocs {
            let target = object
                .symbols
                .iter()
                .find(|s| s.name == reloc.symbol)
                .map(|s| (m, s.section))
                .or_else(|| global_home(&reloc.symbol));
            if let Some(t) = target {
                if !live.contains(&t) {
                    work.push(t);
                }
            }
        }
    }
    let mut out = Vec::with_capacity(objects.len());
    for (m, o) in objects.iter().enumerate() {
        let mut remap = vec![None; o.sections.len()];
        let mut sections = Vec::new();
        for (i, sec) in o.sections.iter().enumerate() {
            if live.contains(&(m, i)) {
                remap[i] = Some(sections.len());
                sections.push(sec.clone());
            }
        }
        let symbols = o
            .symbols
            .iter()
            .filter_map(|s| {
                remap[s.section].map(|section| Symbol {
                    section,
                    ..s.clone()
                })
            })
            .collect();
        let notes = o
            .notes
            .iter()
            .filter_map(|n| {
                remap[n.section].map(|section| SourceNote {
                    section,
                    ..n.clone()
                })
            })
            .collect();
        out.push(Object {
            name: o.name.clone(),
            sections,
            symbols,
            notes,
        });
    }
    Ok(out)
}

/// One CPU address window shared by sections of different banks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    /// First address of the window.
    pub start: u16,
    /// Last address of the window (inclusive).
    pub end: u16,
    /// Each bank section touching the window, by address then bank name.
    pub members: Vec<OverlayMember>,
}

/// One bank section inside an [`Overlay`] window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayMember {
    pub bank: String,
    pub module: String,
    pub section: String,
    pub first: u16,
    /// Last byte, padding included.
    pub last: u16,
}

impl BankedLink {
    /// Overlay map: bank sections (fixed ones excluded) whose byte ranges overlap,
    /// grouped transitively (A overlaps B, B overlaps C: one window spanning all
    /// three). A window is reported only if it holds sections of at least two
    /// banks. Empty sections occupy nothing.
    pub fn overlays(&self) -> Vec<Overlay> {
        let fixed: BTreeSet<(&str, &str)> = self
            .fixed
            .placements
            .iter()
            .map(|p| (p.module.as_str(), p.section.as_str()))
            .collect();
        let mut spans: Vec<(u32, u32, OverlayMember)> = Vec::new();
        for (bank, linked) in &self.banks {
            for p in &linked.placements {
                let size = p.len + p.padding;
                if size == 0 || fixed.contains(&(p.module.as_str(), p.section.as_str())) {
                    continue;
                }
                let a = u32::from(p.address);
                let b = (a + size as u32 - 1).min(0xFFFF);
                spans.push((
                    a,
                    b,
                    OverlayMember {
                        bank: bank.clone(),
                        module: p.module.clone(),
                        section: p.section.clone(),
                        first: a as u16,
                        last: b as u16,
                    },
                ));
            }
        }
        spans.sort_by(|x, y| (x.0, x.1, &x.2.bank).cmp(&(y.0, y.1, &y.2.bank)));
        let mut groups: Vec<Overlay> = Vec::new();
        for (a, b, m) in spans {
            match groups.last_mut() {
                Some(g) if a <= u32::from(g.end) => {
                    g.end = g.end.max(b as u16);
                    g.members.push(m);
                }
                _ => groups.push(Overlay {
                    start: a as u16,
                    end: b as u16,
                    members: vec![m],
                }),
            }
        }
        groups.retain(|g| {
            let banks: BTreeSet<&str> = g.members.iter().map(|m| m.bank.as_str()).collect();
            banks.len() > 1
        });
        groups
    }

    /// Text form of [`BankedLink::overlays`]: `MAPA DE SOBREPOSIÇÃO`, the fixed
    /// sections (`  fixo $AAAA-$BBBB section (module)`), then per window
    /// `  janela $SSSS-$EEEE N seções` with one `    $AAAA-$BBBB bank:section
    /// (module)` line per member, then `  total: N janelas`.
    pub fn overlay_report(&self) -> String {
        let mut out = String::from("MAPA DE SOBREPOSIÇÃO\n");
        let mut fixed = self.fixed.placements.clone();
        fixed.sort_by_key(|p| (p.address, p.section.clone()));
        for p in fixed.iter().filter(|p| p.len + p.padding > 0) {
            let last = (u32::from(p.address) + (p.len + p.padding) as u32 - 1).min(0xFFFF);
            out += &format!(
                "  fixo ${:04X}-${last:04X} {} ({})\n",
                p.address, p.section, p.module
            );
        }
        let groups = self.overlays();
        for g in &groups {
            out += &format!(
                "  janela ${:04X}-${:04X} {} seções\n",
                g.start,
                g.end,
                g.members.len()
            );
            for m in &g.members {
                out += &format!(
                    "    ${:04X}-${:04X} {}:{} ({})\n",
                    m.first, m.last, m.bank, m.section, m.module
                );
            }
        }
        out += &format!("  total: {} janelas\n", groups.len());
        out
    }
}

// ---------------------------------------------------------------------------
// Object libraries.

const LIB_MAGIC: &[u8; 4] = b"M65L";
const LIB_VERSION: u8 = 1;

/// An object library (`.lib`): members are pulled into a link only when they
/// define a global some already-linked object still needs (ld65/ar semantics).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Library {
    pub members: Vec<Object>,
}

impl Library {
    /// Every global and the index of the member that defines it, sorted by name.
    /// Two members exporting the same name is an error, as ld65 reports.
    pub fn index(&self) -> Result<BTreeMap<String, usize>, LinkError> {
        let mut index: BTreeMap<String, (usize, bool)> = BTreeMap::new();
        for (m, member) in self.members.iter().enumerate() {
            for sym in member.symbols.iter().filter(|s| s.global) {
                match index.get(&sym.name) {
                    None => {
                        index.insert(sym.name.clone(), (m, sym.weak));
                    }
                    Some(&(prev, prev_weak)) => {
                        if !sym.weak && !prev_weak {
                            return Err(LinkError::plain(format!(
                                "biblioteca: {} exportado por {} e {}",
                                sym.name, self.members[prev].name, member.name
                            )));
                        }
                        // A strong definition replaces a weak one; later weaks keep the first.
                        if !sym.weak && prev_weak {
                            index.insert(sym.name.clone(), (m, false));
                        }
                    }
                }
            }
        }
        Ok(index.into_iter().map(|(n, (m, _))| (n, m)).collect())
    }

    /// Archive bytes: `M65L`, version 1, member count (u16), the export index
    /// (u16 count, then per entry a u16-length name and the member as u16), then each member as u32
    /// length + its object file bytes ([`write_object`]).
    pub fn to_bytes(&self) -> Result<Vec<u8>, LinkError> {
        let index = self.index()?;
        let mut out = Vec::from(&LIB_MAGIC[..]);
        out.push(LIB_VERSION);
        let count = u16::try_from(self.members.len())
            .map_err(|_| LinkError::plain("biblioteca com membros demais".into()))?;
        write_u16(&mut out, count);
        write_u16(&mut out, index.len() as u16);
        for (name, member) in &index {
            write_str(&mut out, name);
            write_u16(&mut out, *member as u16);
        }
        for member in &self.members {
            let bytes = write_object(member)?;
            out.extend((bytes.len() as u32).to_le_bytes());
            out.extend(bytes);
        }
        Ok(out)
    }

    /// Parse [`Library::to_bytes`]. The stored index must equal the one rebuilt
    /// from the members; trailing bytes, a short read or a bad member are errors.
    pub fn from_bytes(bytes: &[u8]) -> Result<Library, LinkError> {
        let bad = |what: &str| LinkError::plain(format!("biblioteca inválida: {what}"));
        if bytes.len() < 9 || &bytes[..4] != LIB_MAGIC {
            return Err(bad("assinatura"));
        }
        if bytes[4] != LIB_VERSION {
            return Err(bad("versão"));
        }
        let mut at = 5usize;
        let u16_at = |at: &mut usize| -> Result<u16, LinkError> {
            let v = bytes.get(*at..*at + 2).ok_or_else(|| bad("truncada"))?;
            *at += 2;
            Ok(u16::from_le_bytes([v[0], v[1]]))
        };
        let count = usize::from(u16_at(&mut at)?);
        let entries = usize::from(u16_at(&mut at)?);
        let mut stored = BTreeMap::new();
        for _ in 0..entries {
            let len = usize::from(u16_at(&mut at)?);
            let name = bytes.get(at..at + len).ok_or_else(|| bad("truncada"))?;
            let name = String::from_utf8(name.to_vec()).map_err(|_| bad("nome"))?;
            at += len;
            stored.insert(name, usize::from(u16_at(&mut at)?));
        }
        let mut members = Vec::with_capacity(count);
        for _ in 0..count {
            let len = bytes.get(at..at + 4).ok_or_else(|| bad("truncada"))?;
            let len = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
            at += 4;
            let body = bytes.get(at..at + len).ok_or_else(|| bad("truncada"))?;
            members.push(read_object(body)?);
            at += len;
        }
        if at != bytes.len() {
            return Err(bad("bytes sobrando"));
        }
        let lib = Library { members };
        if lib.index()? != stored {
            return Err(bad("índice não confere com os membros"));
        }
        Ok(lib)
    }
}

/// Names referenced by relocations of `objects` that no object defines (as a
/// local of the same object or as any global) and the options do not define.
fn unresolved(objects: &[Object], options: &LinkOptions) -> BTreeSet<String> {
    let globals: BTreeSet<&str> = objects
        .iter()
        .flat_map(|o| {
            o.symbols
                .iter()
                .filter(|s| s.global)
                .map(|s| s.name.as_str())
        })
        .collect();
    let mut out = BTreeSet::new();
    for o in objects {
        for sec in &o.sections {
            for r in &sec.relocs {
                let local = o.symbols.iter().any(|s| s.name == r.symbol);
                if !local
                    && !globals.contains(r.symbol.as_str())
                    && !options.define.contains_key(&r.symbol)
                    && !options.provide.contains_key(&r.symbol)
                {
                    out.insert(r.symbol.clone());
                }
            }
        }
    }
    out
}

/// The objects plus the library members the link needs, in pull order.
///
/// Repeats until nothing changes: for each still-unresolved name, in name order,
/// the first library (in `libs` order) whose index exports it contributes that
/// member once. A pulled member may need further members, from any library, so
/// the order of libraries matters only when two export the same name (the first
/// wins). Names no library exports are left for the link to report.
pub fn resolve_libraries(
    objects: &[Object],
    libs: &[Library],
    options: &LinkOptions,
) -> Result<Vec<Object>, LinkError> {
    let indexes: Vec<BTreeMap<String, usize>> =
        libs.iter().map(Library::index).collect::<Result<_, _>>()?;
    let mut all: Vec<Object> = objects.to_vec();
    let mut pulled: BTreeSet<(usize, usize)> = BTreeSet::new();
    loop {
        let mut progress = false;
        for name in unresolved(&all, options) {
            let hit = indexes
                .iter()
                .enumerate()
                .find_map(|(l, idx)| idx.get(&name).map(|&m| (l, m)));
            if let Some((l, m)) = hit {
                if pulled.insert((l, m)) {
                    all.push(libs[l].members[m].clone());
                    progress = true;
                }
            }
        }
        if !progress {
            return Ok(all);
        }
    }
}

/// [`link_with_options`] over `objects` and the members of `libs` they need.
pub fn link_with_libraries(
    objects: &[Object],
    libs: &[Library],
    options: &LinkOptions,
) -> Result<Linked, LinkError> {
    link_with_options(&resolve_libraries(objects, libs, options)?, options)
}

// ---------------------------------------------------------------------------
// ld65-style memory/segment configuration.

/// One `MEMORY` area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryArea {
    pub name: String,
    pub start: u16,
    /// Bytes, 1 to $10000; `start + size` must not pass $10000.
    pub size: u32,
    /// `fill = yes`: the load image covers the whole area.
    pub fill: bool,
    pub fillval: u8,
}

/// One `SEGMENTS` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRule {
    /// Section name it collects (every object's section of that name).
    pub name: String,
    /// Area holding the bytes in the image.
    pub load: String,
    /// Area the code runs in (addresses are resolved there); `None` = `load`.
    pub run: Option<String>,
    /// Run address alignment (1 = none).
    pub align: u32,
    /// Fixed run start address.
    pub start: Option<u16>,
    /// Define `__NAME_LOAD__`, `__NAME_RUN__`, `__NAME_SIZE__` (lowercase here).
    pub define: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkConfig {
    pub memory: Vec<MemoryArea>,
    pub segments: Vec<SegmentRule>,
}

/// A segment's final placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentPlacement {
    pub name: String,
    pub load: u16,
    pub run: u16,
    pub size: usize,
}

/// Result of [`link_with_config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLinked {
    /// The link at run addresses.
    pub linked: Linked,
    pub segments: Vec<SegmentPlacement>,
    /// Load image of each area that holds bytes: `(name, start, bytes)`. A `fill`
    /// area spans its whole size; otherwise the image runs from the area start to
    /// its last loaded byte, gaps filled with `fillval`.
    pub images: Vec<(String, u16, Vec<u8>)>,
}

impl ConfigLinked {
    /// Segments whose load and run addresses differ and that hold bytes: what
    /// startup code must copy before running them, as `(load, run, size)`.
    pub fn copy_table(&self) -> Vec<(u16, u16, u16)> {
        self.segments
            .iter()
            .filter(|s| s.load != s.run && s.size > 0)
            .map(|s| (s.load, s.run, s.size as u16))
            .collect()
    }

    /// [`ConfigLinked::copy_table`] as bytes for a startup routine: per entry
    /// load, run and size as little-endian words, ended by a zero size word.
    pub fn copy_table_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (load, run, size) in self.copy_table() {
            out.extend(load.to_le_bytes());
            out.extend(run.to_le_bytes());
            out.extend(size.to_le_bytes());
        }
        out.extend([0, 0]);
        out
    }
}

/// Parse a config in ld65 syntax (a subset):
///
/// ```text
/// MEMORY {
///     ROM: start = $8000, size = $8000, fill = yes, fillval = $FF;
///     RAM: start = $0200, size = $0600;
/// }
/// SEGMENTS {
///     CODE: load = ROM;
///     DATA: load = ROM, run = RAM, define = yes, align = $10;
///     VECTORS: load = ROM, start = $FFFA;
/// }
/// ```
///
/// `#` starts a comment. Names are case-insensitive. Numbers use the
/// assembler's expression syntax. Unknown blocks or attributes, a missing
/// `start`/`size`/`load`, a duplicate name, an unknown area, an area past
/// $FFFF, overlapping areas and `align = 0` are errors.
pub fn parse_link_config(text: &str) -> Result<LinkConfig, LinkError> {
    let err = |m: String| LinkError::plain(format!("config: {m}"));
    let clean: String = text
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let mut config = LinkConfig::default();
    let mut rest = clean.as_str();
    while !rest.trim().is_empty() {
        let open = rest
            .find('{')
            .ok_or_else(|| err(format!("bloco sem '{{': {}", rest.trim())))?;
        let block = rest[..open].trim().to_ascii_lowercase();
        let close = rest[open..]
            .find('}')
            .ok_or_else(|| err(format!("{block} sem '}}'")))?
            + open;
        let body = &rest[open + 1..close];
        rest = &rest[close + 1..];
        if block != "memory" && block != "segments" {
            return Err(err(format!("bloco desconhecido: {block}")));
        }
        for entry in body.split(';').map(str::trim).filter(|e| !e.is_empty()) {
            let (name, attrs) = entry
                .split_once(':')
                .ok_or_else(|| err(format!("entrada sem ':': {entry}")))?;
            let name = one_ident(name).map_err(|e| err(e.message))?;
            let mut map: BTreeMap<String, String> = BTreeMap::new();
            for attr in attrs.split(',').map(str::trim).filter(|a| !a.is_empty()) {
                let (k, v) = attr
                    .split_once('=')
                    .ok_or_else(|| err(format!("atributo sem '=': {attr}")))?;
                if map
                    .insert(k.trim().to_ascii_lowercase(), v.trim().to_string())
                    .is_some()
                {
                    return Err(err(format!("atributo repetido em {name}: {}", k.trim())));
                }
            }
            let num = |k: &str| -> Result<Option<u32>, LinkError> {
                map.get(k)
                    .map(|v| {
                        if v.eq_ignore_ascii_case("$10000") {
                            Ok(0x10000)
                        } else {
                            eval_const(v).map(u32::from)
                        }
                    })
                    .transpose()
            };
            let yes = |k: &str| -> Result<bool, LinkError> {
                match map.get(k).map(|v| v.to_ascii_lowercase()) {
                    None => Ok(false),
                    Some(v) if v == "yes" => Ok(true),
                    Some(v) if v == "no" => Ok(false),
                    Some(v) => Err(err(format!("{k} deve ser yes ou no: {v}"))),
                }
            };
            let allowed: &[&str] = match block.as_str() {
                "memory" => &["start", "size", "fill", "fillval", "type", "file"],
                "segments" => &["load", "run", "align", "start", "define", "type"],
                _ => return Err(err(format!("bloco desconhecido: {block}"))),
            };
            if let Some(k) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
                return Err(err(format!("atributo desconhecido em {name}: {k}")));
            }
            if block == "memory" {
                let start = num("start")?.ok_or_else(|| err(format!("{name} sem start")))?;
                let size = num("size")?.ok_or_else(|| err(format!("{name} sem size")))?;
                if size == 0 || start + size > 0x10000 {
                    return Err(err(format!("área {name} fora de $0000-$FFFF")));
                }
                if config.memory.iter().any(|m| m.name == name) {
                    return Err(err(format!("área repetida: {name}")));
                }
                let fillval = u8::try_from(num("fillval")?.unwrap_or(0))
                    .map_err(|_| err(format!("fillval fora da faixa em {name}")))?;
                config.memory.push(MemoryArea {
                    name,
                    start: start as u16,
                    size,
                    fill: yes("fill")?,
                    fillval,
                });
            } else {
                let load = map
                    .get("load")
                    .ok_or_else(|| err(format!("{name} sem load")))
                    .and_then(|v| one_ident(v).map_err(|e| err(e.message)))?;
                let run = map
                    .get("run")
                    .map(|v| one_ident(v).map_err(|e| err(e.message)))
                    .transpose()?;
                let align = num("align")?.unwrap_or(1);
                if align == 0 {
                    return Err(err(format!("align zero em {name}")));
                }
                let start = num("start")?.map(|v| v as u16);
                if config.segments.iter().any(|s| s.name == name) {
                    return Err(err(format!("segmento repetido: {name}")));
                }
                config.segments.push(SegmentRule {
                    name,
                    load,
                    run,
                    align,
                    start,
                    define: yes("define")?,
                });
            }
        }
    }
    for (i, a) in config.memory.iter().enumerate() {
        for b in &config.memory[..i] {
            let (a0, a1) = (u32::from(a.start), u32::from(a.start) + a.size);
            let (b0, b1) = (u32::from(b.start), u32::from(b.start) + b.size);
            if a0 < b1 && b0 < a1 {
                return Err(err(format!("áreas {} e {} se sobrepõem", b.name, a.name)));
            }
        }
    }
    for s in &config.segments {
        for area in std::iter::once(&s.load).chain(s.run.iter()) {
            if !config.memory.iter().any(|m| &m.name == area) {
                return Err(err(format!(
                    "segmento {} usa área desconhecida: {area}",
                    s.name
                )));
            }
        }
    }
    Ok(config)
}

/// Link under a [`LinkConfig`].
///
/// Segments are laid out in config order. In its run area a segment starts at
/// the area's cursor (or at `start`, which must not be behind the cursor),
/// rounded up to `align`; its sections follow one another in object order. A
/// segment with `run` different from `load` also takes the same number of bytes
/// at the load area's cursor (no alignment), and its code is linked at the run
/// address. A segment with no sections takes no space. Sections that no rule
/// names, sections with their own `.org`, and a segment overflowing an area are
/// errors. `define = yes` provides `__<name>_load__`, `__<name>_run__` and
/// `__<name>_size__` weakly. `options.script` must be empty (the config places
/// everything); the other options apply as in [`link_with_options`].
pub fn link_with_config(
    objects: &[Object],
    config: &LinkConfig,
    options: &LinkOptions,
) -> Result<ConfigLinked, LinkError> {
    let err = |m: String| LinkError::plain(format!("config: {m}"));
    if !options.script.is_empty() {
        return Err(err("script e config juntos".into()));
    }
    for o in objects {
        for s in &o.sections {
            if !config.segments.iter().any(|r| r.name == s.name) {
                return Err(err(format!("seção sem segmento: {} ({})", s.name, o.name)));
            }
            if s.origin.is_some() {
                return Err(err(format!("seção com .org: {} ({})", s.name, o.name)));
            }
        }
    }
    let area = |name: &str| config.memory.iter().find(|m| m.name == name).unwrap();
    let mut cursor: BTreeMap<String, u32> = config
        .memory
        .iter()
        .map(|m| (m.name.clone(), u32::from(m.start)))
        .collect();
    let mut placed = objects.to_vec();
    // (module, section index) -> load address
    let mut load_of: BTreeMap<(usize, usize), u16> = BTreeMap::new();
    let mut segments = Vec::new();
    let mut provide = options.provide.clone();
    for rule in &config.segments {
        let members: Vec<(usize, usize)> = objects
            .iter()
            .enumerate()
            .flat_map(|(m, o)| {
                o.sections
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.name == rule.name)
                    .map(move |(i, _)| (m, i))
            })
            .collect();
        let size: usize = members
            .iter()
            .map(|&(m, i)| objects[m].sections[i].bytes.len())
            .sum();
        let run_name = rule.run.as_deref().unwrap_or(&rule.load);
        let run_area = area(run_name);
        let area_end = u32::from(run_area.start) + run_area.size;
        let mut run = cursor[run_name];
        if let Some(start) = rule.start {
            let start = u32::from(start);
            if start < run || start >= area_end {
                return Err(err(format!(
                    "start ${start:04X} de {} fora da área {run_name} ou atrás do cursor ${run:04X}",
                    rule.name
                )));
            }
            run = start;
        }
        run = run.div_ceil(rule.align) * rule.align;
        if size > 0 && run + size as u32 > area_end {
            return Err(err(format!(
                "segmento {} não cabe na área {run_name}",
                rule.name
            )));
        }
        let load = if rule.run.is_some() && size > 0 {
            let load_area = area(&rule.load);
            let at = cursor[&rule.load];
            if at + size as u32 > u32::from(load_area.start) + load_area.size {
                return Err(err(format!(
                    "segmento {} não cabe na área {}",
                    rule.name, rule.load
                )));
            }
            cursor.insert(rule.load.clone(), at + size as u32);
            at
        } else {
            run
        };
        if size > 0 {
            cursor.insert(run_name.to_string(), run + size as u32);
        }
        let mut offset = 0u32;
        for &(m, i) in &members {
            placed[m].sections[i].origin = Some((run + offset) as u16);
            load_of.insert((m, i), (load + offset) as u16);
            offset += objects[m].sections[i].bytes.len() as u32;
        }
        if rule.define {
            provide.insert(format!("__{}_load__", rule.name), load as u16);
            provide.insert(format!("__{}_run__", rule.name), run as u16);
            provide.insert(format!("__{}_size__", rule.name), size as u16);
        }
        segments.push(SegmentPlacement {
            name: rule.name.clone(),
            load: load as u16,
            run: run as u16,
            size,
        });
    }
    let opts = LinkOptions {
        provide,
        ..options.clone()
    };
    let linked = link_with_options(&placed, &opts)?;
    // Load images: put each placed section's linked bytes at its load address.
    let mut images = Vec::new();
    for m in &config.memory {
        let mut bytes: Vec<u8> = Vec::new();
        let mut used = false;
        for (pi, p) in linked.placements.iter().enumerate() {
            let Some(module) = placed.iter().position(|o| o.name == p.module) else {
                continue;
            };
            let Some(index) = placed[module]
                .sections
                .iter()
                .position(|s| s.name == p.section && s.origin == Some(p.address))
            else {
                continue;
            };
            let at = load_of[&(module, index)];
            let data = &linked.segments[pi].bytes;
            let off = u32::from(at).wrapping_sub(u32::from(m.start));
            if data.is_empty() || off >= m.size {
                continue;
            }
            let end = off as usize + data.len();
            if bytes.len() < end {
                bytes.resize(end, m.fillval);
            }
            bytes[off as usize..end].copy_from_slice(data);
            used = true;
        }
        if m.fill {
            bytes.resize(m.size as usize, m.fillval);
            used = true;
        }
        if used {
            images.push((m.name.clone(), m.start, bytes));
        }
    }
    Ok(ConfigLinked {
        linked,
        segments,
        images,
    })
}

// ---------------------------------------------------------------------------
// o65 relocatable object format (André Fachat / cc65 subset).

/// One exported or imported symbol in an o65 file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct O65Symbol {
    pub name: String,
    pub value: u32,
    /// Segment id: 0=absolute, 2=text, 3=data, 4=bss, 5=zero.
    pub segment: u8,
    pub exported: bool,
}

/// Parsed o65 image (32-bit mode, 6502 CPU).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct O65File {
    pub text: Vec<u8>,
    pub data: Vec<u8>,
    pub bss_size: u32,
    pub zero_size: u32,
    pub stack_size: u32,
    pub text_base: u32,
    pub data_base: u32,
    pub exports: Vec<O65Symbol>,
}

impl O65File {
    /// Write a minimal 32-bit o65 file (text+data, exports, no undefineds, no
    /// reloc tables). Magic `o65\\0`, mode `0x0001` (32-bit, 6502), then sizes and
    /// bases, the segment bytes, a zero byte option list terminator, the export
    /// table (count u32, then per symbol: name NUL, segment u8, value u32).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::from(&b"o65\0"[..]);
        out.extend(1u16.to_le_bytes()); // mode: 32-bit
        out.extend(0u16.to_le_bytes()); // version
        for v in [
            self.text_base,
            self.data_base,
            0u32, // bss base = 0 (not placed)
            0u32, // zero base
        ] {
            out.extend(v.to_le_bytes());
        }
        for v in [
            self.text.len() as u32,
            self.data.len() as u32,
            self.bss_size,
            self.zero_size,
            self.stack_size,
        ] {
            out.extend(v.to_le_bytes());
        }
        out.extend(&self.text);
        out.extend(&self.data);
        out.push(0); // end of options
        let exports: Vec<_> = self.exports.iter().filter(|s| s.exported).collect();
        out.extend((exports.len() as u32).to_le_bytes());
        for s in exports {
            out.extend(s.name.as_bytes());
            out.push(0);
            out.push(s.segment);
            out.extend(s.value.to_le_bytes());
        }
        out.extend(0u32.to_le_bytes()); // undefined count
        out
    }

    /// Parse a file written by [`O65File::to_bytes`]. Other o65 variants (16-bit,
    /// reloc tables, options) are rejected.
    pub fn from_bytes(bytes: &[u8]) -> Result<O65File, LinkError> {
        let bad = |m: &str| LinkError::plain(format!("o65 inválido: {m}"));
        if bytes.len() < 4 + 2 + 2 + 4 * 9 {
            return Err(bad("curto"));
        }
        if &bytes[..4] != b"o65\0" {
            return Err(bad("assinatura"));
        }
        let mode = u16::from_le_bytes([bytes[4], bytes[5]]);
        if mode != 1 {
            return Err(bad("só modo 32-bit"));
        }
        let mut at = 8usize;
        let take_u32 = |at: &mut usize| -> Result<u32, LinkError> {
            let v = bytes.get(*at..*at + 4).ok_or_else(|| bad("truncado"))?;
            *at += 4;
            Ok(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
        };
        let text_base = take_u32(&mut at)?;
        let data_base = take_u32(&mut at)?;
        let _bss_base = take_u32(&mut at)?;
        let _zero_base = take_u32(&mut at)?;
        let text_size = take_u32(&mut at)? as usize;
        let data_size = take_u32(&mut at)? as usize;
        let bss_size = take_u32(&mut at)?;
        let zero_size = take_u32(&mut at)?;
        let stack_size = take_u32(&mut at)?;
        let text = bytes
            .get(at..at + text_size)
            .ok_or_else(|| bad("text"))?
            .to_vec();
        at += text_size;
        let data = bytes
            .get(at..at + data_size)
            .ok_or_else(|| bad("data"))?
            .to_vec();
        at += data_size;
        if bytes.get(at) != Some(&0) {
            return Err(bad("options"));
        }
        at += 1;
        let nexp = take_u32(&mut at)? as usize;
        let mut exports = Vec::with_capacity(nexp);
        for _ in 0..nexp {
            let end = bytes[at..]
                .iter()
                .position(|&b| b == 0)
                .ok_or_else(|| bad("nome"))?;
            let name = String::from_utf8(bytes[at..at + end].to_vec()).map_err(|_| bad("utf8"))?;
            at += end + 1;
            let segment = *bytes.get(at).ok_or_else(|| bad("seg"))?;
            at += 1;
            let value = take_u32(&mut at)?;
            exports.push(O65Symbol {
                name,
                value,
                segment,
                exported: true,
            });
        }
        let nund = take_u32(&mut at)?;
        if nund != 0 {
            return Err(bad("undefined não suportado nesta leitura"));
        }
        if at != bytes.len() {
            return Err(bad("bytes sobrando"));
        }
        Ok(O65File {
            text,
            data,
            bss_size,
            zero_size,
            stack_size,
            text_base,
            data_base,
            exports,
        })
    }

    /// Build an o65 file from a linked image: the first two segments become text
    /// and data (or one segment is text and data is empty). Exports are the
    /// globals, with segment 2 (text) or 3 (data) by address.
    pub fn from_linked(linked: &Linked) -> Result<O65File, LinkError> {
        if linked.segments.is_empty() {
            return Err(LinkError::plain("o65: link sem segmentos".into()));
        }
        let text_seg = &linked.segments[0];
        let data_seg = linked.segments.get(1);
        let text_base = u32::from(text_seg.address);
        let data_base = data_seg.map(|s| u32::from(s.address)).unwrap_or(0);
        let text_end = text_base + text_seg.bytes.len() as u32;
        let mut exports = Vec::new();
        for (name, &val) in &linked.symbols {
            let v = u32::from(val);
            let segment = if v >= text_base && v < text_end {
                2
            } else if data_seg.is_some_and(|s| {
                let b = u32::from(s.address);
                v >= b && v < b + s.bytes.len() as u32
            }) {
                3
            } else {
                0
            };
            exports.push(O65Symbol {
                name: name.clone(),
                value: v,
                segment,
                exported: true,
            });
        }
        exports.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(O65File {
            text: text_seg.bytes.clone(),
            data: data_seg.map(|s| s.bytes.clone()).unwrap_or_default(),
            bss_size: 0,
            zero_size: 0,
            stack_size: 0,
            text_base,
            data_base,
            exports,
        })
    }
}
