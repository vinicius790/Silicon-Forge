use mos6502::link::{assemble_object, link, read_object, write_object};
use mos6502::{
    bits, diff_registers, write_listing, Cpu, FlatMemory, MachineImage, MemoryBus, Via6522,
};

fn finish(cpu: &mut Cpu<FlatMemory>) {
    loop {
        if cpu.step_cycle().unwrap().complete {
            break;
        }
    }
}

#[test]
fn assembler_bitwise_and_shift_expressions() {
    let bytes = mos6502::assemble(
        ".byte 1+2<<2, 1<<2+1, 1<<3, $F0&$0F, $0F|$F0, $FF^$0F, $80>>4, <($1234<<1)",
        0,
    )
    .unwrap();
    assert_eq!(bytes, vec![12, 8, 8, 0x00, 0xFF, 0xF0, 0x08, 0x68]);
}

#[test]
fn link_rejects_undefined_duplicate_and_overlap() {
    let missing = assemble_object("m", ".section code\n.org $8000\n  jmp nowhere\n").unwrap();
    let err = link(&[missing], 0).unwrap_err().to_string();
    assert!(
        err.contains("indefinido") && err.contains("nowhere"),
        "{err}"
    );

    let dup = |name: &str, org: &str| {
        assemble_object(
            name,
            &format!(".section code\n.org {org}\n.global sym\nsym:\n  rts\n"),
        )
        .unwrap()
    };
    let err = link(&[dup("a", "$8000"), dup("b", "$9000")], 0)
        .unwrap_err()
        .to_string();
    assert!(err.contains("duplicado") && err.contains("sym"), "{err}");

    let piece = |name: &str, org: &str| {
        assemble_object(name, &format!(".section code\n.org {org}\n  rts\n")).unwrap()
    };
    let err = link(&[piece("a", "$8000"), piece("b", "$8000")], 0)
        .unwrap_err()
        .to_string();
    assert!(err.contains("sobrep"), "{err}");
}

#[test]
fn relocatable_object_roundtrip_places_at_base() {
    let obj = assemble_object("m", ".SECTION code\n.global here\nhere:\n  nop\n").unwrap();
    let bytes = write_object(&obj).unwrap();
    let mut corrupt = bytes.clone();
    corrupt[4] ^= 0x80;
    assert!(read_object(&corrupt).is_err());
    let obj = read_object(&bytes).unwrap();
    assert_eq!(obj.name, "m");
    assert!(obj.sections[0].origin.is_none());
    let linked = link(&[obj], 0x2000).unwrap();
    assert_eq!(linked.symbols["here"], 0x2000);
    assert_eq!(linked.segments[0].bytes, vec![0xEA]);

    let abs = assemble_object("a", ".section code\n.org $8000\n  jmp $9000\n").unwrap();
    let linked = link(&[abs], 0).unwrap();
    assert_eq!(linked.segments[0].bytes, vec![0x4C, 0x00, 0x90]);
}

#[test]
fn assemble_link_load_executes() {
    let main = read_object(&write_object(
        &assemble_object(
            "main",
            ".section code\n.org $8000\n.global start\nstart:\n  clc\n  lda #2\n  adc extra\n  sta answer\n  bne skip\n  nop\nskip:\n  brk\n",
        )
        .unwrap(),
    ).unwrap())
    .unwrap();
    let data = read_object(&write_object(
        &assemble_object(
            "data",
            ".section data\n.org $0010\n.global answer\n.global extra\nanswer:\n  .byte 0\nextra:\n  .byte 3\n",
        )
        .unwrap(),
    ).unwrap())
    .unwrap();
    let linked = link(&[main, data], 0).unwrap();
    assert_eq!(linked.symbols["start"], 0x8000);
    assert_eq!(linked.symbols["answer"], 0x0010);
    assert_eq!(linked.symbols["extra"], 0x0011);
    let code = linked
        .segments
        .iter()
        .find(|seg| seg.address == 0x8000)
        .unwrap();
    assert_eq!(&code.bytes[3..6], &[0x6D, 0x11, 0x00]);
    assert_eq!(&code.bytes[6..9], &[0x8D, 0x10, 0x00]);
    assert_eq!(&code.bytes[9..11], &[0xD0, 0x01]);
    assert_eq!(linked.line_at(0x8001).unwrap().text, "lda #2");

    let mut mem = FlatMemory::new();
    linked.store(&mut mem.data);
    let mut cpu = Cpu::new(mem);
    cpu.pc = linked.symbols["start"];
    cpu.halt_on_brk = true;
    for _ in 0..16 {
        if cpu.is_halted() {
            break;
        }
        cpu.step().unwrap();
    }
    assert!(cpu.is_halted());
    assert_eq!(cpu.a, 5);
    assert_eq!(cpu.bus.data[0x0010], 5);
    assert_eq!(cpu.bus.data[0x0011], 3);
}

