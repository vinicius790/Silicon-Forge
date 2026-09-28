//! Step-over, step-out, VIA shift, data directives, and byte relocations.

use mos6502::acia::{programmed_frame, Acia6551, OVERRUN, RDRF};
use mos6502::asm::{assemble, assemble_with_symbols};
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::debugger::{Debugger, FlagEdge, LoggingBus, StopReason, Watch};
use mos6502::link::{
    assemble_object, link, link_with_script, parse_link_script, read_object, write_object,
    write_object_file, Object, Reloc, RelocKind,
};
use mos6502::snapshot::diff_image;
use mos6502::{bits, Cpu, IoDevice, MachineImage, Via6522};

#[test]
fn step_over_jsr_returns_to_the_outer_caller() {
    let out = assemble_with_symbols(
        "jsr sub\nback:\nlda #$42\nbrk\nsub:\njsr leaf\nlda #$07\nrts\nleaf:\nlda #$03\nrts\n",
        0x0600,
    )
    .unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(out.origin, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = out.origin;
    let stop = Debugger::default().step_over(&mut cpu, 32).unwrap();
    assert_eq!(stop.reason, StopReason::StepOver);
    assert_eq!(stop.instructions, 6);
    assert_eq!(cpu.pc, out.symbols["back"]);
    assert_eq!(cpu.a, 0x07);
}

#[test]
fn via_shift_out_under_cb1_ignores_phi2() {
    let mut via = Via6522::new();
    via.write(0x0B, 0b111 << 2);
    via.write(0x0A, 0xA5);
    for _ in 0..8 {
        via.tick();
    }
    assert_eq!(via.sr_bits_left(), 8);
    assert_eq!(via.sr_shifted_out(), 0);
    assert_eq!(via.sr(), 0xA5);
    for _ in 0..8 {
        via.set_cb1(false); // shift on the falling edge
        via.set_cb1(true);
    }
    assert_eq!(via.sr_bits_left(), 0);
    assert_eq!(via.sr_shifted_out(), 0xA5);
    assert_eq!(via.sr(), 0);
}

#[test]
fn byte_directive_emits_a_string_and_rejects_a_broken_quote() {
    assert_eq!(
        assemble(".byte \"AB\"\n.db \"AB\"", 0).unwrap(),
        [65, 66, 65, 66]
    );
    let err = assemble(".byte \"AB", 0).unwrap_err();
    assert!(err.to_string().contains("aspas"), "{err}");
}

#[test]
fn absolute_byte_reloc_above_255_is_out_of_range() {
    let wide = assemble_object(
        "w",
        ".section data\n.org $0100\nbig:\n  .byte 0\n.section code\n  .byte big\n",
    )
    .unwrap();
    let err = link(&[wide], 0).unwrap_err().to_string();
    assert!(err.contains("fora de faixa"), "{err}");

    let narrow = assemble_object(
        "n",
        ".section data\n.org $0010\nsmall:\n  .byte 0\n.section code\n  .byte small\n",
    )
    .unwrap();
    let linked = link(&[narrow], 0).unwrap();
    let code = linked
        .segments
        .iter()
        .find(|segment| segment.address == 0)
        .unwrap();
    assert_eq!(code.bytes, [0x10]);
}

#[test]
fn step_out_returns_to_the_callers_caller_not_the_inner_rts() {
    let out = assemble_with_symbols(
        "jsr mid\nland:\nlda #$11\nbrk\nmid:\njsr inner\nlda #$22\nrts\ninner:\njsr leaf\nlda #$33\nrts\nleaf:\nlda #$44\nrts\n",
        0x0600,
    )
    .unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(out.origin, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = out.origin;
    while cpu.pc != out.symbols["inner"] {
        cpu.step().unwrap();
    }
    let stop = Debugger::default().step_out(&mut cpu, 32).unwrap();
    assert_eq!(stop.reason, StopReason::StepOut);
    assert_eq!(stop.instructions, 7);
    assert_eq!(cpu.pc, out.symbols["land"]);
    assert_eq!(cpu.a, 0x22);
}

#[test]
fn via_shift_in_on_cb1_rising_edges_ignores_phi2() {
    let mut via = Via6522::new();
    via.write(0x0B, 0b011 << 2);
    via.write(0x0A, 0x00);
    for _ in 0..8 {
        via.tick();
    }
    assert_eq!(via.sr_bits_left(), 8);
    assert_eq!(via.sr(), 0);
    via.set_cb1(false);
    assert_eq!(
        via.sr_bits_left(),
        8,
        "a falling edge does not shift ACR 011"
    );
    for bit in [1, 0, 1, 0, 0, 1, 0, 1] {
        via.set_sr_in_bit(bit == 1);
        via.set_cb1(true);
        via.set_cb1(false);
    }
    assert_eq!(via.sr(), 0xA5);
    assert_eq!(via.sr_bits_left(), 0);
    assert_eq!(via.peek(0x0D) & 0x04, 0x04);
}

#[test]
fn word_rejects_a_two_character_string_and_emits_label_low_then_high() {
    let err = assemble(".word \"AB\"", 0).unwrap_err();
    assert!(err.to_string().contains("string"), "{err}");
    assert_eq!(
        assemble("here:\n.word here\n", 0x1234).unwrap(),
        [0x34, 0x12]
    );
}

#[test]
fn in_range_byte_reloc_is_not_rolled_back_when_another_is_out_of_range() {
    let obj = assemble_object(
        "m",
        ".section data\n.org $0010\nsmall:\n  .byte 0\n.section far\n.org $0200\nbig:\n  .byte 0\n.section code\n  .byte small\n  .byte big\n",
    )
    .unwrap();
    let err = link(&[obj], 0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("fora de faixa"), "{text}");
    assert!(text.contains("big"), "{text}");
    let code = err
        .patched
        .iter()
        .find(|segment| segment.address == 0)
        .expect("code");
    assert_eq!(code.bytes, [0x10, 0x00]);
}

#[test]
fn step_out_outside_a_jsr_callee_does_not_execute() {
    let program = assemble("lda #$42\nbrk\n", 0x0600).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(0x0600, &program);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x0600;
    let cycles = cpu.cycles;
    let err = Debugger::default().step_out(&mut cpu, 32).unwrap_err();
    assert!(err.contains("JSR"), "{err}");
    assert_eq!(cpu.pc, 0x0600);
    assert_eq!(cpu.a, 0);
    assert_eq!(cpu.cycles, cycles);
}

#[test]
fn two_good_loopback_bytes_replace_and_rdrf_clears_on_read() {
    let mut acia = Acia6551::new();
    acia.write(2, 0x0B); // DTR on: the receiver is enabled
    acia.set_loopback(true);
    let first = programmed_frame(0x11, 0, 0);
    let second = programmed_frame(0x22, 0, 0);
    acia.loopback_frame(0x11, first.parity);
    acia.loopback_frame(0x22, second.parity);
    assert_eq!(acia.peek(0), 0x22);
    assert_eq!(acia.peek(1) & RDRF, RDRF);
    assert_eq!(acia.read(1) & RDRF, RDRF);
    assert_eq!(
        acia.peek(1) & RDRF,
        RDRF,
        "status read does not retire the byte"
    );
    assert_eq!(acia.read(0), 0x22);
    assert_eq!(acia.peek(1) & RDRF, 0);
}

#[test]
fn empty_byte_string_emits_nothing() {
    assert_eq!(assemble(".byte \"\"\n", 0x0200).unwrap(), Vec::<u8>::new());
}

#[test]
fn linker_script_missing_section_names_it_and_emits_no_image() {
    let object = assemble_object("m", ".section code\n  nop\n").unwrap();
    let script = parse_link_script("section ghost $2000\n").unwrap();
    let err = link_with_script(&[object], &script, 0x8000).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("ghost"), "{text}");
    let image_bytes: usize = err.patched.iter().map(|segment| segment.bytes.len()).sum();
    assert_eq!(image_bytes, 0);
}

#[test]
fn run_until_halt_cap_zero_executes_nothing() {
    let mut mem = FlatMemory::new();
    mem.write(0x8000, 0xA9);
    mem.write(0x8001, 0x99);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let stop = Debugger::default().run_until_halt(&mut cpu, 0).unwrap();
    assert_eq!(stop.reason, StopReason::CycleCap);
    assert_eq!(stop.instructions, 0);
    assert_eq!(cpu.pc, 0x8000);
    assert_eq!(cpu.a, 0);
    assert_eq!(cpu.cycles, 0);
}

#[test]
fn status_read_does_not_clear_receiver_byte() {
    let mut acia = Acia6551::new();
    acia.write(2, 0x0B); // DTR on: the receiver is enabled
    acia.push_rx(0x5A);
    assert_eq!(acia.peek(1) & RDRF, RDRF);
    let _ = acia.read(1);
    assert_eq!(acia.peek(1) & RDRF, RDRF, "status read must leave RDRF");
    assert_eq!(
        acia.peek(0),
        0x5A,
        "status read must leave the receiver byte"
    );
    assert_eq!(acia.read(0), 0x5A);
    assert_eq!(acia.peek(1) & RDRF, 0);
    assert_eq!(acia.read(0), 0);
}

#[test]
fn word_empty_string_is_error_unlike_empty_byte_string() {
    let err = assemble(".word \"\"\n", 0).unwrap_err();
    assert!(err.to_string().contains("string"), "{err}");
    assert_eq!(assemble(".byte \"\"\n", 0).unwrap(), Vec::<u8>::new());
}

#[test]
fn in_range_relative_reloc_stays_patched_when_another_is_out_of_range() {
    let obj = assemble_object(
        "m",
        ".section code\n.org $8000\n  beq nearlab\n  beq farlab\nnearlab:\n  nop\n.section far\n.org $9000\n.global farlab\nfarlab:\n  rts\n",
    )
    .unwrap();
    let err = link(&[obj], 0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("alcance"), "{text}");
    assert!(text.contains("farlab"), "{text}");
    let code = err
        .patched
        .iter()
        .find(|segment| segment.address == 0x8000)
        .expect("code");
    assert_eq!(code.bytes[0], 0xF0);
    assert_eq!(
        code.bytes[1], 0x02,
        "in-range BEQ displacement must stay patched"
    );
    assert_eq!(code.bytes[2], 0xF0);
    assert_eq!(
        code.bytes[3], 0x00,
        "out-of-range site must not get a wrong displacement"
    );
}

#[test]
fn step_over_jsr_that_never_returns_hits_the_cycle_cap() {
    let out =
        assemble_with_symbols("jsr hang\nback:\nlda #$99\nbrk\nhang:\njmp hang\n", 0x0600).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(out.origin, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = out.origin;
    let stop = Debugger::default().step_over(&mut cpu, 16).unwrap();
    assert_eq!(stop.reason, StopReason::CycleCap);
    assert_ne!(stop.reason, StopReason::StepOver);
    assert_ne!(cpu.pc, out.symbols["back"]);
    assert_eq!(cpu.a, 0);
}

#[test]
fn third_unread_loopback_byte_sets_overrun_and_keeps_the_second() {
    let mut acia = Acia6551::new();
    acia.write(2, 0x0B); // DTR on: the receiver is enabled
    acia.set_loopback(true);
    for byte in [0x11, 0x22, 0x33] {
        let frame = programmed_frame(byte, 0, 0);
        acia.loopback_frame(byte, frame.parity);
    }
    assert_eq!(acia.peek(1) & OVERRUN, OVERRUN);
    assert_eq!(acia.peek(0), 0x22);
    assert_ne!(acia.peek(0), 0x33);
    assert_eq!(acia.read(0), 0x22);
    assert_eq!(acia.peek(1) & OVERRUN, 0);
}

#[test]
fn byte_directive_accepts_quoted_elements_and_requires_commas() {
    assert_eq!(assemble(".byte \"A\",\"B\"\n", 0).unwrap(), [65, 66]);
    let err = assemble(".byte \"A\" \"B\"\n", 0).unwrap_err();
    assert!(err.to_string().contains("vírgula"), "{err}");
}

#[test]
fn linker_script_duplicate_section_is_an_error_without_an_image() {
    let object = assemble_object("m", ".section code\n  nop\n").unwrap();
    let script = parse_link_script("section code $2000\nsection code $3000\n").unwrap();
    let err = link_with_script(&[object], &script, 0x8000).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("code"), "{text}");
    let image_bytes: usize = err.patched.iter().map(|segment| segment.bytes.len()).sum();
    assert_eq!(image_bytes, 0);
}

#[test]
fn character_constant_immediate_emits_ascii_and_rejects_unclosed_quote() {
    assert_eq!(assemble("LDA #'A'\n", 0).unwrap(), [0xA9, 0x41]);
    let err = assemble("LDA #'A\n", 0).unwrap_err();
    assert!(err.to_string().contains("aspas"), "{err}");
}

#[test]
fn read_watch_stops_on_read_and_not_on_write_to_same_address() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xAD, 0x30, 0x00, 0x8D, 0x30, 0x00]);
    mem.data[0x0030] = 0x55;
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch(0x0030, Watch::Read);
    assert_eq!(dbg.step_watched(&mut cpu).unwrap(), StopReason::WatchRead);
    assert_eq!(cpu.a, 0x55);
    assert_eq!(
        dbg.step_watched(&mut cpu).unwrap(),
        StopReason::InstructionLimit,
        "write to a read-watched address must not stop"
    );
    assert_eq!(cpu.bus.peek(0x0030), 0x55);
}

