//! 65C02 maker variants (WDC, Rockwell, Synertek) and the step/trace agreement of
//! the 65C02 dead-cycle corrections.

use mos6502::cpu::CmosModel;
use mos6502::{Cpu, CpuVariant, FlatMemory};

fn cpu(model: CmosModel, bytes: &[u8]) -> Cpu<FlatMemory> {
    let mut m = FlatMemory::new();
    m.load_at(0x0400, bytes);
    m.data[0x10] = 0x01;
    let mut c = Cpu::with_variant(m, CpuVariant::CMOS);
    c.cmos_model = model;
    c.pc = 0x0400;
    c
}

#[test]
fn implemented_sets_per_maker() {
    for op in 0..=255u8 {
        let bit = matches!(op & 15, 7 | 15);
        let ws = matches!(op, 0xCB | 0xDB);
        assert!(CmosModel::Wdc.implements(op));
        assert_eq!(CmosModel::Rockwell.implements(op), !ws, "{op:02X}");
        assert_eq!(CmosModel::Synertek.implements(op), !ws && !bit, "{op:02X}");
    }
}

#[test]
fn missing_opcodes_are_one_byte_one_cycle_nops_in_step_and_trace() {
    for model in [CmosModel::Rockwell, CmosModel::Synertek] {
        for op in 0..=255u8 {
            if model.implements(op) {
                continue;
            }
            let mut a = cpu(model, &[op, 0x10, 0x00]);
            let regs = (a.a, a.x, a.y, a.p, a.sp);
            assert_eq!(a.step().unwrap(), 1, "{model:?} {op:02X}");
            assert_eq!(a.pc, 0x0401);
            assert_eq!((a.a, a.x, a.y, a.p, a.sp), regs);
            assert!(!a.is_halted() && !a.is_waiting());
            assert_eq!(a.bus.data[0x10], 0x01, "RMB/SMB did not run");
            let mut b = cpu(model, &[op, 0x10, 0x00]);
            let trace = b.trace_bus_instruction().unwrap();
            assert_eq!(trace.len(), 1);
            assert_eq!((trace[0].address, trace[0].write), (0x0400, false));
            assert_eq!(b.pc, 0x0401);
        }
    }
}

#[test]
fn wdc_keeps_wai_stp_and_bit_ops_rockwell_keeps_bit_ops() {
    let mut w = cpu(CmosModel::Wdc, &[0xDB]);
    w.step().unwrap();
    assert!(w.is_halted(), "WDC STP");
    let mut w = cpu(CmosModel::Wdc, &[0xCB]);
    w.step().unwrap();
    assert!(w.is_waiting(), "WDC WAI");
    for model in [CmosModel::Wdc, CmosModel::Rockwell] {
        let mut c = cpu(model, &[0x87, 0x10]); // SMB0 $10 (bit already set) -> SMB1 next
        c.step().unwrap();
        let mut c2 = cpu(model, &[0x97, 0x10]); // SMB1
        assert_eq!(c2.step().unwrap(), 5);
        assert_eq!(c2.bus.data[0x10], 0x03, "{model:?}");
        let _ = c.pc;
    }
}

#[test]
fn nmos_ignores_the_model() {
    let mut m = FlatMemory::new();
    m.load_at(0x0400, &[0x07, 0x10]); // SLO $10
    let mut c = Cpu::new(m);
    c.cmos_model = CmosModel::Synertek;
    c.pc = 0x0400;
    assert_eq!(c.step().unwrap(), 5);
    assert_eq!(c.pc, 0x0402);
}

/// Step counts equal the traced cycle count for every opcode under each model.
#[test]
fn step_and_trace_agree_on_every_opcode_and_model() {
    for model in [CmosModel::Wdc, CmosModel::Rockwell, CmosModel::Synertek] {
        for op in 0..=255u8 {
            if matches!(op, 0xCB | 0xDB) && model == CmosModel::Wdc {
                continue; // WAI/STP leave the CPU stopped
            }
            let bytes = [op, 0x10, 0x02, 0xEA];
            let mut a = cpu(model, &bytes);
            a.bus.load_at(0xFFFE, &[0x00, 0x05]);
            a.x = 0xF8;
            a.y = 0xF8;
            let n = a.step().unwrap() as usize;
            let mut b = cpu(model, &bytes);
            b.bus.load_at(0xFFFE, &[0x00, 0x05]);
            b.x = 0xF8;
            b.y = 0xF8;
            let t = b.trace_bus_instruction().unwrap();
            assert_eq!(t.len(), n, "{model:?} {op:02X}");
            assert_eq!(a.pc, b.pc, "{model:?} {op:02X}");
        }
    }
}

#[test]
fn jmp_indirect_dead_cycle_rereads_pc_plus_two() {
    let mut c = cpu(CmosModel::Wdc, &[0x6C, 0xFF, 0x12]);
    c.bus.data[0x12FF] = 0x34;
    c.bus.data[0x1300] = 0x56;
    let t: Vec<_> = c
        .trace_bus_instruction()
        .unwrap()
        .iter()
        .map(|b| b.address)
        .collect();
    assert_eq!(t, [0x0400, 0x0401, 0x0402, 0x0402, 0x12FF, 0x1300]);
    assert_eq!(c.pc, 0x5634, "page carry fixed");
    let mut s = cpu(CmosModel::Wdc, &[0x6C, 0xFF, 0x12]);
    assert_eq!(s.step().unwrap(), 6);
}

