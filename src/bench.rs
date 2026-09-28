//! Benchmark do emulador: instruções por segundo e clock equivalente, medidos de
//! verdade com `Instant` sobre cargas determinísticas. Não mede banda de memória.
//!
//! Cada [`Workload`] é um laço curto com custo exato conhecido, então
//! [`run_workload`] com um número fixo de instruções dá sempre os mesmos ciclos
//! (testado); só o tempo de parede varia. `Alu` usa `step`, `CycleStepped` a mesma
//! carga via `step_cycle`, `Cmos` a carga em 65C02 com `INC A`/`BRA`.

use crate::bus::FlatMemory;
use crate::cpu::Cpu;
use crate::opcodes::CpuVariant;
use std::fmt::Write as _;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    Alu,
    CycleStepped,
    Cmos,
}

impl Workload {
    pub const ALL: [Workload; 3] = [Workload::Alu, Workload::CycleStepped, Workload::Cmos];

    /// Program at $8000 and the variant it runs on.
    pub fn program(self) -> (&'static [u8], CpuVariant) {
        match self {
            // CLC; LDA #1; ADC $10; STA $10; INX; BNE $8000; JMP $8000
            Workload::Alu | Workload::CycleStepped => (
                &[
                    0x18, 0xA9, 0x01, 0x65, 0x10, 0x85, 0x10, 0xE8, 0xD0, 0xF6, 0x4C, 0x00, 0x80,
                ],
                CpuVariant::NMOS,
            ),
            // INC A; STA $10; INX; BRA $8000
            Workload::Cmos => (&[0x1A, 0x85, 0x10, 0xE8, 0x80, 0xFA], CpuVariant::CMOS),
        }
    }

    fn cpu(self) -> Cpu<FlatMemory> {
        let (code, variant) = self.program();
        let mut mem = FlatMemory::new();
        mem.load_at(0x8000, code);
        mem.set_reset_vector(0x8000);
        let mut cpu = Cpu::with_variant(mem, variant);
        cpu.reset();
        cpu
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BenchmarkResult {
    pub workload: Workload,
    pub instructions: u64,
    pub cycles: u64,
    pub duration_secs: f64,
    pub mips: f64,
    pub equivalent_mhz: f64,
}

impl BenchmarkResult {
    pub fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "carga: {:?}", self.workload);
        let _ = writeln!(out, "  tempo:      {:.4} s", self.duration_secs);
        let _ = writeln!(out, "  instruções: {}", self.instructions);
        let _ = writeln!(out, "  ciclos:     {}", self.cycles);
        let _ = writeln!(
            out,
            "  {:.2} MIPS, {:.2} MHz equivalentes ({:.1}x um 6502 a 1 MHz)",
            self.mips, self.equivalent_mhz, self.equivalent_mhz
        );
        out
    }

    pub fn print_report(&self) {
        print!("{}", self.report());
    }
}

/// Exactly `instructions` instructions of `workload`; returns the cycles they took.
pub fn run_workload(workload: Workload, instructions: u64) -> Result<u64, String> {
    let mut cpu = workload.cpu();
    let start = cpu.cycles;
    for _ in 0..instructions {
        if workload == Workload::CycleStepped {
            while !cpu.step_cycle()?.complete {}
        } else {
            cpu.step()?;
        }
    }
    Ok(cpu.cycles - start)
}

/// Run `workload` in batches until `target_duration_secs` of wall time has passed.
pub fn run_timed(workload: Workload, target_duration_secs: f64) -> Result<BenchmarkResult, String> {
    if !target_duration_secs.is_finite() || target_duration_secs <= 0.0 {
        return Err("A duração do benchmark deve ser um número finito maior que zero.".into());
    }
    let mut cpu = workload.cpu();
    let start = Instant::now();
    let first = cpu.cycles;
    let mut instructions = 0u64;
    while start.elapsed().as_secs_f64() < target_duration_secs {
        for _ in 0..10_000 {
            if workload == Workload::CycleStepped {
                while !cpu.step_cycle()?.complete {}
            } else {
                cpu.step()
                    .map_err(|e| format!("Falha durante o benchmark: {e}"))?;
            }
        }
        instructions += 10_000;
    }
    let elapsed = start.elapsed().as_secs_f64();
    let cycles = cpu.cycles - first;
    Ok(BenchmarkResult {
        workload,
        instructions,
        cycles,
        duration_secs: elapsed,
        mips: instructions as f64 / elapsed / 1e6,
        equivalent_mhz: cycles as f64 / elapsed / 1e6,
    })
}

/// The `Alu` workload, as the `bench` command runs it.
pub fn run_benchmark(target_duration_secs: f64) -> Result<BenchmarkResult, String> {
    run_timed(Workload::Alu, target_duration_secs)
}

// ---------------------------------------------------------------------------
// Suite with a saved baseline and regression thresholds.
// ---------------------------------------------------------------------------

/// Instructions every suite run executes to fingerprint a workload's cycle count.
pub const FINGERPRINT_INSTRUCTIONS: u64 = 10_000;

