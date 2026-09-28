//! Unstable NMOS stores, 65C02 WAI/STP, PHI1/PHI2 halves, IRQ polling point, and
//! decimal-mode flags of both families.

use mos6502::bus::FlatMemory;
use mos6502::flags::bits;
use mos6502::opcodes::CpuVariant;
use mos6502::{BusPhase, Cpu, ANE_MAGIC};

fn cpu(variant: CpuVariant, code: &[u8]) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    // NOP padding, so a stray fetch never reads BRK and fakes an IRQ entry.
    mem.data[0x8000..0x8200].fill(0xEA);
    mem.data[0x9000..0x9010].fill(0xEA);
    mem.data[0x8000..0x8000 + code.len()].copy_from_slice(code);
    mem.data[0xFFFE] = 0x00;
    mem.data[0xFFFF] = 0x90;
    mem.data[0xFFFA] = 0x00;
    mem.data[0xFFFB] = 0xA0;
    let mut cpu = Cpu::with_variant(mem, variant);
    cpu.pc = 0x8000;
    cpu.p.set(bits::I, false);
    cpu
}

/// Runs one instruction with `step` and with `step_cycle` and checks both agree.
fn both(variant: CpuVariant, code: &[u8], setup: impl Fn(&mut Cpu<FlatMemory>)) -> Cpu<FlatMemory> {
    let mut stepped = cpu(variant, code);
    let mut cycled = cpu(variant, code);
    setup(&mut stepped);
    setup(&mut cycled);
    let n = stepped.step().unwrap();
    let trace = cycled.trace_bus_instruction().unwrap();
    assert_eq!(trace.len() as u32, n, "{code:02X?}");
    assert_eq!(cycled.registers(), stepped.registers(), "{code:02X?}");
    assert_eq!(cycled.bus.data, stepped.bus.data, "{code:02X?}");
    stepped
}

#[test]
fn unstable_stores_and_with_high_byte_plus_one_and_corrupt_a_crossed_address() {
    // Base $12F0 + $20 crosses into $13. Value = src & $13 = $01, written at $0110.
    let cases: [(&[u8], u8, u32); 5] = [
        (&[0x9E, 0xF0, 0x12], 0x05, 5), // SHX abs,Y: X = $05
        (&[0x9C, 0xF0, 0x12], 0x05, 5), // SHY abs,X: Y = $05
        (&[0x9F, 0xF0, 0x12], 0x05, 5), // SHA abs,Y: A & X = $05
        (&[0x93, 0x40], 0x05, 6),       // SHA (zp),Y
        (&[0x9B, 0xF0, 0x12], 0x05, 5), // TAS abs,Y: SP = A & X
    ];
    for (code, _, cycles) in cases {
        let cpu = both(CpuVariant::NMOS, code, |c| {
            c.a = 0x07;
            c.x = if code[0] == 0x9C { 0x20 } else { 0x0D };
            c.y = if code[0] == 0x9C { 0x05 } else { 0x20 };
            if code[0] == 0x9E {
                c.x = 0x05;
            }
            c.bus.data[0x40] = 0xF0;
            c.bus.data[0x41] = 0x12;
        });
        assert_eq!(cpu.bus.data[0x0110], 0x01, "{code:02X?}");
        assert_eq!(cpu.bus.data[0x1310], 0x00, "{code:02X?}");
        assert_eq!(cpu.cycles, u64::from(cycles));
        if code[0] == 0x9B {
            assert_eq!(cpu.sp, 0x05);
        }
    }
    // No page cross: the address is untouched. Base high $FF + 1 wraps to $00.
    let cpu = both(CpuVariant::NMOS, &[0x9E, 0x00, 0x12], |c| {
        c.x = 0xFF;
        c.y = 0x01;
    });
    assert_eq!(cpu.bus.data[0x1201], 0x13);
    let cpu = both(CpuVariant::NMOS, &[0x9E, 0x00, 0xFF], |c| {
        c.x = 0xFF;
        c.y = 0x01;
        c.bus.data[0xFF01] = 0xAA;
    });
    assert_eq!(cpu.bus.data[0xFF01], 0x00);
}

#[test]
fn cmos_9c_9e_stay_stz_and_las_ane_follow_the_documented_formulas() {
    let cpu = both(CpuVariant::CMOS, &[0x9E, 0xF0, 0x12], |c| {
        c.x = 0x20;
        c.y = 0x05;
        c.bus.data[0x1310] = 0xAA;
    });
    assert_eq!(cpu.bus.data[0x1310], 0x00);
    assert_eq!(cpu.bus.data[0x0110], 0x00);
    let cpu = both(CpuVariant::CMOS, &[0x9C, 0x34, 0x12], |c| {
        c.bus.data[0x1234] = 0xAA
    });
    assert_eq!(cpu.bus.data[0x1234], 0x00);

    let cpu = both(CpuVariant::NMOS, &[0xBB, 0xFF, 0x12], |c| {
        c.sp = 0xF0;
        c.y = 0x01;
        c.bus.data[0x1300] = 0x3C;
    });
    assert_eq!((cpu.a, cpu.x, cpu.sp, cpu.cycles), (0x30, 0x30, 0x30, 5));

    assert_eq!(ANE_MAGIC, 0xEE);
    let cpu = both(CpuVariant::NMOS, &[0x8B, 0xFF], |c| {
        c.a = 0x01;
        c.x = 0xF3;
    });
    assert_eq!(cpu.a, (0x01 | ANE_MAGIC) & 0xF3);
    assert_eq!(cpu.a, 0xE3);
    assert!(cpu.p.get(bits::N));
}