#[test]
fn zero_page_index_dead_cycles_reread_the_operand_on_the_65c02_only() {
    // LDA $10,X ; LDA ($10,X) ; STA ($10),Y with no page cross.
    for (bytes, dead_at) in [(&[0xB5u8, 0x10][..], 2usize), (&[0xA1, 0x10][..], 2)] {
        let mut c = cpu(CmosModel::Wdc, bytes);
        c.x = 1;
        let t = c.trace_bus_instruction().unwrap();
        assert_eq!(t[dead_at].address, 0x0401);
        let mut m = FlatMemory::new();
        m.load_at(0x0400, bytes);
        let mut n = Cpu::new(m);
        n.pc = 0x0400;
        n.x = 1;
        assert_eq!(n.trace_bus_instruction().unwrap()[dead_at].address, 0x0010);
    }
    let mut c = cpu(CmosModel::Wdc, &[0x91, 0x10]);
    c.bus.data[0x10] = 0x00;
    c.bus.data[0x11] = 0x20;
    c.y = 1;
    let t = c.trace_bus_instruction().unwrap();
    assert_eq!(
        t[4].address, 0x0011,
        "store dead cycle on the pointer high byte"
    );
    assert_eq!((t[5].address, t[5].write), (0x2001, true));
}

#[test]
fn missing_opcode_nops_log_the_real_opcode_byte_in_step_and_trace() {
    for model in [CmosModel::Rockwell, CmosModel::Synertek] {
        for op in 0..=255u8 {
            if model.implements(op) {
                continue;
            }
            let mut a = cpu(model, &[op, 0x10, 0x00]);
            a.step().unwrap();
            let log = a.last_insn().unwrap();
            assert_eq!((log.address, log.opcode, log.cycles), (0x0400, op, 1));
            let mut b = cpu(model, &[op, 0x10, 0x00]);
            let trace = b.trace_bus_instruction().unwrap();
            assert_eq!(trace.len(), 1);
            assert_eq!(b.last_insn().unwrap().opcode, op, "{model:?} {op:02X}");
            // The byte kept in the cycle state is the raw one too.
            let mut c = cpu(model, &[op, 0x10, 0x00]);
            c.step_cycle().unwrap();
            assert_eq!(c.cycle_state()[4], op);
        }
    }
}

#[test]
fn cycle_state_records_the_maker_and_old_states_keep_the_current_one() {
    let models = [CmosModel::Wdc, CmosModel::Rockwell, CmosModel::Synertek];
    for (i, &saved) in models.iter().enumerate() {
        let mut a = cpu(saved, &[0xEA]);
        a.step_cycle().unwrap();
        let state = a.cycle_state();
        assert_eq!(state.len(), 36);
        assert_eq!(state[30], i as u8 + 1);
        for &current in &models {
            let mut b = cpu(current, &[0xEA]);
            b.load_cycle_state(&state).unwrap();
            assert_eq!(b.cmos_model, saved);
            assert_eq!(b.cycle_state(), state);
            // A state from before the maker byte (byte 30 = 0) loads and keeps the model.
            let mut old = state.clone();
            old[30] = 0;
            let mut c = cpu(current, &[0xEA]);
            c.load_cycle_state(&old).unwrap();
            assert_eq!(c.cmos_model, current);
        }
    }
    let mut a = cpu(CmosModel::Wdc, &[0xEA]);
    let before = a.cycle_state();
    for bad in 4..=255u8 {
        let mut s = before.clone();
        s[30] = bad;
        assert!(a.load_cycle_state(&s).is_err());
        assert_eq!(a.cycle_state(), before, "rejected state changes nothing");
    }
}

/// W65C02S data sheet (WDC, March 2000) table 5-5: modes 12b (BBR/BBS), 5b (WAI)
/// and 5c (STP), cycle by cycle.
#[test]
fn bbr_bbs_wai_stp_bus_cycles_follow_the_wdc_table() {
    use mos6502::cycles::BusCycle;
    let addrs = |t: &[BusCycle]| t.iter().map(|c| (c.address, c.write)).collect::<Vec<_>>();
    // BBR0 $10 (bit 0 set → not taken), BBS0 $10 taken, BBS0 taken across a page.
    let mut c = cpu(CmosModel::Wdc, &[0x0F, 0x10, 0x05]);
    let t = c.trace_bus_instruction().unwrap();
    assert_eq!(
        addrs(&t),
        [
            (0x400, false),
            (0x401, false),
            (0x10, false),
            (0x10, false),
            (0x402, false)
        ]
    );
    let mut c = cpu(CmosModel::Wdc, &[0x8F, 0x10, 0x05]);
    let t = c.trace_bus_instruction().unwrap();
    assert_eq!(t.len(), 6);
    assert_eq!(
        &addrs(&t)[..5],
        &addrs(&cpu_trace(&[0x0F, 0x10, 0x05]))[..5]
    );
    assert_eq!(c.pc, 0x0408);
    let mut c = cpu(CmosModel::Wdc, &[0x8F, 0x10, 0x80]);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
    assert_eq!(c.pc, 0x0383);
    for op in [0xCBu8, 0xDB] {
        let mut c = cpu(CmosModel::Wdc, &[op]);
        let t = c.trace_bus_instruction().unwrap();
        assert_eq!(
            addrs(&t),
            [(0x400, false), (0x401, false), (0x401, false)],
            "{op:02X}"
        );
    }
}

fn cpu_trace(bytes: &[u8]) -> Vec<mos6502::cycles::BusCycle> {
    cpu(CmosModel::Wdc, bytes).trace_bus_instruction().unwrap()
}
