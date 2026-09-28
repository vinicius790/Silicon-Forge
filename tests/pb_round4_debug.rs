//! Data-breakpoint ranges and call-stack backtrace.

use mos6502::asm::{assemble, assemble_with_symbols};
use mos6502::debugger::{Debugger, StopReason, Watch};
use mos6502::symbols::SymbolTable;
use mos6502::{Cpu, FlatMemory};

#[test]
fn watch_range_read_and_write() {
    let mut cpu = Cpu::new(mos6502::debugger::LoggingBus::new(FlatMemory::new()));
    cpu.bus.inner.load_at(
        0x8000,
        &assemble("lda #$AA\n sta $12\n brk\n", 0x8000).unwrap(),
    );
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_range(0x10, 0x13, Watch::Write);
    assert_eq!(
        dbg.step_watched(&mut cpu).unwrap(),
        StopReason::InstructionLimit
    );
    assert_eq!(dbg.step_watched(&mut cpu).unwrap(), StopReason::WatchWrite);
    assert_eq!(dbg.hit_addr, Some(0x12));
}

#[test]
fn backtrace_shows_jsr_chain_with_symbols() {
    let src = "main: jsr mid\n brk\nmid: jsr leaf\n rts\nleaf: nop\n rts\n";
    let out = assemble_with_symbols(src, 0x8000).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let syms = SymbolTable::from_asm(&out).unwrap();
    for _ in 0..3 {
        cpu.step().unwrap();
    }
    assert_eq!(cpu.jsr_depth(), 2);
    let frames = Debugger::default().backtrace(&cpu, Some(&syms));
    assert_eq!(frames.len(), 3);
    assert!(
        frames[0]
            .name
            .as_ref()
            .is_some_and(|n| n.starts_with("leaf")),
        "{:?}",
        frames[0]
    );
    assert!(
        frames[1]
            .name
            .as_ref()
            .is_some_and(|n| n.starts_with("mid")),
        "{:?}",
        frames[1]
    );
    assert!(
        frames[2]
            .name
            .as_ref()
            .is_some_and(|n| n.starts_with("main")),
        "{:?}",
        frames[2]
    );
    let text = Debugger::format_backtrace(&frames);
    assert!(text.contains("#0") && text.contains("leaf"));
}