#[test]
fn wai_sleeps_until_irq_or_nmi_and_masked_irq_resumes_without_vector() {
    // Masked IRQ: wake and run INX, no stack traffic.
    let mut c = cpu(CpuVariant::CMOS, &[0xCB, 0xE8]);
    c.p.set(bits::I, true);
    assert_eq!(c.step().unwrap(), 3);
    assert!(c.is_waiting());
    assert_eq!(c.step().unwrap(), 1);
    assert_eq!((c.pc, c.cycles), (0x8001, 4));
    c.set_irq_line(true);
    c.step().unwrap();
    assert!(!c.is_waiting());
    assert_eq!((c.pc, c.x, c.sp), (0x8002, 1, 0xFD));

    // Unmasked IRQ: vector taken.
    let mut c = cpu(CpuVariant::CMOS, &[0xCB, 0xE8]);
    c.step().unwrap();
    c.set_irq_line(true);
    assert_eq!(c.step().unwrap(), 7);
    assert_eq!(c.pc, 0x9000);

    // NMI wakes even with I set.
    let mut c = cpu(CpuVariant::CMOS, &[0xCB, 0xE8]);
    c.p.set(bits::I, true);
    c.step().unwrap();
    c.pulse_nmi();
    c.step().unwrap();
    assert_eq!(c.pc, 0xA000);

    // Cycle-stepped: 3 cycles, idle reads of PC, masked IRQ fetches INX next.
    let mut c = cpu(CpuVariant::CMOS, &[0xCB, 0xE8]);
    c.p.set(bits::I, true);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 3);
    let idle = c.step_cycle().unwrap();
    assert!(idle.complete && !idle.write && idle.address == 0x8001);
    assert!(c.is_waiting());
    c.set_irq_line(true);
    let wake = c.step_cycle().unwrap();
    assert!(
        wake.complete && !c.is_waiting(),
        "the waking cycle still idles"
    );
    let fetch = c.step_cycle().unwrap();
    assert!(!fetch.complete);
    assert_eq!((fetch.address, fetch.data), (0x8001, 0xE8));

    // NMOS keeps $CB as SBX.
    let c = both(CpuVariant::NMOS, &[0xCB, 0x01], |c| {
        c.a = 0x0F;
        c.x = 0x03;
    });
    assert!(!c.is_waiting());
    assert_eq!(c.x, 0x02);
}

#[test]
fn stp_halts_until_reset() {
    let mut c = cpu(CpuVariant::CMOS, &[0xDB, 0xE8]);
    c.bus.data[0xFFFC] = 0x01;
    c.bus.data[0xFFFD] = 0x80;
    assert_eq!(c.step().unwrap(), 3);
    assert!(c.is_halted());
    c.set_irq_line(true);
    c.pulse_nmi();
    assert_eq!(c.step().unwrap(), 0);
    assert!(c.step_cycle().unwrap_err().contains("halted"));
    c.set_irq_line(false);
    c.reset();
    assert!(!c.is_halted());
    c.step().unwrap();
    assert_eq!(c.x, 1);

    let mut c = cpu(CpuVariant::CMOS, &[0xDB]);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 3);
    assert!(c.is_halted());
}

#[test]
fn half_cycles_put_the_address_in_phi1_and_the_data_in_phi2() {
    let code = [0xAD, 0x34, 0x12];
    let mut c = cpu(CpuVariant::NMOS, &code);
    c.bus.data[0x1234] = 0x5A;
    let mut halves = Vec::new();
    loop {
        let h = c.step_half_cycle().unwrap();
        halves.push((h.phase, h.address, h.data));
        if h.phase == BusPhase::Phi1 {
            assert!(c.step().is_err() && c.step_cycle().is_err());
        }
        if h.complete {
            break;
        }
    }
    use BusPhase::{Phi1, Phi2};
    assert_eq!(
        halves,
        [
            (Phi1, 0x8000, None),
            (Phi2, 0x8000, Some(0xAD)),
            (Phi1, 0x8001, None),
            (Phi2, 0x8001, Some(0x34)),
            (Phi1, 0x8002, None),
            (Phi2, 0x8002, Some(0x12)),
            (Phi1, 0x1234, None),
            (Phi2, 0x1234, Some(0x5A)),
        ]
    );
    let mut s = cpu(CpuVariant::NMOS, &code);
    s.bus.data[0x1234] = 0x5A;
    s.step().unwrap();
    assert_eq!(c.registers(), s.registers());
}

