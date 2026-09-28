//! Handshake, macros, linker script, source map, snapshot v5, and the checks
//! that sit on top of them. Each assertion fails if that behavior is wrong.

use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::cpu::InsnLog;
use mos6502::link::{assemble_object, link_with_script, parse_link_script};
use mos6502::opcodes::{documented_opcodes, Mode};
use mos6502::snapshot::{diff_image, MachineImage};
use mos6502::{assemble, Cpu};

#[test]
fn macros_expand_four_levels_and_reject_the_fifth() {
    let four = "\
.macro d4
    lda #$11
.endmacro
.macro d3
    d4
.endmacro
.macro d2
    d3
.endmacro
.macro d1
    d2
.endmacro
    d1
";
    assert_eq!(assemble(four, 0x8000).unwrap(), vec![0xA9, 0x11]);

    let five = "\
.macro d5
    nop
.endmacro
.macro d4
    d5
.endmacro
.macro d3
    d4
.endmacro
.macro d2
    d3
.endmacro
.macro d1
    d2
.endmacro
    d1
";
    let err = assemble(five, 0).unwrap_err().to_string();
    assert!(err.contains("quinto"), "{err}");

    let recursive = ".macro rec\n    rec\n.endmacro\n    rec\n";
    let err = assemble(recursive, 0).unwrap_err().to_string();
    assert!(err.contains("quinto"), "{err}");
}

#[test]
fn local_labels_inside_nested_macros_do_not_collide() {
    let src = "\
.macro inner
@slot:
    lda #1
.endmacro
.macro outer
@slot:
    nop
    inner
    jmp @slot
.endmacro
    outer
";
    assert_eq!(
        assemble(src, 0x8000).unwrap(),
        vec![0xEA, 0xA9, 0x01, 0x4C, 0x00, 0x80]
    );

    let twice = "\
.macro inner
@here:
    nop
    jmp @here
.endmacro
.macro outer
    inner
    inner
.endmacro
    outer
";
    assert_eq!(
        assemble(twice, 0x8000).unwrap(),
        vec![0xEA, 0x4C, 0x00, 0x80, 0xEA, 0x4C, 0x04, 0x80]
    );
}

#[test]
fn linker_script_places_two_sections_and_keeps_the_source_map() {
    let script = parse_link_script("section code $4000\nsection data $0100\n").unwrap();
    let alpha = assemble_object(
        "mod_a.asm",
        ".section code\n.global alpha\nalpha:\n  lda #1\n  rts\n",
    )
    .unwrap();
    let beta = assemble_object(
        "mod_b.asm",
        ".section data\n.global beta\nbeta:\n  .byte $22\n",
    )
    .unwrap();
    let linked = link_with_script(&[alpha, beta], &script, 0x0000).unwrap();
    assert_eq!(linked.symbols["alpha"], 0x4000);
    assert_eq!(linked.symbols["beta"], 0x0100);
    let code = linked
        .segments
        .iter()
        .find(|segment| segment.address == 0x4000)
        .unwrap();
    let data = linked
        .segments
        .iter()
        .find(|segment| segment.address == 0x0100)
        .unwrap();
    assert_eq!(code.bytes, vec![0xA9, 0x01, 0x60]);
    assert_eq!(data.bytes, vec![0x22]);
    let at_code = linked.line_at(0x4000).unwrap();
    assert_eq!(at_code.file, "mod_a.asm");
    assert_eq!(at_code.line, 4);
    let at_data = linked.line_at(0x0100).unwrap();
    assert_eq!(at_data.file, "mod_b.asm");
    assert_eq!(at_data.line, 4);
}

#[test]
fn relative_branch_past_the_page_is_an_error() {
    let object = assemble_object(
        "far",
        ".section near\n.org $8000\n  beq farlab\n.section far\n.org $9000\n.global farlab\nfarlab:\n  rts\n",
    )
    .unwrap();
    let err = mos6502::link::link(&[object], 0).unwrap_err().to_string();
    assert!(err.contains("alcance"), "{err}");
}

#[test]
fn checksum_program_links_from_the_script_and_runs() {
    let object = assemble_object("checksum.asm", include_str!("../examples/checksum.asm")).unwrap();
    let script = parse_link_script("section code $0600\nsection data $0030\n").unwrap();
    let linked = link_with_script(&[object], &script, 0).unwrap();
    assert_eq!(linked.symbols["start"], 0x0600);
    assert_eq!(linked.symbols["n0"], 0x0030);
    assert_eq!(linked.symbols["total"], 0x0033);
    let mut mem = FlatMemory::new();
    linked.store(&mut mem.data);
    let mut cpu = Cpu::new(mem);
    cpu.pc = linked.symbols["start"];
    cpu.halt_on_brk = true;
    for _ in 0..8 {
        if cpu.is_halted() {
            break;
        }
        cpu.step().unwrap();
    }
    assert!(cpu.is_halted());
    assert_eq!(cpu.a, 0x3C);
    assert_eq!(cpu.bus.data[0x0033], 0x3C);
    assert_eq!(linked.line_at(0x0601).unwrap().file, "checksum.asm");
    assert_eq!(linked.line_at(0x0601).unwrap().line, 6);
}

