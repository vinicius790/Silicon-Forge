//! Reverse continue, configurable history and recorded runs.

use mos6502::assemble;
use mos6502::debugger::{Debugger, ReverseStop};
use mos6502::{Cpu, FlatMemory};

const SRC: &str =
    "ldx #0\nloop: inx\n stx $0200\n txa\n asl a\n sta $0201\n cpx #$40\n bne loop\n brk";

fn cpu() -> Cpu<FlatMemory> {
    let bytes = assemble(SRC, 0x0400).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &bytes);
    mem.load_at(0xFFFC, &[0x00, 0x04]);
    let mut c = Cpu::new(mem);
    c.halt_on_brk = true;
    c.reset();
    c
}

#[test]
fn reverse_continue_lands_on_each_earlier_breakpoint_exactly() {
    let mut d = Debugger::default();
    d.set_history_limit(1000);
    let mut c = cpu();
    let n = d.run_recording(&mut c, 10_000).unwrap();
    assert!(c.is_halted());
    assert_eq!(n, 1 + 0x40 * 7 + 1, "ldx, 64 loops of 7, brk");
    assert_eq!(d.history_len(), n as usize);
    // Reference states at each arrival at `loop` ($0402), by forward execution.
    let mut refs = Vec::new();
    let mut f = cpu();
    while !f.is_halted() {
        if f.pc == 0x0402 {
            refs.push((f.registers(), f.bus.data[0x200], f.bus.data[0x201]));
        }
        f.step().unwrap();
    }
    d.breakpoints.insert(0x0402);
    for want in refs.iter().rev() {
        let (stop, undone) = d.reverse_continue(&mut c).unwrap();
        assert_eq!(stop, ReverseStop::Breakpoint);
        assert!(undone >= 1);
        assert_eq!((c.registers(), c.bus.data[0x200], c.bus.data[0x201]), *want);
        assert!(!c.is_halted());
    }
    assert_eq!(
        d.reverse_continue(&mut c).unwrap(),
        (ReverseStop::HistoryStart, 1)
    );
    assert_eq!(c.pc, 0x0400);
}

#[test]
fn reverse_to_a_condition_and_forward_again_is_identical() {
    let mut d = Debugger::default();
    d.set_history_limit(400);
    let mut c = cpu();
    d.run_recording(&mut c, 10_000).unwrap();
    let end = (c.registers(), c.bus.data[0x200]);
    d.break_when("[$0200] == $20 && A == $20").unwrap();
    let (stop, _) = d.reverse_continue(&mut c).unwrap();
    assert_eq!(stop, ReverseStop::Condition);
    assert_eq!((c.bus.data[0x200], c.a), (0x20, 0x20));
    d.watch_condition = None;
    while !c.is_halted() {
        c.step().unwrap();
    }
    assert_eq!((c.registers(), c.bus.data[0x200]), end);
}

#[test]
fn history_limit_bounds_and_default() {
    let mut d = Debugger::default();
    assert_eq!(d.history_limit(), 32);
    let mut c = cpu();
    d.run_recording(&mut c, 100).unwrap();
    assert_eq!(d.history_len(), 32);
    d.set_history_limit(5);
    assert_eq!(d.history_len(), 5);
    d.set_history_limit(0);
    assert_eq!(d.history_limit(), 1);
    assert_eq!(d.history_len(), 1);
    // A breakpoint stops a recorded run (not on the first instruction).
    let mut d = Debugger::default();
    let mut c = cpu();
    d.breakpoints.insert(0x0402);
    assert_eq!(d.run_recording(&mut c, 100).unwrap(), 1);
    assert_eq!(d.run_recording(&mut c, 100).unwrap(), 7);
}