#[test]
fn snapshot_v5_rejects_a_non_string_listing_and_still_loads_valid_v5() {
    let mut image = MachineImage::capture(&Cpu::new(FlatMemory::new()));
    image.version = 5;
    image.next_listing = "8000  A9 41  LDA #$41".into();
    let json = image.to_json();
    let back = MachineImage::from_json(&json).unwrap();
    assert_eq!(back.version, 5);
    assert!(back.next_listing.contains("LDA"), "{}", back.next_listing);
    let bad = json.replacen(
        "\"next_listing\":\"8000  A9 41  LDA #$41\"",
        "\"next_listing\":99",
        1,
    );
    let err = MachineImage::from_json(&bad).unwrap_err();
    assert_eq!(err, mos6502::snapshot::SnapshotError::WrongType("string"));
}

#[test]
fn character_plus_one_immediate_emits_a9_42_and_keeps_precedence() {
    assert_eq!(assemble("LDA #'A'+1\n", 0).unwrap(), [0xA9, 0x42]);
    assert_eq!(assemble("LDA #1+2*3\n", 0).unwrap(), [0xA9, 0x07]);
}

#[test]
fn reading_ifr_does_not_clear_timer1_writing_one_does() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xC0);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.tick();
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.read(0x0D) & 0x40, 0x40);
    assert_eq!(via.read(0x0D) & 0x40, 0x40);
    assert!(via.t1_flag());
    via.write(0x0D, 0x40);
    assert_eq!(via.read(0x0D) & 0x40, 0);
    assert!(!via.t1_flag());
}

