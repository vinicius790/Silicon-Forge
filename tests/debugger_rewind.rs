//! Step-back restores registers and memory exactly, over the whole 32-entry history.

use mos6502::assemble;
use mos6502::bus::MemoryBus;
use mos6502::debugger::Debugger;
use mos6502::{Cpu, FlatMemory};

fn image(cpu: &Cpu<FlatMemory>) -> (String, Vec<u8>) {
    (
        format!("{:?}", cpu.registers()),
        (0..=0xFFFFu16).map(|a| cpu.bus.peek(a)).collect(),
    )
}

#[test]
fn every_prefix_of_32_steps_rewinds_to_the_exact_state() {
    let src = "ldx #0\nloop: txa\n sta $0300,x\n pha\n inc $10\n asl $0300,x\n inx\n bne loop";
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &assemble(src, 0x0400).unwrap());
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    for depth in 1..=32usize {
        let mut fresh = cpu.clone();
        let mut dbg = Debugger::default();
        let mut states = vec![image(&fresh)];
        for _ in 0..depth {
            dbg.step_recording(&mut fresh).unwrap();
            states.push(image(&fresh));
        }
        for back in (0..depth).rev() {
            dbg.step_back(&mut fresh).unwrap();
            assert_eq!(image(&fresh), states[back], "depth {depth} back to {back}");
        }
        assert!(dbg.step_back(&mut fresh).is_none());
    }
}

#[test]
fn history_keeps_only_the_newest_32() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &[0xE8; 64]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    let mut dbg = Debugger::default();
    for _ in 0..40 {
        dbg.step_recording(&mut cpu).unwrap();
    }
    assert_eq!(dbg.history_len(), 32);
    let mut n = 0;
    while dbg.step_back(&mut cpu).is_some() {
        n += 1;
    }
    assert_eq!((n, cpu.x, cpu.pc), (32, 8, 0x0408));
}