#[test]
fn snapshot_v5_stores_the_next_listing_and_keeps_older_versions() {
    let mut cpu = Cpu::new(FlatMemory::new());
    cpu.pc = 0x8000;
    cpu.bus.write(0x8000, 0xA9);
    cpu.bus.write(0x8001, 0x42);
    let v2 = MachineImage::capture(&cpu);
    assert_eq!(v2.version, 2);
    assert!(MachineImage::from_json(&v2.to_json())
        .unwrap()
        .next_listing
        .is_empty());
    let v3 = v2.clone().at_scanline(7);
    assert_eq!(MachineImage::from_json(&v3.to_json()).unwrap().scanline, 7);
    let v4 = v3.clone().at_console([9; 16], [4; 256]);
    let loaded4 = MachineImage::from_json(&v4.to_json()).unwrap();
    assert_eq!(loaded4.version, 4);
    assert_eq!(loaded4.palette[0], 9);
    assert!(loaded4.next_listing.is_empty());
    let v5 = loaded4.at_next_instruction();
    assert_eq!(v5.version, 5);
    let json = v5.to_json();
    assert!(json.contains("LDA"), "{json}");
    let back = MachineImage::from_json(&json).unwrap();
    assert_eq!(back.version, 5);
    assert!(back.next_listing.contains("8000"), "{}", back.next_listing);
    assert!(back.next_listing.contains("A9"), "{}", back.next_listing);
    assert!(back.next_listing.contains("LDA"), "{}", back.next_listing);
    assert_eq!(back.palette[0], 9);
    assert_eq!(back.name_table[0], 4);
    let rejected = json.replacen("\"version\":5", "\"version\":99", 1);
    let err = MachineImage::from_json(&rejected).unwrap_err();
    assert_eq!(
        err,
        mos6502::snapshot::SnapshotError::UnsupportedVersion(99)
    );
}

#[test]
fn snapshot_diff_reports_changed_via_latch_bytes() {
    let cpu = Cpu::new(FlatMemory::new());
    let via = mos6502::Via6522::new();
    let before = MachineImage::capture_with(&cpu, &via, &mos6502::acia::Acia6551::new());
    let mut via = via;
    via.write(0x04, 0x34);
    via.write(0x07, 0x12);
    via.write(0x08, 0x56);
    let after = MachineImage::capture_with(&cpu, &via, &mos6502::acia::Acia6551::new());
    let names: Vec<_> = diff_image(&before, &after)
        .into_iter()
        .map(|change| change.name)
        .collect();
    assert_eq!(names, vec!["t1_latch_lo", "t1_latch_hi", "t2_latch_lo"]);
}

fn operand(mode: Mode) -> [u8; 2] {
    match mode {
        Mode::Imp | Mode::Acc => [0x00, 0x00],
        Mode::Imm
        | Mode::Zp
        | Mode::ZpX
        | Mode::ZpY
        | Mode::IndX
        | Mode::IndY
        | Mode::Rel
        | Mode::ZpInd
        | Mode::ZpRel => [0x10, 0x00],
        Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind | Mode::AbsIndX => [0x34, 0x12],
    }
}

fn cpu_at(op: u8, operand: [u8; 2]) -> Cpu<FlatMemory> {
    let pc = 0x2000u16;
    let mut mem = FlatMemory::new();
    for addr in 0..65536u32 {
        mem.data[addr as usize] = (addr as u8).wrapping_mul(3).wrapping_add(0x11);
    }
    mem.data[pc as usize] = op;
    mem.data[pc.wrapping_add(1) as usize] = operand[0];
    mem.data[pc.wrapping_add(2) as usize] = operand[1];
    let mut cpu = Cpu::new(mem);
    cpu.pc = pc;
    cpu.a = 0x40;
    cpu.x = 0x02;
    cpu.y = 0x03;
    cpu.sp = 0xF8;
    cpu.p = mos6502::flags::Status(mos6502::bits::U);
    cpu.cycles = 0;
    cpu
}

#[test]
fn last_instruction_log_matches_step_cycle_for_official_opcodes() {
    let ops = documented_opcodes();
    assert_eq!(ops.len(), 151);
    for op in ops {
        let bytes = operand(op.mode);
        let mut stepped = cpu_at(op.code, bytes);
        let mut cycled = cpu_at(op.code, bytes);
        stepped.step().unwrap();
        let mut n = 0u32;
        loop {
            n += 1;
            let cycle = cycled.step_cycle().unwrap();
            if cycle.complete {
                break;
            }
            assert!(n < 16, "${:02X} did not finish", op.code);
        }
        let want = InsnLog {
            address: 0x2000,
            opcode: op.code,
            cycles: n,
        };
        assert_eq!(stepped.last_insn(), Some(want), "{}", op.mnemonic);
        assert_eq!(cycled.last_insn(), Some(want), "{}", op.mnemonic);
    }
}
