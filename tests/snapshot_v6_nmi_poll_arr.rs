//! Snapshot v6 with the WAI state, NMI polling point, and ARR flags.

use mos6502::bus::FlatMemory;
use mos6502::flags::bits;
use mos6502::opcodes::CpuVariant;
use mos6502::snapshot::MachineImage;
use mos6502::Cpu;

fn cpu(variant: CpuVariant, code: &[u8]) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.data[0x8000..0x8200].fill(0xEA);
    mem.data[0x8000..0x8000 + code.len()].copy_from_slice(code);
    mem.data[0xFFFA] = 0x00;
    mem.data[0xFFFB] = 0xA0;
    let mut cpu = Cpu::with_variant(mem, variant);
    cpu.pc = 0x8000;
    cpu
}

#[test]
fn snapshot_v6_keeps_wai_and_stp_and_older_versions_still_load() {
    let mut c = cpu(CpuVariant::CMOS, &[0xCB]);
    c.step().unwrap();
    let json = MachineImage::capture(&c).with_wait_state(&c).to_json();
    assert!(json.starts_with("{\"version\":6,"));
    assert!(json.contains("\"waiting\":true"));
    let mut back = cpu(CpuVariant::CMOS, &[]);
    MachineImage::from_json(&json)
        .unwrap()
        .restore(&mut back)
        .unwrap();
    assert!(back.is_waiting());
    assert_eq!(back.step().unwrap(), 1, "restored CPU keeps sleeping");

    let mut s = cpu(CpuVariant::CMOS, &[0xDB]);
    s.step().unwrap();
    let json = MachineImage::capture(&s).with_wait_state(&s).to_json();
    let mut back = cpu(CpuVariant::CMOS, &[]);
    MachineImage::from_json(&json)
        .unwrap()
        .restore(&mut back)
        .unwrap();
    assert!(back.is_halted() && !back.is_waiting());

    let v99 = json.replacen("\"version\":6", "\"version\":99", 1);
    assert_eq!(
        MachineImage::from_json(&v99).unwrap_err(),
        mos6502::snapshot::SnapshotError::UnsupportedVersion(99)
    );

    let base = MachineImage::capture(&c);
    for json in [
        base.clone().to_json(),
        base.clone().at_scanline(3).to_json(),
        base.clone().at_console([0; 16], [0; 256]).to_json(),
        base.clone().at_next_instruction().to_json(),
    ] {
        assert!(!json.contains("\"waiting\":"));
        let mut back = cpu(CpuVariant::CMOS, &[]);
        back.set_waiting(true);
        MachineImage::from_json(&json)
            .unwrap()
            .restore(&mut back)
            .unwrap();
        assert!(!back.is_waiting());
    }
}

/// Pulses NMI before cycle `at` (1-based) of the first instruction, then reports
/// whether the next sequence is the NMI entry.
fn nmi_right_after(code: &[u8], at: usize) -> bool {
    // A leading NOP gives the sequencer a polling history; the edge then lands
    // inside `code`, not on a fresh boundary.
    let mut c = cpu(CpuVariant::NMOS, &[[0xEA].as_slice(), code].concat());
    c.trace_bus_instruction().unwrap();
    let mut n = 0;
    loop {
        n += 1;
        if n == at {
            c.pulse_nmi();
        }
        if c.step_cycle().unwrap().complete {
            break;
        }
    }
    c.trace_bus_instruction().unwrap();
    c.pc == 0xA000
}

#[test]
fn nmi_uses_the_irq_polling_point() {
    // LDA $1234 (4 cycles): before cycle 4 is late, before cycle 3 is on time.
    assert!(nmi_right_after(&[0xAD, 0x34, 0x12], 3));
    assert!(!nmi_right_after(&[0xAD, 0x34, 0x12], 4));
    // Taken branch, same page (Z clear): only an edge before cycle 2 is on time.
    assert!(nmi_right_after(&[0xD0, 0x02], 1));
    assert!(!nmi_right_after(&[0xD0, 0x02], 2));
    // The late NMI is still latched and runs after one more instruction.
    let mut c = cpu(CpuVariant::NMOS, &[0xEA, 0xEA]);
    c.step_cycle().unwrap();
    c.pulse_nmi();
    c.step_cycle().unwrap();
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 2);
    assert!(c.nmi_pending());
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
    assert_eq!(c.pc, 0xA000);
}

fn arr(a: u8, imm: u8, carry: bool, decimal: bool) -> Cpu<FlatMemory> {
    let mut c = cpu(CpuVariant::NMOS, &[0x6B, imm]);
    c.a = a;
    c.p.set(bits::C, carry);
    c.p.set(bits::D, decimal);
    let mut t = cpu(CpuVariant::NMOS, &[0x6B, imm]);
    t.a = a;
    t.p.set(bits::C, carry);
    t.p.set(bits::D, decimal);
    c.step().unwrap();
    t.trace_bus_instruction().unwrap();
    assert_eq!(c.registers(), t.registers());
    c
}

#[test]
fn arr_sets_c_from_bit6_and_v_from_bit6_xor_bit5() {
    let c = arr(0x40, 0xFF, false, false);
    assert_eq!(c.a, 0x20);
    assert!(!c.p.get(bits::C) && c.p.get(bits::V));
    let c = arr(0xFF, 0xFF, true, false);
    assert_eq!(c.a, 0xFF);
    assert!(c.p.get(bits::C) && !c.p.get(bits::V) && c.p.get(bits::N));
    // NMOS decimal: $FF -> rotate $7F, low fix $75, high fix $D5 with carry.
    let c = arr(0xFF, 0xFF, false, true);
    assert_eq!(c.a, 0xD5);
    assert!(c.p.get(bits::C) && !c.p.get(bits::V) && !c.p.get(bits::N));
    // The 2A03 ignores D.
    let mut r = cpu(CpuVariant::Ricoh2A03, &[0x6B, 0xFF]);
    r.a = 0xFF;
    r.p.set(bits::D, true);
    r.step().unwrap();
    assert_eq!(r.a, 0x7F);
}
