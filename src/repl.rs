//! Scriptable debugger REPL: commands from any `BufRead`, output to any `Write`.
//!
//! No network and no terminal escapes, so a test can feed a script and compare text.
//! Locations go through [`SymbolTable::resolve`]: label names, `name+4`, `$C000`,
//! and bare numbers in hex (the assembler's rule, so `mem 0200 10` dumps 16 bytes).
//!
//! | command | effect |
//! | --- | --- |
//! | `break`/`b LOC`, `delete`/`bd LOC`, `breaks` | breakpoints, checked before an instruction |
//! | `step`/`s [N]` | N instructions (default 1); breakpoints do not stop it |
//! | `next`/`n` | one instruction; a `JSR` runs until it returns to the same depth |
//! | `finish`/`f` | run until the current `JSR` callee returns |
//! | `continue`/`c [N]` | run up to N instructions (default 1000000) |
//! | `regs`/`r`, `mem`/`x LOC [LEN]`, `dis`/`d [LOC] [N]`, `poke LOC VAL` | inspect and edit |
//! | `watch`/`w LOC`, `unwatch LOC`, `watches` | stop after an instruction that changes the byte |
//! | `sym NAME`, `syms`, `load PATH` | symbols; `load` reads a symbol file |
//! | `profile on|off|report [N]` | cycle profiler over what the REPL runs |
//! | `watch`/`w EXPR [if COND]` | the location is any expression (`table + 2`); with `if`, stop only when COND holds after the change |
//! | `snap`, `diff [reset]` | keep a 64 KiB image; list the bytes changed since (`reset` takes a new image) |
//! | `trace PATH`, `trace off` | write `PC BYTES TEXT A X Y P SP CYC` before each executed instruction |
//! | `source PATH` | run the file's lines as commands (`#` lines skipped, up to 8 levels; `quit` ends the session) |
//! | `histfile PATH|off`, `commands [N]` | append each typed command to a file; list the last N (hex, default $10) of up to 1000 |
//! | `history [N|off]` | show the step-back history, turn it on with depth N, or off |
//! | `back`/`rs [N]` | undo N recorded instructions (default 1) |
//! | `rc`/`reverse-continue` | undo until a breakpoint (whose `cond` holds) or `when` holds |
//! | `reset`, `help`/`h`, `quit`/`q` | |
//!
//! `next`, `finish` and `continue` stop early on a breakpoint (not on the instruction
//! they start at), a watch, a halt, or the instruction cap. Every run stop prints one
//! `parou:` line and the status line `REGS` / `==> ADDR BYTES TEXT`. A bad command
//! prints one `erro:` line and the session goes on. Watches compare `peek` values,
//! so they see any change, including one made by an interrupt handler.
//!
//! History is off until `history N`. Then every instruction the REPL runs first
//! records registers, cycles, halt state and the bytes changed since the previous
//! record (see [`crate::debugger::Debugger::history_bytes`]), so a `poke` between
//! two steps is undone too. Recording scans the 64 KiB space once per instruction.
//! Stepping back (`back` or `rc`) undoes the breakpoint accounting of every
//! instruction it undoes: an arrival counted by a forward run is subtracted from
//! `passagens`, and an `ignore` count it consumed is given back. So running
//! forward again over the same path reproduces the same stops. `rc` then treats
//! each restored state on a breakpoint as a reverse arrival: if its `cond` holds
//! and the breakpoint still has `ignore` left, one is consumed and rewinding goes
//! on; otherwise it stops there. Reverse arrivals are not added to `passagens`.
//! It also stops where `when` holds, or prints `parou: início do histórico` when
//! the history runs out.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, Write};

use crate::bus::MemoryBus;
use crate::cond::Condition;
use crate::cpu::Cpu;
use crate::debugger::Debugger;
use crate::profiler::Profiler;
use crate::symbols::SymbolTable;

const DEFAULT_RUN: u64 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunStop {
    Breakpoint,
    Watch {
        addr: u16,
        old: u8,
        new: u8,
    },
    Halted,
    Limit,
    Done,
    /// The `when` condition became true.
    Condition,
}

