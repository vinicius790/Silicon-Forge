//! The self-test written in the project's assembler passes on the NMOS core and
//! stops at a known test number when the core or the program is wrong.

use mos6502::{assemble_with_symbols, Cpu, CpuVariant, FlatMemory};

/// Runs until `done` or a trap (a `JMP` to itself). Returns (PC, test number, done).
fn run(variant: CpuVariant, patch: Option<(u16, u8)>) -> (u16, u8, u16) {
    let out = assemble_with_symbols(include_str!("../examples/selftest.asm"), 0x8000).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(out.origin, &out.bytes);
    if let Some((addr, byte)) = patch {
        mem.data[usize::from(addr)] = byte;
    }
    let mut cpu = Cpu::with_variant(mem, variant);
    cpu.reset();
    let done = out.symbols["done"];
    for _ in 0..10_000 {
        let pc = cpu.pc;
        cpu.step().unwrap();
        if pc == done || cpu.pc == pc {
            break;
        }
    }
    (cpu.pc, cpu.bus.data[0x0200], done)
}

#[test]
fn nmos_core_passes_every_test() {
    let (pc, test, done) = run(CpuVariant::NMOS, None);
    assert_eq!((pc, test), (done, 0));
}

#[test]
fn a_cmos_core_fails_the_lax_test() {
    let (pc, test, done) = run(CpuVariant::CMOS, None);
    assert_ne!(pc, done);
    assert_eq!(test, 0x13, "$A7 is SMB2 on the 65C02, A stays 0");
}

#[test]
fn a_ricoh_core_fails_the_decimal_test() {
    let (pc, test, done) = run(CpuVariant::Ricoh2A03, None);
    assert_ne!(pc, done);
    assert_eq!(test, 5, "no decimal mode on the 2A03");
}

#[test]
fn a_broken_program_byte_is_caught_by_its_test() {
    let out = assemble_with_symbols(include_str!("../examples/selftest.asm"), 0x8000).unwrap();
    let line = out
        .lines
        .iter()
        .position(|l| l.source.trim() == "adc #$01")
        .unwrap();
    let addr = out.lines[line].address;
    let (_, test, _) = run(CpuVariant::NMOS, Some((addr + 1, 0x02)));
    assert_eq!(test, 2, "ADC #$02 breaks the overflow test");
}