#[test]
fn read_watch_does_not_stop_when_a_different_address_is_read() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xAD, 0x31, 0x00, 0xAD, 0x30, 0x00]);
    mem.data[0x0030] = 0x11;
    mem.data[0x0031] = 0x22;
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch(0x0030, Watch::Read);
    assert_eq!(
        dbg.step_watched(&mut cpu).unwrap(),
        StopReason::InstructionLimit
    );
    assert_eq!(cpu.a, 0x22);
    assert_eq!(dbg.step_watched(&mut cpu).unwrap(), StopReason::WatchRead);
    assert_eq!(cpu.a, 0x11);
}

#[test]
fn object_writer_rejects_section_name_longer_than_16_bytes() {
    let mut object = assemble_object("m", ".section code\n nop\n").unwrap();
    let path = std::path::Path::new("/tmp/mos6502-section-name-cap.obj");
    let _ = std::fs::remove_file(path);
    object.sections[0].name = "s".repeat(17);
    let err = write_object_file(&object, path).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("16"), "{text}");
    assert!(text.contains("seção"), "{text}");
    assert!(!path.exists());
    assert!(write_object(&object).is_err());
    object.sections[0].name = "s".repeat(16);
    write_object_file(&object, path).unwrap();
    assert!(!std::fs::read(path).unwrap().is_empty());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn two_character_constants_do_not_add_but_char_plus_one_still_emits_a9_42() {
    let err = assemble("LDA #'A'+'B'\n", 0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("não se somam"), "{text}");
    assert_eq!(assemble("LDA #'A'+1\n", 0).unwrap(), [0xA9, 0x42]);
}

#[test]
fn writing_zero_to_timer1_ifr_bit_does_not_clear_it() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xC0);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.tick();
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.read(0x0D) & 0x40, 0x40);
    via.write(0x0D, 0x00);
    assert_eq!(via.read(0x0D) & 0x40, 0x40);
    assert!(via.t1_flag());
}

#[test]
fn second_read_watch_stops_and_names_that_address() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xAD, 0x31, 0x00]);
    mem.data[0x0030] = 0x11;
    mem.data[0x0031] = 0x22;
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch(0x0030, Watch::Read);
    dbg.watch(0x0031, Watch::Read);
    assert_eq!(dbg.step_watched(&mut cpu).unwrap(), StopReason::WatchRead);
    assert_eq!(dbg.hit_addr, Some(0x0031));
    assert_ne!(dbg.hit_addr, Some(0x0030));
    assert_eq!(cpu.a, 0x22);
}

#[test]
fn linker_empty_section_list_is_an_error_without_an_image() {
    let object = Object {
        name: "m".into(),
        sections: Vec::new(),
        symbols: Vec::new(),
        notes: Vec::new(),
    };
    let err = link(&[object], 0x8000).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("lista de seções vazia"), "{text}");
    let image_bytes: usize = err.patched.iter().map(|segment| segment.bytes.len()).sum();
    assert_eq!(image_bytes, 0);
    let err = link(&[], 0x8000).unwrap_err();
    assert!(err.patched.is_empty());
}

#[test]
fn empty_character_immediate_is_an_error_and_letter_still_emits_a9_41() {
    let err = assemble("LDA #''\n", 0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("vazio"), "{text}");
    assert_eq!(assemble("LDA #'A'\n", 0).unwrap(), [0xA9, 0x41]);
}

#[test]
fn timer1_sets_irq_summary_and_clearing_the_timer_clears_it_when_alone() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xC0);
    via.write(0x04, 0x01);
    via.write(0x05, 0x00);
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    let ifr = via.read(0x0D);
    assert_eq!(ifr & 0x40, 0x40, "timer bit");
    assert_eq!(ifr & 0x80, 0x80, "IRQ summary");
    assert!(via.t1_flag());
    via.write(0x0D, 0x40);
    let ifr = via.read(0x0D);
    assert_eq!(ifr & 0x40, 0);
    assert_eq!(ifr & 0x80, 0, "no other interrupt pending");
    assert!(!via.t1_flag());

    via.write(0x0E, 0xE0);
    via.write(0x04, 0x01);
    via.write(0x05, 0x00);
    via.write(0x08, 0x01);
    via.write(0x09, 0x00);
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.read(0x0D) & 0xE0, 0xE0);
    via.write(0x0D, 0x40);
    let ifr = via.read(0x0D);
    assert_eq!(ifr & 0x40, 0);
    assert_eq!(ifr & 0x20, 0x20);
    assert_eq!(ifr & 0x80, 0x80, "other interrupt still pending");
}

#[test]
fn read_with_both_watches_stops_for_read_not_write() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xAD, 0x30, 0x00]);
    mem.data[0x0030] = 0x55;
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch(0x0030, Watch::Write);
    dbg.watch(0x0030, Watch::Read);
    let reason = dbg.step_watched(&mut cpu).unwrap();
    assert_eq!(reason, StopReason::WatchRead);
    assert_ne!(reason, StopReason::WatchWrite);
}

#[test]
fn object_reader_rejects_a_zero_length_name_and_returns_no_section() {
    let mut object = assemble_object("m", ".section code\n nop\n").unwrap();
    assert_eq!(object.sections.len(), 1);
    object.sections[0].name.clear();
    let err = read_object(&write_object(&object).unwrap()).unwrap_err();
    let text = err.to_string();
    assert!(text.contains('0'), "{text}");
    object.sections[0].name = "code".into();
    let back = read_object(&write_object(&object).unwrap()).unwrap();
    assert_eq!(back.sections.len(), 1);
    assert_eq!(back.sections[0].name, "code");
}

#[test]
fn character_longer_than_one_is_an_error_and_letter_still_emits_a9_41() {
    let err = assemble("LDA #'AB'\n", 0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("maior que um"), "{text}");
    assert!(!text.contains("sem fechar"), "{text}");
    assert_eq!(assemble("LDA #'A'\n", 0).unwrap(), [0xA9, 0x41]);
}

