//! `Cpu::next_cycle_writes` predicts every bus cycle of every opcode, and the DMC DMA
//! window loses fewer CPU cycles when it meets write cycles.

use mos6502::bus::MemoryBus;
use mos6502::systems::nes2a03::Nes2A03;
use mos6502::{Cpu, CpuVariant, FlatMemory};

#[test]
fn prediction_matches_the_bus_for_all_opcodes_on_both_cores() {
    for variant in [CpuVariant::NMOS, CpuVariant::CMOS] {
        for op in 0..=255u8 {
            let mut mem = FlatMemory::new();
            mem.load_at(0x0300, &[op, 0x10, 0x02, 0xEA]);
            mem.data[0x10] = 0x20;
            mem.data[0x11] = 0x02;
            mem.load_at(0xFFFE, &[0x00, 0x05]);
            let mut cpu = Cpu::with_variant(mem, variant);
            cpu.pc = 0x0300;
            cpu.p.0 &= !mos6502::bits::D;
            for n in 0..10 {
                let predicted = cpu.next_cycle_writes();
                let Ok(cycle) = cpu.step_cycle() else { break };
                assert_eq!(predicted, cycle.write, "{variant:?} {op:02X} cycle {n}");
                if cycle.complete || cpu.is_halted() {
                    break;
                }
            }
            assert!(!cpu.next_cycle_writes(), "opcode fetch next");
        }
    }
}

fn machine() -> Nes2A03 {
    Nes2A03::from_source(
        "
.org $8000
    jsr sub
idle:
    jmp idle
sub:
    rts
.org $C000
    .byte $FF
.org $FFFC
    .word $8000
",
    )
    .unwrap()
}

fn stall_from(cycles_into_jsr: u32) -> u64 {
    let mut nes = machine();
    for _ in 0..cycles_into_jsr {
        nes.cpu.step_cycle().unwrap();
    }
    nes.cpu.bus.apu.write_register(0x15, 0x10);
    nes.run_cycles(4).unwrap();
    assert_eq!(
        nes.cpu.bus.apu.dmc.remaining, 0,
        "byte fetched at the window's end"
    );
    nes.stalled
}

#[test]
fn dmc_window_steals_four_on_reads_and_less_on_pushes() {
    assert_eq!(stall_from(0), 4, "JSR opcode fetch: all reads");
    assert_eq!(
        stall_from(3),
        2,
        "both return-address pushes run inside the window"
    );
    assert_eq!(stall_from(4), 3, "only the low push");
    assert_eq!(stall_from(5), 4);
}

#[test]
fn rmw_middle_cycle_is_a_write_on_nmos_and_a_read_on_cmos() {
    // INC $10, ASL $1234, ROR $1234,X, TSB $10 (CMOS only)
    let cases: &[(u8, &[u8], usize)] = &[
        (0xE6, &[0x10], 5),
        (0x0E, &[0x34, 0x12], 6),
        (0x7E, &[0x34, 0x12], 7),
        (0x04, &[0x10], 5),
    ];
    for variant in [CpuVariant::NMOS, CpuVariant::CMOS] {
        for &(op, operand, len) in cases {
            if op == 0x04 && variant == CpuVariant::NMOS {
                continue;
            }
            let mut mem = FlatMemory::new();
            mem.load_at(0x0300, &[op]);
            mem.load_at(0x0301, operand);
            let mut cpu = Cpu::with_variant(mem, variant);
            cpu.pc = 0x0300;
            let mut writes = Vec::new();
            loop {
                let c = cpu.step_cycle().unwrap();
                writes.push(c.write);
                if c.complete {
                    break;
                }
            }
            let expect = if variant == CpuVariant::NMOS { 2 } else { 1 };
            assert_eq!(
                writes.iter().filter(|&&w| w).count(),
                expect,
                "{variant:?} {op:02X}"
            );
            assert!(writes[writes.len() - 1], "last cycle writes the result");
            let len = if variant == CpuVariant::CMOS && op == 0x7E {
                6
            } else {
                len
            };
            assert_eq!(writes.len(), len, "{variant:?} {op:02X} cycles");
        }
    }
}

#[test]
fn a_dma_window_split_across_run_calls_matches_one_call() {
    let mut whole = machine();
    whole.cpu.bus.apu.write_register(0x15, 0x10);
    whole.run_cycles(4).unwrap();
    let mut split = machine();
    split.cpu.bus.apu.write_register(0x15, 0x10);
    for _ in 0..4 {
        split.run_cycles(1).unwrap();
    }
    assert_eq!((split.stalled, split.dma_window), (whole.stalled, 0));
    assert_eq!(split.cpu.bus.apu.dmc.buffer, whole.cpu.bus.apu.dmc.buffer);
    assert_eq!(split.cpu.pc, whole.cpu.pc);
    let mut open = machine();
    open.cpu.bus.apu.write_register(0x15, 0x10);
    open.run_cycles(2).unwrap();
    assert_eq!(
        (open.dma_window, open.cpu.bus.apu.dmc.buffer),
        (2, None),
        "still open"
    );
}

#[test]
fn cmos_shift_abs_x_is_six_cycles_without_page_cross() {
    for op in [0x1Eu8, 0x3E, 0x5E, 0x7E, 0xFE, 0xDE] {
        for x in 0..=255u8 {
            for variant in [CpuVariant::NMOS, CpuVariant::CMOS] {
                let lo = 0x80u8;
                let cross = u16::from(lo) + u16::from(x) > 0xFF;
                let shift = op & 0xC0 != 0xC0;
                let want = if variant == CpuVariant::CMOS && shift && !cross {
                    6
                } else {
                    7
                };
                let run = |cycle_mode: bool| {
                    let mut mem = FlatMemory::new();
                    mem.load_at(0x0300, &[op, lo, 0x12]);
                    let ea = 0x1280u16 + u16::from(x);
                    mem.load_at(ea, &[0x81]);
                    let mut cpu = Cpu::with_variant(mem, variant);
                    cpu.pc = 0x0300;
                    cpu.x = x;
                    let n = if cycle_mode {
                        let mut n = 0;
                        loop {
                            n += 1;
                            if cpu.step_cycle().unwrap().complete {
                                break n;
                            }
                        }
                    } else {
                        cpu.step().unwrap()
                    };
                    (n, cpu.bus.peek(ea), format!("{:?}", cpu.p))
                };
                let (a, b) = (run(false), run(true));
                assert_eq!(a.0, want, "{variant:?} {op:02X} x={x:02X} step");
                assert_eq!(a, b, "{variant:?} {op:02X} x={x:02X} step vs cycles");
            }
        }
    }
}
