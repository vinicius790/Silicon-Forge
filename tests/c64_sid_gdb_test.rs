use std::collections::HashSet;
use mos6502::bus::{FlatMemory, MemoryBus};
use mos6502::cpu::Cpu;
use mos6502::gdb::{format_packet, gdb_checksum, handle_command};
use mos6502::sid::{EnvelopeState, Sid6581, SID_PAL_CLOCK_HZ};
use mos6502::systems::c64::C64Computer;

#[test]
fn test_sid_waveforms_and_envelope_lifecycle() {
    let mut sid = Sid6581::new();

    // Voice 1: Sawtooth at ~440Hz
    sid.write_reg(0x00, 0x10);
    sid.write_reg(0x01, 0x1C);
    sid.write_reg(0x05, 0x10); // Attack 1, Decay 0
    sid.write_reg(0x06, 0x82); // Sustain 8, Release 2
    sid.write_reg(0x04, 0x21); // Sawtooth + Gate
    sid.write_reg(0x18, 0x0F); // Full Volume

    assert_eq!(sid.voices[0].env_state, EnvelopeState::Attack);

    // Clock voice through attack until it reaches max level
    for _ in 0..5000 {
        sid.clock();
    }
    assert!(sid.voices[0].env_level > 0);

    // Turn off Gate: transitions to Release
    sid.write_reg(0x04, 0x20);
    assert_eq!(sid.voices[0].env_state, EnvelopeState::Release);

    // Render audio samples into PCM
    let pcm = sid.render_pcm(256, 44100, SID_PAL_CLOCK_HZ);
    assert_eq!(pcm.len(), 256);
}

#[test]
fn test_c64_pla_banking_ram_and_screen() {
    let mut c64 = C64Computer::new();

    // Verify reset state
    assert_eq!(c64.cpu.bus.port_data, 0x37);

    // Write to screen matrix and read back
    c64.print_char_at(0, 0, 0x08, 0x01); // 'H'
    c64.print_char_at(1, 0, 0x05, 0x01); // 'E'
    c64.print_char_at(2, 0, 0x0C, 0x01); // 'L'
    c64.print_char_at(3, 0, 0x0C, 0x01); // 'L'
    c64.print_char_at(4, 0, 0x0F, 0x01); // 'O'

    let text = c64.cpu.bus.screen_text();
    assert!(text.starts_with("HELLO"));

    // Write to SID register through bus
    c64.cpu.bus.write(0xD400, 0xAA);
    c64.cpu.bus.write(0xD401, 0x55);
    assert_eq!(c64.cpu.bus.sid.voices[0].freq, 0x55AA);

    // RAM under ROM test: write to $A000 writes to RAM, while reading returns ROM unless LORAM disabled
    c64.cpu.bus.basic_rom[0] = 0x60; // RTS
    assert_eq!(c64.cpu.bus.read(0xA000), 0x60);
    c64.cpu.bus.write(0xA000, 0xEA); // writes into underlying RAM
    assert_eq!(c64.cpu.bus.read(0xA000), 0x60); // still reads ROM

    // Disable BASIC ROM (clear bit 0 of port_data)
    c64.cpu.bus.write(0x0001, 0x36);
    assert_eq!(c64.cpu.bus.read(0xA000), 0xEA); // now reads underlying RAM!
}

#[test]
fn test_gdb_rsp_protocol_stub() {
    let mut mem = FlatMemory::new();
    mem.data[0x2000] = 0xA9; // LDA #$42
    mem.data[0x2001] = 0x42;
    mem.data[0x2002] = 0x8D; // STA $3000
    mem.data[0x2003] = 0x00;
    mem.data[0x2004] = 0x30;
    mem.data[0x2005] = 0x00; // BRK

    let mut cpu = Cpu::new(mem);
    cpu.regs.pc = 0x2000;
    let mut breakpoints = HashSet::new();

    // Checksum verification
    assert_eq!(gdb_checksum(b"?"), b'?');
    assert_eq!(format_packet("?"), b"$?#3f");

    // Command: Read registers
    let regs = handle_command(&mut cpu, "g", &mut breakpoints, 100).expect("g");
    assert!(!regs.is_empty());

    // Command: Insert breakpoint at 0x2002
    assert_eq!(handle_command(&mut cpu, "Z0,2002,1", &mut breakpoints, 100).unwrap(), "OK");
    assert!(breakpoints.contains(&0x2002));

    // Command: Continue until breakpoint
    assert_eq!(handle_command(&mut cpu, "c", &mut breakpoints, 100).unwrap(), "S05");
    assert_eq!(cpu.regs.pc, 0x2002);
    assert_eq!(cpu.regs.a, 0x42);

    // Command: Step instruction
    assert_eq!(handle_command(&mut cpu, "s", &mut breakpoints, 100).unwrap(), "S05");
    assert_eq!(cpu.regs.pc, 0x2005);
    assert_eq!(cpu.bus.read(0x3000), 0x42);
}

#[test]
fn test_c64_cia_timer_irq() {
    let mut c64 = C64Computer::new();
    // Program Timer A with small count (10 cycles)
    c64.cpu.bus.write(0xDC04, 10);
    c64.cpu.bus.write(0xDC05, 0);
    c64.cpu.bus.write(0xDC0E, 0x01); // start timer A

    // Ensure interrupts are enabled in CPU
    c64.cpu.regs.p.remove(mos6502::flags::Status::I);
    // Put NOPs in RAM
    c64.cpu.bus.ram[0x0800] = 0xEA;
    c64.cpu.regs.pc = 0x0800;

    // Run cycles and verify CIA 1 timer triggers
    c64.run_cycles(30);
    assert!(c64.cpu.bus.cia1_irq_flag);
}

#[test]
fn test_atari2600_vcs_and_tia() {
    let mut atari = mos6502::Atari2600::new();
    // Test WSYNC strobe
    atari.cpu.bus.write(0x02, 0); // WSYNC
    assert!(atari.cpu.bus.tia.wsync_active);

    // Run cycles to clear WSYNC
    atari.run_cycles(80);
    assert!(!atari.cpu.bus.tia.wsync_active);
}

#[test]
fn test_static_binary_analyzer() {
    let code = [0xA9, 0x00, 0x8D, 0x00, 0x02, 0x60]; // LDA #0, STA $0200, RTS
    let report = mos6502::analyze_binary(&code, 0xC000);
    assert_eq!(report.base_addr, 0xC000);
    assert_eq!(report.instruction_count, 3);
}