#[test]
fn timer2_sets_its_ifr_bit_and_summary_and_clearing_it_keeps_timer1() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xA0);
    via.write(0x08, 0x01);
    via.write(0x09, 0x00);
    via.tick();
    via.tick(); // T2 flags on the roll to $FFFF: N+1 ticks.
    let ifr = via.read(0x0D);
    assert_eq!(ifr & 0x20, 0x20, "timer-2 bit");
    assert_eq!(ifr & 0x80, 0x80, "IRQ summary");
    via.write(0x0E, 0xE0);
    via.write(0x04, 0x01);
    via.write(0x05, 0x00);
    via.write(0x08, 0x01);
    via.write(0x09, 0x00);
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.read(0x0D) & 0xE0, 0xE0);
    via.write(0x0D, 0x20);
    let ifr = via.read(0x0D);
    assert_eq!(ifr & 0x20, 0);
    assert_eq!(ifr & 0x40, 0x40);
    assert_eq!(ifr & 0x80, 0x80, "timer-1 summary still pending");
}

#[test]
fn removing_a_read_watch_does_not_stop_on_a_later_read() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xAD, 0x30, 0x00]);
    mem.data[0x0030] = 0x55;
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch(0x0030, Watch::Read);
    dbg.clear_watch(0x0030);
    let reason = dbg.step_watched(&mut cpu).unwrap();
    assert_ne!(reason, StopReason::WatchRead);
    assert_eq!(reason, StopReason::InstructionLimit);
    assert_eq!(cpu.a, 0x55);
}

#[test]
fn linker_script_address_overlapping_a_placed_range_emits_no_image() {
    let object =
        assemble_object("m", ".section code\n  nop\n  nop\n.section data\n  nop\n").unwrap();
    let script = parse_link_script("section code $2000\nsection data $2001\n").unwrap();
    let err = link_with_script(&[object], &script, 0x8000).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("sobrep"), "{text}");
    let image_bytes: usize = err.patched.iter().map(|segment| segment.bytes.len()).sum();
    assert_eq!(image_bytes, 0);
}

#[test]
fn comment_after_character_constant_still_emits_a9_41() {
    assert_eq!(assemble("LDA #'A' ; nota\n", 0).unwrap(), [0xA9, 0x41]);
}

#[test]
fn ca1_active_edge_sets_ifr_bit1_and_summary_clear_keeps_timer1() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xC0 | 0x02);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.tick();
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.peek(0x0D) & 0x40, 0x40);
    via.set_ca1(true);
    via.set_ca1(false);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x02, 0x02);
    assert_eq!(ifr & 0x80, 0x80);
    via.write(0x0D, 0x02);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x02, 0);
    assert_eq!(ifr & 0x40, 0x40);
    assert_eq!(ifr & 0x80, 0x80);
}

#[test]
fn break_when_written_does_not_stop_on_a_different_address() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x02, 0x85, 0x11, 0xA9, 0x02, 0x85, 0x10]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.reset();
    let mut dbg = Debugger::default();
    dbg.break_when_written(0x0010, 0x02);
    assert_eq!(
        dbg.step_watched(&mut cpu).unwrap(),
        StopReason::InstructionLimit
    );
    assert_eq!(
        dbg.step_watched(&mut cpu).unwrap(),
        StopReason::InstructionLimit
    );
    assert_eq!(cpu.bus.inner.peek(0x0011), 0x02);
    assert_eq!(
        dbg.step_watched(&mut cpu).unwrap(),
        StopReason::InstructionLimit
    );
    assert_eq!(dbg.step_watched(&mut cpu).unwrap(), StopReason::WatchValue);
}

#[test]
fn object_writer_rejects_two_sections_with_the_same_name() {
    let mut object = assemble_object("m", ".section code\n nop\n.section data\n nop\n").unwrap();
    let path = std::path::Path::new("/tmp/mos6502-dup-section.obj");
    let _ = std::fs::remove_file(path);
    object.sections[1].name = object.sections[0].name.clone();
    let err = write_object_file(&object, path).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("duplicada"), "{text}");
    assert!(text.contains("seção"), "{text}");
    assert!(!path.exists());
    assert!(write_object(&object).is_err());
    object.sections[1].name = "data".into();
    write_object_file(&object, path).unwrap();
    assert!(!std::fs::read(path).unwrap().is_empty());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn character_is_not_an_address_but_prior_character_cases_stay() {
    let err = assemble("STA 'A'\n", 0).unwrap_err();
    assert!(err.to_string().contains("endereço"), "{err}");
    let sum = assemble("LDA #'A'+'B'\n", 0).unwrap_err();
    assert!(sum.to_string().contains("somam"), "{sum}");
    assert_eq!(assemble("LDA #'A' ; nota\n", 0).unwrap(), [0xA9, 0x41]);
}

#[test]
fn cb1_active_edge_sets_ifr_bit4_and_summary_clear_keeps_timer1() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xC0 | 0x10);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.tick();
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.peek(0x0D) & 0x40, 0x40);
    via.set_cb1(true);
    via.set_cb1(false);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x10, 0x10);
    assert_eq!(ifr & 0x80, 0x80);
    via.write(0x0D, 0x10);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x10, 0);
    assert_eq!(ifr & 0x40, 0x40);
    assert_eq!(ifr & 0x80, 0x80);
}

#[test]
fn removing_break_when_written_does_not_stop_on_a_later_write() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x02, 0x85, 0x10]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.reset();
    let mut dbg = Debugger::default();
    dbg.break_when_written(0x0010, 0x02);
    dbg.clear_break_when_written();
    assert_eq!(
        dbg.step_watched(&mut cpu).unwrap(),
        StopReason::InstructionLimit
    );
    let reason = dbg.step_watched(&mut cpu).unwrap();
    assert_ne!(reason, StopReason::WatchValue);
    assert_eq!(reason, StopReason::InstructionLimit);
    assert_eq!(cpu.bus.inner.peek(0x0010), 0x02);
}

#[test]
fn relative_reloc_to_a_symbol_in_another_section_in_range_is_patched() {
    let code = assemble_object("code", ".section code\n.org $8000\n  beq farlab\n").unwrap();
    let near = assemble_object(
        "near",
        ".section near\n.org $8004\n.global farlab\nfarlab:\n  nop\n",
    )
    .unwrap();
    let linked = link(&[code, near], 0).unwrap();
    let seg = linked
        .segments
        .iter()
        .find(|segment| segment.address == 0x8000)
        .expect("code");
    assert_eq!(seg.bytes, [0xF0, 0x02]);
    assert_eq!(linked.symbols.get("farlab"), Some(&0x8004));
}

#[test]
fn jmp_character_is_not_an_address_and_immediate_still_emits_a9_41() {
    let err = assemble("JMP 'A'\n", 0).unwrap_err();
    assert!(err.to_string().contains("endereço"), "{err}");
    assert_eq!(assemble("LDA #'A'\n", 0).unwrap(), [0xA9, 0x41]);
}

#[test]
fn ca2_input_handshake_sets_ifr_bit_and_summary_clear_keeps_timer1() {
    let mut via = Via6522::new();
    via.write(0x0C, 0x00);
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xC0 | 0x01);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.tick();
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.peek(0x0D) & 0x40, 0x40);
    via.set_ca2(false);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x01, 0x01, "CA2 input-handshake bit");
    assert_eq!(ifr & 0x80, 0x80, "IRQ summary");
    via.write(0x0D, 0x01);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x01, 0);
    assert_eq!(ifr & 0x40, 0x40);
    assert_eq!(ifr & 0x80, 0x80, "timer-1 summary still pending");
}

