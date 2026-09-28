//! REPL `cond`, `ignore`, `when` and the `breaks` listing, driven by scripts.

use mos6502::asm::assemble_with_symbols;
use mos6502::repl::Repl;
use mos6502::symbols::SymbolTable;
use mos6502::{Cpu, FlatMemory};

const SRC: &str = "start: ldx #0\nloop: stx counter\n inx\n jmp loop\ncounter = $0200";

fn session(script: &str) -> (String, Cpu<FlatMemory>, Repl) {
    let out = assemble_with_symbols(SRC, 0x8000).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(out.origin, &out.bytes);
    mem.load_at(0xFFFC, &[0x00, 0x80]);
    let mut cpu = Cpu::new(mem);
    cpu.reset();
    let mut repl = Repl::new(SymbolTable::from_asm(&out).unwrap());
    let mut text = Vec::new();
    repl.run(&mut cpu, &mut script.as_bytes(), &mut text)
        .unwrap();
    (String::from_utf8(text).unwrap(), cpu, repl)
}

#[test]
fn cond_stops_only_when_true_and_uses_symbols() {
    for target in [1u8, 7, 0x40, 0xFE] {
        let (out, cpu, repl) = session(&format!("cond loop X == ${target:02X}\ncontinue\nquit\n"));
        assert!(out.contains("parou: breakpoint em loop"), "{out}");
        assert_eq!(cpu.x, target);
        assert_eq!(
            repl.hits[&0x8002],
            u64::from(target) + 1,
            "arrival k sees X = k - 1; every one counts"
        );
    }
    let (_, cpu, _) = session("cond loop [counter] == $10\ncontinue\nquit\n");
    assert_eq!((cpu.x, cpu.bus.data[0x200]), (0x11, 0x10));
}

#[test]
fn ignore_skips_arrivals_and_breaks_lists_everything() {
    let (out, cpu, _) = session("break loop\nignore loop 4\ncontinue\nbreaks\nquit\n");
    assert_eq!(cpu.x, 4, "fifth arrival");
    assert!(out.contains("loop ($8002) passagens 5"), "{out}");
    let (out, _, _) =
        session("cond loop X > 3\nignore loop 2\nwhen [counter] == $33\nbreaks\nquit\n");
    assert!(out.contains("se X > 3 ignora 2 passagens 0"), "{out}");
    assert!(out.contains("quando [counter] == $33"), "{out}");
}

#[test]
fn when_watches_globally_and_can_be_cleared() {
    let (out, cpu, _) = session("when [counter] == $20 && X == $21\ncontinue\nquit\n");
    assert!(
        out.contains("parou: condição [counter] == $20 && X == $21"),
        "{out}"
    );
    assert_eq!(cpu.x, 0x21);
    let (out, cpu, repl) = session("when X == 5\nwhen\ncontinue 30\nquit\n");
    assert!(
        out.contains("condição global removida") && out.contains("limite"),
        "{out}"
    );
    assert!(cpu.x > 5 && repl.when.is_none());
}

#[test]
fn errors_and_cleanup() {
    let (out, _, repl) = session(
        "cond loop X ==\ncond loop\nignore start 1\nbreak loop\nignore loop\nwhen [X\ncond loop X == 1\ncond loop\ndelete loop\nbreaks\nquit\n",
    );
    for needle in [
        "operando ausente",
        "sem condição em loop",
        "sem breakpoint em start",
        "falta a contagem",
        "colchetes",
        "condição removida de loop",
        "nenhum breakpoint",
    ] {
        assert!(out.contains(needle), "{needle}: {out}");
    }
    assert!(repl.conditions.is_empty() && repl.hits.is_empty());
}
