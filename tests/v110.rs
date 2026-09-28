//! v1.1: unofficial and CMOS cycle traces, reset reads, ACIA runner, assembler
//! listing, echo terminal, debugger watches, disassembler, VIA timer, bus trace.

use mos6502::acia::{run_cycles_with_acia, RamAcia};
use mos6502::asm::assemble_with_symbols;
use mos6502::bus::{FlatMemory, IoDevice, MemoryBus};
use mos6502::cycles::{format_bus_trace, is_bus_cycle_stepped_variant};
use mos6502::debugger::{Debugger, LoggingBus, StopReason, Watch};
use mos6502::disasm::disasm_variant;
use mos6502::flags::bits;
use mos6502::opcodes::{CpuVariant, OpType, CMOS_OVERRIDES, OPCODES_TABLE};
use mos6502::{BusCycle, Cpu, ViaTimer1};

struct Probe {
    mem: FlatMemory,
    reads: Vec<u16>,
    writes: Vec<u16>,
}

impl MemoryBus for Probe {
    fn read(&mut self, addr: u16) -> u8 {
        self.reads.push(addr);
        self.mem.read(addr)
    }
    fn write(&mut self, addr: u16, value: u8) {
        self.writes.push(addr);
        self.mem.write(addr, value);
    }
    fn peek(&self, addr: u16) -> u8 {
        self.mem.peek(addr)
    }
}

fn image(pc: u16, op: u8, operand: [u8; 2], variant: CpuVariant) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    for addr in 0..65536u32 {
        mem.data[addr as usize] = ((addr as u8) ^ 0x5A).wrapping_add((addr >> 8) as u8);
    }
    mem.data[pc as usize] = op;
    mem.data[pc.wrapping_add(1) as usize] = operand[0];
    mem.data[pc.wrapping_add(2) as usize] = operand[1];
    let mut cpu = Cpu::with_variant(mem, variant);
    cpu.pc = pc;
    cpu.a = 0x3C;
    cpu.x = 0x01;
    cpu.y = 0x02;
    cpu.sp = 0xF0;
    cpu.p = mos6502::flags::Status(bits::U | bits::C);
    cpu.cycles = 0;
    cpu
}

fn run_cycles(cpu: &mut Cpu<FlatMemory>) -> Vec<BusCycle> {
    let mut trace = Vec::new();
    for _ in 0..16 {
        let cycle = cpu.step_cycle().expect("opcode should cycle-step");
        let done = cycle.complete;
        trace.push(cycle);
        if done {
            break;
        }
    }
    assert!(trace.last().unwrap().complete);
    trace
}

fn same_arch(op: u8, pc: u16, operand: [u8; 2], variant: CpuVariant) {
    let mut stepped = image(pc, op, operand, variant);
    let mut cycled = image(pc, op, operand, variant);
    let cycles = stepped.step().unwrap();
    let trace = run_cycles(&mut cycled);
    assert_eq!(
        trace.len() as u32,
        cycles,
        "op ${op:02X} @{pc:04X} {variant:?}"
    );
    assert_eq!(cycled.a, stepped.a, "A ${op:02X}");
    assert_eq!(cycled.x, stepped.x, "X ${op:02X}");
    assert_eq!(cycled.y, stepped.y, "Y ${op:02X}");
    assert_eq!(cycled.sp, stepped.sp, "SP ${op:02X}");
    assert_eq!(cycled.pc, stepped.pc, "PC ${op:02X}");
    assert_eq!(cycled.p.raw(), stepped.p.raw(), "P ${op:02X}");
    assert_eq!(cycled.cycles, stepped.cycles, "cycles ${op:02X}");
    assert_eq!(cycled.bus.data, stepped.bus.data, "RAM ${op:02X}");
}