#[test]
fn write_outside_pc_range_stops_as_write_not_as_pc_range() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0x85, 0x10]);
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_pc_range(0x8002, 0x8004);
    dbg.watch(0x0010, Watch::Write);
    let reason = dbg.step_watched(&mut cpu).unwrap();
    assert_eq!(reason, StopReason::WatchWrite);
    assert_ne!(reason, StopReason::PcRange);
    assert_eq!(dbg.hit_addr, Some(0x0010));
    assert_eq!(cpu.pc, 0x8002);
}

#[test]
fn object_reader_rejects_reloc_offset_past_section_bytes() {
    let mut object = assemble_object("m", ".section code\n nop\n.section data\n nop\n").unwrap();
    let len = object.sections[1].bytes.len();
    assert!(len > 0);
    object.sections[1].relocs.push(Reloc {
        offset: len as u16 + 1,
        symbol: "sym".into(),
        kind: RelocKind::Relative,
    });
    let bytes = write_object(&object).unwrap();
    let parsed = read_object(&bytes);
    let err = parsed.as_ref().expect_err("reloc past section");
    let text = err.to_string();
    assert!(text.contains("seção") || text.contains("reloc"), "{text}");
    assert!(parsed.ok().map(|got| got.sections).is_none());
    object.sections[1].relocs.clear();
    let back = read_object(&write_object(&object).unwrap()).unwrap();
    assert_eq!(back.sections.len(), 2);
}

#[test]
fn lo_of_a_character_is_not_a_16_bit_label() {
    let err = assemble("LDA #lo('A')\n", 0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("caractere"), "{text}");
    assert!(text.contains("16"), "{text}");
    assert_eq!(
        assemble("rot = $1234\nLDA #hi(rot)\n", 0).unwrap(),
        [0xA9, 0x12]
    );
    assert_eq!(
        assemble("rot = $1234\nLDA #lo(rot)\n", 0).unwrap(),
        [0xA9, 0x34]
    );
}

#[test]
fn cb2_output_handshake_sets_ifr_bit_and_summary_clear_keeps_timer1() {
    let mut via = Via6522::new();
    via.write(0x0C, 0x80);
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xC0 | 0x08);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.tick();
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.peek(0x0D) & 0x40, 0x40);
    via.write(0x00, 0x01);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x08, 0x08, "CB2 output-handshake bit");
    assert_eq!(ifr & 0x80, 0x80, "IRQ summary");
    via.write(0x0D, 0x08);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x08, 0);
    assert_eq!(ifr & 0x40, 0x40);
    assert_eq!(ifr & 0x80, 0x80, "timer-1 summary still pending");
}

#[test]
fn pc_range_entry_without_a_write_does_not_report_write_watch() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xEA]);
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_pc_range(0x8001, 0x8002);
    dbg.watch(0x0010, Watch::Write);
    dbg.hit_addr = Some(0x0010);
    let reason = dbg.step_watched(&mut cpu).unwrap();
    assert_eq!(reason, StopReason::PcRange);
    assert_ne!(reason, StopReason::WatchWrite);
    assert_eq!(dbg.hit_addr, None);
    assert_eq!(cpu.pc, 0x8001);
}

#[test]
fn linker_gap_byte_is_zero_and_second_section_keeps_its_opcode() {
    let object = assemble_object(
        "m",
        ".section code\n.org $3000\n  lda #$01\n.section data\n.org $3003\n  inx\n",
    )
    .unwrap();
    let linked = link(&[object], 0).unwrap();
    let mut image = vec![0u8; 0x3010];
    linked.store(&mut image);
    assert_eq!(image[0x3000], 0xA9);
    assert_eq!(image[0x3001], 0x01);
    assert_eq!(image[0x3002], 0);
    assert_eq!(image[0x3003], 0xE8);
    assert_ne!(image[0x3003], image[0x3000]);
}

#[test]
fn hi_of_a_character_is_an_error_and_hex_still_emits_a9_12() {
    let err = assemble("LDA #hi('A')\n", 0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("caractere"), "{text}");
    assert!(text.contains("16"), "{text}");
    assert_eq!(assemble("LDA #hi($1234)\n", 0).unwrap(), [0xA9, 0x12]);
}

#[test]
fn cb1_shift_complete_sets_ifr_bit_and_summary_clear_keeps_timer1() {
    let mut via = Via6522::new();
    via.write(0x0B, 0b111 << 2);
    via.write(0x0E, 0xC0 | 0x04);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.tick();
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.peek(0x0D) & 0x40, 0x40);
    via.write(0x0A, 0xA5);
    for _ in 0..8 {
        via.set_cb1(false); // shift on the falling edge
        via.set_cb1(true);
    }
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x04, 0x04, "shift IFR bit");
    assert_eq!(ifr & 0x80, 0x80, "IRQ summary");
    via.write(0x0D, 0x04);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x04, 0);
    assert_eq!(ifr & 0x40, 0x40);
    assert_eq!(ifr & 0x80, 0x80, "timer-1 summary still pending");
}

#[test]
fn second_pc_range_stops_with_its_bounds_not_the_first() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xEA; 8]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_pc_range(0x8100, 0x8102);
    dbg.add_pc_range(0x8002, 0x8003);
    let stop = dbg.run(&mut cpu, 10).unwrap();
    assert_eq!(stop.reason, StopReason::PcRange);
    assert_eq!(cpu.pc, 0x8002);
    assert_eq!(dbg.hit_range, Some((0x8002, 0x8003)));
    assert_ne!(dbg.hit_range, Some((0x8100, 0x8102)));
}

#[test]
fn snapshot_diff_reports_name_table_byte_and_not_a_register() {
    let cpu = Cpu::new(FlatMemory::new());
    let before = MachineImage::capture(&cpu).at_console([0; 16], [0; 256]);
    let mut names = [0u8; 256];
    names[3] = 0xAB;
    let after = MachineImage::capture(&cpu).at_console([0; 16], names);
    let changes = diff_image(&before, &after);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].name, "name_table");
    assert_eq!(changes[0].before, 0);
    assert_eq!(changes[0].after, 0xAB);
    assert_ne!(changes[0].name, "a");
    assert_ne!(changes[0].name, "pc");
}

#[test]
fn lo_of_hex_emits_a9_34_and_lo_of_char_stays_an_error() {
    assert_eq!(assemble("LDA #lo($1234)\n", 0).unwrap(), [0xA9, 0x34]);
    let err = assemble("LDA #lo('A')\n", 0).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("caractere"), "{text}");
    assert!(text.contains("16"), "{text}");
}

#[test]
fn cb1_shift_in_complete_sets_ifr_and_clear_keeps_timer1() {
    let mut via = Via6522::new();
    via.write(0x0B, 0b011 << 2);
    via.write(0x0E, 0xC0 | 0x04);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.tick();
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.peek(0x0D) & 0x40, 0x40);
    via.write(0x0A, 0x00);
    via.set_sr_in_bit(true);
    for _ in 0..8 {
        via.set_cb1(false);
        via.set_cb1(true);
    }
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x04, 0x04, "shift IFR bit");
    assert_eq!(ifr & 0x80, 0x80, "IRQ summary");
    via.write(0x0D, 0x04);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x04, 0);
    assert_eq!(ifr & 0x40, 0x40);
    assert_eq!(ifr & 0x80, 0x80, "timer-1 summary still pending");
}

