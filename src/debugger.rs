//! Interactive and bounded debugger with breakpoint evaluation before instruction fetch.
//!
//! Provides both programmatic bounded execution ([`Debugger::run`]) and an
//! interactive command-line REPL ([`Debugger::interactive_session`]).

use crate::cond::{CondError, Condition};
use crate::{
    bus::MemoryBus,
    cpu::{Cpu, Registers},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint,
    Halted,
    InstructionLimit,
    /// A watched address was read by the instruction that just ran.
    /// The address is [`Debugger::hit_addr`], not the first watch in the set.
    WatchRead,
    /// A watched address was written by the instruction that just ran.
    WatchWrite,
    /// `A` equals the value armed with [`Debugger::break_when_a`], seen before the next fetch.
    Accumulator,
    /// A watched address was written with the value armed by [`Debugger::break_when_written`].
    WatchValue,
    /// `PC` is inside the inclusive range armed by [`Debugger::watch_pc_range`].
    PcRange,
    /// N or Z rose from clear to set during the instruction that just finished.
    FlagSet,
    /// [`Debugger::step_over`] finished on the instruction after the outer `JSR`.
    StepOver,
    /// [`Debugger::step_out`] finished on the instruction after the `RTS`
    /// that returns to the caller's caller.
    StepOut,
    /// Cycle cap hit before the stop this call was waiting for.
    CycleCap,
    /// The global condition armed with [`Debugger::break_when`] became true.
    Condition,
}

/// Status flag whose rising edge [`Debugger::break_when_flag_becomes_set`] watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagEdge {
    Negative,
    Zero,
}

/// Which access a memory watchpoint observes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watch {
    Read,
    Write,
}

/// Inclusive address range watched for [`Watch`] accesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchRange {
    pub start: u16,
    pub end: u16,
    pub kind: Watch,
}

/// One frame of [`Debugger::backtrace`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Address that will run when this frame resumes (current PC, or the return
    /// address on the stack for callers).
    pub pc: u16,
    /// Symbol description within 256 bytes, when a table is given.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Stop {
    pub reason: StopReason,
    pub instructions: u64,
    pub registers: Registers,
}

#[derive(Default)]
pub struct Debugger {
    pub breakpoints: BTreeSet<u16>,
    pub watch_read: BTreeSet<u16>,
    pub watch_write: BTreeSet<u16>,
    /// Address named by the latest read-watch or write-watch stop.
    pub hit_addr: Option<u16>,
    /// Stop before the next instruction when `A` equals this value.
    pub acc_break: Option<u8>,
    /// Stop in [`Debugger::step_watched`] when this address is written with this value.
    pub write_value: Option<(u16, u8)>,
    /// Inclusive PC range. [`Debugger::run`] stops before executing an instruction whose PC is inside.
    pub pc_range: Option<(u16, u16)>,
    /// Extra inclusive PC ranges, armed after `pc_range` and checked only if that one misses.
    pc_ranges: Vec<(u16, u16)>,
    /// Bounds of the range that produced the latest [`StopReason::PcRange`] stop.
    pub hit_range: Option<(u16, u16)>,
    /// Stop after the instruction that changes this flag from clear to set.
    flag_edge: Option<FlagEdge>,
    /// Pre-instruction snapshots, newest at the back. Capped at [`Debugger::history_limit`].
    history: VecDeque<StepSnap>,
    /// Condition attached to a breakpoint: [`Debugger::run`] stops there only when it holds.
    pub conditions: BTreeMap<u16, Condition>,
    /// Times execution reached each breakpoint address (counted before conditions).
    pub hits: BTreeMap<u16, u64>,
    /// Hits to skip at each breakpoint before it stops (GDB `ignore`).
    pub ignore: BTreeMap<u16, u64>,
    /// Global condition checked before every instruction.
    pub watch_condition: Option<Condition>,
    /// History depth; 0 means the default of 32.
    history_limit: usize,
    /// Inclusive address ranges watched for read/write (see [`Debugger::watch_range`]).
    watch_ranges: Vec<WatchRange>,
    /// Address space as `peek` saw it when the newest snapshot was pushed (empty
    /// while the history is empty). Older states are this image with the undo
    /// lists of the newer snapshots applied, so each entry keeps only the bytes
    /// its instruction (or anything else between two pushes) changed.
    mirror: Vec<u8>,
}