#[derive(Default)]
pub struct Repl {
    pub symbols: SymbolTable,
    pub breakpoints: BTreeSet<u16>,
    /// Watched address → last value seen.
    watches: BTreeMap<u16, u8>,
    profiler: Option<Profiler>,
    /// `cond`: breakpoint address → condition that must hold to stop.
    pub conditions: BTreeMap<u16, Condition>,
    /// Arrivals at each breakpoint while running.
    pub hits: BTreeMap<u16, u64>,
    /// `ignore`: arrivals still to skip per breakpoint.
    pub ignore: BTreeMap<u16, u64>,
    /// `when`: global condition checked before each instruction.
    pub when: Option<Condition>,
    /// Step-back history, `None` while `history` is off.
    pub rewind: Option<Debugger>,
    /// One entry per recorded step, aligned with `rewind`'s history: the
    /// breakpoint accounting done on the state that step produced (address, and
    /// whether an `ignore` count was consumed), so stepping back undoes it.
    journal: std::collections::VecDeque<Option<(u16, bool)>>,
    /// `watch LOC if COND`: the watch stops only when COND holds after the change.
    watch_conds: BTreeMap<u16, Condition>,
    /// Image taken by `snap`, compared by `diff`.
    snapshot: Option<Vec<u8>>,
    /// `trace PATH`: one line per executed instruction.
    trace: Option<io::BufWriter<std::fs::File>>,
    /// Command lines run at top level, oldest first, at most [`HISTORY_CAP`].
    pub commands: std::collections::VecDeque<String>,
    /// `histfile PATH`: every top-level command line is appended there.
    histfile: Option<std::fs::File>,
    /// Nesting of `source` files.
    source_depth: usize,
    /// A `quit` inside a `source`d file ends the session.
    quit_requested: bool,
}

/// Commands kept in memory for `commands`.
pub const HISTORY_CAP: usize = 1000;
/// Deepest `source` nesting.
pub const SOURCE_DEPTH: usize = 8;

/// Whether the session continues after a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

impl Repl {
    pub fn new(symbols: SymbolTable) -> Self {
        Self {
            symbols,
            ..Self::default()
        }
    }

