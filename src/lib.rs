//! # mos6502 — MOS 6502 / 65C02 emulator and tools
//!
//! Implements the documented NMOS opcode set, common unofficial instructions, CMOS extensions,
//! and the Ricoh 2A03 variant. Instruction-level `Cpu::step` counts cycles but is not a
//! bus trace. `Cpu::step_cycle` traces official NMOS opcodes, the stable unofficial
//! groups, and the CMOS opcodes this core implements ([`cycles::CYCLE_STEPPED`]).
//! `JAM`/`KIL` is a one-cycle halt that does not advance `PC`. The unstable
//! ANE/SHA/SHX/SHY/TAS/LAS group is traced too. `Cpu::step_half_cycle` splits a cycle
//! into PHI1 (address) and PHI2 (data). IRQ is polled on the cycle before the last;
//! `CLI` delays IRQ by one instruction. 65C02 `WAI`/`STP` are implemented.
//! `Cpu::reset` is seven reads. Not a visual6502 netlist.
//!
//! Also: two-pass assembler (macros nest four levels deep, local labels, `.scope`,
//! `.error`/`.warn`, in-memory `.include`, bitwise expressions), relocating linker,
//! disassembler, debugger, localhost debug API,
//! JSON image of registers + 64 KiB RAM (snapshot v2, v3 with a scanline, v4 with palette
//! and name table), 6551 ACIA model (control-register baud nibble selects the period),
//! 6522 with CA1/CB1, CA2/CB2 output modes, T2 pulse edges, PB7 and shift-in, PCM square
//! wave, 32×32 console with an 8×8 blit, a 256-byte name table and a 16-color palette,
//! Apple-1 with an original monitor, educational Mini BASIC.
//! The 2A03 APU records register writes in an [`apu::RegisterLog`]; entry register `$1F`
//! ([`apu::DMC_SAMPLE_REG`]) is a DMC sample byte delivered to the channel, so DMC
//! playback replays exactly.

pub mod acia;
pub mod alu;
pub mod analyzer;
pub mod apu;
pub mod asm;
pub mod audio;
pub mod bcd;
pub mod bench;
pub mod bus;
pub mod cond;
pub mod coverage;
pub mod cpu;
pub mod cycles;
pub mod debug_api;
pub mod debugger;
pub mod disasm;
pub mod escape;
pub mod expr;
pub mod flags;
pub mod gdb;
pub mod graphics;
pub mod hexfmt;
pub mod ines;
pub mod input;
pub mod link;
pub mod mapper;
pub mod nes_state;
pub mod opcodes;
pub mod pia;
pub mod ppu;
pub mod profile;
pub mod profiler;
pub mod repl;
pub mod sid;
pub mod snapshot;
pub mod symbols;
pub mod systems;
pub mod tape;
pub mod tia;
pub mod tui;
pub mod via;

pub use acia::{run_cycles_with_acia, Acia6551, AciaState};
pub use analyzer::{analyze_binary, AnalysisReport};
pub use asm::{
    assemble, assemble_with_blobs, assemble_with_files, assemble_with_symbols, to_intel_hex,
    AsmError, AsmOutput,
};
pub use bus::{FlatMemory, IoDevice, MemoryBus, MemoryMapError, Region, RegionKind, SystemBus};
pub use cpu::{Cpu, Registers, ANE_MAGIC};
pub use cycles::{format_bus_trace, is_bus_cycle_stepped_variant, BusPhase, HalfCycle};
pub use cycles::{is_bus_cycle_stepped, BusCycle, CYCLE_STEPPED};
pub use debugger::{Debugger, Stop, StopReason};
pub use disasm::{disasm, disasm_at, disasm_range, disasm_variant, write_listing, Line};
pub use flags::{bits, Status};
pub use gdb::GdbServer;
pub use opcodes::{CpuVariant, Mode, OpType, Opcode, OPCODES, OPCODES_TABLE};
pub use pia::{Pia6821, Riot6532};
pub use profile::Profile;
pub use sid::Sid6581;
pub use snapshot::{diff_registers, MachineImage, RegChange};
pub use systems::apple1::Apple1;
pub use systems::atari2600::Atari2600;
pub use systems::beneater::BenEaterComputer;
pub use systems::c64::C64Computer;
pub use systems::term::TermComputer;
pub use tia::Tia2600;
pub use tui::TuiDashboard;
pub use via::{Via6522, ViaState, ViaTimer1};
