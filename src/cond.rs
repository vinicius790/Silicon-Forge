//! Breakpoint conditions for the debugger.
//!
//! Grammar (loosest first): `cond := and ("||" and)*`, `and := cmp ("&&" cmp)*`,
//! `cmp := term (op term)?` with `op` one of `== != <= >= < >`; a lone term is true
//! when non-zero. A term is an [`crate::expr`] expression in which:
//! * `A X Y SP P PC` (any case, whole words) are the registers;
//! * `N V B D I Z C` are the status flags as 0/1;
//! * `CYC` is the low 16 bits of the cycle counter;
//! * `[e]` is the byte at address `e` (nests: `[[$10]+1]`), read with `peek` so the
//!   condition has no side effects.
//!
//! Because bare words are registers, write hexadecimal digits with a `$` (`A == $0A`).
//! Bare numbers keep the assembler's convention (hexadecimal). Parsing happens once
//! in [`Condition::parse`]; evaluation cannot fail for syntax, only for division by
//! zero, which is reported as [`CondError::Eval`].

use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CondError {
    Empty,
    /// Unbalanced `[`/`]` at the given byte offset.
    Brackets(usize),
    /// A comparison or `&&`/`||` with a missing side.
    MissingOperand(String),
    /// The expression evaluator rejected a term.
    Eval(String),
}

impl fmt::Display for CondError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("condição vazia"),
            Self::Brackets(at) => write!(f, "colchetes desbalanceados na posição {at}"),
            Self::MissingOperand(s) => write!(f, "operando ausente em '{s}'"),
            Self::Eval(m) => write!(f, "expressão: {m}"),
        }
    }
}

impl std::error::Error for CondError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Eq,
    Ne,
    Le,
    Ge,
    Lt,
    Gt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Clause {
    left: String,
    cmp: Option<(Cmp, String)>,
}

/// A parsed condition, OR of ANDs of comparisons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub source: String,
    any: Vec<Vec<Clause>>,
}

const REGS: &[&str] = &[
    "A", "X", "Y", "SP", "P", "PC", "N", "V", "B", "D", "I", "Z", "C", "CYC",
];

/// Values a condition sees; built from a CPU by [`Condition::eval`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct View {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub p: u8,
    pub pc: u16,
    pub cycles: u64,
}

impl View {
    fn reg(&self, name: &str) -> u16 {
        let flag = |bit: u8| u16::from(self.p & bit != 0);
        match name {
            "A" => self.a.into(),
            "X" => self.x.into(),
            "Y" => self.y.into(),
            "SP" => self.sp.into(),
            "P" => self.p.into(),
            "PC" => self.pc,
            "N" => flag(0x80),
            "V" => flag(0x40),
            "B" => flag(0x10),
            "D" => flag(0x08),
            "I" => flag(0x04),
            "Z" => flag(0x02),
            "C" => flag(0x01),
            _ => self.cycles as u16,
        }
    }
}

fn split_top<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let (mut depth, mut start, mut i) = (0i32, 0, 0);
    let b = s.as_bytes();
    while i < b.len() {
        match b[i] {
            b'[' | b'(' => depth += 1,
            b']' | b')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && s[i..].starts_with(sep) {
            out.push(&s[start..i]);
            i += sep.len();
            start = i;
            continue;
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

fn find_cmp(s: &str) -> Option<(usize, Cmp, usize)> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'[' | b'(' => depth += 1,
            b']' | b')' => depth -= 1,
            _ => {}
        }
        if depth != 0 {
            continue;
        }
        let two = &s[i..(i + 2).min(s.len())];
        let op = match two {
            "==" => Some((Cmp::Eq, 2)),
            "!=" => Some((Cmp::Ne, 2)),
            "<=" => Some((Cmp::Le, 2)),
            ">=" => Some((Cmp::Ge, 2)),
            "<<" | ">>" => None,
            _ => None,
        };
        if let Some((c, n)) = op {
            return Some((i, c, n));
        }
        let prev_shift = i > 0 && (b[i - 1] == b'<' || b[i - 1] == b'>');
        if (b[i] == b'<' || b[i] == b'>')
            && !two.starts_with("<<")
            && !two.starts_with(">>")
            && !prev_shift
        {
            // A unary `<`/`>` (low/high byte) has nothing before it.
            if s[..i].trim().is_empty() {
                continue;
            }
            return Some((i, if b[i] == b'<' { Cmp::Lt } else { Cmp::Gt }, 1));
        }
    }
    None
}

fn check_brackets(s: &str) -> Result<(), CondError> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth < 0 {
                    return Err(CondError::Brackets(i));
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(CondError::Brackets(s.len()));
    }
    Ok(())
}

