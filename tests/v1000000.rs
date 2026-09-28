//! 1000000.0.0: CA2/CB2 pulse from a CPU store, name table, palette blit,
//! PC-range watch, and the in-tree 1 KiB monitor echo.

use mos6502::acia::run_cycles_with_acia;
use mos6502::asm::{assemble, assemble_with_symbols};
use mos6502::bus::IoDevice;
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::debugger::{Debugger, StopReason};
use mos6502::graphics::FantasyConsoleBus;
use mos6502::systems::via_board::ViaBoard;
use mos6502::Cpu;

#[test]
fn cpu_write_to_ora_pulses_ca2_and_orb_pulses_cb2() {
    let program = assemble(
        "lda #$0A\nsta $C00C\nlda #$55\nsta $C001\nlda #$A0\nsta $C00C\nlda #$01\nsta $C000\nbrk\n",
        0x0200,
    )
    .unwrap();
    let mut board = ViaBoard::with_rom(&[]);
    for (index, byte) in program.iter().enumerate() {
        board.write(0x0200 + index as u16, *byte);
    }
    let mut cpu = Cpu::new(board);
    cpu.pc = 0x0200;
    cpu.halt_on_brk = true;
    for _ in 0..12 {
        if cpu.is_halted() {
            break;
        }
        cpu.step().unwrap();
    }
    assert!(cpu.is_halted());
    assert_eq!(cpu.bus.via.ca2_pulses(), 1);
    assert_eq!(cpu.bus.via.cb2_pulses(), 1);
    assert!(
        !cpu.bus.via.cb2(),
        "pulse is still low because Cpu::step does not tick the VIA"
    );
    assert_eq!(cpu.bus.via.ora_latch(), 0x55);
    assert_eq!(cpu.bus.via.orb_latch(), 0x01);
}

#[test]
fn name_table_render_paints_one_pattern_pixel() {
    let mut patterns = vec![0u8; 128];
    patterns[64] = 0x00;
    patterns[65] = 0x06; // tile 1, pixel (1, 0)
    let mut bus = FantasyConsoleBus::new();
    bus.set_patterns(&patterns);
    bus.device.name_table[16 + 2] = 1; // row 1, col 2 of the 16-wide table
    bus.device.render_name_tile(2, 1);
    let screen_x = 2 * 8 + 1;
    let row = 1;
    let screen_y = row * 8;
    assert_eq!(bus.device.screen.pixels[screen_y * 32 + screen_x], 0x06);
    assert_eq!(
        bus.device.screen.pixels[screen_y * 32 + 2 * 8],
        0,
        "source 0 stays transparent"
    );
}

#[test]
fn blit_maps_palette_index_1_to_the_programmed_color() {
    let mut patterns = vec![0u8; 64];
    patterns[0] = 0x01;
    let mut bus = FantasyConsoleBus::new();
    bus.set_patterns(&patterns);
    bus.device.set_palette(1, 0x0C);
    bus.write(0x00F8, 0);
    bus.write(0x00F9, 0);
    bus.write(0x00FA, 0);
    assert_eq!(bus.peek(0x0200), 0x0C);
    assert_eq!(
        bus.device.screen.pixels[1], 0,
        "the next pattern byte was transparent"
    );
}

#[test]
fn pc_range_watch_stops_inside_and_not_outside() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xEA; 8]);
    let mut cpu = Cpu::new(mem);
    cpu.pc = 0x8000;
    let mut dbg = Debugger::default();
    dbg.watch_pc_range(0x8003, 0x8005);
    let stop = dbg.run(&mut cpu, 10).unwrap();
    assert_eq!(stop.reason, StopReason::PcRange);
    assert_eq!(cpu.pc, 0x8003);
    assert_eq!(stop.instructions, 3);

    cpu.pc = 0x8000;
    dbg.watch_pc_range(0x9000, 0x9004);
    let stop = dbg.run(&mut cpu, 4).unwrap();
    assert_eq!(stop.reason, StopReason::InstructionLimit);
    assert_eq!(cpu.pc, 0x8004);
}

#[test]
fn one_kib_monitor_echoes_a_6551_byte() {
    let image = assemble_with_symbols(include_str!("../examples/monitor_1k.asm"), 0).unwrap();
    assert_eq!(image.bytes.len(), 1024, "the monitor image is 1 KiB");
    assert_eq!(image.origin, 0xFC00);
    let mut bus = mos6502::acia::RamAcia::new(0xD000);
    bus.acia.write(2, 0x0B); // DTR on: the receiver is enabled
    bus.memory.load_at(0xFC00, &image.bytes);
    let mut cpu = Cpu::new(bus);
    cpu.reset();
    assert_eq!(cpu.pc, 0xFC00);
    cpu.bus.acia.push_rx(0x5A);
    run_cycles_with_acia(&mut cpu, 16).unwrap();
    assert_eq!(cpu.bus.acia.pop_tx(), Some(0x5A));
}
