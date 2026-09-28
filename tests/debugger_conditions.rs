//! Conditional breakpoints, hit/ignore counts and global conditions.

use mos6502::assemble;
use mos6502::cond::{CondError, Condition, View};
use mos6502::debugger::{Debugger, StopReason};
use mos6502::{Cpu, FlatMemory};

fn view(a: u8, x: u8, p: u8) -> View {
    View {
        a,
        x,
        y: 0x33,
        sp: 0xFD,
        p,
        pc: 0x1234,
        cycles: 0x1_0005,
    }
}

#[test]
fn every_comparison_over_every_accumulator_value() {
    type Op = (&'static str, fn(u8, u8) -> bool);
    let ops: [Op; 6] = [
        ("==", |a, b| a == b),
        ("!=", |a, b| a != b),
        ("<", |a, b| a < b),
        ("<=", |a, b| a <= b),
        (">", |a, b| a > b),
        (">=", |a, b| a >= b),
    ];
    for (op, f) in ops {
        let c = Condition::parse(&format!("A {op} $80")).unwrap();
        for a in 0..=255u8 {
            assert_eq!(
                c.eval_view(&view(a, 0, 0), &|_| 0).unwrap(),
                f(a, 0x80),
                "A {op} $80 with {a:02X}"
            );
        }
    }
}

#[test]
fn flags_registers_and_cycles() {
    let names = ["C", "Z", "I", "D", "B", "", "V", "N"];
    for p in 0..=255u8 {
        for (bit, name) in names.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            let c = Condition::parse(name).unwrap();
            assert_eq!(
                c.eval_view(&view(0, 0, p), &|_| 0).unwrap(),
                p & (1 << bit) != 0,
                "{name} p={p:02X}"
            );
        }
    }
    let v = view(1, 2, 0);
    for (src, want) in [
        ("y == $33", true),
        ("SP == $FD && PC == $1234", true),
        ("cyc == 5", true),
        ("x+a == 3", true),
        ("A == $0A", false),
        ("A == 1 && X == 3 || Y == $33", true),
        ("A == 2 || X == 3", false),
        ("<PC == $34 && >PC == $12", true),
        ("(A << 4) > $F", true),
        ("0", false),
        ("X", true),
    ] {
        assert_eq!(
            Condition::parse(src)
                .unwrap()
                .eval_view(&v, &|_| 0)
                .unwrap(),
            want,
            "{src}"
        );
    }
}

#[test]
fn memory_reads_nest_and_do_not_disturb() {
    let mem = |a: u16| match a {
        0x10 => 0x20,
        0x21 => 0x99,
        0x1234 => 7,
        _ => 0,
    };
    let v = view(0x99, 0, 0);
    for (src, want) in [
        ("[$10] == $20", true),
        ("[[$10]+1] == A", true),
        ("[PC] == 7", true),
        ("[[[$10]]] == 0", true),
    ] {
        assert_eq!(
            Condition::parse(src).unwrap().eval_view(&v, &mem).unwrap(),
            want,
            "{src}"
        );
    }
}

#[test]
fn typed_parse_and_eval_errors() {
    assert_eq!(Condition::parse("  "), Err(CondError::Empty));
    assert_eq!(Condition::parse("[A"), Err(CondError::Brackets(2)));
    assert_eq!(Condition::parse("A]"), Err(CondError::Brackets(1)));
    assert!(matches!(
        Condition::parse("A =="),
        Err(CondError::MissingOperand(_))
    ));
    assert!(matches!(
        Condition::parse("== 1"),
        Err(CondError::MissingOperand(_))
    ));
    assert!(matches!(
        Condition::parse("A && "),
        Err(CondError::MissingOperand(_))
    ));
    assert!(matches!(
        Condition::parse("A == foo"),
        Err(CondError::Eval(_))
    ));
    // Division by zero depends on state: parses, fails only when it happens.
    let c = Condition::parse("1 / X == 1").unwrap();
    assert!(c.eval_view(&view(0, 1, 0), &|_| 0).unwrap());
    assert!(matches!(
        c.eval_view(&view(0, 0, 0), &|_| 0),
        Err(CondError::Eval(_))
    ));
}

fn machine() -> Cpu<FlatMemory> {
    // X counts up; $0200 records X each pass.
    let src = "ldx #0\nloop: stx $0200\n inx\n jmp loop";
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &assemble(src, 0x0400).unwrap());
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    cpu
}

#[test]
fn conditional_breakpoint_counts_hits_and_honours_ignore() {
    let mut cpu = machine();
    let mut dbg = Debugger::default();
    dbg.break_if(0x0402, "X == $10").unwrap();
    let stop = dbg.run(&mut cpu, 10_000).unwrap();
    assert_eq!((stop.reason, cpu.x), (StopReason::Breakpoint, 0x10));
    assert_eq!(dbg.hits[&0x0402], 0x11, "every arrival counts");
    // Plain breakpoint with ignore 5: stops on the sixth arrival.
    let mut cpu = machine();
    let mut dbg = Debugger::default();
    dbg.breakpoints.insert(0x0405);
    dbg.ignore.insert(0x0405, 5);
    let stop = dbg.run(&mut cpu, 10_000).unwrap();
    assert_eq!(
        (stop.reason, cpu.x),
        (StopReason::Breakpoint, 5),
        "sixth arrival at INX sees X = 5"
    );
    assert_eq!(dbg.ignore[&0x0405], 0);
    assert!(dbg.break_if(0x0400, "A ==").is_err());
    assert!(
        !dbg.breakpoints.contains(&0x0400),
        "a rejected condition arms nothing"
    );
}

#[test]
fn global_condition_watches_memory() {
    let mut cpu = machine();
    let mut dbg = Debugger::default();
    dbg.break_when("[$0200] == $2A && X == $2B").unwrap();
    let stop = dbg.run(&mut cpu, 10_000).unwrap();
    assert_eq!(stop.reason, StopReason::Condition);
    assert_eq!((cpu.x, cpu.bus.data[0x200], cpu.pc), (0x2B, 0x2A, 0x0406));
    let mut dbg = Debugger::default();
    dbg.break_when("1 / [$0300] == 0").unwrap();
    assert!(
        dbg.run(&mut machine(), 10).is_err(),
        "evaluation errors surface"
    );
}