/// Replace register words and `[e]` reads with numbers, then evaluate.
type Lookup<'a> = &'a dyn Fn(&str) -> Option<u16>;

fn term(
    src: &str,
    view: &View,
    peek: &dyn Fn(u16) -> u8,
    lookup: Lookup,
) -> Result<u16, CondError> {
    let mut out = String::new();
    let mut rest = src;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let mut depth = 0;
        let mut close = open;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '[' => depth += 1,
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let addr = term(&rest[open + 1..close], view, peek, lookup)?;
        out.push_str(&format!("${:02X}", peek(addr)));
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    let mut text = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, text: &mut String| {
        let up = word.to_ascii_uppercase();
        if REGS.contains(&up.as_str()) && !text.ends_with('$') {
            text.push_str(&format!("${:X}", view.reg(&up)));
        } else {
            text.push_str(word);
        }
        word.clear();
    };
    for c in out.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut text);
            text.push(c);
        }
    }
    flush(&mut word, &mut text);
    crate::expr::eval(text.trim(), lookup).map_err(|e| CondError::Eval(e.0))
}

impl Condition {
    pub fn parse(source: &str) -> Result<Self, CondError> {
        Self::parse_with(source, &|_| None)
    }

    /// Parse with symbol names resolved by `lookup` (labels from the assembler).
    pub fn parse_with(source: &str, lookup: Lookup) -> Result<Self, CondError> {
        if source.trim().is_empty() {
            return Err(CondError::Empty);
        }
        check_brackets(source)?;
        let mut any = Vec::new();
        for alt in split_top(source, "||") {
            let mut all = Vec::new();
            for part in split_top(alt, "&&") {
                if part.trim().is_empty() {
                    return Err(CondError::MissingOperand(source.into()));
                }
                let clause = match find_cmp(part) {
                    Some((i, c, n)) => {
                        let (l, r) = (part[..i].trim(), part[i + n..].trim());
                        if l.is_empty() || r.is_empty() {
                            return Err(CondError::MissingOperand(part.trim().into()));
                        }
                        Clause {
                            left: l.into(),
                            cmp: Some((c, r.into())),
                        }
                    }
                    None => Clause {
                        left: part.trim().into(),
                        cmp: None,
                    },
                };
                all.push(clause);
            }
            any.push(all);
        }
        let cond = Self {
            source: source.into(),
            any,
        };
        // Syntax check every term against a zero machine; only /0 may fail later.
        let zero = View::default();
        for clause in cond.any.iter().flatten() {
            for t in std::iter::once(&clause.left).chain(clause.cmp.iter().map(|(_, r)| r)) {
                match term(t, &zero, &|_| 1, lookup) {
                    Err(CondError::Eval(m)) if m.contains("zero") => {}
                    Err(e) => return Err(e),
                    Ok(_) => {}
                }
            }
        }
        Ok(cond)
    }

    /// Evaluate against explicit values and a memory reader.
    pub fn eval_view(&self, view: &View, peek: &dyn Fn(u16) -> u8) -> Result<bool, CondError> {
        self.eval_view_with(view, peek, &|_| None)
    }

    pub fn eval_view_with(
        &self,
        view: &View,
        peek: &dyn Fn(u16) -> u8,
        lookup: Lookup,
    ) -> Result<bool, CondError> {
        for all in &self.any {
            let mut ok = true;
            for c in all {
                let l = term(&c.left, view, peek, lookup)?;
                let v = match &c.cmp {
                    None => l != 0,
                    Some((op, r)) => {
                        let r = term(r, view, peek, lookup)?;
                        match op {
                            Cmp::Eq => l == r,
                            Cmp::Ne => l != r,
                            Cmp::Le => l <= r,
                            Cmp::Ge => l >= r,
                            Cmp::Lt => l < r,
                            Cmp::Gt => l > r,
                        }
                    }
                };
                if !v {
                    ok = false;
                    break;
                }
            }
            if ok {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn eval<B: MemoryBus>(&self, cpu: &Cpu<B>) -> Result<bool, CondError> {
        self.eval_with(cpu, &|_| None)
    }

    pub fn eval_with<B: MemoryBus>(&self, cpu: &Cpu<B>, lookup: Lookup) -> Result<bool, CondError> {
        let r = cpu.registers();
        let view = View {
            a: r.a,
            x: r.x,
            y: r.y,
            sp: r.sp,
            p: r.p,
            pc: r.pc,
            cycles: r.cycles,
        };
        self.eval_view_with(&view, &|a| cpu.bus.peek(a), lookup)
    }
}