    /// Read commands until `quit` or end of input. A prompt `(dbg XXXX)` is written
    /// before each line.
    pub fn run<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<B>,
        input: &mut dyn BufRead,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        let mut line = String::new();
        loop {
            write!(out, "(dbg {:04X}) ", cpu.pc)?;
            out.flush()?;
            line.clear();
            if input.read_line(&mut line)? == 0 {
                writeln!(out)?;
                return Ok(());
            }
            if self.execute(cpu, &line, out)? == Flow::Quit {
                return Ok(());
            }
        }
    }

    /// Run one command line.
    pub fn execute<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<B>,
        line: &str,
        out: &mut dyn Write,
    ) -> io::Result<Flow> {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&cmd, args)) = words.split_first() else {
            return Ok(Flow::Continue);
        };
        if cmd.starts_with('#') {
            return Ok(Flow::Continue);
        }
        if self.source_depth == 0 {
            let text = words.join(" ");
            if let Some(f) = &mut self.histfile {
                writeln!(f, "{text}")?;
            }
            self.commands.push_back(text);
            while self.commands.len() > HISTORY_CAP {
                self.commands.pop_front();
            }
        }
        if let Err(msg) = self.command(cpu, cmd, args, out)? {
            writeln!(out, "erro: {msg}")?;
        }
        let quit = std::mem::take(&mut self.quit_requested);
        Ok(if quit || matches!(cmd, "quit" | "q") {
            Flow::Quit
        } else {
            Flow::Continue
        })
    }

    fn loc(&self, text: Option<&&str>) -> Result<u16, String> {
        let text = text.ok_or("falta o endereço")?;
        self.symbols.resolve(text)
    }

    fn label(&self, addr: u16) -> String {
        match self.symbols.name_at(addr) {
            Some(name) => format!("{name} (${addr:04X})"),
            None => format!("${addr:04X}"),
        }
    }

    fn command<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<B>,
        cmd: &str,
        args: &[&str],
        out: &mut dyn Write,
    ) -> io::Result<Result<(), String>> {
        let count = |i: usize, default: u64| -> Result<u64, String> {
            args.get(i)
                .map_or(Ok(default), |t| self.symbols.resolve(t).map(u64::from))
        };
        macro_rules! tryr {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => return Ok(Err(e)),
                }
            };
        }
        match cmd {
            "help" | "h" => writeln!(
                out,
                "break delete breaks cond ignore when step next finish continue regs mem dis poke watch unwatch watches sym syms load profile history back rc snap diff trace source histfile commands reset quit"
            )?,
            "break" | "b" => {
                let addr = tryr!(self.loc(args.first()));
                let new = self.breakpoints.insert(addr);
                let verb = if new { "breakpoint em" } else { "já existe em" };
                writeln!(out, "{verb} {}", self.label(addr))?;
            }
            "delete" | "bd" => {
                let addr = tryr!(self.loc(args.first()));
                if !self.breakpoints.remove(&addr) {
                    return Ok(Err(format!("sem breakpoint em {}", self.label(addr))));
                }
                self.conditions.remove(&addr);
                self.ignore.remove(&addr);
                self.hits.remove(&addr);
                writeln!(out, "removido {}", self.label(addr))?;
            }
            "breaks" => {
                if self.breakpoints.is_empty() {
                    writeln!(out, "nenhum breakpoint")?;
                }
                for &addr in &self.breakpoints {
                    let mut line = format!("  {}", self.label(addr));
                    if let Some(c) = self.conditions.get(&addr) {
                        line += &format!(" se {}", c.source);
                    }
                    if let Some(n) = self.ignore.get(&addr).filter(|&&n| n > 0) {
                        line += &format!(" ignora {n}");
                    }
                    line += &format!(" passagens {}", self.hits.get(&addr).copied().unwrap_or(0));
                    writeln!(out, "{line}")?;
                }
                if let Some(c) = &self.when {
                    writeln!(out, "  quando {}", c.source)?;
                }
            }
            "cond" => {
                let addr = tryr!(self.loc(args.first()));
                if args.len() == 1 {
                    if self.conditions.remove(&addr).is_none() {
                        return Ok(Err(format!("sem condição em {}", self.label(addr))));
                    }
                    writeln!(out, "condição removida de {}", self.label(addr))?;
                } else {
                    let text = args[1..].join(" ");
                    let c = tryr!(Condition::parse_with(&text, &|n| self.symbols.address_of(n))
                        .map_err(|e| e.to_string()));
                    self.breakpoints.insert(addr);
                    self.conditions.insert(addr, c);
                    writeln!(out, "breakpoint em {} se {text}", self.label(addr))?;
                }
            }
            "ignore" => {
                let addr = tryr!(self.loc(args.first()));
                if !self.breakpoints.contains(&addr) {
                    return Ok(Err(format!("sem breakpoint em {}", self.label(addr))));
                }
                let n = tryr!(args
                    .get(1)
                    .ok_or("falta a contagem".to_string())
                    .and_then(|t| self.symbols.resolve(t)));
                self.ignore.insert(addr, u64::from(n));
                writeln!(out, "ignorando {n} passagens em {}", self.label(addr))?;
            }
            "when" => {
                if args.is_empty() {
                    self.when = None;
                    writeln!(out, "condição global removida")?;
                } else {
                    let text = args.join(" ");
                    let c = tryr!(Condition::parse_with(&text, &|n| self.symbols.address_of(n))
                        .map_err(|e| e.to_string()));
                    self.when = Some(c);
                    writeln!(out, "parar quando {text}")?;
                }
            }
            "step" | "s" => {
                let n = tryr!(count(0, 1));
                let mut ran = 0;
                while ran < n && !cpu.is_halted() {
                    tryr!(self.exec_one(cpu));
                    ran += 1;
                }
                self.status(cpu, out)?;
            }
            "next" | "n" => {
                let depth = cpu.jsr_depth();
                let is_jsr = cpu.bus.peek(cpu.pc) == 0x20;
                tryr!(self.exec_one(cpu));
                let stop = if is_jsr && cpu.jsr_depth() > depth {
                    tryr!(self.run_until(cpu, DEFAULT_RUN, |c| c.jsr_depth() <= depth))
                } else {
                    RunStop::Done
                };
                self.report(cpu, &stop, out)?;
            }
            "finish" | "f" => {
                let depth = cpu.jsr_depth();
                if depth == 0 {
                    return Ok(Err("fora de uma rotina chamada por JSR".into()));
                }
                let stop = tryr!(self.run_until(cpu, DEFAULT_RUN, |c| c.jsr_depth() < depth));
                self.report(cpu, &stop, out)?;
            }
            "continue" | "c" => {
                let n = tryr!(count(0, DEFAULT_RUN));
                let stop = tryr!(self.run_until(cpu, n, |_| false));
                self.report(cpu, &stop, out)?;
            }
            "regs" | "r" => writeln!(out, "{}", cpu.registers())?,
            "mem" | "x" => {
                let start = tryr!(self.loc(args.first()));
                let len = tryr!(count(1, 16)).clamp(1, 0x1_0000);
                for row in 0..len.div_ceil(16) {
                    let base = start.wrapping_add((row * 16) as u16);
                    let n = (len - row * 16).min(16) as u16;
                    let bytes: Vec<u8> = (0..n).map(|i| cpu.bus.peek(base.wrapping_add(i))).collect();
                    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02X}")).collect();
                    let text: String = bytes
                        .iter()
                        .map(|&b| if (0x20..0x7F).contains(&b) { b as char } else { '.' })
                        .collect();
                    writeln!(out, "{base:04X}  {:<47}  |{text}|", hex.join(" "))?;
                }
            }
            "dis" | "d" => {
                let start = match args.first() {
                    Some(_) => tryr!(self.loc(args.first())),
                    None => cpu.pc,
                };
                let n = tryr!(count(1, 8)) as usize;
                let mem: Vec<u8> = (0..=0xFFFFu16).map(|a| cpu.bus.peek(a)).collect();
                let listing = self.symbols.listing(&mem, start, n, cpu.variant);
                write!(out, "{listing}")?;
            }
            "poke" => {
                let addr = tryr!(self.loc(args.first()));
                let value = tryr!(self.loc(args.get(1)));
                let byte = tryr!(u8::try_from(value).map_err(|_| format!("${value:X} não cabe num byte")));
                cpu.bus.write(addr, byte);
                writeln!(out, "{} = ${byte:02X}", self.label(addr))?;
            }
            "watch" | "w" => {
                let text = args.join(" ");
                let (loc, cond) = match text.split_once(" if ") {
                    Some((l, c)) => (l.trim().to_string(), Some(c.trim().to_string())),
                    None => (text.trim().to_string(), None),
                };
                if loc.is_empty() {
                    return Ok(Err("falta o endereço".into()));
                }
                let addr = tryr!(self.symbols.resolve(&loc));
                match &cond {
                    Some(c) => {
                        let parsed = tryr!(Condition::parse_with(c, &|n| self.symbols.address_of(n))
                            .map_err(|e| e.to_string()));
                        self.watch_conds.insert(addr, parsed);
                    }
                    None => {
                        self.watch_conds.remove(&addr);
                    }
                }
                self.watches.insert(addr, cpu.bus.peek(addr));
                match cond {
                    Some(c) => writeln!(out, "vigiando {} se {c}", self.label(addr))?,
                    None => writeln!(out, "vigiando {}", self.label(addr))?,
                }
            }
            "unwatch" => {
                let addr = tryr!(self.loc(args.first()));
                if self.watches.remove(&addr).is_none() {
                    return Ok(Err(format!("{} não estava vigiado", self.label(addr))));
                }
                self.watch_conds.remove(&addr);
                writeln!(out, "sem vigia em {}", self.label(addr))?;
            }
            "watches" => {
                if self.watches.is_empty() {
                    writeln!(out, "nenhuma vigia")?;
                }
                for (&addr, &value) in &self.watches {
                    match self.watch_conds.get(&addr) {
                        Some(c) => writeln!(out, "  {} = ${value:02X} se {}", self.label(addr), c.source)?,
                        None => writeln!(out, "  {} = ${value:02X}", self.label(addr))?,
                    }
                }
            }
            "sym" => {
                let name = tryr!(args.first().ok_or("falta o nome".to_string()));
                let addr = tryr!(self.symbols.resolve(name));
                writeln!(out, "{name} = ${addr:04X}")?;
            }
            "syms" => {
                if self.symbols.is_empty() {
                    writeln!(out, "nenhum símbolo")?;
                }
                write!(out, "{}", self.symbols.to_text())?;
            }
            "load" => {
                let path = tryr!(args.first().ok_or("falta o caminho".to_string()));
                let text = tryr!(std::fs::read_to_string(path).map_err(|e| e.to_string()));
                self.symbols = tryr!(SymbolTable::parse(&text).map_err(|e| e.to_string()));
                writeln!(out, "{} símbolos", self.symbols.len())?;
            }
            "profile" => match args.first().copied() {
                Some("on") => {
                    self.profiler = Some(Profiler::new());
                    writeln!(out, "perfil ligado")?;
                }
                Some("off") => {
                    self.profiler = None;
                    writeln!(out, "perfil desligado")?;
                }
                Some("report") => {
                    let n = tryr!(count(1, 10)) as usize;
                    let profiler = tryr!(self.profiler.as_ref().ok_or("perfil desligado".to_string()));
                    write!(out, "{}", profiler.report(n, Some(&self.symbols)))?;
                }
                _ => return Ok(Err("uso: profile on|off|report [N]".into())),
            },
            "history" => match args.first().copied() {
                None => match &self.rewind {
                    None => writeln!(out, "histórico desligado")?,
                    Some(d) => writeln!(
                        out,
                        "histórico: {}/{} passos, {} bytes",
                        d.history_len(),
                        d.history_limit(),
                        d.history_bytes()
                    )?,
                },
                Some("off") => {
                    self.rewind = None;
                    self.journal.clear();
                    writeln!(out, "histórico desligado")?;
                }
                Some(_) => {
                    let n = tryr!(count(0, 1));
                    if n == 0 {
                        return Ok(Err("o limite do histórico é pelo menos 1".into()));
                    }
                    let d = self.rewind.get_or_insert_with(Debugger::default);
                    d.set_history_limit(n as usize);
                    while self.journal.len() > d.history_len() {
                        self.journal.pop_front();
                    }
                    writeln!(out, "histórico de {n} passos")?;
                }
            },
            "back" | "rs" => {
                let n = tryr!(count(0, 1));
                if self.rewind.is_none() {
                    return Ok(Err("histórico desligado".into()));
                }
                let mut undone = 0u64;
                while undone < n && self.undo_one(cpu) {
                    undone += 1;
                }
                if undone == 0 {
                    return Ok(Err("histórico vazio".into()));
                }
                writeln!(out, "voltou {undone} instruções")?;
                self.status(cpu, out)?;
            }
            "rc" | "reverse-continue" => {
                if self.rewind.as_ref().is_none_or(|d| d.history_len() == 0) {
                    return Ok(Err(if self.rewind.is_none() {
                        "histórico desligado".into()
                    } else {
                        "histórico vazio".into()
                    }));
                }
                let mut undone = 0u64;
                let mut reason = None;
                while self.undo_one(cpu) {
                    undone += 1;
                    let symbols = &self.symbols;
                    let lookup = |n: &str| symbols.address_of(n);
                    if self.breakpoints.contains(&cpu.pc) {
                        let ok = match self.conditions.get(&cpu.pc) {
                            Some(c) => tryr!(c.eval_with(cpu, &lookup).map_err(|e| e.to_string())),
                            None => true,
                        };
                        if ok {
                            if let Some(left) = self.ignore.get_mut(&cpu.pc).filter(|n| **n > 0) {
                                *left -= 1;
                                continue;
                            }
                            reason = Some(format!("breakpoint em {}", self.label(cpu.pc)));
                            break;
                        }
                    }
                    if let Some(c) = &self.when {
                        if tryr!(c.eval_with(cpu, &lookup).map_err(|e| e.to_string())) {
                            reason = Some(format!("condição {} em {}", c.source, self.label(cpu.pc)));
                            break;
                        }
                    }
                }
                let reason = reason.unwrap_or_else(|| "início do histórico".into());
                writeln!(out, "parou: {reason} (voltou {undone} instruções)")?;
                self.status(cpu, out)?;
            }
            "snap" => {
                self.snapshot = Some(crate::debugger::memory_image(&cpu.bus));
                writeln!(out, "memória guardada")?;
            }
            "diff" => {
                let before = tryr!(self.snapshot.as_ref().ok_or("sem snap".to_string()));
                let now = crate::debugger::memory_image(&cpu.bus);
                let changes = crate::debugger::memory_diff(before, &now);
                write!(out, "{}", crate::debugger::format_memory_diff(&changes, Some(&self.symbols)))?;
                if args.first() == Some(&"reset") {
                    self.snapshot = Some(now);
                }
            }
            "trace" => match args.first().copied() {
                None => return Ok(Err("uso: trace PATH|off".into())),
                Some("off") => {
                    let Some(mut t) = self.trace.take() else {
                        return Ok(Err("trace desligado".into()));
                    };
                    t.flush()?;
                    writeln!(out, "trace desligado")?;
                }
                Some(path) => {
                    let file = tryr!(std::fs::File::create(path).map_err(|e| e.to_string()));
                    if let Some(mut old) = self.trace.replace(io::BufWriter::new(file)) {
                        old.flush()?;
                    }
                    writeln!(out, "trace em {path}")?;
                }
            },
            "source" => {
                let path = tryr!(args.first().ok_or("falta o caminho".to_string()));
                if self.source_depth >= SOURCE_DEPTH {
                    return Ok(Err(format!("source aninhado além de {SOURCE_DEPTH}")));
                }
                let text = tryr!(std::fs::read_to_string(path).map_err(|e| e.to_string()));
                self.source_depth += 1;
                let mut result = Ok(());
                for line in text.lines() {
                    match self.execute(cpu, line, out) {
                        Ok(Flow::Quit) => {
                            self.quit_requested = true;
                            break;
                        }
                        Ok(Flow::Continue) => {}
                        Err(e) => {
                            result = Err(e);
                            break;
                        }
                    }
                }
                self.source_depth -= 1;
                result?;
            }
            "histfile" => match args.first().copied() {
                None => return Ok(Err("uso: histfile PATH|off".into())),
                Some("off") => {
                    self.histfile = None;
                    writeln!(out, "histórico de comandos sem arquivo")?;
                }
                Some(path) => {
                    let file = tryr!(std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .map_err(|e| e.to_string()));
                    self.histfile = Some(file);
                    writeln!(out, "comandos em {path}")?;
                }
            },
            "commands" => {
                let n = tryr!(count(0, 0x10)) as usize;
                let skip = self.commands.len().saturating_sub(n);
                for (i, c) in self.commands.iter().enumerate().skip(skip) {
                    writeln!(out, "{:>5}  {c}", i + 1)?;
                }
            }
            "reset" => {
                cpu.reset();
                self.status(cpu, out)?;
            }
            "quit" | "q" => writeln!(out, "fim")?,
            other => return Ok(Err(format!("comando desconhecido: {other}"))),
        }
        Ok(Ok(()))
    }

    /// Undo one recorded instruction and the breakpoint accounting it caused.
    fn undo_one<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> bool {
        let Some(d) = self.rewind.as_mut() else {
            return false;
        };
        if d.step_back(cpu).is_none() {
            return false;
        }
        if let Some(Some((pc, consumed))) = self.journal.pop_back() {
            if let Some(h) = self.hits.get_mut(&pc) {
                *h = h.saturating_sub(1);
            }
            if consumed {
                *self.ignore.entry(pc).or_insert(0) += 1;
            }
        }
        true
    }

    fn exec_one<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> Result<(), String> {
        if self.trace.is_some() {
            let pc = cpu.pc;
            let b = [
                cpu.bus.peek(pc),
                cpu.bus.peek(pc.wrapping_add(1)),
                cpu.bus.peek(pc.wrapping_add(2)),
            ];
            let line = crate::disasm::format_named(b[0], b[1], b[2], pc, cpu.variant, &|a| {
                self.symbols.name_at(a).map(str::to_string)
            });
            let r = cpu.registers();
            let text = format!(
                "{pc:04X}  {:<8}  {:<16} A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} CYC:{}",
                line.bytes_hex, line.text, r.a, r.x, r.y, r.p, r.sp, r.cycles
            );
            if let Some(t) = &mut self.trace {
                writeln!(t, "{text}").map_err(|e| e.to_string())?;
            }
        }
        if let Some(d) = &mut self.rewind {
            d.record(cpu);
            self.journal.push_back(None);
            while self.journal.len() > d.history_len() {
                self.journal.pop_front();
            }
        }
        match &mut self.profiler {
            Some(profiler) => profiler.step(cpu).map(|_| ()),
            None => cpu.step().map(|_| ()),
        }
    }

    fn changed_watch<B: MemoryBus>(&mut self, cpu: &Cpu<B>) -> Option<RunStop> {
        for (&addr, last) in self.watches.iter_mut() {
            let now = cpu.bus.peek(addr);
            if now != *last {
                let old = std::mem::replace(last, now);
                if let Some(c) = self.watch_conds.get(&addr) {
                    let symbols = &self.symbols;
                    // A condition that cannot be evaluated stops, so the error is seen.
                    if !c.eval_with(cpu, &|n| symbols.address_of(n)).unwrap_or(true) {
                        continue;
                    }
                }
                return Some(RunStop::Watch {
                    addr,
                    old,
                    new: now,
                });
            }
        }
        None
    }

    /// Run until `done`, a breakpoint (after the first instruction), a watch, a halt
    /// or `max` instructions.
    fn breakpoint_fires<B: MemoryBus>(&mut self, cpu: &Cpu<B>) -> Result<bool, String> {
        let pc = cpu.pc;
        *self.hits.entry(pc).or_insert(0) += 1;
        if self.rewind.is_some() {
            if let Some(slot) = self.journal.back_mut() {
                *slot = Some((pc, false));
            }
        }
        if let Some(c) = self.conditions.get(&pc) {
            let symbols = &self.symbols;
            if !c
                .eval_with(cpu, &|n| symbols.address_of(n))
                .map_err(|e| e.to_string())?
            {
                return Ok(false);
            }
        }
        if let Some(left) = self.ignore.get_mut(&pc).filter(|n| **n > 0) {
            *left -= 1;
            if self.rewind.is_some() {
                if let Some(slot) = self.journal.back_mut() {
                    *slot = Some((pc, true));
                }
            }
            return Ok(false);
        }
        Ok(true)
    }

    pub fn run_until<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<B>,
        max: u64,
        done: impl Fn(&Cpu<B>) -> bool,
    ) -> Result<RunStop, String> {
        if let Some(stop) = self.changed_watch(cpu) {
            return Ok(stop);
        }
        let mut ran = 0u64;
        loop {
            if done(cpu) {
                return Ok(RunStop::Done);
            }
            if cpu.is_halted() {
                return Ok(RunStop::Halted);
            }
            if ran > 0 && self.breakpoints.contains(&cpu.pc) && self.breakpoint_fires(cpu)? {
                return Ok(RunStop::Breakpoint);
            }
            if ran > 0 {
                if let Some(c) = &self.when {
                    let symbols = &self.symbols;
                    if c.eval_with(cpu, &|n| symbols.address_of(n))
                        .map_err(|e| e.to_string())?
                    {
                        return Ok(RunStop::Condition);
                    }
                }
            }
            if ran == max {
                return Ok(RunStop::Limit);
            }
            self.exec_one(cpu)?;
            ran += 1;
            if let Some(stop) = self.changed_watch(cpu) {
                return Ok(stop);
            }
        }
    }

    fn report<B: MemoryBus>(
        &self,
        cpu: &Cpu<B>,
        stop: &RunStop,
        out: &mut dyn Write,
    ) -> io::Result<()> {
        match stop {
            RunStop::Breakpoint => writeln!(out, "parou: breakpoint em {}", self.label(cpu.pc))?,
            RunStop::Watch { addr, old, new } => writeln!(
                out,
                "parou: {} mudou ${old:02X} -> ${new:02X}",
                self.label(*addr)
            )?,
            RunStop::Halted => writeln!(out, "parou: CPU parada")?,
            RunStop::Limit => writeln!(out, "parou: limite de instruções")?,
            RunStop::Done => writeln!(out, "parou: em {}", self.label(cpu.pc))?,
            RunStop::Condition => writeln!(
                out,
                "parou: condição {} em {}",
                self.when.as_ref().map_or("", |c| c.source.as_str()),
                self.label(cpu.pc)
            )?,
        }
        self.status(cpu, out)
    }

    fn status<B: MemoryBus>(&self, cpu: &Cpu<B>, out: &mut dyn Write) -> io::Result<()> {
        let mem = [
            cpu.bus.peek(cpu.pc),
            cpu.bus.peek(cpu.pc.wrapping_add(1)),
            cpu.bus.peek(cpu.pc.wrapping_add(2)),
        ];
        let line = crate::disasm::format_named(mem[0], mem[1], mem[2], cpu.pc, cpu.variant, &|a| {
            self.symbols.name_at(a).map(str::to_string)
        });
        writeln!(out, "{}", cpu.registers())?;
        writeln!(
            out,
            "==> {:04X}  {:<8}  {}",
            cpu.pc, line.bytes_hex, line.text
        )
    }
}
