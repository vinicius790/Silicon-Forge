//! Cpu::run_until / run_to_pc / run_to_return.

use mos6502::assemble;
use mos6502::cpu::RunEnd;
use mos6502::{Cpu, FlatMemory};

const SRC: &str = "
 ldx #0
main: jsr outer
 inx
 cpx #3
 bne main
 brk
outer: jsr inner
 jsr inner
 rts
inner: iny
 rts";

fn cpu() -> Cpu<FlatMemory> {
    let bytes = assemble(SRC, 0x0300).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(0x0300, &bytes);
    mem.load_at(0xFFFC, &[0x00, 0x03]);
    let mut c = Cpu::new(mem);
    c.halt_on_brk = true;
    c.reset();
    c
}

#[test]
fn predicate_halt_and_budget() {
    let mut c = cpu();
    let before = c.cycles;
    assert_eq!(
        c.run_until(1000, |_| true).unwrap(),
        (RunEnd::Stopped, 0),
        "checked first"
    );
    let (end, used) = c.run_until(10_000, |c| c.y == 4).unwrap();
    assert_eq!((end, c.y), (RunEnd::Stopped, 4));
    assert_eq!(used, c.cycles - before);
    let (end, _) = c.run_until(100_000, |_| false).unwrap();
    assert_eq!((end, c.x, c.y), (RunEnd::Halted, 3, 6));
    // Every budget from 0 to 60: stops at the first boundary at or past it.
    for budget in 0..60u64 {
        let mut c = cpu();
        let (end, used) = c.run_until(budget, |_| false).unwrap();
        assert_eq!(end, RunEnd::CycleBudget);
        assert!(used >= budget && used < budget + 7, "{budget} {used}");
    }
}

#[test]
fn run_to_pc_and_return() {
    let mut c = cpu();
    let inner = 0x0300 + 2 + 3 + 1 + 2 + 2 + 1 + 3 + 3 + 1;
    assert_eq!(c.run_to_pc(inner, 1000).unwrap().0, RunEnd::Stopped);
    assert_eq!((c.pc, c.jsr_depth()), (inner, 2));
    let (end, used) = c.run_to_return(1000).unwrap();
    assert_eq!((end, c.jsr_depth()), (RunEnd::Stopped, 1));
    assert_eq!(used, 2 + 6, "iny + rts");
    // From outer: runs through the second nested call.
    assert_eq!(c.run_to_return(1000).unwrap().0, RunEnd::Stopped);
    assert_eq!((c.jsr_depth(), c.y), (0, 2));
    assert!(c.run_to_return(1000).is_err(), "no active subroutine");
    assert_eq!(c.run_to_pc(0x1234, 500).unwrap().0, RunEnd::Halted);
}
