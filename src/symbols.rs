//! Symbol files: labels exported by the assembler or the linker, loaded back into the
//! debugger and the disassembler.
//!
//! The text format is the one [`crate::asm::AsmOutput::to_symbol_table`] always wrote:
//! one `name = $XXXX` per line, sorted by name. Blank lines and lines starting with `;`
//! are ignored when loading. A name is a letter, `_`, `.` or `@` followed by letters,
//! digits, `_`, `.`, `@` or `:`. The same name twice with two addresses is an error;
//! the same name twice with one address is accepted once.
//!
//! Several names may share an address. [`SymbolTable::name_at`] picks one stably: a
//! global name (no `.` or `@`) before a local one, then the shortest, then the first
//! in byte order. [`SymbolTable::resolve`] evaluates a location with the assembler's
//! expression rules ([`crate::expr`]): names, `name+4`, `$C000`, `<name`, and bare
//! numbers in hex.

use std::fmt::Write as _;

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use crate::disasm::{format_named, Line};
use crate::opcodes::CpuVariant;

/// Why a symbol file did not load. `line` is 1-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolError {
    pub line: usize,
    pub kind: SymbolErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolErrorKind {
    /// No `=` between name and address.
    MissingEquals,
    BadName(String),
    /// The address is not `$` followed by one to four hex digits.
    BadAddress(String),
    /// The name was already bound to `first`.
    Conflict {
        name: String,
        first: u16,
        second: u16,
    },
}

impl fmt::Display for SymbolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "linha {}: ", self.line)?;
        match &self.kind {
            SymbolErrorKind::MissingEquals => f.write_str("falta '=' entre nome e endereço"),
            SymbolErrorKind::BadName(name) => write!(f, "nome inválido: {name:?}"),
            SymbolErrorKind::BadAddress(text) => write!(f, "endereço inválido: {text:?}"),
            SymbolErrorKind::Conflict {
                name,
                first,
                second,
            } => write!(f, "{name} já vale ${first:04X}, não ${second:04X}"),
        }
    }
}

impl std::error::Error for SymbolError {}

/// Names and addresses, searchable both ways.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolTable {
    by_name: BTreeMap<String, u16>,
    by_addr: BTreeMap<u16, Vec<String>>,
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '_' | '.' | '@'))
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | ':'))
}

