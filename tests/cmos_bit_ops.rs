//! Rockwell/WDC RMB/SMB/BBR/BBS over every opcode, bit and byte value, in instruction
//! and bus-cycle stepping; the NMOS meaning of the same bytes is untouched.

use mos6502::disasm::format_at_variant;
use mos6502::{assemble, Cpu, CpuVariant, FlatMemory};

fn cpu(bytes: &[u8], at: u16, zp: u8) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.load_at(at, bytes);
    mem.data[0x42] = zp;
    let mut cpu = Cpu::with_variant(mem, CpuVariant::CMOS);
    cpu.pc = at;
    cpu
}

fn by_cycles(cpu: &mut Cpu<FlatMemory>) -> (u32, Vec<(u16, bool)>) {
    let mut trace = Vec::new();
    loop {
        let c = cpu.step_cycle().unwrap();
        trace.push((c.address, c.write));
        if c.complete {
            return (trace.len() as u32, trace);
        }
    }
}

#[test]
fn rmb_smb_every_bit_every_value() {
    for hi in 0..16u8 {
        let op = (hi << 4) | 0x07;
        let mask = 1u8 << (hi & 7);
        for value in 0..=255u8 {
            let want = if hi < 8 { value & !mask } else { value | mask };
            let mut a = cpu(&[op, 0x42], 0x0400, value);
            let flags = a.p;
            assert_eq!(a.step().unwrap(), 5);
            assert_eq!(
                (a.bus.data[0x42], a.pc, a.p),
                (want, 0x0402, flags),
                "{op:02X} {value:02X}"
            );
            let mut b = cpu(&[op, 0x42], 0x0400, value);
            let (n, trace) = by_cycles(&mut b);
            assert_eq!(n, 5);
            assert_eq!(b.bus.data[0x42], want);
            assert_eq!(
                trace[2..],
                [(0x42, false), (0x42, false), (0x42, true)],
                "one write"
            );
        }
    }
}

#[test]
fn bbr_bbs_every_bit_value_and_page_case() {
    // At $04F0, next instruction $04F3: +$05 stays on page 4, +$20 crosses to page 5,
    // -$80 lands on $0473 (same page).
    for hi in 0..16u8 {
        let op = (hi << 4) | 0x0F;
        for value in 0..=255u8 {
            let set = value & (1 << (hi & 7)) != 0;
            let taken = set == (hi >= 8);
            for (offset, cross) in [(0x05u8, false), (0x20, true), (0x80, false)] {
                let target = 0x04F3u16.wrapping_add_signed(i16::from(offset as i8));
                let cycles = match (taken, cross) {
                    (false, _) => 5,
                    (true, false) => 6,
                    (true, true) => 7,
                };
                let pc = if taken { target } else { 0x04F3 };
                let mut a = cpu(&[op, 0x42, offset], 0x04F0, value);
                assert_eq!(
                    a.step().unwrap(),
                    cycles,
                    "{op:02X} {value:02X} {offset:02X}"
                );
                assert_eq!((a.pc, a.bus.data[0x42]), (pc, value));
                let mut b = cpu(&[op, 0x42, offset], 0x04F0, value);
                assert_eq!(by_cycles(&mut b).0, cycles);
                assert_eq!(b.pc, pc);
            }
        }
    }
}

#[test]
fn backward_branch_across_a_page_costs_seven() {
    // At $0400, next $0403, -$10 is $03F3 on page 3.
    let mut a = cpu(&[0x8F, 0x42, 0xF0], 0x0400, 0x01);
    assert_eq!((a.step().unwrap(), a.pc), (7, 0x03F3));
    let mut b = cpu(&[0x8F, 0x42, 0xF0], 0x0400, 0x01);
    let (n, trace) = by_cycles(&mut b);
    assert_eq!((n, b.pc), (7, 0x03F3));
    assert_eq!(trace[6], (0x04F3, false), "fix-up read on the old page");
}

#[test]
fn nmos_keeps_slo_and_the_assembler_and_disassembler_know_the_forms() {
    let mut nmos = Cpu::new(FlatMemory::new());
    nmos.bus.load_at(0x0400, &[0x07, 0x42]);
    nmos.bus.data[0x42] = 0x81;
    nmos.pc = 0x0400;
    nmos.a = 0x10;
    assert_eq!(nmos.step().unwrap(), 5);
    assert_eq!(
        (nmos.bus.data[0x42], nmos.a),
        (0x02, 0x12),
        "SLO: ASL then ORA"
    );

    let code = assemble(
        "loop:\nrmb3 $42\nsmb7 $42\nbbs7 $42,loop\nbbr0 $10,done\ndone:\n",
        0x0400,
    )
    .unwrap();
    assert_eq!(
        code,
        [0x37, 0x42, 0xF7, 0x42, 0xFF, 0x42, 0xF9, 0x0F, 0x10, 0x00]
    );
    let text = |at: usize| {
        format_at_variant(
            code[at],
            code[at + 1],
            code[at + 2],
            0x0400 + at as u16,
            CpuVariant::CMOS,
        )
        .text
    };
    assert_eq!(text(4), "BBS7 $42,$0400");
    assert_eq!(text(0), "RMB3 $42");
    assert!(
        assemble("bbr0 $142,0\n", 0x0400).is_err(),
        "zp operand above $FF"
    );
    assert!(assemble("bbr0 $10\n", 0x0400).is_err(), "missing target");
    assert!(
        assemble("bbr0 $10,$0500\n", 0x0400).is_err(),
        "out of range"
    );
}
