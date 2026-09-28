//! Opcode histogram on top of [`Profiler`]: the profiler keeps per-PC executions and
//! cycles; this adds counts per opcode byte and per mnemonic, for instruction-mix
//! questions ("how many LDA immediates ran").

use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::opcodes::decode_variant;
use crate::profiler::Profiler;

#[derive(Debug, Clone)]
pub struct Profile {
    pub opcode_counts: [u64; 256],
    pub profiler: Profiler,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            opcode_counts: [0; 256],
            profiler: Profiler::default(),
        }
    }
}

impl Profile {
    pub fn new() -> Self {
        Self::default()
    }

    /// One instruction; a halted CPU is not stepped and not counted.
    pub fn step<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>) -> Result<(), String> {
        if cpu.is_halted() {
            return Ok(());
        }
        let op = cpu.bus.peek(cpu.pc);
        self.profiler.step(cpu)?;
        self.opcode_counts[usize::from(op)] += 1;
        Ok(())
    }

    /// Up to `limit` instructions; returns how many ran.
    pub fn run<B: MemoryBus>(&mut self, cpu: &mut Cpu<B>, limit: u64) -> Result<u64, String> {
        let mut n = 0u64;
        while n < limit && !cpu.is_halted() {
            self.step(cpu)?;
            n += 1;
        }
        Ok(n)
    }

    pub fn instructions(&self) -> u64 {
        self.opcode_counts.iter().sum()
    }

    /// Counts summed by mnemonic for `variant`, most frequent first, ties by name.
    pub fn by_mnemonic(&self, variant: crate::opcodes::CpuVariant) -> Vec<(&'static str, u64)> {
        let mut map = std::collections::BTreeMap::new();
        for (op, &count) in self.opcode_counts.iter().enumerate() {
            if count > 0 {
                let name = decode_variant(op as u8, variant).map_or("???", |o| o.mnemonic);
                *map.entry(name).or_insert(0) += count;
            }
        }
        let mut list: Vec<_> = map.into_iter().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        list
    }

    pub fn report(&self) -> String {
        use std::fmt::Write as _;
        let mut out = format!(
            "instruções {} ciclos {}\n",
            self.instructions(),
            self.profiler.total_cycles()
        );
        let mut ops: Vec<(u8, u64)> = (0..=255u8)
            .map(|op| (op, self.opcode_counts[usize::from(op)]))
            .filter(|&(_, c)| c > 0)
            .collect();
        ops.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (op, c) in ops.into_iter().take(16) {
            let _ = writeln!(out, "  op ${op:02X}  {c}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CpuVariant, FlatMemory};

    #[test]
    fn mnemonics_merge_addressing_modes_and_halt_is_not_counted() {
        let mut mem = FlatMemory::new();
        mem.load_at(0x8000, &[0xA9, 0x01, 0xA5, 0x00, 0xA9, 0x02, 0xEA, 0x00]);
        mem.set_reset_vector(0x8000);
        let mut cpu = Cpu::new(mem);
        cpu.halt_on_brk = true;
        cpu.reset();
        let mut p = Profile::new();
        assert_eq!(p.run(&mut cpu, 100).unwrap(), 5);
        assert_eq!(p.run(&mut cpu, 100).unwrap(), 0);
        assert_eq!(p.instructions(), 5);
        assert_eq!(p.by_mnemonic(CpuVariant::NMOS)[0], ("LDA", 3));
        assert_eq!(p.profiler.pc_stat(0x8002).executions, 1);
        assert_eq!(p.profiler.total_cycles(), 2 + 3 + 2 + 2 + 7);
    }
}