#[test]
fn stable_unofficial_opcodes_match_step_and_exclusions_stay_granular() {
    let mut stable = 0;
    for op in &OPCODES_TABLE {
        if op.op_type == OpType::Official {
            continue;
        }
        if op.mnemonic == "JAM" {
            assert!(mos6502::is_bus_cycle_stepped(op.code));
            let mut cpu = image(0x2000, op.code, [0x10, 0x40], CpuVariant::NMOS);
            let cycle = cpu.step_cycle().unwrap();
            assert!(cycle.complete && !cycle.write);
            assert!(cpu.is_halted());
            assert_eq!(cpu.pc, 0x2000);
            assert_eq!(cpu.cycles, 1);
            continue;
        }
        stable += 1;
        same_arch(op.code, 0x2000, [0x10, 0x40], CpuVariant::NMOS);
        same_arch(op.code, 0x10FE, [0xFF, 0x20], CpuVariant::Ricoh2A03);
    }
    assert!(
        stable >= 87,
        "expected the stable unofficial set, got {stable}"
    );
}

#[test]
fn cmos_opcodes_match_step() {
    for op in &CMOS_OVERRIDES {
        assert!(
            is_bus_cycle_stepped_variant(op.code, CpuVariant::CMOS),
            "${:02X} {}",
            op.code,
            op.mnemonic
        );
        same_arch(op.code, 0x2000, [0x10, 0x40], CpuVariant::CMOS);
        same_arch(op.code, 0x10FE, [0xFF, 0x21], CpuVariant::CMOS);
    }
    // $CB is WAI on CMOS and SBX on NMOS.
    assert_eq!(
        disasm_variant(0xCB, 0x01, 0x00, CpuVariant::CMOS).text,
        "WAI"
    );
    assert_eq!(
        disasm_variant(0xCB, 0x01, 0x00, CpuVariant::NMOS).text,
        "SBX #$01"
    );
}

#[test]
fn reset_is_seven_reads_and_no_writes() {
    let mut mem = FlatMemory::new();
    mem.data[0xFFFC] = 0x00;
    mem.data[0xFFFD] = 0x80;
    let mut cpu = Cpu::new(Probe {
        mem,
        reads: Vec::new(),
        writes: Vec::new(),
    });
    cpu.pc = 0x1234;
    cpu.reset();
    assert_eq!(cpu.bus.reads.len(), 7);
    assert!(cpu.bus.writes.is_empty());
    assert_eq!(&cpu.bus.reads[5..], &[0xFFFC, 0xFFFD]);
    assert_eq!(cpu.pc, 0x8000);
    assert_eq!(cpu.sp, 0xFD);
    assert_eq!(cpu.cycles, 7);
    assert!(cpu.p.get(bits::I));
}

#[test]
fn acia_runner_delivers_a_byte_after_the_baud_period() {
    let mut bus = RamAcia::new(0xA000);
    bus.acia.write(2, 0x0B); // DTR on: the receiver is enabled
    bus.acia.set_baud_period(6);
    bus.memory.load_at(0x8000, &[0xEA, 0xEA, 0xEA, 0xEA]);
    let mut cpu = Cpu::new(bus);
    cpu.pc = 0x8000;
    cpu.cycles = 0;
    cpu.bus.acia.push_rx(0x5A);
    assert_eq!(cpu.bus.acia.peek(1) & mos6502::acia::RDRF, 0);

    // Cpu::step must not tick the countdown.
    let mut alone = RamAcia::new(0xA000);
    alone.acia.write(2, 0x0B); // DTR on: the receiver is enabled
    alone.acia.set_baud_period(6);
    alone.acia.push_rx(0x11);
    let left = alone.acia.rx_cycles_left();
    let _ = alone;
    let mut quiet = Cpu::new({
        let mut b = RamAcia::new(0xA000);
        b.acia.write(2, 0x0B); // DTR on: the receiver is enabled
        b.acia.set_baud_period(6);
        b.memory.data[0x4000] = 0xEA;
        b.acia.push_rx(0x11);
        b
    });
    quiet.pc = 0x4000;
    quiet.step().unwrap();
    assert_eq!(quiet.bus.acia.rx_cycles_left(), left);
    assert_eq!(quiet.bus.acia.peek(1) & mos6502::acia::RDRF, 0);

    run_cycles_with_acia(&mut cpu, 5).unwrap();
    assert_eq!(cpu.bus.acia.peek(1) & mos6502::acia::RDRF, 0);
    run_cycles_with_acia(&mut cpu, 1).unwrap();
    assert_eq!(
        cpu.bus.acia.peek(1) & mos6502::acia::RDRF,
        mos6502::acia::RDRF
    );
    assert_eq!(cpu.bus.acia.read(0), 0x5A);
}