/// Why [`Debugger::reverse_continue`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReverseStop {
    /// Rewound onto a breakpoint address.
    Breakpoint,
    /// Rewound onto a state where the global condition holds.
    Condition,
    /// The recorded history ran out (the oldest recorded state is restored).
    HistoryStart,
}

struct StepSnap {
    regs: Registers,
    ram_hash: u64,
    /// `(address, value before)` of every byte that differed between this
    /// snapshot and the next one, filled in when the next one is pushed. Empty on
    /// the newest snapshot, whose pre-state is [`Debugger::mirror`] itself.
    undo: Vec<(u16, u8)>,
    halted: bool,
}

/// 64-bit FNV-1a of `bytes`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// Full address space through `peek`.
fn peek_all<B: MemoryBus>(bus: &B) -> Vec<u8> {
    memory_image(bus)
}

/// The whole 64 KiB address space as `peek` sees it (no read side effects).
pub fn memory_image<B: MemoryBus>(bus: &B) -> Vec<u8> {
    (0..=0xFFFFu16).map(|a| bus.peek(a)).collect()
}

impl Debugger {
    /// Breakpoint at `pc` that stops only when `condition` holds.
    pub fn break_if(&mut self, pc: u16, condition: &str) -> Result<(), CondError> {
        let c = Condition::parse(condition)?;
        self.breakpoints.insert(pc);
        self.conditions.insert(pc, c);
        Ok(())
    }

    /// Stop before any instruction once `condition` holds.
    pub fn break_when(&mut self, condition: &str) -> Result<(), CondError> {
        self.watch_condition = Some(Condition::parse(condition)?);
        Ok(())
    }

