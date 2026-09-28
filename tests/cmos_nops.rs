//! Undefined 65C02 opcodes are NOPs with the documented byte and cycle counts, the same
//! in instruction stepping and bus-cycle stepping.

use mos6502::opcodes::{cmos_nop, decode_variant, Mode};
use mos6502::{Cpu, CpuVariant, FlatMemory};

/// (opcode, bytes, cycles), from the 65C02 data sheet.
const EXPECTED: &[(u8, u16, u32)] = &[
    (0x03, 1, 1),
    (0x0B, 1, 1),
    (0xBB, 1, 1),
    (0xF3, 1, 1),
    (0x02, 2, 2),
    (0x22, 2, 2),
    (0xE2, 2, 2),
    (0x44, 2, 3),
    (0x54, 2, 4),
    (0xD4, 2, 4),
    (0xF4, 2, 4),
    (0xDC, 3, 4),
    (0xFC, 3, 4),
    (0x5C, 3, 8),
];

fn cpu_with(op: u8) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &[op, 0x30, 0x12, 0xEA]);
    let mut cpu = Cpu::with_variant(mem, CpuVariant::CMOS);
    cpu.pc = 0x0400;
    cpu.a = 0x11;
    cpu.x = 0xF0;
    cpu.y = 0x22;
    cpu
}

#[test]
fn table_covers_exactly_the_undefined_bytes() {
    let count = (0..=255u8).filter(|&c| cmos_nop(c).is_some()).count();
    assert_eq!(count, 16 + 14 + 7 + 1 + 3 + 3);
    for c in [
        0xCB, 0xDB, 0x12, 0x80, 0x89, 0x1A, 0x64, 0x7C, 0xEA, 0x07, 0xFF,
    ] {
        assert!(cmos_nop(c).is_none(), "{c:02X} is defined");
    }
    assert_eq!(
        decode_variant(0xA7, CpuVariant::CMOS).unwrap().mnemonic,
        "SMB2"
    );
    assert_eq!(
        decode_variant(0xA3, CpuVariant::CMOS).unwrap().mnemonic,
        "NOP"
    );
    assert_eq!(
        decode_variant(0xA7, CpuVariant::NMOS).unwrap().mnemonic,
        "LAX"
    );
    assert_eq!(cmos_nop(0x54).unwrap().mode, Mode::ZpX);
}

#[test]
fn step_matches_length_cycles_and_leaves_state() {
    for &(op, len, cycles) in EXPECTED {
        let mut cpu = cpu_with(op);
        let p = cpu.p;
        assert_eq!(cpu.step().unwrap(), cycles, "{op:02X}");
        assert_eq!(cpu.pc, 0x0400 + len, "{op:02X}");
        assert_eq!(
            (cpu.a, cpu.x, cpu.y, cpu.p, cpu.sp),
            (0x11, 0xF0, 0x22, p, cpu_with(op).sp)
        );
    }
}

#[test]
fn bus_cycles_agree_and_read_the_documented_addresses() {
    for &(op, len, cycles) in EXPECTED {
        let mut cpu = cpu_with(op);
        let mut trace = Vec::new();
        loop {
            let c = cpu.step_cycle().unwrap();
            assert!(!c.write, "{op:02X} never writes");
            trace.push(c.address);
            if c.complete {
                break;
            }
        }
        assert_eq!(trace.len() as u32, cycles, "{op:02X}");
        assert_eq!(cpu.pc, 0x0400 + len);
        let tail = &trace[len as usize..];
        match op {
            0x44 => assert_eq!(tail, [0x0030]),
            0x54 | 0xD4 | 0xF4 => assert_eq!(tail, [0x0030, 0x0020], "zp+X wraps"),
            0xDC | 0xFC => assert_eq!(tail, [0x1230]),
            0x5C => assert_eq!(tail, [0x1230; 5]),
            _ => assert!(tail.is_empty()),
        }
    }
}
