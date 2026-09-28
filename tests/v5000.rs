//! v50.0: VIA edges, pulse count, PB7, shift-in, assembler diagnostics and
//! scopes, value watch, snapshot v3, sprite blit and collision.

use mos6502::asm::{assemble, assemble_with_symbols};
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::debugger::{Debugger, LoggingBus, StopReason};
use mos6502::graphics::FantasyConsoleBus;
use mos6502::snapshot::MachineImage;
use mos6502::via::Via6522;
use mos6502::Cpu;

#[test]
fn ca1_cb1_negative_edge_sets_ifr_and_ier_gates_irq() {
    let mut via = Via6522::new();
    via.set_ca1(false);
    via.set_cb1(false);
    assert_eq!(via.peek(0x0D), 0x12);
    via.write(0x0D, 0x12);

    via.set_ca1(true);
    via.write(0x0E, 0x82);
    via.set_ca1(true);
    assert_eq!(via.peek(0x0D) & 0x02, 0, "rising edge must not set CA1");
    via.set_ca1(false);
    assert_eq!(via.peek(0x0D) & 0x02, 0x02);

    via.set_cb1(true);
    via.write(0x0E, 0x90);
    via.set_cb1(false);
    assert_eq!(via.peek(0x0D) & 0x10, 0x10);
}

#[test]
fn timer2_counts_three_pb6_falling_edges() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x20);
    via.write(0x0E, 0xA0);
    via.write(0x08, 3);
    via.write(0x09, 0);
    for left in [2u16, 1, 0] {
        via.set_pb6(true);
        via.set_pb6(false);
        assert_eq!(via.t2_counter(), left);
        assert_eq!(via.t2_flag(), left == 0);
    }
    assert_eq!(via.read(0x0D) & 0xA0, 0xA0);
}

#[test]
fn pb7_flips_on_each_free_run_underflow() {
    let mut via = Via6522::new();
    via.write(0x0B, 0xC0);
    via.write(0x06, 1);
    via.write(0x05, 0);
    assert!(!via.pb7());
    via.tick();
    via.tick();
    assert!(via.pb7());
    for _ in 0..3 {
        via.tick();
    }
    assert!(!via.pb7(), "free-run period is N+2");
}

#[test]
fn shift_in_eight_phi2_ticks_msb_first() {
    let mut via = Via6522::new();
    via.write(0x0B, 0b010 << 2);
    via.write(0x0A, 0);
    for bit in [true, false, true, false, false, true, false, true] {
        via.set_sr_in_bit(bit);
        via.tick();
    }
    assert_eq!(via.sr(), 0xA5);
    assert_eq!(via.peek(0x0D) & 0x04, 0x04);
}

#[test]
fn assembler_error_fails_and_warn_records() {
    let err = assemble(".error \"falhou o teste\"\n nop\n", 0).unwrap_err();
    assert!(err.to_string().contains("falhou o teste"), "{err}");

    let skipped =
        assemble_with_symbols(".if 0\n.error \"nao\"\n.warn \"nao\"\n.endif\n nop\n", 0).unwrap();
    assert_eq!(skipped.bytes, [0xEA]);
    assert!(skipped.warnings.is_empty());

    let warned = assemble_with_symbols(".warn \"cuidado\"\n nop\n", 0).unwrap();
    assert_eq!(warned.bytes, [0xEA]);
    assert_eq!(warned.warnings, vec!["cuidado".to_string()]);
}

#[test]
fn scope_local_label_does_not_leak() {
    let kept = assemble_with_symbols(
        "outer:\n  @here: nop\n  .scope\n    @here: lda #$11\n  .endscope\n  jmp @here\n",
        0x8000,
    )
    .unwrap();
    assert_eq!(kept.bytes, [0xEA, 0xA9, 0x11, 0x4C, 0x00, 0x80]);

    let leaked = assemble("  .scope\n    @only: nop\n  .endscope\n  jmp @only\n", 0);
    assert!(leaked.is_err(), "local label leaked out of .scope");
}