impl Workload {
    pub fn name(self) -> &'static str {
        match self {
            Workload::Alu => "alu",
            Workload::CycleStepped => "cycle-stepped",
            Workload::Cmos => "cmos",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.name() == name)
    }
}

/// One baseline line: speed and the deterministic cycle fingerprint.
#[derive(Debug, Clone, PartialEq)]
pub struct BaselineEntry {
    pub workload: Workload,
    pub mips: f64,
    /// Cycles of exactly [`FINGERPRINT_INSTRUCTIONS`] instructions.
    pub cycles: u64,
}

/// Measured run of the whole suite.
#[derive(Debug, Clone, PartialEq)]
pub struct SuiteResult {
    pub entries: Vec<BaselineEntry>,
}

/// What failed a comparison against a baseline.
#[derive(Debug, Clone, PartialEq)]
pub enum Regression {
    /// Slower than the baseline by more than the tolerance (percent drop given).
    Slower {
        workload: Workload,
        baseline: f64,
        measured: f64,
        drop_percent: f64,
    },
    /// The cycle fingerprint changed: CPU timing changed, not just speed.
    Cycles {
        workload: Workload,
        baseline: u64,
        measured: u64,
    },
    /// Workload in the baseline that the run did not measure.
    Missing(Workload),
}

impl std::fmt::Display for Regression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Slower {
                workload,
                baseline,
                measured,
                drop_percent,
            } => write!(
                f,
                "{}: {measured:.2} MIPS, base {baseline:.2} (queda de {drop_percent:.1}%)",
                workload.name()
            ),
            Self::Cycles {
                workload,
                baseline,
                measured,
            } => write!(
                f,
                "{}: {measured} ciclos em {FINGERPRINT_INSTRUCTIONS} instruções, base {baseline}",
                workload.name()
            ),
            Self::Missing(w) => write!(f, "{}: não medida", w.name()),
        }
    }
}

/// Measure every workload for `seconds_each` and fingerprint its cycles.
pub fn run_suite(seconds_each: f64) -> Result<SuiteResult, String> {
    let mut entries = Vec::new();
    for w in Workload::ALL {
        let timed = run_timed(w, seconds_each)?;
        entries.push(BaselineEntry {
            workload: w,
            mips: timed.mips,
            cycles: run_workload(w, FINGERPRINT_INSTRUCTIONS)?,
        });
    }
    Ok(SuiteResult { entries })
}

impl SuiteResult {
    /// Baseline text: `# comment` lines, then `name mips cycles` per workload.
    pub fn to_baseline(&self) -> String {
        let mut out = String::from("# mos6502 bench baseline: carga MIPS ciclos/10000\n");
        for e in &self.entries {
            let _ = writeln!(out, "{} {:.3} {}", e.workload.name(), e.mips, e.cycles);
        }
        out
    }

    /// Parse [`SuiteResult::to_baseline`] text. Errors name the 1-based line.
    pub fn parse_baseline(text: &str) -> Result<Self, String> {
        let mut entries: Vec<BaselineEntry> = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let err = |m: &str| format!("linha {}: {m}", i + 1);
            let f: Vec<&str> = line.split_whitespace().collect();
            let [name, mips, cycles] = f[..] else {
                return Err(err("esperado: carga MIPS ciclos"));
            };
            let workload = Workload::from_name(name).ok_or_else(|| err("carga desconhecida"))?;
            let mips: f64 = mips
                .parse()
                .ok()
                .filter(|m: &f64| m.is_finite() && *m > 0.0)
                .ok_or_else(|| err("MIPS inválido"))?;
            let cycles = cycles.parse().map_err(|_| err("ciclos inválidos"))?;
            if entries.iter().any(|e| e.workload == workload) {
                return Err(err("carga repetida"));
            }
            entries.push(BaselineEntry {
                workload,
                mips,
                cycles,
            });
        }
        Ok(Self { entries })
    }

    /// Regressions of `self` against `baseline`: speed lower by more than
    /// `tolerance_percent`, any cycle fingerprint difference, or a missing workload.
    pub fn compare(&self, baseline: &SuiteResult, tolerance_percent: f64) -> Vec<Regression> {
        let mut out = Vec::new();
        for b in &baseline.entries {
            let Some(m) = self.entries.iter().find(|e| e.workload == b.workload) else {
                out.push(Regression::Missing(b.workload));
                continue;
            };
            if m.cycles != b.cycles {
                out.push(Regression::Cycles {
                    workload: b.workload,
                    baseline: b.cycles,
                    measured: m.cycles,
                });
            }
            let drop = (b.mips - m.mips) / b.mips * 100.0;
            if drop > tolerance_percent {
                out.push(Regression::Slower {
                    workload: b.workload,
                    baseline: b.mips,
                    measured: m.mips,
                    drop_percent: drop,
                });
            }
        }
        out
    }

    pub fn report(&self) -> String {
        let mut out = String::new();
        for e in &self.entries {
            let _ = writeln!(
                out,
                "{:<14} {:>9.2} MIPS {:>8} ciclos/{FINGERPRINT_INSTRUCTIONS}",
                e.workload.name(),
                e.mips,
                e.cycles
            );
        }
        out
    }
}