    /// Count a hit at the breakpoint under `PC`; true when it should stop
    /// (ignore count used up, condition absent or true).
    fn breakpoint_fires<B: MemoryBus>(&mut self, cpu: &Cpu<B>) -> Result<bool, String> {
        let pc = cpu.pc;
        *self.hits.entry(pc).or_insert(0) += 1;
        if let Some(c) = self.conditions.get(&pc) {
            if !c.eval(cpu).map_err(|e| e.to_string())? {
                return Ok(false);
            }
        }
        if let Some(left) = self.ignore.get_mut(&pc) {
            if *left > 0 {
                *left -= 1;
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Bounded run stopping at breakpoint, halt, or instruction limit.
    pub fn run<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>, limit: u64) -> Result<Stop, String> {
        let mut n = 0;
        let mut hit_range = None;
        let reason = loop {
            if cpu.is_halted() {
                break StopReason::Halted;
            }
            if self.breakpoints.contains(&cpu.pc) && self.breakpoint_fires(cpu)? {
                break StopReason::Breakpoint;
            }
            if self.acc_break.is_some_and(|want| cpu.a == want) {
                break StopReason::Accumulator;
            }
            if let Some(c) = &self.watch_condition {
                if c.eval(cpu).map_err(|e| e.to_string())? {
                    break StopReason::Condition;
                }
            }
            if let Some(bounds) = self.matching_pc_range(cpu.pc) {
                hit_range = Some(bounds);
                break StopReason::PcRange;
            }
            if n == limit {
                break StopReason::InstructionLimit;
            }
            let was = self.flag_edge.map(|edge| flag_is_set(cpu, edge));
            cpu.step()?;
            n += 1;
            if was == Some(false) && self.flag_edge.is_some_and(|edge| flag_is_set(cpu, edge)) {
                break StopReason::FlagSet;
            }
        };
        self.hit_range = hit_range;
        Ok(Stop {
            reason,
            instructions: n,
            registers: cpu.registers(),
        })
    }

    /// Run until a BRK or JAM executes, then stop. The halt instruction itself
    /// runs; the next one does not. On JAM, `PC` stays on that opcode. A loop
    /// with neither stops once `cycle_cap` cycles have been spent in this call
    /// (`StopReason::CycleCap`). A cap of 0 executes nothing and returns `CycleCap`.
    /// A BRK whose cycles finish exactly on `cycle_cap` is [`StopReason::Halted`], not
    /// `CycleCap`.
    /// An armed PC range stops with [`StopReason::PcRange`] before that cap when
    /// the range is reached first; the instruction at that PC does not run.
    pub fn run_until_halt<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<B>,
        cycle_cap: u64,
    ) -> Result<Stop, String> {
        let mut spent = 0u64;
        let mut n = 0u64;
        let mut hit_range = None;
        let reason = loop {
            if cpu.is_halted() {
                break StopReason::Halted;
            }
            if let Some(bounds) = self.matching_pc_range(cpu.pc) {
                hit_range = Some(bounds);
                break StopReason::PcRange;
            }
            if spent >= cycle_cap {
                break StopReason::CycleCap;
            }
            let halt_pc = cpu.pc;
            let opcode = cpu.bus.peek(cpu.pc);
            let cyc = cpu.step()?;
            spent += u64::from(cyc);
            n += 1;
            if is_jam(opcode) {
                cpu.pc = halt_pc;
                break StopReason::Halted;
            }
            if opcode == 0x00 || cpu.is_halted() {
                break StopReason::Halted;
            }
        };
        if hit_range.is_some() {
            self.hit_range = hit_range;
        }
        Ok(Stop {
            reason,
            instructions: n,
            registers: cpu.registers(),
        })
    }

    /// From a `JSR`, run until the matching `RTS` lands on the instruction after
    /// that `JSR`. Nested `JSR`s return to the outer caller. Any other opcode
    /// is one step. `limit` is a cycle cap once inside the call, including the
    /// `JSR` itself. If the callee never returns, the reason is
    /// [`StopReason::CycleCap`], not [`StopReason::StepOver`].
    pub fn step_over<B: MemoryBus>(&self, cpu: &mut Cpu<B>, limit: u64) -> Result<Stop, String> {
        let mut depth = 0u32;
        let mut n = 0u64;
        let mut spent = 0u64;
        let reason = loop {
            if cpu.is_halted() {
                break StopReason::Halted;
            }
            if depth > 0 && spent >= limit {
                break StopReason::CycleCap;
            }
            let opcode = cpu.bus.peek(cpu.pc);
            let cyc = cpu.step()?;
            spent += u64::from(cyc);
            n += 1;
            if opcode == 0x20 {
                depth += 1;
            } else if opcode == 0x60 && depth > 0 {
                depth -= 1;
                if depth == 0 {
                    break StopReason::StepOver;
                }
            }
            if depth == 0 {
                break StopReason::StepOver;
            }
        };
        Ok(Stop {
            reason,
            instructions: n,
            registers: cpu.registers(),
        })
    }

    /// From inside a `JSR` callee, run until the `RTS` that returns to that
    /// caller's caller. Nested calls do not stop at the inner `RTS`.
    /// `limit` is a cycle cap. If the return needs more cycles than `limit`,
    /// the reason is [`StopReason::CycleCap`], not [`StopReason::StepOut`].
    /// If `PC` is not inside a `JSR` callee, returns an error and does not
    /// execute an instruction.
    pub fn step_out<B: MemoryBus>(&self, cpu: &mut Cpu<B>, limit: u64) -> Result<Stop, String> {
        if cpu.jsr_depth() == 0 {
            return Err("step_out: PC fora de um callee de JSR".into());
        }
        let mut depth = 2u32;
        let mut n = 0u64;
        let mut spent = 0u64;
        let reason = loop {
            if cpu.is_halted() {
                break StopReason::Halted;
            }
            if spent >= limit {
                break StopReason::CycleCap;
            }
            let opcode = cpu.bus.peek(cpu.pc);
            let cyc = cpu.step()?;
            spent += u64::from(cyc);
            n += 1;
            if opcode == 0x20 {
                depth += 1;
            } else if opcode == 0x60 {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break StopReason::StepOut;
                }
            }
        };
        Ok(Stop {
            reason,
            instructions: n,
            registers: cpu.registers(),
        })
    }

    /// Interactive REPL on stdin/stdout ([`crate::repl::Repl`]); breakpoints are
    /// shared both ways.
    pub fn interactive_session<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> io::Result<()> {
        let mut repl = crate::repl::Repl::new(crate::symbols::SymbolTable::new());
        repl.breakpoints = std::mem::take(&mut self.breakpoints);
        let result = repl.run(cpu, &mut io::stdin().lock(), &mut io::stdout());
        self.breakpoints = repl.breakpoints;
        result
    }

