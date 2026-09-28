//! Halt run, 6551 loopback parity, `.org`, and truncated objects.

use mos6502::acia::{programmed_frame, Acia6551, RDRF};
use mos6502::asm::{assemble, assemble_with_symbols};
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::debugger::{Debugger, StopReason};
use mos6502::link::{assemble_object, read_object, write_object};
use mos6502::{Cpu, IoDevice};

#[test]
fn run_until_halt_stops_on_brk_jam_or_cycle_cap() {
    let mut mem = FlatMemory::new();
    mem.write(0x8000, 0xA9);
    mem.write(0x8001, 0x11);
    mem.write(0x8002, 0x00);
    mem.write(0x9000, 0xA9);
    mem.write(0x9001, 0x22);
    mem.write(0xFFFE, 0x00);
    mem.write(0xFFFF, 0x90);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = Debugger::default().run_until_halt(&mut cpu, 30).unwrap();
    assert_eq!(stop.reason, StopReason::Halted);
    assert_eq!(cpu.a, 0x11, "handler at the BRK vector must not run");
    assert_eq!(cpu.pc, 0x9000);
    assert_eq!(stop.instructions, 2);

    let mut mem = FlatMemory::new();
    mem.write(0x8000, 0xA9);
    mem.write(0x8001, 0x11);
    mem.write(0x8002, 0x02);
    mem.write(0x8003, 0xA9);
    mem.write(0x8004, 0x22);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = Debugger::default().run_until_halt(&mut cpu, 30).unwrap();
    assert_eq!(stop.reason, StopReason::Halted);
    assert!(cpu.is_halted());
    assert_eq!(cpu.a, 0x11, "the byte after JAM must not run");
    assert_eq!(stop.instructions, 2);

    let mut mem = FlatMemory::new();
    mem.write(0x8000, 0x4C);
    mem.write(0x8001, 0x00);
    mem.write(0x8002, 0x80);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = Debugger::default().run_until_halt(&mut cpu, 5).unwrap();
    assert_eq!(stop.reason, StopReason::CycleCap);
    assert!(!cpu.is_halted());
    assert_eq!(cpu.pc, 0x8000);
    assert!(stop.instructions >= 1 && stop.instructions <= 3);
    assert!(cpu.cycles <= 8);
}

#[test]
fn loopback_wrong_parity_sets_the_bit_and_skips_the_receiver() {
    let mut acia = Acia6551::new();
    acia.set_loopback(true);
    acia.write(2, 0x2B);
    acia.write(3, 0x00);
    let good = programmed_frame(0xA5, 0x2B, 0x00);
    let wrong = good.parity.map(|bit| !bit);
    acia.loopback_frame(0xA5, wrong);
    assert_eq!(acia.peek(1) & 1, 1);
    assert_eq!(acia.peek(1) & RDRF, 0);
    assert_ne!(acia.peek(0), 0xA5);

    acia.loopback_frame(0xA5, good.parity);
    assert_eq!(acia.peek(1) & 1, 0);
    assert_eq!(acia.read(0), 0xA5);
}

#[test]
fn org_sets_the_counter_fills_a_gap_and_rejects_going_back() {
    let out =
        assemble_with_symbols(".org $4000\n.byte $AB\n.org $4003\nmark: .byte $CD\n", 0).unwrap();
    assert_eq!(out.origin, 0x4000);
    assert_eq!(out.symbols["mark"], 0x4003);
    assert_eq!(out.bytes[0x4003 - 0x4000], 0xCD);
    assert_eq!(out.bytes[1], 0);
    assert_eq!(out.bytes[2], 0);
    let err = assemble(".org $4000\n.byte $01\n.org $4000\n.byte $02\n", 0).unwrap_err();
    assert!(err.to_string().contains("org para trás"), "{err}");
}

#[test]
fn truncated_object_is_rejected_before_any_section() {
    let err = read_object(b"M65O\x02").unwrap_err();
    assert!(err.to_string().contains("truncado"), "{err}");

    let obj = assemble_object("m", ".section code\n nop\n nop\n nop\n nop\n").unwrap();
    let full = write_object(&obj).unwrap();
    let err = read_object(&full[..full.len() - 1]).unwrap_err();
    assert!(err.to_string().contains("truncado"), "{err}");
    assert!(read_object(&full).is_ok());
}
