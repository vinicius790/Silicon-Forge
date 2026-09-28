//! Cycle profiler: cycles and executions per `PC`, and per routine through `JSR`/`RTS`.
//!
//! [`Profiler::step`] runs one [`Cpu::step`] and charges its cycles to the `PC` it
//! started at. A `JSR` opens a frame for its target; its own six cycles belong to the
//! caller. The `RTS` that closes the frame belongs to the callee. On close, the frame's
//! *inclusive* cycles (everything from the first callee instruction through the `RTS`)
//! are added to the routine, and its *exclusive* cycles are the inclusive ones minus
//! those of the calls it made. Cycles outside any frame go to the top level.
//!
//! An `RTS` with no open frame (a program that pushed its own return address) is
//! counted at its `PC` and closes nothing. Interrupt entries and `RTI` are not frames:
//! a handler's cycles go to whatever routine was running. Frames still open when the
//! report is taken are listed with their cycles so far and not counted as calls.

use std::collections::BTreeMap;

use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::symbols::SymbolTable;

/// Executions and cycles at one address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PcStat {
    pub executions: u64,
    pub cycles: u64,
}

/// One routine, keyed by its entry address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoutineStat {
    /// Completed calls (matched `RTS`).
    pub calls: u64,
    pub inclusive: u64,
    pub exclusive: u64,
    /// Deepest nesting this routine was entered at (1 = called from the top level).
    pub max_depth: usize,
}

#[derive(Debug, Clone)]
struct Frame {
    entry: u16,
    start: u64,
    children: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Profiler {
    per_pc: BTreeMap<u16, PcStat>,
    routines: BTreeMap<u16, RoutineStat>,
    stack: Vec<Frame>,
    total: u64,
    top_level: u64,
    unmatched_rts: u64,
    /// (caller entry or `None` for top level, callee entry) -> calls and inclusive cycles.
    edges: BTreeMap<(Option<u16>, u16), EdgeStat>,
    /// Call stack (entries, outermost first) -> cycles spent with exactly that stack.
    folded: BTreeMap<Vec<u16>, u64>,
}

/// One caller-to-callee edge of the call graph.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeStat {
    /// Completed calls along this edge.
    pub calls: u64,
    /// Inclusive cycles of those calls.
    pub inclusive: u64,
}