    /// Arm a read or write watchpoint. The same address may have both.
    pub fn watch(&mut self, addr: u16, kind: Watch) {
        match kind {
            Watch::Read => {
                self.watch_read.insert(addr);
            }
            Watch::Write => {
                self.watch_write.insert(addr);
            }
        }
    }

    pub fn clear_watch(&mut self, addr: u16) {
        self.watch_read.remove(&addr);
        self.watch_write.remove(&addr);
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Stop when `A` equals `value`, checked before the next instruction fetch.
    pub fn break_when_a(&mut self, value: u8) {
        self.acc_break = Some(value);
    }

    /// Arm a breakpoint for a store of `value` to `addr`. Checked by [`Debugger::step_watched`].
    pub fn break_when_written(&mut self, addr: u16, value: u8) {
        self.write_value = Some((addr, value));
    }

    /// Drop the store breakpoint from [`Debugger::break_when_written`].
    /// A later write of that value does not stop.
    pub fn clear_break_when_written(&mut self) {
        self.write_value = None;
    }

    /// Stop in [`Debugger::run`] when `PC` is inside `start..=end` (inclusive) before the fetch.
    /// A PC outside the range does not stop. Replaces `pc_range` and does not drop ranges added
    /// with [`Debugger::add_pc_range`].
    pub fn watch_pc_range(&mut self, start: u16, end: u16) {
        self.pc_range = Some((start, end));
    }

    /// Arm another inclusive PC range without replacing [`Debugger::pc_range`].
    /// Entering this range reports these bounds, not the first range.
    pub fn add_pc_range(&mut self, start: u16, end: u16) {
        self.pc_ranges.push((start, end));
    }

    pub fn clear_pc_range(&mut self) {
        self.pc_range = None;
        self.pc_ranges.clear();
    }

    /// After an instruction, stop if `flag` changed from clear to set.
    /// A flag that was already set does not stop. This is not an `A` or memory watch.
    pub fn break_when_flag_becomes_set(&mut self, flag: FlagEdge) {
        self.flag_edge = Some(flag);
    }

    fn matching_pc_range(&self, pc: u16) -> Option<(u16, u16)> {
        let hit = |(start, end): (u16, u16)| start <= end && pc >= start && pc <= end;
        if self.pc_range.is_some_and(hit) {
            return self.pc_range;
        }
        self.pc_ranges.iter().copied().find(|bounds| hit(*bounds))
    }

    /// FNV-1a of every byte `peek` returns from `$0000` through `$FFFF`.
    ///
    /// The step-back ring stores this checksum instead of a second 64 KiB copy.
    pub fn ram_checksum<B: MemoryBus>(bus: &B) -> u64 {
        fnv1a(&peek_all(bus))
    }

    /// Checksum stored with the newest pre-instruction snapshot, if any.
    pub fn newest_ram_checksum(&self) -> Option<u64> {
        self.history.back().map(|snap| snap.ram_hash)
    }

    fn push_snapshot<B: MemoryBus>(&mut self, cpu: &Cpu<B>) {
        let now = peek_all(&cpu.bus);
        if let Some(newest) = self.history.back_mut() {
            newest.undo = self
                .mirror
                .iter()
                .zip(&now)
                .enumerate()
                .filter(|(_, (old, new))| old != new)
                .map(|(addr, (&old, _))| (addr as u16, old))
                .collect();
        }
        while self.history.len() >= self.history_limit() {
            self.history.pop_front();
        }
        self.history.push_back(StepSnap {
            regs: cpu.registers(),
            ram_hash: fnv1a(&now),
            undo: Vec::new(),
            halted: cpu.is_halted(),
        });
        self.mirror = now;
    }

    /// Bytes held by the step-back history: one 64 KiB image plus 3 bytes per
    /// changed byte per step (the address and the old value), instead of 64 KiB
    /// per step.
    pub fn history_bytes(&self) -> usize {
        self.mirror.len()
            + self
                .history
                .iter()
                .map(|snap| snap.undo.len() * 3)
                .sum::<usize>()
    }

    /// Push the pre-instruction state of `cpu` without running anything; the caller
    /// then executes the instruction its own way (profiler, coverage, ...).
    pub fn record<B: MemoryBus>(&mut self, cpu: &Cpu<B>) {
        self.push_snapshot(cpu);
    }

    /// Drop the whole step-back history.
    pub fn clear_history(&mut self) {
        self.history.clear();
        self.mirror = Vec::new();
    }

    /// Steps of history kept by the recording steppers.
    pub fn history_limit(&self) -> usize {
        if self.history_limit == 0 {
            32
        } else {
            self.history_limit
        }
    }

    /// Keep up to `limit` steps (at least 1); older entries are dropped at once.
    pub fn set_history_limit(&mut self, limit: usize) {
        self.history_limit = limit.max(1);
        while self.history.len() > self.history_limit {
            self.history.pop_front();
        }
    }

    /// Run up to `limit` instructions, recording each for rewind, stopping early at
    /// a halt, a breakpoint (after the first instruction) or when the global
    /// condition holds. Returns the instructions executed.
    pub fn run_recording<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<B>,
        limit: u64,
    ) -> Result<u64, String> {
        for n in 0..limit {
            if cpu.is_halted() {
                return Ok(n);
            }
            if n > 0 && self.breakpoints.contains(&cpu.pc) {
                return Ok(n);
            }
            self.step_recording(cpu)?;
        }
        Ok(limit)
    }

