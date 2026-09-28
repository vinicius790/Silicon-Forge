//! REPL `history`, `back` and `rc` (reverse continue).

use mos6502::asm::assemble_with_symbols;
use mos6502::bus::MemoryBus;
use mos6502::repl::Repl;
use mos6502::symbols::SymbolTable;
use mos6502::{Cpu, FlatMemory};

const SRC: &str = "start: ldx #0\nloop: inx\n stx $10\nmark: nop\n cpx #5\n bne loop\nend: brk\n";

fn session(script: &str) -> (String, Cpu<FlatMemory>) {
    let out = assemble_with_symbols(SRC, 0x0400).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    let mut repl = Repl::new(SymbolTable::from_asm(&out).unwrap());
    let mut text = Vec::new();
    repl.run(&mut cpu, &mut script.as_bytes(), &mut text)
        .unwrap();
    (String::from_utf8(text).unwrap(), cpu)
}

#[test]
fn history_is_off_until_enabled() {
    let (out, _) = session("history\nback\nrc\nhistory 0\n");
    assert!(out.contains("histórico desligado"));
    assert_eq!(out.matches("erro: histórico desligado").count(), 2);
    assert!(out.contains("erro: o limite do histórico é pelo menos 1"));
}

#[test]
fn back_undoes_registers_and_memory() {
    let (out, cpu) = session("history 40\ns 4\nback 3\nhistory\n");
    assert!(
        out.contains("histórico de 64 passos"),
        "bare numbers are hex"
    );
    assert!(out.contains("voltou 3 instruções"));
    assert!(out.contains("histórico: 1/64 passos"));
    assert_eq!(cpu.pc, 0x0402);
    assert_eq!(cpu.x, 0);
    assert_eq!(cpu.bus.peek(0x10), 0);
}

#[test]
fn reverse_continue_stops_on_breakpoint_and_when() {
    let (out, cpu) = session("history 100\nc 0d14\nb mark\nrc\n");
    assert!(out.contains("parou: breakpoint em mark ($0405) (voltou"));
    assert_eq!(cpu.pc, 0x0405);
    let x_at_mark = cpu.x;
    let (out, cpu) = session("history 100\nc 0d30\nwhen X == 2\nrc\n");
    assert!(out.contains("parou: condição X == 2"));
    assert_eq!(cpu.x, 2);
    assert!(x_at_mark >= 1);
}

#[test]
fn reverse_continue_respects_cond_and_runs_out() {
    let (out, cpu) = session("history 100\nc 0d40\ncond mark X == 1\nrc\n");
    assert!(out.contains("parou: breakpoint em mark"));
    assert_eq!(cpu.x, 1);
    let (out, cpu) = session("history 100\ns 5\nrc\nrc\n");
    assert!(out.contains("parou: início do histórico (voltou 5 instruções)"));
    assert!(out.contains("erro: histórico vazio"));
    assert_eq!(cpu.pc, 0x0400);
}

#[test]
fn history_limit_caps_depth_and_off_drops_it() {
    let (out, _) = session("history 3\ns 0a\nrc\nhistory off\nback\n");
    assert!(out.contains("voltou 3 instruções"));
    assert!(out.contains("erro: histórico desligado"));
}
