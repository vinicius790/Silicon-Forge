//! Delta step-back history against an independent full-copy model.

use mos6502::assemble;
use mos6502::bus::MemoryBus;
use mos6502::debugger::Debugger;
use mos6502::{Cpu, FlatMemory};

type State = (String, Vec<u8>, bool);

fn image(cpu: &Cpu<FlatMemory>) -> State {
    (
        format!("{:?}", cpu.registers()),
        (0..=0xFFFFu16).map(|a| cpu.bus.peek(a)).collect(),
        cpu.is_halted(),
    )
}

fn machine() -> Cpu<FlatMemory> {
    let src = "ldx #0\nloop: txa\n sta $0300,x\n pha\n inc $10\n asl $0300,x\n jsr sub\n inx\n bne loop\n brk\nsub: lda $10\n sta $2000,x\n rts";
    let mut mem = FlatMemory::new();
    mem.load_at(0x0400, &assemble(src, 0x0400).unwrap());
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0400;
    cpu
}

/// xorshift for a fixed, reproducible schedule.
fn rng(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[test]
fn random_step_back_poke_and_limit_schedules_match_full_copies() {
    for seed0 in 1..=12u64 {
        let mut seed = seed0 * 0x9E37_79B9;
        let mut cpu = machine();
        let mut dbg = Debugger::default();
        let limit = 1 + (rng(&mut seed) % 40) as usize;
        dbg.set_history_limit(limit);
        let mut model: Vec<State> = Vec::new();
        for _ in 0..600 {
            match rng(&mut seed) % 10 {
                0..=5 => {
                    if model.len() == limit {
                        model.remove(0);
                    }
                    model.push(image(&cpu));
                    dbg.step_recording(&mut cpu).unwrap();
                }
                6..=8 => {
                    let want = model.pop();
                    let got = dbg.step_back(&mut cpu);
                    assert_eq!(got.is_some(), want.is_some());
                    if let Some(want) = want {
                        assert_eq!(image(&cpu), want, "seed {seed0}");
                    }
                }
                _ => {
                    // An edit between steps (as a REPL `poke` would) is undone too.
                    let addr = 0x2000 + (rng(&mut seed) % 0x100) as u16;
                    cpu.bus.write(addr, rng(&mut seed) as u8);
                }
            }
            assert_eq!(dbg.history_len(), model.len());
        }
        while let Some(want) = model.pop() {
            dbg.step_back(&mut cpu).unwrap();
            assert_eq!(image(&cpu), want);
        }
        assert!(dbg.step_back(&mut cpu).is_none());
    }
}

#[test]
fn memory_is_one_image_plus_changed_bytes() {
    let mut cpu = machine();
    let mut dbg = Debugger::default();
    dbg.set_history_limit(1000);
    for _ in 0..1000 {
        dbg.step_recording(&mut cpu).unwrap();
    }
    let full_copy = 1000 * 0x10000;
    let used = dbg.history_bytes();
    assert!(used >= 0x10000);
    // Each instruction here changes at most 3 bytes (stack pushes / one store).
    assert!(used <= 0x10000 + 1000 * 3 * 3, "{used}");
    assert!(used * 500 < full_copy * 3, "far below the full-copy size");
    dbg.clear_history();
    assert_eq!(dbg.history_len(), 0);
    assert_eq!(dbg.history_bytes(), 0);
    assert!(dbg.step_back(&mut cpu).is_none());
}

#[test]
fn checksum_of_newest_snapshot_matches_the_restored_memory() {
    let mut cpu = machine();
    let mut dbg = Debugger::default();
    for _ in 0..20 {
        dbg.step_recording(&mut cpu).unwrap();
    }
    for _ in 0..20 {
        let want = dbg.newest_ram_checksum().unwrap();
        dbg.step_back(&mut cpu).unwrap();
        assert_eq!(Debugger::ram_checksum(&cpu.bus), want);
    }
}

#[test]
fn shrinking_the_limit_keeps_newest_and_still_rewinds_exactly() {
    let mut cpu = machine();
    let mut dbg = Debugger::default();
    dbg.set_history_limit(50);
    let mut model = Vec::new();
    for _ in 0..50 {
        model.push(image(&cpu));
        dbg.step_recording(&mut cpu).unwrap();
    }
    dbg.set_history_limit(7);
    model.drain(..43);
    while let Some(want) = model.pop() {
        dbg.step_back(&mut cpu).unwrap();
        assert_eq!(image(&cpu), want);
    }
    assert!(dbg.step_back(&mut cpu).is_none());
}