    /// Reverse continue: undo recorded instructions until the restored state sits
    /// on a breakpoint or satisfies the global condition (the state reached at
    /// least one step back), or the history runs out. Returns the reason and how
    /// many instructions were undone.
    pub fn reverse_continue<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<B>,
    ) -> Result<(ReverseStop, u64), String> {
        let mut undone = 0;
        while self.step_back(cpu).is_some() {
            undone += 1;
            if self.breakpoints.contains(&cpu.pc) {
                return Ok((ReverseStop::Breakpoint, undone));
            }
            if let Some(c) = &self.watch_condition {
                if c.eval(cpu).map_err(|e| e.to_string())? {
                    return Ok((ReverseStop::Condition, undone));
                }
            }
        }
        Ok((ReverseStop::HistoryStart, undone))
    }

    /// Undo the newest recorded instruction: registers, cycle count, halt state and
    /// every byte whose `peek` differs from the snapshot (written back with `write`,
    /// so ROM and I/O that ignore or remap writes keep their own state). The stored
    /// checksum is readable with [`Debugger::newest_ram_checksum`] until this pop.
    pub fn step_back<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> Option<Registers> {
        let snap = self.history.pop_back()?;
        for (addr, &old) in self.mirror.iter().enumerate() {
            let addr = addr as u16;
            if cpu.bus.peek(addr) != old {
                cpu.bus.write(addr, old);
            }
        }
        match self.history.back_mut() {
            Some(prev) => {
                for (addr, old) in std::mem::take(&mut prev.undo) {
                    self.mirror[usize::from(addr)] = old;
                }
            }
            None => self.mirror = Vec::new(),
        }
        cpu.set_halted(snap.halted);
        let regs = snap.regs;
        cpu.a = regs.a;
        cpu.x = regs.x;
        cpu.y = regs.y;
        cpu.sp = regs.sp;
        cpu.pc = regs.pc;
        cpu.p = crate::flags::Status(regs.p);
        cpu.cycles = regs.cycles;
        Some(regs)
    }

    /// Record registers and a RAM checksum, then execute one instruction.
    pub fn step_recording<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> Result<(), String> {
        self.push_snapshot(cpu);
        cpu.step()?;
        Ok(())
    }

    /// One instruction on a [`LoggingBus`]. Stops when a watchpoint is touched.
    /// The snapshot pushed is the state *before* the instruction, so [`Debugger::step_back`]
    /// undoes the registers (not the memory write).
    pub fn step_watched<B: MemoryBus>(
        &mut self,
        cpu: &mut Cpu<LoggingBus<B>>,
    ) -> Result<StopReason, String> {
        if cpu.is_halted() {
            return Ok(StopReason::Halted);
        }
        if self.breakpoints.contains(&cpu.pc) {
            return Ok(StopReason::Breakpoint);
        }
        if let Some(bounds) = self.matching_pc_range(cpu.pc) {
            self.hit_range = Some(bounds);
            return Ok(StopReason::PcRange);
        }
        self.push_snapshot(cpu);
        self.hit_addr = None;
        self.hit_range = None;
        cpu.bus.reads.clear();
        cpu.bus.writes.clear();
        cpu.bus.write_values.clear();
        cpu.step()?;
        if let Some(addr) = self
            .watch_read
            .iter()
            .copied()
            .find(|addr| cpu.bus.reads.contains(addr))
            .or_else(|| {
                cpu.bus
                    .reads
                    .iter()
                    .copied()
                    .find(|&a| self.range_hit(a, Watch::Read))
            })
        {
            self.hit_addr = Some(addr);
            return Ok(StopReason::WatchRead);
        }
        if let Some((addr, value)) = self.write_value {
            if cpu
                .bus
                .write_values
                .iter()
                .any(|(wrote, byte)| *wrote == addr && *byte == value)
            {
                return Ok(StopReason::WatchValue);
            }
        }
        if let Some(addr) = self
            .watch_write
            .iter()
            .copied()
            .find(|addr| cpu.bus.writes.contains(addr))
            .or_else(|| {
                cpu.bus
                    .writes
                    .iter()
                    .copied()
                    .find(|&a| self.range_hit(a, Watch::Write))
            })
        {
            let outside = self
                .pc_range
                .is_none_or(|(start, end)| start > end || addr < start || addr > end);
            if outside {
                self.hit_addr = Some(addr);
                return Ok(StopReason::WatchWrite);
            }
        }
        if let Some(bounds) = self.matching_pc_range(cpu.pc) {
            self.hit_range = Some(bounds);
            return Ok(StopReason::PcRange);
        }
        if let Some(addr) = self
            .watch_write
            .iter()
            .copied()
            .find(|addr| cpu.bus.writes.contains(addr))
        {
            self.hit_addr = Some(addr);
            return Ok(StopReason::WatchWrite);
        }
        Ok(StopReason::InstructionLimit)
    }

    /// Watch every address in `start..=end` for `kind` accesses.
    pub fn watch_range(&mut self, start: u16, end: u16, kind: Watch) {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.watch_ranges.push(WatchRange { start, end, kind });
    }

    pub fn clear_watch_ranges(&mut self) {
        self.watch_ranges.clear();
    }

    pub fn watch_ranges(&self) -> &[WatchRange] {
        &self.watch_ranges
    }

    fn range_hit(&self, addr: u16, kind: Watch) -> bool {
        self.watch_ranges
            .iter()
            .any(|r| r.kind == kind && addr >= r.start && addr <= r.end)
    }

    /// Call stack: the current PC, then up to `cpu.jsr_depth()` return addresses
    /// from the hardware stack (PCl at SP+1, PCh at SP+2 for the newest frame).
    pub fn backtrace<B: MemoryBus>(
        &self,
        cpu: &Cpu<B>,
        symbols: Option<&crate::symbols::SymbolTable>,
    ) -> Vec<Frame> {
        let describe = |pc: u16| {
            symbols.and_then(|t| {
                t.describe(pc, 256)
                    .or_else(|| t.name_at(pc).map(str::to_string))
            })
        };
        let mut frames = vec![Frame {
            pc: cpu.pc,
            name: describe(cpu.pc),
        }];
        let mut sp = cpu.sp;
        for _ in 0..cpu.jsr_depth() {
            let lo = cpu.bus.peek(0x100 | u16::from(sp.wrapping_add(1)));
            let hi = cpu.bus.peek(0x100 | u16::from(sp.wrapping_add(2)));
            let pc = u16::from_le_bytes([lo, hi]);
            frames.push(Frame {
                pc,
                name: describe(pc),
            });
            sp = sp.wrapping_add(2);
        }
        frames
    }

    /// Text form of [`Debugger::backtrace`]: `#N $PC name` per frame.
    pub fn format_backtrace(frames: &[Frame]) -> String {
        let mut out = String::new();
        for (i, f) in frames.iter().enumerate() {
            match &f.name {
                Some(n) => out += &format!("#{i} ${:04X} {n}\n", f.pc),
                None => out += &format!("#{i} ${:04X}\n", f.pc),
            }
        }
        out
    }
}