fn is_local(name: &str) -> bool {
    name.contains(['.', '@'])
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `name` to `addr`. Rebinding to the same address is a no-op.
    pub fn insert(&mut self, name: &str, addr: u16) -> Result<(), SymbolErrorKind> {
        if !valid_name(name) {
            return Err(SymbolErrorKind::BadName(name.into()));
        }
        match self.by_name.get(name) {
            Some(&first) if first == addr => return Ok(()),
            Some(&first) => {
                return Err(SymbolErrorKind::Conflict {
                    name: name.into(),
                    first,
                    second: addr,
                })
            }
            None => {}
        }
        self.by_name.insert(name.into(), addr);
        let names = self.by_addr.entry(addr).or_default();
        names.push(name.into());
        names.sort_by(|a, b| {
            (is_local(a), a.len(), a.as_str()).cmp(&(is_local(b), b.len(), b.as_str()))
        });
        Ok(())
    }

    /// Table from any name → address map (assembler `symbols`, linker `symbols`).
    pub fn from_map<'a>(
        map: impl IntoIterator<Item = (&'a String, &'a u16)>,
    ) -> Result<Self, SymbolErrorKind> {
        let mut table = Self::new();
        for (name, &addr) in map {
            table.insert(name, addr)?;
        }
        Ok(table)
    }

    pub fn from_asm(output: &crate::asm::AsmOutput) -> Result<Self, SymbolErrorKind> {
        Self::from_map(&output.symbols)
    }

    pub fn from_linked(linked: &crate::link::Linked) -> Result<Self, SymbolErrorKind> {
        Self::from_map(&linked.symbols)
    }

    /// Load the text format. Stops at the first bad line.
    pub fn parse(text: &str) -> Result<Self, SymbolError> {
        let mut table = Self::new();
        for (index, raw) in text.lines().enumerate() {
            let line = index + 1;
            let fail = |kind| SymbolError { line, kind };
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with(';') {
                continue;
            }
            let (name, value) = trimmed
                .split_once('=')
                .ok_or_else(|| fail(SymbolErrorKind::MissingEquals))?;
            let (name, value) = (name.trim(), value.trim());
            let addr = value
                .strip_prefix('$')
                .filter(|hex| (1..=4).contains(&hex.len()))
                .and_then(|hex| u16::from_str_radix(hex, 16).ok())
                .ok_or_else(|| fail(SymbolErrorKind::BadAddress(value.into())))?;
            table.insert(name, addr).map_err(fail)?;
        }
        Ok(table)
    }

    /// The text format, sorted by name; [`SymbolTable::parse`] reads it back unchanged.
    pub fn to_text(&self) -> String {
        self.by_name
            .iter()
            .fold(String::new(), |mut out, (name, addr)| {
                let _ = writeln!(out, "{name:<20} = ${addr:04X}");
                out
            })
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    pub fn address_of(&self, name: &str) -> Option<u16> {
        self.by_name.get(name).copied()
    }

    /// Preferred name at exactly `addr` (see the module docs).
    pub fn name_at(&self, addr: u16) -> Option<&str> {
        self.by_addr
            .get(&addr)
            .and_then(|names| names.first())
            .map(String::as_str)
    }

    /// Every name at `addr`, preferred first.
    pub fn names_at(&self, addr: u16) -> &[String] {
        self.by_addr.get(&addr).map_or(&[], Vec::as_slice)
    }

    /// `name` or `name+N` for the nearest symbol at or below `addr` within `reach`
    /// bytes. Used to say which routine an address belongs to.
    pub fn describe(&self, addr: u16, reach: u16) -> Option<String> {
        let (&base, names) = self.by_addr.range(..=addr).next_back()?;
        let offset = addr - base;
        if offset > reach {
            return None;
        }
        let name = &names[0];
        Some(if offset == 0 {
            name.clone()
        } else {
            format!("{name}+{offset}")
        })
    }

    /// Evaluate a location: a name, an expression over names, or a number.
    pub fn resolve(&self, text: &str) -> Result<u16, String> {
        crate::expr::eval(text, &|name| self.address_of(name)).map_err(|e| e.0)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, u16)> {
        self.by_name
            .iter()
            .map(|(name, &addr)| (name.as_str(), addr))
    }

    /// Address → preferred name, the shape [`crate::disasm::disasm_range`] takes.
    pub fn address_map(&self) -> HashMap<u16, String> {
        self.by_addr
            .iter()
            .map(|(&addr, names)| (addr, names[0].clone()))
            .collect()
    }

    /// One instruction with memory operands and branch targets named.
    pub fn disasm_at(&self, mem: &[u8], pc: u16, variant: CpuVariant) -> Line {
        let read = |offset: u16| {
            mem.get(usize::from(pc.wrapping_add(offset)))
                .copied()
                .unwrap_or(0)
        };
        format_named(read(0), read(1), read(2), pc, variant, &|addr| {
            self.name_at(addr).map(str::to_string)
        })
    }

    /// Listing of `count` instructions from `start`, with a `name:` line before each
    /// labelled address.
    pub fn listing(&self, mem: &[u8], start: u16, count: usize, variant: CpuVariant) -> String {
        let mut out = String::new();
        let mut pc = start;
        for _ in 0..count {
            for name in self.names_at(pc) {
                out.push_str(&format!("{name}:\n"));
            }
            let line = self.disasm_at(mem, pc, variant);
            out.push_str(&format!(
                "  {pc:04X}  {:<8}  {}\n",
                line.bytes_hex, line.text
            ));
            pc = pc.wrapping_add(u16::from(line.size));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_name_is_global_then_shortest() {
        let mut table = SymbolTable::new();
        table.insert("loop.inner", 0x8000).unwrap();
        table.insert("start_here", 0x8000).unwrap();
        table.insert("go", 0x8000).unwrap();
        assert_eq!(table.name_at(0x8000), Some("go"));
        assert_eq!(table.names_at(0x8000), ["go", "start_here", "loop.inner"]);
    }

    #[test]
    fn describe_names_the_enclosing_label_within_reach() {
        let mut table = SymbolTable::new();
        table.insert("print", 0x9000).unwrap();
        assert_eq!(table.describe(0x9003, 16).as_deref(), Some("print+3"));
        assert_eq!(table.describe(0x9020, 16), None);
        assert_eq!(table.describe(0x8FFF, 16), None);
    }
}
