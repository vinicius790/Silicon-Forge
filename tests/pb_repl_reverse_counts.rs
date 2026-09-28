//! Reverse commands undo hit counters and given-back `ignore` counts, and `rc`
//! honours `ignore` on reverse arrivals.

use mos6502::asm::assemble_with_symbols;
use mos6502::repl::Repl;
use mos6502::symbols::SymbolTable;
use mos6502::{Cpu, FlatMemory};

const SRC: &str = "start: ldx #0\nloop: inx\nmark: nop\n cpx #8\n bne loop\nend: brk\n";

fn session(script: &str) -> (String, Cpu<FlatMemory>, Repl) {
    let out = assemble_with_symbols(SRC, 0x0400).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    let mut repl = Repl::new(SymbolTable::from_asm(&out).unwrap());
    let mut text = Vec::new();
    repl.run(&mut cpu, &mut script.as_bytes(), &mut text)
        .unwrap();
    (String::from_utf8(text).unwrap(), cpu, repl)
}

#[test]
fn back_undoes_hits_and_restores_ignore() {
    let (_, cpu, repl) = session("history 100\nb mark\nignore mark 2\nc\n");
    assert_eq!(cpu.x, 3);
    assert_eq!(repl.hits[&0x0403], 3);
    assert_eq!(repl.ignore[&0x0403], 0);
    // Undo back to just after the first arrival at mark (x == 1).
    let (_, cpu, repl) = session("history 100\nb mark\nignore mark 2\nc\nback 8\n");
    assert_eq!(cpu.x, 1);
    assert_eq!(cpu.pc, 0x0403);
    assert_eq!(repl.hits[&0x0403], 1);
    assert_eq!(
        repl.ignore[&0x0403], 1,
        "the second arrival's skip is given back"
    );
}

#[test]
fn forward_after_back_reproduces_the_same_stop() {
    let (a, cpu_a, repl_a) = session("history 100\nb mark\nignore mark 2\nc\n");
    let (b, cpu_b, repl_b) = session("history 100\nb mark\nignore mark 2\nc\nback 8\nc\n");
    assert_eq!(cpu_a.registers(), cpu_b.registers());
    assert_eq!(repl_a.hits, repl_b.hits);
    assert_eq!(repl_a.ignore, repl_b.ignore);
    assert!(a.contains("parou: breakpoint em mark"));
    assert_eq!(b.matches("parou: breakpoint em mark").count(), 2);
}

#[test]
fn rc_consumes_ignore_on_reverse_arrivals() {
    // Run to x == 7 at mark without stopping there, then reverse with one skip.
    let (out, cpu, repl) = session("history 100\nc 0d32\nb mark\nignore mark 1\nrc\n");
    assert!(out.contains("parou: breakpoint em mark"));
    let x_after = cpu.x;
    assert_eq!(repl.ignore.get(&0x0403).copied(), Some(0));
    let (_, cpu_plain, _) = session("history 100\nc 0d32\nb mark\nrc\n");
    assert_eq!(x_after + 1, cpu_plain.x, "one reverse arrival was skipped");
}

#[test]
fn history_off_clears_the_journal() {
    let (_, _, repl) = session("history 100\nb mark\nc\nhistory off\nhistory 5\nback\n");
    assert_eq!(
        repl.hits[&0x0403], 1,
        "nothing recorded after re-enabling, nothing undone"
    );
}