/// Bus wrapper that records every `read` and `write` address. `peek` is silent.
pub struct LoggingBus<B> {
    pub inner: B,
    pub reads: Vec<u16>,
    pub writes: Vec<u16>,
    pub write_values: Vec<(u16, u8)>,
}

impl<B> LoggingBus<B> {
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            reads: Vec::new(),
            writes: Vec::new(),
            write_values: Vec::new(),
        }
    }
}

impl<B: MemoryBus> MemoryBus for LoggingBus<B> {
    fn read(&mut self, addr: u16) -> u8 {
        self.reads.push(addr);
        self.inner.read(addr)
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.writes.push(addr);
        self.write_values.push((addr, value));
        self.inner.write(addr, value);
    }

    fn peek(&self, addr: u16) -> u8 {
        self.inner.peek(addr)
    }

    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        self.inner.phase(phase);
    }
}

fn is_jam(opcode: u8) -> bool {
    matches!(
        opcode,
        0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2
    )
}
fn flag_is_set<B: MemoryBus>(cpu: &Cpu<B>, edge: FlagEdge) -> bool {
    let mask = match edge {
        FlagEdge::Negative => crate::flags::bits::N,
        FlagEdge::Zero => crate::flags::bits::Z,
    };
    cpu.p.get(mask)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatMemory;

    #[test]
    fn breakpoint_before_instruction() {
        let mut bus = FlatMemory::new();
        bus.load_at(0x8000, &[0xea, 0xea, 0x4c, 0x00, 0x80]);
        bus.set_reset_vector(0x8000);
        let mut cpu = Cpu::new(bus);
        cpu.reset();
        let mut d = Debugger::default();
        d.breakpoints.insert(0x8001);
        assert_eq!(d.run(&mut cpu, 10).unwrap().instructions, 1);
        d.breakpoints.clear();
        assert_eq!(
            d.run(&mut cpu, 5).unwrap().reason,
            StopReason::InstructionLimit
        );
    }
}

