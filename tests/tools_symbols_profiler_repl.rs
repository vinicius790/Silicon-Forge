//! Symbol files, profiler, coverage map and the scripted REPL.

use mos6502::coverage::{Coverage, Mark};
use mos6502::link::{assemble_object, link};
use mos6502::profiler::Profiler;
use mos6502::repl::Repl;
use mos6502::symbols::{SymbolErrorKind, SymbolTable};
use mos6502::{assemble_with_symbols, Cpu, CpuVariant, FlatMemory};

const LOOP: &str = "
counter = $10
.org $8000
start:
    ldx #$00
loop:
    jsr sub
    inx
    cpx #$04
    bne loop
    brk
sub:
    inc counter
    rts
";

fn machine(src: &str) -> (Cpu<FlatMemory>, SymbolTable) {
    let out = assemble_with_symbols(src, 0x8000).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(out.origin, &out.bytes);
    mem.load_at(0xFFFC, &[0x00, 0x80]);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();
    (cpu, SymbolTable::from_asm(&out).unwrap())
}

#[test]
fn assembler_export_round_trips_and_names_operands() {
    let out = assemble_with_symbols(LOOP, 0x8000).unwrap();
    let table = SymbolTable::parse(&out.to_symbol_table()).unwrap();
    assert_eq!(table, SymbolTable::from_asm(&out).unwrap());
    assert_eq!(table.to_text(), out.to_symbol_table());
    assert_eq!(table.address_of("sub"), Some(0x800B));
    assert_eq!(table.resolve("sub+2").unwrap(), 0x800D);
    assert_eq!(
        table.resolve("8000").unwrap(),
        0x8000,
        "bare numbers are hex"
    );
    assert!(table.resolve("nowhere").is_err());

    let (cpu, _) = machine(LOOP);
    let mem = &cpu.bus.data[..];
    assert_eq!(
        table.disasm_at(mem, 0x8002, CpuVariant::NMOS).text,
        "JSR sub"
    );
    assert_eq!(
        table.disasm_at(mem, 0x8008, CpuVariant::NMOS).text,
        "BNE loop"
    );
    assert_eq!(
        table.disasm_at(mem, 0x800B, CpuVariant::NMOS).text,
        "INC counter"
    );
    assert_eq!(
        table.disasm_at(mem, 0x8006, CpuVariant::NMOS).text,
        "CPX #$04",
        "immediates stay numbers"
    );
    assert_eq!(
        table.listing(mem, 0x8000, 2, CpuVariant::NMOS),
        "start:\n  8000  A2 00     LDX #$00\nloop:\n  8002  20 0B 80  JSR sub\n"
    );
}

#[test]
fn symbol_file_errors_carry_the_line() {
    let err = SymbolTable::parse("; header\nok = $10\nbad line\n").unwrap_err();
    assert_eq!((err.line, err.kind), (3, SymbolErrorKind::MissingEquals));
    let err = SymbolTable::parse("x = 1234\n").unwrap_err();
    assert_eq!(err.kind, SymbolErrorKind::BadAddress("1234".into()));
    let err = SymbolTable::parse("x = $12345\n").unwrap_err();
    assert_eq!(err.kind, SymbolErrorKind::BadAddress("$12345".into()));
    let err = SymbolTable::parse("9x = $10\n").unwrap_err();
    assert_eq!(err.kind, SymbolErrorKind::BadName("9x".into()));
    let err = SymbolTable::parse("a = $10\na = $11\n").unwrap_err();
    assert_eq!(
        (err.line, err.kind),
        (
            2,
            SymbolErrorKind::Conflict {
                name: "a".into(),
                first: 0x10,
                second: 0x11
            }
        )
    );
    assert_eq!(SymbolTable::parse("a = $10\na = $0010\n").unwrap().len(), 1);
    assert_eq!(err_text("q\n"), "linha 1: falta '=' entre nome e endereço");
}

fn err_text(text: &str) -> String {
    SymbolTable::parse(text).unwrap_err().to_string()
}

#[test]
fn linker_symbols_load_into_the_same_table() {
    let code = assemble_object(
        "c",
        ".section code\n.org $9000\n.global entry\nentry:\n  rts\n",
    )
    .unwrap();
    let linked = link(&[code], 0).unwrap();
    let table = SymbolTable::from_linked(&linked).unwrap();
    assert_eq!(table.address_of("entry"), Some(0x9000));
    assert_eq!(SymbolTable::parse(&table.to_text()).unwrap(), table);
}

