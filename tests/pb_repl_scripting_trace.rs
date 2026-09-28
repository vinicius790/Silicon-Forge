//! Memory diff, trace-to-file, expression watchpoints with conditions, `source`
//! scripts and the command history file.

use mos6502::asm::assemble_with_symbols;
use mos6502::debugger::{format_memory_diff, memory_diff, MemChange};
use mos6502::repl::{Repl, HISTORY_CAP};
use mos6502::symbols::SymbolTable;
use mos6502::{Cpu, FlatMemory};
use std::path::PathBuf;

const SRC: &str = "start: ldx #0\nloop: inx\n stx table+2\n txa\n sta table,x\n cpx #4\n bne loop\ndone: jmp done\ntable: .byte 0,0,0,0,0,0,0,0\n";

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pb_repl_{}_{name}", std::process::id()));
    let _ = std::fs::remove_file(&dir);
    dir
}

fn session_with(repl: &mut Repl, script: &str) -> (String, Cpu<FlatMemory>) {
    let out = assemble_with_symbols(SRC, 0x0400).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    repl.symbols = SymbolTable::from_asm(&out).unwrap();
    let mut text = Vec::new();
    repl.run(&mut cpu, &mut script.as_bytes(), &mut text)
        .unwrap();
    (String::from_utf8(text).unwrap(), cpu)
}

fn session(script: &str) -> (String, Cpu<FlatMemory>, Repl) {
    let mut repl = Repl::default();
    let (o, c) = session_with(&mut repl, script);
    (o, c, repl)
}

#[test]
fn memory_diff_runs_and_symbols() {
    let mut a = vec![0u8; 0x10000];
    let mut b = a.clone();
    for i in 0..10 {
        b[0x300 + i] = i as u8 + 1;
    }
    b[0xFFFF] = 9;
    let d = memory_diff(&a, &b);
    assert_eq!(d.len(), 11);
    assert_eq!(
        d[0],
        MemChange {
            addr: 0x300,
            old: 0,
            new: 1
        }
    );
    let mut t = SymbolTable::new();
    t.insert("buf", 0x300).unwrap();
    let text = format_memory_diff(&d, Some(&t));
    assert_eq!(
        text,
        "  $0300 buf: 00 00 00 00 00 00 00 00 -> 01 02 03 04 05 06 07 08\n  $0308 buf+8: 00 00 -> 09 0A\n  $FFFF: 00 -> 09\n  11 bytes mudaram em 3 faixas\n"
    );
    a[5] = 1;
    assert_eq!(memory_diff(&a, &a), []);
    assert_eq!(
        format_memory_diff(&[], None),
        "  0 bytes mudaram em 0 faixas\n"
    );
}

#[test]
fn snap_and_diff_in_the_repl() {
    let (out, _, _) = session("diff\nsnap\nc 64\ndiff reset\ndiff\n");
    assert!(out.contains("erro: sem snap"));
    assert!(
        out.contains(
            "  $0412 table+1: 00 00 00 00 -> 01 04 03 04\n  4 bytes mudaram em 1 faixas\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("  0 bytes mudaram em 0 faixas\n"),
        "reset took a new image"
    );
}

#[test]
fn trace_writes_one_line_per_instruction() {
    let path = tmp("trace");
    let p = path.display();
    let (_, _, _) = session(&format!("trace {p}\ns 3\ntrace off\ns 2\n"));
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "{text}");
    assert!(
        lines[0].starts_with("0400  A2 00     LDX #$00"),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].contains("A:00 X:00 Y:00") && lines[0].contains(" CYC:"),
        "{}",
        lines[0]
    );
    assert!(lines[1].starts_with("0402  E8        INX"), "{}", lines[1]);
    assert!(
        lines[2].starts_with("0403  8E 13 04  STX $0413"),
        "{}",
        lines[2]
    );
    let (out, _, _) = session("trace off\ntrace\n");
    assert!(out.contains("erro: trace desligado"));
    assert!(out.contains("erro: uso: trace PATH|off"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn watch_takes_expressions_and_conditions() {
    let (out, cpu, _) = session("w table + 2\nc\n");
    assert!(out.contains("vigiando $0413"), "{out}");
    assert!(out.contains("mudou $00 -> $01"));
    assert_eq!(cpu.x, 1);
    let (out, cpu, _) = session("w table+2 if X == 3\nwatches\nc\n");
    assert!(out.contains("se X == 3"));
    assert_eq!(cpu.x, 3, "changes at X = 1 and 2 are skipped");
    assert!(out.contains("mudou $02 -> $03") || out.contains("-> $03"));
    let (out, _, _) = session("w\nw table if X ==\n");
    assert!(out.contains("erro: falta o endereço"));
    assert_eq!(out.matches("erro:").count(), 2);
}

#[test]
fn source_runs_scripts_nests_and_quits() {
    let inner = tmp("inner");
    let outer = tmp("outer");
    std::fs::write(&inner, "# comment\nb loop\n").unwrap();
    std::fs::write(&outer, format!("source {}\nc\nregs\n", inner.display())).unwrap();
    let (out, cpu, repl) = session(&format!("source {}\n", outer.display()));
    assert!(out.contains("breakpoint em loop"));
    assert!(out.contains("parou: breakpoint em loop"));
    assert_eq!((cpu.x, cpu.pc), (0, 0x0402));
    assert_eq!(repl.commands.len(), 1, "only the typed line is history");
    // quit inside a script ends the session.
    let q = tmp("quit");
    std::fs::write(&q, "quit\nregs\n").unwrap();
    let (out, _, _) = session(&format!("source {}\nregs\n", q.display()));
    assert!(out.contains("fim"));
    assert!(!out.contains("A:"), "nothing after quit: {out}");
    // A script that sources itself stops at the depth limit.
    let selfref = tmp("self");
    std::fs::write(&selfref, format!("source {}\n", selfref.display())).unwrap();
    let (out, _, _) = session(&format!("source {}\n", selfref.display()));
    assert!(out.contains("source aninhado além de 8"));
    let (out, _, _) = session("source /nonexistent/x\nsource\n");
    assert_eq!(out.matches("erro:").count(), 2);
    for f in [inner, outer, q, selfref] {
        let _ = std::fs::remove_file(f);
    }
}

#[test]
fn history_file_and_commands() {
    let path = tmp("hist");
    let (out, _, repl) = session(&format!(
        "histfile {}\nregs\n  s   2\ncommands 3\nhistfile off\nregs\n",
        path.display()
    ));
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "regs\ns 2\ncommands 3\nhistfile off\n");
    assert!(out.contains("    3  s 2"), "{out}");
    assert_eq!(repl.commands.len(), 6);
    // Appends across sessions.
    let _ = session(&format!("histfile {}\nregs\n", path.display()));
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .ends_with("histfile off\nregs\n"));
    let mut repl = Repl::default();
    let script: String = (0..HISTORY_CAP + 5).map(|_| "regs\n").collect();
    session_with(&mut repl, &script);
    assert_eq!(repl.commands.len(), HISTORY_CAP);
    let _ = std::fs::remove_file(path);
}
