//! The selectable 65C02 read-modify-write dead cycle (measured vs WDC table 5-5).

use mos6502::cpu::RmwDeadCycle;
use mos6502::cycles::BusCycle;
use mos6502::{Cpu, CpuVariant, FlatMemory};

fn trace(variant: CpuVariant, dead: RmwDeadCycle, bytes: &[u8], x: u8) -> Vec<BusCycle> {
    let mut m = FlatMemory::new();
    m.load_at(0x0400, bytes);
    let mut c = Cpu::with_variant(m, variant);
    c.cmos_rmw_dead = dead;
    c.pc = 0x0400;
    c.x = x;
    c.trace_bus_instruction().unwrap()
}

fn dead_read(t: &[BusCycle]) -> u16 {
    // The cycle before the final write.
    t[t.len() - 2].address
}

#[test]
fn every_cmos_rmw_opcode_under_both_options() {
    // (opcode, mode): zp, zp,X, abs, abs,X for ASL ROL LSR ROR DEC INC, plus TSB/TRB.
    let zp = [0x06u8, 0x26, 0x46, 0x66, 0xC6, 0xE6, 0x04, 0x14];
    let zpx = [0x16u8, 0x36, 0x56, 0x76, 0xD6, 0xF6];
    let abs = [0x0Eu8, 0x2E, 0x4E, 0x6E, 0xCE, 0xEE, 0x0C, 0x1C];
    let absx = [0x1Eu8, 0x3E, 0x5E, 0x7E, 0xDE, 0xFE];
    for x in [0u8, 1, 0x7F, 0xFF] {
        for b1 in [0x00u8, 0x10, 0xFE, 0xFF] {
            for &op in &zp {
                let ea = u16::from(b1);
                let m = trace(CpuVariant::CMOS, RmwDeadCycle::Measured, &[op, b1], x);
                let w = trace(CpuVariant::CMOS, RmwDeadCycle::WdcTable, &[op, b1], x);
                assert_eq!(dead_read(&m), ea);
                assert_eq!(dead_read(&w), (ea + 1) & 0xFF, "{op:02X} {b1:02X}");
                assert_eq!(m.len(), w.len());
            }
            for &op in &zpx {
                let ea = u16::from(b1.wrapping_add(x));
                let m = trace(CpuVariant::CMOS, RmwDeadCycle::Measured, &[op, b1], x);
                let w = trace(CpuVariant::CMOS, RmwDeadCycle::WdcTable, &[op, b1], x);
                assert_eq!(dead_read(&m), ea);
                assert_eq!(dead_read(&w), (ea + 1) & 0xFF);
            }
            for &op in &abs {
                let ea = u16::from_le_bytes([b1, 0x12]);
                for dead in [RmwDeadCycle::Measured, RmwDeadCycle::WdcTable] {
                    let t = trace(CpuVariant::CMOS, dead, &[op, b1, 0x12], x);
                    assert_eq!(dead_read(&t), ea, "absolute is the same in both");
                }
            }
            for &op in &absx {
                let ea = u16::from_le_bytes([b1, 0xFF]).wrapping_add(u16::from(x));
                let m = trace(CpuVariant::CMOS, RmwDeadCycle::Measured, &[op, b1, 0xFF], x);
                let w = trace(CpuVariant::CMOS, RmwDeadCycle::WdcTable, &[op, b1, 0xFF], x);
                assert_eq!(dead_read(&m), ea);
                assert_eq!(dead_read(&w), ea.wrapping_add(1), "{op:02X} wraps at $FFFF");
                assert_eq!(m.last().unwrap().address, ea);
            }
        }
    }
}

#[test]
fn rmb_smb_and_nmos_are_unaffected() {
    for op in [0x07u8, 0x87, 0x77, 0xF7] {
        let m = trace(CpuVariant::CMOS, RmwDeadCycle::Measured, &[op, 0x40], 0);
        let w = trace(CpuVariant::CMOS, RmwDeadCycle::WdcTable, &[op, 0x40], 0);
        assert_eq!(m, w);
    }
    for op in [0x06u8, 0x16, 0x0E, 0x1E] {
        let m = trace(
            CpuVariant::NMOS,
            RmwDeadCycle::Measured,
            &[op, 0x40, 0x12],
            3,
        );
        let w = trace(
            CpuVariant::NMOS,
            RmwDeadCycle::WdcTable,
            &[op, 0x40, 0x12],
            3,
        );
        assert_eq!(m, w);
        assert!(m[m.len() - 2].write, "NMOS dummy write");
    }
}

#[test]
fn results_do_not_depend_on_the_option() {
    for op in [0x06u8, 0xE6, 0x04, 0x14, 0x1E, 0xFE] {
        let mut mem = [FlatMemory::new(), FlatMemory::new()];
        let mut out = Vec::new();
        for (i, dead) in [RmwDeadCycle::Measured, RmwDeadCycle::WdcTable]
            .into_iter()
            .enumerate()
        {
            let mut m = std::mem::take(&mut mem[i]);
            m.load_at(0x0400, &[op, 0x30, 0x00]);
            m.data[0x30] = 0x81;
            m.data[0x31] = 0x42;
            let mut c = Cpu::with_variant(m, CpuVariant::CMOS);
            c.cmos_rmw_dead = dead;
            c.pc = 0x0400;
            c.a = 0x0F;
            c.trace_bus_instruction().unwrap();
            out.push((c.bus.data[0x30], c.bus.data[0x31], c.p));
        }
        assert_eq!(out[0], out[1], "{op:02X}");
    }
}

#[test]
fn cycle_state_carries_the_option_and_old_states_keep_it() {
    for saved in [RmwDeadCycle::Measured, RmwDeadCycle::WdcTable] {
        let mut a = Cpu::with_variant(FlatMemory::new(), CpuVariant::CMOS);
        a.cmos_rmw_dead = saved;
        let s = a.cycle_state();
        assert_eq!(s[31], 1 + u8::from(saved == RmwDeadCycle::WdcTable));
        for current in [RmwDeadCycle::Measured, RmwDeadCycle::WdcTable] {
            let mut b = Cpu::with_variant(FlatMemory::new(), CpuVariant::CMOS);
            b.cmos_rmw_dead = current;
            b.load_cycle_state(&s).unwrap();
            assert_eq!(b.cmos_rmw_dead, saved);
            let mut old = s.clone();
            old[31] = 0;
            b.cmos_rmw_dead = current;
            b.load_cycle_state(&old).unwrap();
            assert_eq!(b.cmos_rmw_dead, current);
        }
        for bad in 3..=255u8 {
            let mut x = s.clone();
            x[31] = bad;
            assert!(a.load_cycle_state(&x).is_err());
        }
    }
}