#[test]
fn break_when_a_watched_address_is_written_with_a_value() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x01, 0x85, 0x10, 0xA9, 0x02, 0x85, 0x10]);
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
    assert_eq!(cpu.bus.inner.peek(0x0010), 0x01);
    assert_eq!(
        dbg.step_watched(&mut cpu).unwrap(),
        StopReason::InstructionLimit
    );
    assert_eq!(dbg.step_watched(&mut cpu).unwrap(), StopReason::WatchValue);
    assert_eq!(cpu.a, 0x02);
}

#[test]
fn snapshot_v3_round_trips_scanline_and_halted_and_rejects_99() {
    let mut cpu = Cpu::new(FlatMemory::new());
    cpu.a = 0x19;
    cpu.pc = 0x4321;
    cpu.set_halted(true);
    let image = MachineImage::capture(&cpu).at_scanline(17);
    assert_eq!(image.version, 3);
    assert!(image.halted);
    let json = image.to_json();
    assert!(json.contains("\"scanline\":17"), "{json}");
    assert!(json.contains("\"halted\":true"), "{json}");
    let back = MachineImage::from_json(&json).unwrap();
    assert_eq!(back.version, 3);
    assert_eq!(back.scanline, 17);
    assert!(back.halted);
    assert_eq!(back.a, 0x19);
    assert_eq!(back.pc, 0x4321);
    let mut restored = Cpu::new(FlatMemory::new());
    back.restore(&mut restored).unwrap();
    assert!(restored.is_halted());
    assert_eq!(restored.a, 0x19);

    let bad = json.replacen("\"version\":3", "\"version\":99", 1);
    let err = MachineImage::from_json(&bad).unwrap_err();
    assert_eq!(
        err,
        mos6502::snapshot::SnapshotError::UnsupportedVersion(99)
    );
}

#[test]
fn cpu_blit_lands_one_sprite_pixel_and_not_the_transparent_one() {
    let mut patterns = vec![0u8; 64];
    patterns[0] = 0x07;
    let mut bus = FantasyConsoleBus::new();
    bus.set_patterns(&patterns);
    bus.write(0x0200 + 5 * 32 + 5, 0x0E);
    let program = [
        0xA9, 0x00, 0x8D, 0xF8, 0x00, // index 0
        0xA9, 0x04, 0x8D, 0xF9, 0x00, // x = 4
        0xA9, 0x05, 0x8D, 0xFA, 0x00, // y = 5, host blits
        0x00,
    ];
    bus.load_at(0x8000, &program);
    bus.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(bus);
    cpu.halt_on_brk = true;
    cpu.reset();
    for _ in 0..8 {
        if cpu.is_halted() {
            break;
        }
        cpu.step().unwrap();
    }
    assert!(cpu.is_halted());
    assert_eq!(cpu.bus.peek(0x0200 + 5 * 32 + 4), 0x07);
    assert_eq!(cpu.bus.peek(0x0200 + 5 * 32 + 5), 0x0E);
}

#[test]
fn second_sprite_collides_only_on_opaque_overlap() {
    let mut patterns = vec![0u8; 128];
    patterns[0] = 0x01;
    patterns[64] = 0x00;
    patterns[65] = 0x02;
    let mut bus = FantasyConsoleBus::new();
    bus.set_patterns(&patterns);
    bus.write(0x00F8, 0);
    bus.write(0x00F9, 1);
    bus.write(0x00FA, 1);
    bus.write(0x00F5, 1);
    bus.write(0x00F6, 1);
    bus.write(0x00F7, 1);
    assert_eq!(
        bus.peek(0x00F4),
        0,
        "rectangles overlap but opaque pixels do not"
    );

    bus.write(0x00F6, 0);
    bus.write(0x00F7, 1);
    assert_eq!(bus.peek(0x00F4), 1);

    patterns[64] = 0x03;
    patterns[65] = 0x00;
    bus.set_patterns(&patterns);
    bus.write(0x00F6, 1);
    bus.write(0x00F7, 1);
    assert_eq!(bus.peek(0x00F4), 1, "both opaque at (1,1)");
}