/// Raises the IRQ line before cycle `raise_at` (1-based) of the first instruction,
/// then reports whether the IRQ ran right after it.
fn irq_taken_after_first(code: &[u8], z: bool, raise_at: usize) -> bool {
    // A leading NOP gives the sequencer a polling history, so the line is raised
    // inside `code` and not on a fresh boundary.
    let mut c = cpu(CpuVariant::NMOS, &[[0xEA].as_slice(), code].concat());
    c.p.set(bits::Z, z);
    c.trace_bus_instruction().unwrap();
    let mut n = 0;
    loop {
        n += 1;
        if n == raise_at {
            c.set_irq_line(true);
        }
        if c.step_cycle().unwrap().complete {
            break;
        }
    }
    let next = c.trace_bus_instruction().unwrap();
    next.len() == 7 && c.pc == 0x9000
}

#[test]
fn irq_is_polled_before_the_last_cycle() {
    // LDA $1234: 4 cycles. Raised before cycle 3 -> taken; only on cycle 4 -> late.
    assert!(irq_taken_after_first(&[0xAD, 0x34, 0x12, 0xEA], false, 3));
    assert!(!irq_taken_after_first(&[0xAD, 0x34, 0x12, 0xEA], false, 4));
    // The late IRQ runs after the following NOP.
    let mut c = cpu(CpuVariant::NMOS, &[0xEA, 0xEA, 0xEA]);
    c.step_cycle().unwrap();
    c.set_irq_line(true);
    c.step_cycle().unwrap();
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 2);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
    assert_eq!(c.pc, 0x9000);
}

#[test]
fn taken_branch_without_page_cross_delays_irq_by_one_instruction() {
    // BNE +2 taken (Z clear), 3 cycles, same page. Raised before cycle 2 -> late.
    assert!(irq_taken_after_first(&[0xD0, 0x02], false, 1));
    assert!(!irq_taken_after_first(&[0xD0, 0x02], false, 2));
    // A 3-cycle LDA zp raised before cycle 2 is taken right after it.
    assert!(irq_taken_after_first(&[0xA5, 0x10], false, 2));
    // Not taken (2 cycles): the cycle before the last is the operand fetch's predecessor.
    assert!(irq_taken_after_first(&[0xD0, 0x02], true, 1));
    assert!(!irq_taken_after_first(&[0xD0, 0x02], true, 2));
    // Taken across a page (4 cycles): polled again before the fix-up cycle.
    let mut code = vec![0xEA; 0x100];
    code[0xFC] = 0xD0;
    code[0xFD] = 0x10;
    let mut c = cpu(CpuVariant::NMOS, &code);
    c.pc = 0x80FC;
    for n in 1..=4 {
        if n == 3 {
            c.set_irq_line(true);
        }
        c.step_cycle().unwrap();
    }
    assert_eq!(c.pc, 0x810E);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
}

#[test]
fn decimal_flags_follow_each_family() {
    // NMOS: $79 + $00 + C = $80 with V and N from the intermediate sum.
    let c = both(CpuVariant::NMOS, &[0x69, 0x00], |c| {
        c.a = 0x79;
        c.p.set(bits::D, true);
        c.p.set(bits::C, true);
    });
    assert_eq!(c.a, 0x80);
    assert!(c.p.get(bits::V) && c.p.get(bits::N) && !c.p.get(bits::C));
    assert_eq!(c.cycles, 2);

    // 65C02: N and Z from the BCD result, one extra cycle that rereads PC.
    let mut s = cpu(CpuVariant::CMOS, &[0x69, 0x01]);
    s.a = 0x99;
    s.p.set(bits::D, true);
    assert_eq!(s.step().unwrap(), 3);
    assert!(s.p.get(bits::Z) && !s.p.get(bits::N) && s.p.get(bits::C));
    let mut t = cpu(CpuVariant::CMOS, &[0x69, 0x01]);
    t.a = 0x99;
    t.p.set(bits::D, true);
    let trace = t.trace_bus_instruction().unwrap();
    assert_eq!(trace.len(), 3);
    assert_eq!(trace[2].address, 0x8002);
    assert_eq!(t.registers(), s.registers());

    // 65C02 SBC: $00 - $01 = $99, borrow, N from the result.
    let c = both(CpuVariant::CMOS, &[0xE9, 0x01], |c| {
        c.p.set(bits::D, true);
        c.p.set(bits::C, true);
    });
    assert_eq!(c.a, 0x99);
    assert!(c.p.get(bits::N) && !c.p.get(bits::C) && !c.p.get(bits::Z));
    // Binary mode on CMOS has no extra cycle.
    let c = both(CpuVariant::CMOS, &[0x69, 0x01], |_| {});
    assert_eq!(c.cycles, 2);
}