#[test]
fn single_address_pc_range_stops_only_on_that_pc() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xEA; 6]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_pc_range(0x8002, 0x8002);
    let stop = dbg.run(&mut cpu, 6).unwrap();
    assert_eq!(stop.reason, StopReason::PcRange);
    assert_eq!(cpu.pc, 0x8002);
    assert_eq!(dbg.hit_range, Some((0x8002, 0x8002)));
    cpu.pc = 0x8003;
    let stop = dbg.run(&mut cpu, 2).unwrap();
    assert_eq!(stop.reason, StopReason::InstructionLimit);
    assert_ne!(stop.reason, StopReason::PcRange);
    assert_eq!(cpu.pc, 0x8005);
}

#[test]
fn object_reader_rejects_zero_checksum_with_non_empty_body() {
    let object = assemble_object("m", ".section code\n nop\n").unwrap();
    assert!(!object.sections[0].bytes.is_empty());
    let mut bytes = write_object(&object).unwrap();
    let body = bytes.len() - 4;
    assert!(body > 0);
    bytes[body..].fill(0);
    let err = read_object(&bytes).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("checksum"), "{text}");
    assert!(text.contains("zero"), "{text}");
    let back = read_object(&write_object(&object).unwrap()).unwrap();
    assert_eq!(back.sections[0].bytes, object.sections[0].bytes);
}

#[test]
fn lo_and_hi_of_00ff_emit_a9_ff_and_a9_00() {
    assert_eq!(assemble("LDA #lo($00FF)\n", 0).unwrap(), [0xA9, 0xFF]);
    assert_eq!(assemble("LDA #hi($00FF)\n", 0).unwrap(), [0xA9, 0x00]);
}

#[test]
fn timer2_pulse_count_holds_ifr_until_zero_and_phi2_does_not_decrement() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x20);
    via.write(0x0E, 0xA0);
    via.write(0x08, 0x02);
    via.write(0x09, 0x00);
    via.tick();
    assert_eq!(via.t2_counter(), 2, "PHI2 must not decrement pulse count");
    assert_eq!(
        via.peek(0x0D) & 0x20,
        0,
        "timer-2 IFR stays clear before zero"
    );
    via.set_pb6(true);
    via.set_pb6(false);
    assert_eq!(via.t2_counter(), 1);
    via.tick();
    assert_eq!(via.t2_counter(), 1, "PHI2 still must not decrement");
    assert_eq!(via.peek(0x0D) & 0x20, 0);
    via.set_pb6(true);
    via.set_pb6(false);
    assert_eq!(via.t2_counter(), 0);
    assert_eq!(via.peek(0x0D) & 0x20, 0x20);
}

#[test]
fn clearing_both_pc_ranges_does_not_stop_in_the_old_range() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xEA; 8]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_pc_range(0x8002, 0x8003);
    dbg.add_pc_range(0x8004, 0x8005);
    dbg.clear_pc_range();
    let stop = dbg.run(&mut cpu, 6).unwrap();
    assert_ne!(stop.reason, StopReason::PcRange);
    assert_eq!(stop.reason, StopReason::InstructionLimit);
    assert_eq!(cpu.pc, 0x8006);
    assert_eq!(dbg.hit_range, None);
}

#[test]
fn snapshot_v5_rejects_a_missing_listing_and_a_complete_v5_still_loads() {
    let mut image = MachineImage::capture(&Cpu::new(FlatMemory::new()));
    image.version = 5;
    image.next_listing = "8000  EA  NOP".into();
    let json = image.to_json();
    let back = MachineImage::from_json(&json).unwrap();
    assert_eq!(back.version, 5);
    assert!(back.next_listing.contains("NOP"), "{}", back.next_listing);
    let bad = json.replacen(",\"next_listing\":\"8000  EA  NOP\"", "", 1);
    assert!(!bad.contains("next_listing"), "{bad}");
    let err = MachineImage::from_json(&bad).unwrap_err();
    assert_eq!(
        err,
        mos6502::snapshot::SnapshotError::MissingField("next_listing".into())
    );
}

#[test]
fn lo_and_hi_of_100_emit_a9_00_and_a9_01() {
    assert_eq!(assemble("LDA #lo($100)\n", 0).unwrap(), [0xA9, 0x00]);
    assert_eq!(assemble("LDA #hi($100)\n", 0).unwrap(), [0xA9, 0x01]);
}

#[test]
fn timer2_pulse_zero_sets_bit5_and_summary_and_phi2_holds_counter() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x20);
    via.write(0x0E, 0xA0);
    via.write(0x08, 0x01);
    via.write(0x09, 0x00);
    via.set_pb6(false);
    assert_eq!(via.t2_counter(), 0);
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x20, 0x20, "timer-2 bit");
    assert_eq!(ifr & 0x80, 0x80, "IRQ summary");
    let held = via.t2_counter();
    via.tick();
    assert_eq!(via.t2_counter(), held);
}

#[test]
fn read_outside_pc_range_stops_as_read_not_as_pc_range() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA5, 0x10]);
    mem.data[0x0010] = 0x5A;
    let mut cpu = Cpu::new(LoggingBus::new(mem));
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_pc_range(0x8002, 0x8004);
    dbg.watch(0x0010, Watch::Read);
    let reason = dbg.step_watched(&mut cpu).unwrap();
    assert_eq!(reason, StopReason::WatchRead);
    assert_ne!(reason, StopReason::PcRange);
    assert_eq!(dbg.hit_addr, Some(0x0010));
    assert_eq!(cpu.a, 0x5A);
    assert_eq!(cpu.pc, 0x8002);
}

#[test]
fn linker_sections_at_0000_and_0002_leave_the_gap_byte_zero() {
    let object =
        assemble_object("m", ".section lo\n  .byte $11\n.section hi\n  .byte $22\n").unwrap();
    let script = parse_link_script("section lo $0000\nsection hi $0002\n").unwrap();
    let linked = link_with_script(&[object], &script, 0x8000).unwrap();
    let mut image = vec![0u8; 4];
    linked.store(&mut image);
    assert_eq!(image[0x0000], 0x11);
    assert_eq!(image[0x0001], 0);
    assert_eq!(image[0x0002], 0x22);
}

#[test]
fn hi_and_lo_of_ffff_both_emit_a9_ff() {
    assert_eq!(assemble("LDA #hi($FFFF)\n", 0).unwrap(), [0xA9, 0xFF]);
    assert_eq!(assemble("LDA #lo($FFFF)\n", 0).unwrap(), [0xA9, 0xFF]);
}