#[test]
fn assembler_expressions_and_listing() {
    let src = "\
.org $8000
val = $1234
.byte lo(val), hi(val), (2+3)*4, 1+2*3, <val+1
.word 2*$10
";
    let out = assemble_with_symbols(src, 0).unwrap();
    assert_eq!(out.bytes, vec![0x34, 0x12, 20, 7, 0x35, 0x20, 0x00]);
    let listing = out.listing();
    assert!(listing.contains("8000:"), "{listing}");
    assert!(listing.contains(".byte"), "{listing}");
    assert!(listing.contains("34 12"), "{listing}");
}

#[test]
fn disassembler_names_unofficial_and_cmos_bytes() {
    let slo = disasm_variant(0x07, 0x10, 0x00, CpuVariant::NMOS);
    assert_eq!(slo.text, "SLO $10");
    let lax = disasm_variant(0xAB, 0x3C, 0x00, CpuVariant::NMOS);
    assert_eq!(lax.text, "LAX #$3C");
    let sax = disasm_variant(0x87, 0x42, 0x00, CpuVariant::NMOS);
    assert_eq!(sax.text, "SAX $42");
    let bra = disasm_variant(0x80, 0xFE, 0x00, CpuVariant::CMOS);
    assert!(bra.text.starts_with("BRA "), "{bra:?}");
    let stz = disasm_variant(0x9C, 0x00, 0x30, CpuVariant::CMOS);
    assert_eq!(stz.text, "STZ $3000");
    let jmp = disasm_variant(0x7C, 0x00, 0x20, CpuVariant::CMOS);
    assert_eq!(jmp.text, "JMP ($2000,X)");
    let trb = disasm_variant(0x14, 0x10, 0x00, CpuVariant::CMOS);
    assert_eq!(trb.text, "TRB $10");
}

#[test]
fn watchpoints_and_step_back_ring() {
    let mut mem = FlatMemory::new();
    // LDA $30 / STA $20 / then 40 NOPs
    mem.load_at(0x8000, &[0xAD, 0x30, 0x00, 0x8D, 0x20, 0x00]);
    mem.data[0x0030] = 0x77;
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch(0x0030, Watch::Read);
    assert_eq!(dbg.step_watched(&mut cpu).unwrap(), StopReason::WatchRead);
    assert_eq!(cpu.a, 0x77);
    let back = dbg.step_back(&mut cpu).unwrap();
    assert_eq!(back.pc, 0x8000);
    assert_eq!(cpu.pc, 0x8000);
    assert_eq!(cpu.a, 0);

    cpu.pc = 0x8003;
    cpu.a = 0x77;
    dbg.watch(0x0020, Watch::Write);
    assert_eq!(dbg.step_watched(&mut cpu).unwrap(), StopReason::WatchWrite);
    assert_eq!(cpu.bus.peek(0x0020), 0x77);

    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xEA; 40]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    for _ in 0..40 {
        dbg.step_recording(&mut cpu).unwrap();
    }
    assert_eq!(dbg.history_len(), 32);
    assert_eq!(cpu.pc, 0x8028);
    let restored = dbg.step_back(&mut cpu).unwrap();
    assert_eq!(restored.pc, 0x8027);
    assert_eq!(cpu.pc, 0x8027);
    assert_eq!(dbg.history_len(), 31);
}

#[test]
fn official_instruction_bus_trace_lists_address_data_and_rw() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x42]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let trace = cpu.trace_bus_instruction().unwrap();
    assert_eq!(trace.len(), 2);
    assert!(!trace[0].write && trace[0].address == 0x8000 && trace[0].data == 0xA9);
    assert!(!trace[1].write && trace[1].address == 0x8001 && trace[1].data == 0x42);
    let text = format_bus_trace(&trace);
    assert_eq!(text, "8000 R A9\n8001 R 42");
    assert_eq!(cpu.a, 0x42);

    // The one-shot timer is covered in via::tests; touch the type so the suite links it.
    let mut timer = ViaTimer1::new();
    timer.write(0x04, 2);
    timer.write(0x05, 0);
    timer.tick();
    assert!(timer.running());
    timer.tick();
    assert!(timer.timed_out());
}