/// One byte that differs between two memory images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemChange {
    pub addr: u16,
    pub old: u8,
    pub new: u8,
}

/// Every address where `before` and `after` differ, ascending. Only the common
/// length is compared (images from [`memory_image`] are both 64 KiB).
pub fn memory_diff(before: &[u8], after: &[u8]) -> Vec<MemChange> {
    before
        .iter()
        .zip(after)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(addr, (&old, &new))| MemChange {
            addr: addr as u16,
            old,
            new,
        })
        .collect()
}

/// Text form of [`memory_diff`]: changes of consecutive addresses form one run,
/// split every 8 bytes; each run is `  $AAAA name: OO OO -> NN NN`, where `name`
/// is the symbol (or `symbol+n`, within 256 bytes) for the run's first address,
/// omitted when none. A last line `  N bytes mudaram em R faixas` sums it up.
pub fn format_memory_diff(
    changes: &[MemChange],
    symbols: Option<&crate::symbols::SymbolTable>,
) -> String {
    let mut runs: Vec<Vec<MemChange>> = Vec::new();
    for c in changes {
        match runs.last_mut() {
            Some(run)
                if run.len() < 8
                    && u32::from(run.last().unwrap().addr) + 1 == u32::from(c.addr) =>
            {
                run.push(*c)
            }
            _ => runs.push(vec![*c]),
        }
    }
    let mut out = String::new();
    for run in &runs {
        let start = run[0].addr;
        let name = symbols
            .and_then(|t| t.describe(start, 256))
            .map(|n| format!(" {n}"))
            .unwrap_or_default();
        let old: Vec<String> = run.iter().map(|c| format!("{:02X}", c.old)).collect();
        let new: Vec<String> = run.iter().map(|c| format!("{:02X}", c.new)).collect();
        out += &format!(
            "  ${start:04X}{name}: {} -> {}\n",
            old.join(" "),
            new.join(" ")
        );
    }
    out += &format!(
        "  {} bytes mudaram em {} faixas\n",
        changes.len(),
        runs.len()
    );
    out
}