#[test]
fn timer1_one_shot_sets_bit6_and_summary_once_then_a_tick_does_not() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xC0);
    via.write(0x04, 0x01);
    via.write(0x05, 0x00);
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    let ifr = via.peek(0x0D);
    assert_eq!(ifr & 0x40, 0x40, "timer-1 bit");
    assert_eq!(ifr & 0x80, 0x80, "IRQ summary");
    via.write(0x0D, 0x40);
    assert_eq!(via.peek(0x0D) & 0xC0, 0);
    via.tick();
    assert_eq!(
        via.peek(0x0D) & 0x40,
        0,
        "one-shot must not set the flag again"
    );
    assert_eq!(via.peek(0x0D) & 0x80, 0);
}

#[test]
fn run_until_halt_stops_on_pc_range_before_the_cycle_cap() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xEA; 16]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_pc_range(0x8003, 0x8005);
    let stop = dbg.run_until_halt(&mut cpu, 100).unwrap();
    assert_eq!(stop.reason, StopReason::PcRange);
    assert_ne!(stop.reason, StopReason::CycleCap);
    assert_eq!(cpu.pc, 0x8003);
    assert_eq!(stop.instructions, 3);
    assert!(cpu.cycles < 100);
    assert_eq!(dbg.hit_range, Some((0x8003, 0x8005)));
}

#[test]
fn object_writer_rejects_relative_reloc_at_section_length() {
    let mut object = assemble_object("m", ".section code\n nop\n").unwrap();
    let path = std::path::Path::new("/tmp/mos6502-rel-end.obj");
    let _ = std::fs::remove_file(path);
    let len = object.sections[0].bytes.len();
    assert!(len > 0);
    object.sections[0].relocs.push(Reloc {
        offset: len as u16,
        symbol: "x".into(),
        kind: RelocKind::Relative,
    });
    let err = write_object_file(&object, path).unwrap_err();
    assert!(err.to_string().contains("relativa"), "{err}");
    assert!(!path.exists());
    assert!(write_object(&object).is_err());
    object.sections[0].relocs[0].offset = 0;
    write_object_file(&object, path).unwrap();
    assert!(!std::fs::read(path).unwrap().is_empty());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn lda_immediate_high_and_low_byte_prefixes() {
    assert_eq!(assemble("LDA #>$1234\n", 0).unwrap(), [0xA9, 0x12]);
    assert_eq!(assemble("LDA #<$1234\n", 0).unwrap(), [0xA9, 0x34]);
}

#[test]
fn timer1_free_run_reloads_the_latch_and_keeps_the_flag() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x40);
    via.write(0x0E, 0xC0);
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.write(0x04, 0x05);
    assert_eq!(via.t1_counter(), 2);
    via.tick();
    via.tick();
    assert_eq!(via.peek(0x0D) & 0x40, 0);
    via.tick();
    assert_eq!(via.t1_counter(), 0xFFFF, "flag on the roll past zero");
    assert_eq!(via.peek(0x0D) & 0xC0, 0xC0);
    via.tick();
    assert_eq!(via.t1_counter(), 5, "the next tick copies the latch: N+2");
    assert!(via.t1_running());
    assert_eq!(via.peek(0x0D) & 0xC0, 0xC0);
    via.tick();
    assert_eq!(via.t1_counter(), 4);
    assert_eq!(
        via.peek(0x0D) & 0xC0,
        0xC0,
        "reload must not clear the flag"
    );
    via.write(0x0D, 0x40);
    assert_eq!(via.peek(0x0D) & 0xC0, 0);
    assert!(!via.t1_flag());
}

#[test]
fn step_out_stops_on_cycle_cap_without_claiming_return() {
    let out = assemble_with_symbols(
        "jsr outer\ntop:\nlda #$99\nbrk\nouter:\njsr sub\nmid:\nlda #$77\nrts\nsub:\nnop\nnop\nnop\nrts\n",
        0x0600,
    )
    .unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(out.origin, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = out.origin;
    cpu.step().unwrap();
    cpu.step().unwrap();
    assert_eq!(cpu.pc, out.symbols["sub"]);
    assert!(cpu.jsr_depth() >= 1);
    let stop = Debugger::default().step_out(&mut cpu, 3).unwrap();
    assert_eq!(stop.reason, StopReason::CycleCap);
    assert_ne!(stop.reason, StopReason::StepOut);
    assert_ne!(cpu.pc, out.symbols["top"]);
    assert_eq!(cpu.a, 0);
}

#[test]
fn linker_script_hex_10_and_decimal_16_place_the_same_byte() {
    let object = assemble_object("m", ".section code\n  .byte $AB\n").unwrap();
    let hex = parse_link_script("section code $10\n").unwrap();
    let dec = parse_link_script("section code 16\n").unwrap();
    let from_hex = link_with_script(&[object.clone()], &hex, 0x8000).unwrap();
    let from_dec = link_with_script(&[object], &dec, 0x8000).unwrap();
    let mut image_hex = vec![0u8; 32];
    let mut image_dec = vec![0u8; 32];
    from_hex.store(&mut image_hex);
    from_dec.store(&mut image_dec);
    assert_eq!(from_hex.segments[0].address, 0x10);
    assert_eq!(from_dec.segments[0].address, from_hex.segments[0].address);
    assert_eq!(image_hex[0x10], 0xAB);
    assert_eq!(image_dec[0x10], 0xAB);
}

#[test]
fn low_byte_of_parenthesized_sum_emits_a9_00() {
    assert_eq!(assemble("LDA #<($FF+1)\n", 0).unwrap(), [0xA9, 0x00]);
}

#[test]
fn timer2_one_shot_sets_bit5_once_then_a_phi2_tick_does_not() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x00);
    via.write(0x0E, 0xA0);
    via.write(0x08, 0x01);
    via.write(0x09, 0x00);
    via.tick();
    via.tick(); // T2 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.t2_counter(), 0xFFFF);
    assert_eq!(via.peek(0x0D) & 0x20, 0x20, "timer-2 bit");
    via.write(0x0D, 0x20);
    assert_eq!(via.peek(0x0D) & 0x20, 0);
    via.tick();
    assert_eq!(
        via.peek(0x0D) & 0x20,
        0,
        "one-shot must not set the flag again"
    );
}

#[test]
fn step_over_jsr_returning_on_the_last_allowed_cycle_is_not_cycle_cap() {
    let out = assemble_with_symbols("jsr sub\nback:\nlda #$42\nbrk\nsub:\nrts\n", 0x0600).unwrap();
    let mut mem = FlatMemory::new();
    mem.load_at(out.origin, &out.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = out.origin;
    let stop = Debugger::default().step_over(&mut cpu, 12).unwrap();
    assert_eq!(stop.reason, StopReason::StepOver);
    assert_ne!(stop.reason, StopReason::CycleCap);
    assert_eq!(cpu.pc, out.symbols["back"]);
    assert_eq!(cpu.cycles, 12);
}

#[test]
fn snapshot_diff_reports_palette_byte_and_not_a_register() {
    let cpu = Cpu::new(FlatMemory::new());
    let before = MachineImage::capture(&cpu).at_console([0; 16], [0; 256]);
    let mut palette = [0u8; 16];
    palette[2] = 0x0C;
    let after = MachineImage::capture(&cpu).at_console(palette, [0; 256]);
    let changes = diff_image(&before, &after);
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].name, "palette");
    assert_eq!(changes[0].before, 0);
    assert_eq!(changes[0].after, 0x0C);
    assert_ne!(changes[0].name, "a");
    assert_ne!(changes[0].name, "pc");
}

