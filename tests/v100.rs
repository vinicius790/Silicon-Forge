//! v1.0: every documented NMOS opcode's bus trace matches Cpu::step on final state.

use mos6502::bus::FlatMemory;
use mos6502::cycles::is_bus_cycle_stepped;
use mos6502::flags::bits;
use mos6502::opcodes::{OpType, Opcode, OPCODES_TABLE};
use mos6502::{BusCycle, Cpu, CpuVariant};

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
    for _ in 0..12 {
        let cycle = cpu.step_cycle().expect("official opcode should cycle-step");
        let done = cycle.complete;
        trace.push(cycle);
        if done {
            break;
        }
    }
    assert!(trace.last().unwrap().complete, "sequence did not finish");
    trace
}

fn same_arch(op: u8, pc: u16, operand: [u8; 2], variant: CpuVariant, decimal: bool) {
    let mut stepped = image(pc, op, operand, variant);
    let mut cycled = image(pc, op, operand, variant);
    if decimal {
        stepped.p.set(bits::D, true);
        cycled.p.set(bits::D, true);
    }
    let cycles = stepped.step().unwrap();
    let trace = run_cycles(&mut cycled);
    assert_eq!(
        trace.len() as u32,
        cycles,
        "op ${op:02X} @{pc:04X} {variant:?} cycle count"
    );
    assert_eq!(cycled.a, stepped.a, "A ${op:02X}");
    assert_eq!(cycled.x, stepped.x, "X ${op:02X}");
    assert_eq!(cycled.y, stepped.y, "Y ${op:02X}");
    assert_eq!(cycled.sp, stepped.sp, "SP ${op:02X}");
    assert_eq!(cycled.pc, stepped.pc, "PC ${op:02X}");
    assert_eq!(cycled.p.raw(), stepped.p.raw(), "P ${op:02X}");
    assert_eq!(cycled.cycles, stepped.cycles, "cycle counter ${op:02X}");
    assert_eq!(
        cycled.bus.data, stepped.bus.data,
        "RAM ${op:02X} {variant:?}"
    );
    assert!(!cycled.cycle_step_in_progress());
}

#[test]
fn every_official_opcode_matches_step_including_page_cross_and_cmos_jmp() {
    let official: Vec<&Opcode> = OPCODES_TABLE
        .iter()
        .filter(|op| op.op_type == OpType::Official)
        .collect();
    assert_eq!(official.len(), 151);
    for op in &official {
        assert!(
            is_bus_cycle_stepped(op.code),
            "${:02X} {}",
            op.code,
            op.mnemonic
        );
    }
    for op in &OPCODES_TABLE {
        assert!(
            is_bus_cycle_stepped(op.code),
            "${:02X} {} should cycle-step on NMOS",
            op.code,
            op.mnemonic
        );
    }

    let pcs = [0x1234u16, 0x10FEu16];
    let operands = [[0x10, 0x40], [0xFF, 0x20], [0x00, 0x00]];
    for variant in [CpuVariant::NMOS, CpuVariant::CMOS, CpuVariant::Ricoh2A03] {
        for op in &official {
            for &pc in &pcs {
                for operand in operands {
                    same_arch(op.code, pc, operand, variant, false);
                }
            }
            // Decimal ADC/SBC on one shape. Ricoh ignores D inside the ALU.
            if matches!(op.mnemonic, "ADC" | "SBC") {
                same_arch(op.code, 0x2000, [0x10, 0x30], variant, true);
            }
        }
    }
}

