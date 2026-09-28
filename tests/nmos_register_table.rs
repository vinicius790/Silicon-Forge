//! Table-driven check: every documented NMOS opcode ends with the same registers
//! after `Cpu::step` and after `Cpu::step_cycle`.

use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::flags::bits;
use mos6502::opcodes::{documented_opcodes, Mode};
use mos6502::Cpu;

fn operand(mode: Mode) -> [u8; 2] {
    match mode {
        Mode::Imp | Mode::Acc => [0x00, 0x00],
        Mode::Imm | Mode::Zp | Mode::ZpX | Mode::ZpY | Mode::IndX | Mode::IndY | Mode::Rel => {
            [0x10, 0x00]
        }
        Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind | Mode::AbsIndX => [0x34, 0x12],
        Mode::ZpInd | Mode::ZpRel => [0x10, 0x00],
    }
}

fn cpu_at(op: u8, operand: [u8; 2]) -> Cpu<FlatMemory> {
    let pc = 0x2000u16;
    let mut mem = FlatMemory::new();
    for addr in 0..65536u32 {
        mem.data[addr as usize] = (addr as u8).wrapping_mul(3).wrapping_add(0x11);
    }
    mem.data[pc as usize] = op;
    mem.data[pc.wrapping_add(1) as usize] = operand[0];
    mem.data[pc.wrapping_add(2) as usize] = operand[1];
    let mut cpu = Cpu::new(mem);
    cpu.pc = pc;
    cpu.a = 0x40;
    cpu.x = 0x02;
    cpu.y = 0x03;
    cpu.sp = 0xF8;
    cpu.p = mos6502::flags::Status(bits::U);
    cpu.cycles = 0;
    cpu
}

fn finish(cpu: &mut Cpu<FlatMemory>) {
    for _ in 0..8 {
        let cycle = cpu
            .step_cycle()
            .expect("documented opcode should cycle-step");
        if cycle.complete {
            return;
        }
    }
    panic!(
        "step_cycle did not finish opcode ${:02X}",
        cpu.bus.peek(0x2000)
    );
}

#[test]
fn every_documented_nmos_opcode_matches_registers_across_step_and_step_cycle() {
    let ops = documented_opcodes();
    assert_eq!(ops.len(), 151, "the documented NMOS set is 151 opcodes");
    let mut rows = Vec::with_capacity(ops.len());
    for op in ops {
        rows.push((op.code, op.mnemonic, op.mode, operand(op.mode)));
    }
    for (code, mnemonic, mode, operand) in rows {
        let mut stepped = cpu_at(code, operand);
        let mut cycled = cpu_at(code, operand);
        stepped
            .step()
            .unwrap_or_else(|err| panic!("{mnemonic} ${code:02X} {mode:?}: {err}"));
        finish(&mut cycled);
        assert_eq!(cycled.a, stepped.a, "A {mnemonic} ${code:02X}");
        assert_eq!(cycled.x, stepped.x, "X {mnemonic} ${code:02X}");
        assert_eq!(cycled.y, stepped.y, "Y {mnemonic} ${code:02X}");
        assert_eq!(cycled.sp, stepped.sp, "SP {mnemonic} ${code:02X}");
        assert_eq!(cycled.pc, stepped.pc, "PC {mnemonic} ${code:02X}");
        assert_eq!(cycled.p.raw(), stepped.p.raw(), "P {mnemonic} ${code:02X}");
        assert_eq!(
            cycled.cycles, stepped.cycles,
            "cycles {mnemonic} ${code:02X}"
        );
    }
}