#[test]
fn profiler_splits_inclusive_and_exclusive_through_nested_calls() {
    let (mut cpu, table) = machine(
        "
.org $8000
main:
    jsr sub
    jsr sub
    brk
sub:
    jsr leaf
    rts
leaf:
    nop
    rts
",
    );
    let mut profiler = Profiler::new();
    profiler.run(&mut cpu, 100).unwrap();
    let sub = profiler.routine(table.address_of("sub").unwrap()).unwrap();
    let leaf = profiler.routine(table.address_of("leaf").unwrap()).unwrap();
    assert_eq!(
        (leaf.calls, leaf.inclusive, leaf.exclusive, leaf.max_depth),
        (2, 16, 16, 2)
    );
    assert_eq!(
        (sub.calls, sub.inclusive, sub.exclusive, sub.max_depth),
        (2, 40, 24, 1)
    );
    assert_eq!(profiler.top_level_cycles(), 19);
    assert_eq!(profiler.total_cycles(), 59);
    assert_eq!(
        profiler.total_cycles(),
        cpu.cycles - 7,
        "reset cycles excluded"
    );
    let hot: Vec<u16> = profiler.hot_spots(3).iter().map(|(pc, _)| *pc).collect();
    assert_eq!(
        hot,
        [0x8007, 0x800A, 0x800C],
        "12-cycle ties in address order"
    );
    let report = profiler.report(2, Some(&table));
    assert!(
        report.starts_with("total 59 ciclos, nível superior 19 (32.2%)\n"),
        "{report}"
    );
    assert!(
        report.contains("  sub                    24       40      2\n"),
        "{report}"
    );
    assert!(
        report.contains("  sub              ") && report.contains("20.3%"),
        "{report}"
    );
}

#[test]
fn profiler_reports_unmatched_rts_and_open_frames() {
    let (mut cpu, _) = machine(
        "
.org $8000
    lda #$80
    pha
    lda #$06
    pha
    rts
    nop
    jsr stay
stay:
    jmp stay
",
    );
    let mut profiler = Profiler::new();
    profiler.run(&mut cpu, 10).unwrap();
    assert_eq!(profiler.unmatched_rts(), 1);
    assert_eq!(profiler.open_frames(), [0x800B]);
    assert!(profiler.report(1, None).contains("aberta: $800B"));
}

#[test]
fn coverage_marks_opcodes_operands_and_gaps() {
    let (mut cpu, _) = machine(
        "
.org $8000
    lda #$01
    beq skip
    brk
skip:
    nop
",
    );
    let mut coverage = Coverage::new();
    for _ in 0..3 {
        coverage.step(&mut cpu).unwrap();
    }
    assert_eq!(coverage.mark(0x8000), Mark::Opcode);
    assert_eq!(coverage.mark(0x8001), Mark::Operand);
    assert_eq!(coverage.mark(0x8005), Mark::Never, "skip never reached");
    assert_eq!(coverage.gaps(0x8000, 0x8005), [(0x8005, 0x8005)]);
    assert_eq!(coverage.count(0x8000, 0x8005), 5);
    assert_eq!(coverage.per_mille(0x8000, 0x8005), 833);
    assert_eq!(coverage.render(0x8000, 0x8005), "8000 OoOoO.\n");
    let mut other = Coverage::new();
    let (mut cpu2, _) = machine(".org $8000\n nop\n nop\n nop\n nop\n nop\n nop\n");
    for _ in 0..6 {
        other.step(&mut cpu2).unwrap();
    }
    coverage.merge(&other);
    assert_eq!(
        coverage.mark(0x8001),
        Mark::Opcode,
        "opcode wins over operand"
    );
    assert!(coverage.gaps(0x8000, 0x8005).is_empty());
}

#[test]
fn scripted_repl_session() {
    let (mut cpu, table) = machine(LOOP);
    let mut repl = Repl::new(table);
    let script =
        "sym sub\nnext\nnext\nbreak sub\ncontinue\nfinish\nwatch counter\ncontinue\ncontinue\n\
mem counter 1\ndelete sub\nunwatch counter\nprofile on\ncontinue\nprofile report 1\nbogus\n\
finish\ndis start 2\nwatches\nquit\nregs\n";
    let mut out = Vec::new();
    repl.run(&mut cpu, &mut script.as_bytes(), &mut out)
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    let expect = [
        "sub = $800B",
        "parou: em loop ($8002)",
        "parou: em $8005",
        "breakpoint em sub ($800B)",
        "parou: breakpoint em sub ($800B)",
        "==> 800B  E6 10     INC counter",
        "parou: em $8005",
        "vigiando counter ($0010)",
        "parou: breakpoint em sub ($800B)",
        "parou: counter ($0010) mudou $02 -> $03",
        "0010  03 ",
        "removido sub ($800B)",
        "sem vigia em counter ($0010)",
        "perfil ligado",
        "parou: CPU parada",
        "  sub                    11       11      1",
        "erro: comando desconhecido: bogus",
        "erro: fora de uma rotina chamada por JSR",
        "start:\n  8000  A2 00     LDX #$00\nloop:\n  8002  20 0B 80  JSR sub\n",
        "nenhuma vigia",
        "fim",
    ];
    let mut from = 0;
    for piece in expect {
        let at = text[from..]
            .find(piece)
            .unwrap_or_else(|| panic!("{piece:?} after {from} in\n{text}"));
        from += at + piece.len();
    }
    assert!(!text[from..].contains("A="), "nothing runs after quit");
    assert_eq!(cpu.bus.data[0x10], 4);
}
