//! Non-JSR step-over, framing error, same-address org, section length.

use mos6502::acia::{programmed_frame, Acia6551, RDRF};
use mos6502::asm::assemble_with_symbols;
use mos6502::bus::FlatMemory;
use mos6502::bus::MemoryBus;
use mos6502::debugger::{Debugger, StopReason};
use mos6502::link::{assemble_object, read_object, write_object};
use mos6502::{Cpu, IoDevice};

#[test]
fn step_over_non_jsr_is_a_single_instruction() {
    let mut mem = FlatMemory::new();
    mem.write(0x8000, 0xA9);
    mem.write(0x8001, 0x42);
    mem.write(0x8002, 0xA9);
    mem.write(0x8003, 0x99);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = Debugger::default().step_over(&mut cpu, 8).unwrap();
    assert_eq!(stop.reason, StopReason::StepOver);
    assert_eq!(stop.instructions, 1);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.pc, 0x8002);
}

#[test]
fn loopback_wrong_stop_sets_framing_and_skips_a_clean_byte() {
    let mut acia = Acia6551::new();
    acia.set_loopback(true);
    acia.write(2, 0x2B);
    acia.write(3, 0x00);
    let good = programmed_frame(0xA5, 0x2B, 0x00);
    acia.loopback_frame_with_stop(0xA5, good.parity, false);
    assert_eq!(acia.peek(1) & 2, 2);
    assert_eq!(acia.peek(1) & RDRF, 0);
    assert_ne!(acia.peek(0), 0xA5);
}

#[test]
fn org_to_the_same_address_is_a_noop() {
    let out = assemble_with_symbols(".org $3000\n.org $3000\nmark: nop\n", 0).unwrap();
    assert_eq!(out.origin, 0x3000);
    assert_eq!(out.symbols["mark"], 0x3000);
    assert_eq!(out.bytes, vec![0xEA]);
}

#[test]
fn section_declared_length_must_match_its_bytes() {
    let obj = assemble_object("m", ".section code\n nop\n").unwrap();
    let mut bytes = write_object(&obj).unwrap();
    let pos = bytes.iter().position(|b| *b == 0xEA).unwrap();
    assert_eq!(&bytes[pos + 1..pos + 3], &[1, 0]);
    bytes[pos + 1] = 2;
    let err = read_object(&bytes).unwrap_err();
    assert!(err.to_string().contains("comprimento da seção"), "{err}");
    assert!(read_object(&write_object(&obj).unwrap()).is_ok());
}