#[test]
fn representative_traces_for_modes_added_in_1_0() {
    // TAX is official: 2 cycles, dummy read, X = A.
    let mut cpu = image(0x8000, 0xAA, [0x00, 0x00], CpuVariant::NMOS);
    cpu.a = 0x0F;
    let trace = run_cycles(&mut cpu);
    assert_eq!(trace.len(), 2);
    assert_eq!(cpu.x, 0x0F);
    assert!(!trace.iter().any(|c| c.write));

    // ASL zp: read, write original, write result.
    let mut cpu = image(0x8000, 0x06, [0x10, 0x00], CpuVariant::NMOS);
    cpu.bus.data[0x10] = 0x81;
    let trace = run_cycles(&mut cpu);
    assert_eq!(trace.len(), 5);
    assert!(trace[3].write && trace[3].data == 0x81 && trace[3].address == 0x0010);
    assert!(trace[4].write && trace[4].data == 0x02 && trace[4].complete);
    assert!(cpu.p.get(bits::C));

    // ASL abs,X is 7 cycles even without a page cross.
    let mut cpu = image(0x8000, 0x1E, [0x00, 0x30], CpuVariant::NMOS);
    cpu.x = 1;
    cpu.bus.data[0x3001] = 0x01;
    let trace = run_cycles(&mut cpu);
    assert_eq!(trace.len(), 7);
    assert!(trace.iter().all(|c| !c.page_cross_penalty));
    assert_eq!(cpu.bus.data[0x3001], 0x02);

    // LDA abs,Y page cross.
    let mut cpu = image(0x8000, 0xB9, [0xFF, 0x20], CpuVariant::NMOS);
    cpu.y = 1;
    cpu.bus.data[0x2100] = 0x5A;
    let trace = run_cycles(&mut cpu);
    assert_eq!(trace.len(), 5);
    assert!(trace.iter().any(|c| c.page_cross_penalty && !c.write));
    assert_eq!(cpu.a, 0x5A);

    // STA (zp),Y is 6 cycles; the fix-up read is a penalty only when the page changes.
    let mut cpu = image(0x8000, 0x91, [0x20, 0x00], CpuVariant::NMOS);
    cpu.y = 1;
    cpu.a = 0x77;
    cpu.bus.data[0x20] = 0x10;
    cpu.bus.data[0x21] = 0x40;
    let trace = run_cycles(&mut cpu);
    assert_eq!(trace.len(), 6);
    assert!(trace.iter().all(|c| !c.page_cross_penalty));
    assert!(trace.last().unwrap().write);
    assert_eq!(cpu.bus.data[0x4011], 0x77);

    // NMOS JMP ($xxFF) page-wrap bug, 5 cycles.
    let mut cpu = image(0x8000, 0x6C, [0xFF, 0x30], CpuVariant::NMOS);
    cpu.bus.data[0x30FF] = 0x34;
    cpu.bus.data[0x3000] = 0x12;
    cpu.bus.data[0x3100] = 0x99;
    let trace = run_cycles(&mut cpu);
    assert_eq!(trace.len(), 5);
    assert_eq!(cpu.pc, 0x1234);

    // CMOS JMP indirect does not wrap and takes 6 cycles.
    let mut cpu = image(0x8000, 0x6C, [0xFF, 0x30], CpuVariant::CMOS);
    cpu.bus.data[0x30FF] = 0x34;
    cpu.bus.data[0x3000] = 0x12;
    cpu.bus.data[0x3100] = 0x99;
    let trace = run_cycles(&mut cpu);
    assert_eq!(trace.len(), 6);
    assert_eq!(cpu.pc, 0x9934);

    // RTI pulls P, then PC, and does not add one (unlike RTS).
    let mut cpu = image(0x4000, 0x40, [0x00, 0x00], CpuVariant::NMOS);
    cpu.sp = 0xF0;
    let pushed_p = bits::U | bits::C | bits::Z;
    cpu.bus.data[0x01F1] = pushed_p;
    cpu.bus.data[0x01F2] = 0x00;
    cpu.bus.data[0x01F3] = 0x90;
    let trace = run_cycles(&mut cpu);
    assert_eq!(trace.len(), 6);
    assert_eq!(cpu.pc, 0x9000);
    assert_eq!(cpu.sp, 0xF3);
    assert!(cpu.p.get(bits::Z));
    assert!(!cpu.p.get(bits::B));

    // JAM is a one-cycle halt: PC stays put and the CPU stops.
    let mut cpu = image(0x8000, 0x02, [0x00, 0x00], CpuVariant::NMOS);
    let before = cpu.pc;
    let cycle = cpu.step_cycle().unwrap();
    assert!(cycle.complete);
    assert!(!cycle.write);
    assert_eq!(cpu.pc, before);
    assert!(cpu.is_halted());
    assert_eq!(cpu.cycles, 1);
    assert!(cpu.step_cycle().is_err());
}
