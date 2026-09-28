//! v5.0: fuller 6522, VIA IRQ board, assembler control directives, debugger
//! checksum and A breakpoint, scanline wait, JAM halt, snapshot version 2.

use std::collections::HashMap;

use mos6502::asm::{assemble, assemble_with_blobs, assemble_with_files};
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::debugger::{Debugger, StopReason};
use mos6502::graphics::FantasyConsoleBus;
use mos6502::opcodes::CpuVariant;
use mos6502::systems::via_board::ViaComputer;
use mos6502::{is_bus_cycle_stepped, Cpu};

#[test]
fn assembler_rept_if_incbin_and_align() {
    let repeated = assemble(".rept 3\n  nop\n.endr\n", 0).unwrap();
    assert_eq!(repeated, [0xEA, 0xEA, 0xEA]);

    let skipped = assemble(
        ".if 0\n  lda #$01\n.else\n  lda #$02\n.endif\n.if 1\n  nop\n.endif\n.if 0d2-0d2\n  brk\n.endif\n",
        0,
    )
    .unwrap();
    assert_eq!(skipped, [0xA9, 0x02, 0xEA]);

    let aligned = assemble("  nop\n  .align 4\n  lda #$01\n", 0).unwrap();
    assert_eq!(aligned, [0xEA, 0x00, 0x00, 0x00, 0xA9, 0x01]);

    let mut files = HashMap::new();
    files.insert("glyph.bin".into(), "Hi".into());
    let from_text = assemble_with_files("  .incbin \"glyph.bin\"\n", 0x0200, &files).unwrap();
    assert_eq!(from_text.bytes, b"Hi");

    let mut blobs = HashMap::new();
    blobs.insert("sprite.bin".into(), vec![0x00, 0xFF, 0x10]);
    let from_blob = assemble_with_blobs(
        "  .incbin \"sprite.bin\"\n  nop\n",
        0,
        &HashMap::new(),
        &blobs,
    )
    .unwrap();
    assert_eq!(from_blob.bytes, [0x00, 0xFF, 0x10, 0xEA]);
}

#[test]
fn step_back_checksum_sees_a_store_and_break_on_a() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x11, 0x8D, 0x00, 0x02, 0xEA]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(mem);
    cpu.reset();
    let mut dbg = Debugger::default();
    dbg.step_recording(&mut cpu).unwrap();
    let before_store = dbg.newest_ram_checksum().unwrap();
    dbg.step_recording(&mut cpu).unwrap();
    let stored = dbg.newest_ram_checksum().unwrap();
    let now = Debugger::ram_checksum(&cpu.bus);
    assert_eq!(stored, before_store);
    assert_ne!(now, stored);
    assert_eq!(cpu.bus.peek(0x0200), 0x11);
    let restored = dbg.step_back(&mut cpu).unwrap();
    assert_eq!(restored.pc, 0x8002);
    assert_eq!(
        cpu.bus.peek(0x0200),
        0x00,
        "step-back also undoes the store"
    );
    assert_eq!(Debugger::ram_checksum(&cpu.bus), stored);

    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x42, 0xEA]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(mem);
    cpu.reset();
    let mut dbg = Debugger::default();
    dbg.break_when_a(0x42);
    let stop = dbg.run(&mut cpu, 10).unwrap();
    assert_eq!(stop.reason, StopReason::Accumulator);
    assert_eq!(stop.registers.a, 0x42);
    assert_eq!(stop.instructions, 1);
}

#[test]
fn program_waits_until_scanline_zero() {
    let program = assemble("wait:\n  lda $00FD\n  bne wait\n  brk\n", 0x8000).unwrap();
    let mut bus = FantasyConsoleBus::new();
    bus.load_at(0x8000, &program);
    bus.set_reset_vector(0x8000);
    bus.set_scanline(30);
    let mut cpu = Cpu::new(bus);
    cpu.halt_on_brk = true;
    cpu.reset();
    for _ in 0..40 {
        if cpu.is_halted() {
            break;
        }
        cpu.step().unwrap();
        if !cpu.is_halted() {
            cpu.bus.tick_scanline();
        }
    }
    assert!(cpu.is_halted());
    assert_eq!(cpu.a, 0);
    assert!(cpu.bus.scanline() < 32);
}

#[test]
fn jam_halts_without_moving_pc_and_unstable_opcodes_stay_out() {
    let mut mem = FlatMemory::new();
    mem.data[0x4000] = 0x02;
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x4000;
    let cycle = cpu.step_cycle().unwrap();
    assert!(cycle.complete);
    assert_eq!(cycle.address, 0x4000);
    assert_eq!(cycle.data, 0x02);
    assert_eq!(cpu.pc, 0x4000);
    assert!(cpu.is_halted());
    assert!(cpu.step_cycle().unwrap_err().contains("halted"));

    for op in 0..=255u8 {
        assert!(is_bus_cycle_stepped(op), "${op:02X}");
    }

    let mut cmos = Cpu::with_variant(FlatMemory::new(), CpuVariant::CMOS);
    cmos.bus.data[0x1000] = 0x12;
    cmos.bus.data[0x0010] = 0x00;
    cmos.bus.data[0x0011] = 0x20;
    cmos.bus.data[0x2000] = 0x01;
    cmos.pc = 0x1000;
    let trace_done = {
        let mut done = false;
        for _ in 0..8 {
            let cycle = cmos.step_cycle().unwrap();
            done = cycle.complete;
            if done {
                break;
            }
        }
        done
    };
    assert!(trace_done);
    assert!(!cmos.is_halted());
    assert_ne!(cmos.pc, 0x1000);
}

#[test]
fn via_timer_irq_toggles_port_a_bit() {
    let mut machine = ViaComputer::new().unwrap();
    assert_eq!(machine.port_a() & 1, 0);
    let mut toggled = false;
    for _ in 0..2000 {
        machine.run_cycles(1).unwrap();
        if machine.port_a() & 1 == 1 {
            toggled = true;
            break;
        }
    }
    assert!(toggled, "port A bit 0 never flipped");
}