#[test]
fn high_byte_of_parenthesized_sum_emits_a9_01() {
    assert_eq!(assemble("LDA #>($FF+1)\n", 0).unwrap(), [0xA9, 0x01]);
    assert_eq!(assemble("LDA #<($FF+1)\n", 0).unwrap(), [0xA9, 0x00]);
}

#[test]
fn timer1_free_run_sets_bit6_on_the_next_reload_one_shot_does_not() {
    let mut free = Via6522::new();
    free.write(0x0B, 0x40);
    free.write(0x0E, 0xC0);
    free.write(0x04, 0x02);
    free.write(0x05, 0x00);
    free.tick();
    free.tick();
    assert_eq!(free.peek(0x0D) & 0x40, 0);
    free.tick();
    assert_eq!(free.peek(0x0D) & 0x40, 0x40);
    free.tick();
    assert_eq!(free.t1_counter(), 2);
    free.write(0x0D, 0x40);
    assert_eq!(free.peek(0x0D) & 0x40, 0);
    for _ in 0..2 {
        free.tick();
        assert_eq!(
            free.peek(0x0D) & 0x40,
            0,
            "a tick that is not the roll stays clear"
        );
    }
    free.tick();
    assert_eq!(
        free.peek(0x0D) & 0x40,
        0x40,
        "the next free-run reload sets bit 6 again"
    );

    let mut once = Via6522::new();
    once.write(0x0B, 0x00);
    once.write(0x0E, 0xC0);
    once.write(0x04, 0x02);
    once.write(0x05, 0x00);
    once.tick();
    once.tick();
    once.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(once.peek(0x0D) & 0x40, 0x40);
    once.write(0x0D, 0x40);
    once.tick();
    once.tick();
    assert_eq!(
        once.peek(0x0D) & 0x40,
        0,
        "one-shot must not set bit 6 again"
    );
}

#[test]
fn run_until_halt_brk_on_the_last_allowed_cycle_is_not_cycle_cap() {
    let program = assemble("lda #$01\nbrk\n", 0x8000).unwrap();
    let boot = |cap: u64| {
        let mut mem = FlatMemory::new();
        mem.load_at(0x8000, &program);
        mem.write(0xFFFE, 0x00);
        mem.write(0xFFFF, 0x90);
        let mut cpu = Cpu::new(mem);
        cpu.pc = 0x8000;
        let stop = Debugger::default().run_until_halt(&mut cpu, cap).unwrap();
        (stop, cpu.a, cpu.pc, cpu.cycles)
    };
    let (wide, _, _, used) = boot(1_000);
    assert_eq!(wide.reason, StopReason::Halted);
    assert!(used > 0);
    let (stop, a, pc, cycles) = boot(used);
    assert_eq!(stop.reason, StopReason::Halted);
    assert_ne!(stop.reason, StopReason::CycleCap);
    assert_eq!(a, 0x01);
    assert_eq!(pc, 0x9000);
    assert_eq!(cycles, used);
    assert_eq!(stop.instructions, 2);
}

#[test]
fn absolute_word_reloc_of_1234_patches_low_then_high() {
    let word = assemble_object(
        "w",
        ".section data\n.org $1234\ndest:\n  .byte 0\n.section code\n.org $0000\n  jmp dest\n",
    )
    .unwrap();
    let linked = link(&[word], 0).unwrap();
    let code = linked
        .segments
        .iter()
        .find(|segment| segment.address == 0)
        .unwrap();
    assert_eq!(code.bytes, [0x4C, 0x34, 0x12]);

    let both = assemble_object(
        "b",
        ".section data\n.org $1234\ndest:\n  .byte 0\n.section far\n.org $0200\nbig:\n  .byte 0\n.section code\n.org $0000\n  .byte big\n  jmp dest\n",
    )
    .unwrap();
    let err = link(&[both], 0).unwrap_err();
    assert!(err.to_string().contains("fora de faixa"), "{err}");
    let code = err
        .patched
        .iter()
        .find(|segment| segment.address == 0)
        .expect("code");
    let pair = [code.bytes[2], code.bytes[3]];
    assert!(
        pair == [0x34, 0x12] || pair == [0x00, 0x00],
        "word bytes {pair:02X?}"
    );
}

#[test]
fn high_and_low_of_parenthesized_ff_emit_a9_00_and_a9_ff() {
    assert_eq!(assemble("LDA #>($FF)\n", 0).unwrap(), [0xA9, 0x00]);
    assert_eq!(assemble("LDA #<($FF)\n", 0).unwrap(), [0xA9, 0xFF]);
}

#[test]
fn timer1_one_shot_timeout_does_not_flip_pb7() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x80);
    via.write(0x06, 1);
    via.write(0x05, 0);
    assert!(!via.pb7());
    via.tick();
    via.tick(); // T1 flags on the roll to $FFFF: N+1 ticks.
    assert_eq!(via.peek(0x0D) & 0x40, 0x40);
    assert!(!via.pb7(), "one-shot timeout must not flip PB7");
    via.tick();
    assert!(!via.pb7());
}

#[test]
fn zero_flag_set_to_clear_does_not_break() {
    let mut dbg = Debugger::default();
    dbg.break_when_flag_becomes_set(FlagEdge::Zero);
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x01, 0xEA]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    cpu.p.set(bits::Z, true);
    let stop = dbg.run(&mut cpu, 2).unwrap();
    assert_ne!(stop.reason, StopReason::FlagSet);
    assert_eq!(stop.reason, StopReason::InstructionLimit);
    assert!(!cpu.p.get(bits::Z));
    assert_eq!(stop.instructions, 2);
}

#[test]
fn second_section_length_mismatch_returns_no_sections() {
    let obj = assemble_object(
        "m",
        ".section one\n  .byte $11\n.section two\n  .byte $22\n",
    )
    .unwrap();
    let good = write_object(&obj).unwrap();
    let mut bad = good.clone();
    patch_second_declared_length(&mut bad);
    match read_object(&bad) {
        Ok(got) => assert!(got.sections.is_empty(), "first section leaked"),
        Err(err) => assert!(err.to_string().contains("comprimento"), "{err}"),
    }
    let back = read_object(&good).unwrap();
    assert_eq!(back.sections.len(), 2);
    assert_eq!(back.sections[0].bytes, [0x11]);
    assert_eq!(back.sections[1].bytes, [0x22]);
}

fn patch_second_declared_length(bytes: &mut [u8]) {
    let mut i = 5usize;
    let nlen = u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    i += 2 + nlen;
    let count = u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    assert!(count >= 2);
    i += 2;
    for sec in 0..count {
        let nlen = u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        i += 2 + nlen;
        i += 3;
        let nbytes = u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        i += 2 + nbytes;
        if sec == 1 {
            bytes[i] = bytes[i].wrapping_add(1);
            return;
        }
        i += 2;
        let relocs = u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
        i += 2;
        for _ in 0..relocs {
            i += 2;
            let slen = u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
            i += 2 + slen + 1;
        }
    }
    panic!("second section declared length not found");
}