impl Profiler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run one instruction and record it. Errors from `Cpu::step` pass through and
    /// record nothing.
    pub fn step<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> Result<u32, String> {
        let pc = cpu.pc;
        let opcode = cpu.bus.peek(pc);
        let cycles = cpu.step()?;
        let spent = u64::from(cycles);
        let stat = self.per_pc.entry(pc).or_default();
        stat.executions += 1;
        stat.cycles += spent;
        let path: Vec<u16> = self.stack.iter().map(|f| f.entry).collect();
        *self.folded.entry(path).or_default() += spent;
        match opcode {
            0x20 => {
                self.charge_outside_frame(spent);
                self.total += spent;
                self.stack.push(Frame {
                    entry: cpu.pc,
                    start: self.total,
                    children: 0,
                });
                let depth = self.stack.len();
                let routine = self.routines.entry(cpu.pc).or_default();
                routine.max_depth = routine.max_depth.max(depth);
            }
            0x60 if !self.stack.is_empty() => {
                self.total += spent;
                self.close_frame();
            }
            _ => {
                if opcode == 0x60 {
                    self.unmatched_rts += 1;
                }
                self.charge_outside_frame(spent);
                self.total += spent;
            }
        }
        Ok(cycles)
    }

    fn charge_outside_frame(&mut self, spent: u64) {
        if self.stack.is_empty() {
            self.top_level += spent;
        }
    }

    fn close_frame(&mut self) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        let inclusive = self.total - frame.start;
        let routine = self.routines.entry(frame.entry).or_default();
        routine.calls += 1;
        routine.inclusive += inclusive;
        routine.exclusive += inclusive - frame.children;
        let caller = self.stack.last().map(|f| f.entry);
        let edge = self.edges.entry((caller, frame.entry)).or_default();
        edge.calls += 1;
        edge.inclusive += inclusive;
        if let Some(parent) = self.stack.last_mut() {
            parent.children += inclusive;
        }
    }

    /// Step until the CPU halts or `max_instructions` ran. Returns instructions run.
    pub fn run<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<B>,
        max_instructions: u64,
    ) -> Result<u64, String> {
        let mut n = 0;
        while n < max_instructions && !cpu.is_halted() {
            self.step(cpu)?;
            n += 1;
        }
        Ok(n)
    }

    /// Completed-call edges `(caller, callee)`; caller `None` is the top level.
    pub fn call_edges(&self) -> Vec<((Option<u16>, u16), EdgeStat)> {
        self.edges.iter().map(|(k, v)| (*k, *v)).collect()
    }

    pub fn edge(&self, caller: Option<u16>, callee: u16) -> EdgeStat {
        self.edges
            .get(&(caller, callee))
            .copied()
            .unwrap_or_default()
    }

    /// Brendan Gregg's folded-stack format, one line per distinct stack, sorted:
    /// `top;outer;inner cycles`, frames named by `label` (or `$XXXX`). The cycles of
    /// every instruction go to the stack open when it started, so a `JSR` counts for
    /// its caller and the `RTS` for its callee, matching the exclusive figures. The
    /// line sum equals [`Profiler::total_cycles`]. Feed it to `flamegraph.pl`.
    pub fn folded(&self, symbols: Option<&SymbolTable>) -> String {
        let name = |pc: u16| {
            symbols
                .and_then(|t| t.name_at(pc))
                .map_or_else(|| format!("${pc:04X}"), str::to_string)
        };
        let mut out = String::new();
        for (path, cycles) in &self.folded {
            if *cycles == 0 {
                continue;
            }
            out.push_str("top");
            for &pc in path {
                out.push(';');
                out.push_str(&name(pc));
            }
            out.push_str(&format!(" {cycles}\n"));
        }
        out
    }

    /// Graphviz `dot` of the call graph: nodes with exclusive/inclusive cycles, edges
    /// labelled with calls and inclusive cycles.
    pub fn call_graph_dot(&self) -> String {
        let mut out = String::from("digraph calls {\n  top [label=\"top\"];\n");
        for (entry, r) in &self.routines {
            out.push_str(&format!(
                "  r{entry:04X} [label=\"${entry:04X}\\nexcl {} incl {}\"];\n",
                r.exclusive, r.inclusive
            ));
        }
        for ((caller, callee), e) in &self.edges {
            let from = caller.map_or_else(|| "top".to_string(), |c| format!("r{c:04X}"));
            out.push_str(&format!(
                "  {from} -> r{callee:04X} [label=\"{}x {}\"];\n",
                e.calls, e.inclusive
            ));
        }
        out.push_str("}\n");
        out
    }

    pub fn total_cycles(&self) -> u64 {
        self.total
    }

    /// Cycles spent outside every routine frame.
    pub fn top_level_cycles(&self) -> u64 {
        self.top_level
    }

    pub fn unmatched_rts(&self) -> u64 {
        self.unmatched_rts
    }

    pub fn pc_stat(&self, pc: u16) -> PcStat {
        self.per_pc.get(&pc).copied().unwrap_or_default()
    }

    pub fn routine(&self, entry: u16) -> Option<RoutineStat> {
        self.routines.get(&entry).copied()
    }

    /// Entry addresses of the frames still open, outermost first.
    pub fn open_frames(&self) -> Vec<u16> {
        self.stack.iter().map(|frame| frame.entry).collect()
    }

    /// The `n` addresses with the most cycles; ties go to the lower address.
    pub fn hot_spots(&self, n: usize) -> Vec<(u16, PcStat)> {
        let mut all: Vec<_> = self.per_pc.iter().map(|(&pc, &stat)| (pc, stat)).collect();
        all.sort_by(|a, b| b.1.cycles.cmp(&a.1.cycles).then(a.0.cmp(&b.0)));
        all.truncate(n);
        all
    }

    /// Routines by exclusive cycles, highest first; ties go to the lower entry.
    pub fn routines_by_exclusive(&self) -> Vec<(u16, RoutineStat)> {
        let mut all: Vec<_> = self
            .routines
            .iter()
            .map(|(&pc, &stat)| (pc, stat))
            .collect();
        all.sort_by(|a, b| b.1.exclusive.cmp(&a.1.exclusive).then(a.0.cmp(&b.0)));
        all
    }

    /// Plain-text report: totals, the `n` hottest addresses and every routine, named
    /// through `symbols` when given. Percentages are of the total cycles, rounded down
    /// to tenths.
    pub fn report(&self, n: usize, symbols: Option<&SymbolTable>) -> String {
        let label = |addr: u16| {
            symbols
                .and_then(|table| table.describe(addr, 0xFF))
                .unwrap_or_else(|| format!("${addr:04X}"))
        };
        let share = |cycles: u64| {
            let tenths = (cycles * 1000).checked_div(self.total).unwrap_or(0);
            format!("{}.{}%", tenths / 10, tenths % 10)
        };
        let mut out = format!(
            "total {} ciclos, nível superior {} ({})\n",
            self.total,
            self.top_level,
            share(self.top_level)
        );
        out.push_str("pontos quentes:\n");
        for (pc, stat) in self.hot_spots(n) {
            out.push_str(&format!(
                "  {:<16} {:>8} ciclos {:>6} {:>8} execuções\n",
                label(pc),
                stat.cycles,
                share(stat.cycles),
                stat.executions
            ));
        }
        out.push_str("rotinas (exclusivo / inclusivo / chamadas):\n");
        for (entry, stat) in self.routines_by_exclusive() {
            out.push_str(&format!(
                "  {:<16} {:>8} {:>8} {:>6}\n",
                label(entry),
                stat.exclusive,
                stat.inclusive,
                stat.calls
            ));
        }
        for frame in &self.stack {
            out.push_str(&format!(
                "  aberta: {} há {} ciclos\n",
                label(frame.entry),
                self.total - frame.start
            ));
        }
        out
    }
}