#[test]
fn nmi_during_branch_waits_for_the_boundary() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0x10, 0x02, 0xA9, 0x99, 0xEA]);
    mem.set_nmi_vector(0xA000);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    cpu.step_cycle().unwrap();
    assert!(cpu.cycle_step_in_progress());
    cpu.pulse_nmi();
    finish(&mut cpu);
    assert_eq!(cpu.pc, 0x8004);
    assert_eq!(cpu.a, 0);
    assert!(cpu.nmi_pending());
    // Latched after the branch's polling point: the NOP at $8004 runs first.
    finish(&mut cpu);
    assert_eq!(cpu.pc, 0x8005);
    assert!(cpu.nmi_pending());
    finish(&mut cpu);
    assert_eq!(cpu.pc, 0xA000);
    assert!(!cpu.nmi_pending());
    assert_eq!(cpu.bus.peek(0x01FD), 0x80);
    assert_eq!(cpu.bus.peek(0x01FC), 0x05);
}

#[test]
fn irq_is_delayed_one_instruction_after_cli() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0x58, 0xE8, 0xEA]);
    mem.set_irq_vector(0x9000);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    cpu.set_irq_line(true);
    finish(&mut cpu);
    assert_eq!(cpu.pc, 0x8001);
    assert!(!cpu.p.get(bits::I));
    assert_eq!(cpu.x, 0);
    finish(&mut cpu);
    assert_eq!(cpu.x, 1, "INX after CLI must run before IRQ");
    assert_eq!(cpu.pc, 0x8002);
    finish(&mut cpu);
    assert_eq!(cpu.pc, 0x9000);
    assert_eq!(cpu.x, 1);
    assert_eq!(cpu.bus.peek(0x01FD), 0x80);
    assert_eq!(cpu.bus.peek(0x01FC), 0x02);
}

#[test]
fn via_latch_reads_back_and_free_run_reloads_it() {
    let mut via = Via6522::new();
    via.write(0x04, 0x34);
    via.write(0x07, 0x12);
    assert_eq!(via.read(0x06), 0x34);
    assert_eq!(via.read(0x07), 0x12);
    assert_eq!(via.t1_counter(), 0, "latch high does not load the counter");

    via.write(0x0B, 0x40);
    via.write(0x06, 0x03);
    via.write(0x05, 0x00);
    for _ in 0..5 {
        via.tick(); // N+2: three to zero, the roll to $FFFF, the reload.
    }
    assert_eq!(via.t1_counter(), 3);
    assert!(via.t1_running());
    assert!(via.t1_flag());
    assert_eq!(via.read(0x06), 0x03);
    assert!(via.t1_flag(), "reading the latch does not clear the flag");
    via.write(0x06, 0x05);
    assert_eq!(via.t1_counter(), 3);
    for _ in 0..5 {
        via.tick();
    }
    assert_eq!(via.t1_counter(), 5, "free-run reloads the updated latch");
    assert!(via.t1_running());
}

#[test]
fn listing_line_has_address_bytes_and_mnemonic() {
    let mut buf = [0u8; 32];
    buf[0] = 0xA9;
    buf[1] = 0x42;
    let line = write_listing(&buf, 0x8000)
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(line.contains("8000"), "{line}");
    assert!(line.contains("A9") && line.contains("42"), "{line}");
    assert!(line.contains("LDA"), "{line}");
}

#[test]
fn snapshot_diff_names_changed_registers() {
    let mut cpu = Cpu::new(FlatMemory::new());
    cpu.a = 1;
    cpu.pc = 0x1000;
    let before = MachineImage::capture(&cpu);
    cpu.a = 2;
    cpu.pc = 0x1001;
    let after = MachineImage::capture(&cpu);
    let names: Vec<_> = diff_registers(&before, &after)
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, vec!["a", "pc"]);
}
