//! KIM-1 board: memory map, keypad matrix, LED scan and the monitor at the entry
//! points of the KIM-1 User Manual.

use mos6502::bus::MemoryBus;
use mos6502::systems::kim1::{entry, map, monitor_rom, zp, Key, Kim1, Kim1Bus, MONITOR_SRC};

#[test]
fn monitor_symbols_sit_at_the_documented_entry_points() {
    let out = mos6502::asm::assemble_with_symbols(MONITOR_SRC, 0x1C00).unwrap();
    let sym = |n: &str| out.symbols[n];
    for (name, addr) in [
        ("save", entry::SAVE),
        ("nmit", entry::NMIT),
        ("irqt", entry::IRQT),
        ("rst", entry::RST),
        ("start", entry::START),
        ("ak", entry::AK),
        ("scands", entry::SCANDS),
        ("keyin", entry::KEYIN),
        ("getkey", entry::GETKEY),
        ("table", entry::TABLE),
    ] {
        assert_eq!(sym(name), addr, "{name}");
    }
    let rom = monitor_rom();
    assert_eq!(rom.len(), 0x400);
    let w = |a: usize| u16::from_le_bytes([rom[a], rom[a + 1]]);
    assert_eq!(
        (w(0x3FA), w(0x3FC), w(0x3FE)),
        (entry::NMIT, entry::RST, entry::IRQT)
    );
    // NMIT/IRQT are JMP ($17FA) / JMP ($17FE).
    assert_eq!(&rom[0x1C..0x22], &[0x6C, 0xFA, 0x17, 0x6C, 0xFE, 0x17]);
}

#[test]
fn memory_map_decodes_thirteen_address_lines() {
    let mut b = Kim1Bus::new();
    for a in 0..0x400u16 {
        b.write(a, a as u8 ^ 0x5A);
    }
    for a in 0..0x400u16 {
        for mirror in [0u16, 0x2000, 0xE000] {
            assert_eq!(b.read(a | mirror), a as u8 ^ 0x5A);
        }
    }
    // Two separate 64-byte RRIOT RAMs.
    for i in 0..64u16 {
        b.write(0x1780 + i, i as u8);
        b.write(0x17C0 + i, 0x80 | i as u8);
    }
    for i in 0..64u16 {
        assert_eq!(b.read(0x1780 + i), i as u8);
        assert_eq!(b.read(0x17C0 + i), 0x80 | i as u8);
        assert_eq!(b.riot003.ram[usize::from(i)], i as u8);
        assert_eq!(b.riot002.ram[usize::from(i)], 0x80 | i as u8);
    }
    // Unpopulated space reads $FF and ignores writes; ROM is read-only.
    for a in [0x0400u16, 0x1000, 0x16FF] {
        b.write(a, 0);
        assert_eq!(b.read(a), 0xFF);
    }
    let before = b.read(0x1C00);
    b.write(0x1C00, !before);
    assert_eq!(b.read(0x1C00), before);
    assert_eq!(b.read(0xFFFC), b.read(0x1FFC));
    // The two I/O windows are separate chips.
    b.write(0x1701, 0xFF);
    b.write(0x1700, 0x3C);
    assert_eq!(b.read(0x1700), 0x3C);
    assert_eq!(b.riot002.ddr_a, 0);
    b.write(0x1707, 2); // 6530-003 timer, /1024
    assert_eq!(b.riot003.timer_div, 1024);
    assert_eq!(b.riot002.timer_div, 1);
}

#[test]
fn keypad_matrix_every_key_on_its_row_and_bit() {
    for code in 0..21u8 {
        let mut b = Kim1Bus::new();
        b.keys[usize::from(code)] = true;
        b.write(map::PADD, 0);
        b.write(map::PBDD, 0x1E);
        for sel in 0..10u8 {
            b.write(map::SBD, sel << 1);
            let pa = b.read(map::SAD);
            let expect = if sel == code / 7 {
                !(1 << (code % 7))
            } else {
                0xFF
            };
            assert_eq!(pa, expect, "key {code:02X} select {sel}");
        }
    }
    let mut b = Kim1Bus::new();
    b.tty_in = false;
    assert_eq!(b.read(map::SAD) & 0x80, 0, "PA7 follows the TTY input");
}

#[test]
fn display_latches_segments_on_outputs_four_to_nine() {
    let mut b = Kim1Bus::new();
    b.write(map::PADD, 0x7F);
    b.write(map::PBDD, 0x1E);
    for d in 0..6u8 {
        b.write(map::SAD, 0);
        b.write(map::SBD, (4 + d) << 1);
        b.write(
            map::SAD,
            [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D][usize::from(d)],
        );
    }
    assert_eq!(b.display_text(), "012345");
    // A key row select does not light a digit; PA7 is not a segment.
    b.write(map::SBD, 0);
    b.write(map::SAD, 0xFF);
    assert_eq!(b.display_text(), "012345");
    assert_eq!(b.strobes, [1; 6]);
}

fn kim() -> Kim1 {
    let mut k = Kim1::new();
    k.cpu.bus.write(map::NMIV, 0x00);
    k.cpu.bus.write(map::NMIV + 1, 0x1C);
    k.run_instructions(10000).unwrap();
    k
}

#[test]
fn reset_enters_the_monitor_and_scans_the_display() {
    let k = kim();
    assert_eq!(k.cpu.bus.display_text(), "000000");
    assert!(k.cpu.bus.strobes.iter().all(|&n| n > 3));
}

#[test]
fn address_and_data_entry_plus_and_go() {
    let mut k = kim();
    // AD 0200, DA A9 42 + 85 + 10 + 00 → LDA #$42 / STA $10 / BRK
    k.tap(Key::Ad).unwrap();
    for n in [0, 2, 0, 0] {
        k.tap(Key::Hex(n)).unwrap();
    }
    assert_eq!(k.cpu.bus.display_text()[..4], *"0200");
    k.tap(Key::Da).unwrap();
    for byte in [0xA9u8, 0x42, 0x85, 0x10, 0x00] {
        k.tap(Key::Hex(byte >> 4)).unwrap();
        k.tap(Key::Hex(byte & 15)).unwrap();
        assert_eq!(
            k.cpu.bus.ram[usize::from(k.cpu.bus.peek(zp::POINTL)) + 0x200],
            byte
        );
        k.tap(Key::Plus).unwrap();
    }
    assert_eq!(
        &k.cpu.bus.ram[0x200..0x205],
        &[0xA9, 0x42, 0x85, 0x10, 0x00]
    );
    assert_eq!(k.cpu.bus.display_text(), "020500");
    k.tap(Key::Ad).unwrap();
    for n in [0, 2, 0, 0] {
        k.tap(Key::Hex(n)).unwrap();
    }
    // IRQ pointer → SAVE so BRK comes back to the monitor.
    k.cpu.bus.write(map::IRQV, 0x00);
    k.cpu.bus.write(map::IRQV + 1, 0x1C);
    k.cpu.bus.write(zp::SPUSER, 0xFF);
    k.tap(Key::Go).unwrap();
    assert_eq!(k.cpu.bus.ram[0x10], 0x42);
    assert_eq!(k.cpu.bus.peek(zp::ACC), 0x42);
    // BRK pushes PC+2: the monitor shows the address after the signature byte.
    assert_eq!(k.cpu.bus.display_text()[..4], *"0206");
    k.tap(Key::Pc).unwrap();
    assert_eq!(k.cpu.bus.display_text()[..4], *"0206");
}

#[test]
fn st_key_stops_a_running_program_through_the_nmi_pointer() {
    let mut k = kim();
    k.cpu.bus.ram[0x300..0x303].copy_from_slice(&[0x4C, 0x00, 0x03]); // JMP $0300
    k.cpu.bus.write(zp::POINTL, 0);
    k.cpu.bus.write(zp::POINTH, 3);
    k.cpu.bus.write(zp::SPUSER, 0xFF);
    k.tap(Key::Go).unwrap();
    assert_eq!(k.cpu.pc, 0x300);
    k.st();
    k.run_instructions(3000).unwrap();
    assert_eq!(k.cpu.bus.peek(zp::PCH), 0x03);
    assert_eq!(k.cpu.bus.peek(zp::PCL), 0x00);
    assert_eq!(k.cpu.bus.display_text()[..4], *"0300");
    k.rs();
    assert_eq!(k.cpu.pc, entry::RST);
}

#[test]
fn getkey_and_ak_return_documented_codes_for_every_key() {
    for code in 0..=0x15u8 {
        let mut k = kim();
        if code < 0x15 {
            k.cpu.bus.keys[usize::from(code)] = true;
        }
        // JSR GETKEY ; STA $20 ; JSR AK ; STA $21 ; JMP *
        let prog = [
            0x20, 0x6A, 0x1F, 0x85, 0x20, 0x20, 0xFE, 0x1E, 0x85, 0x21, 0x4C, 0x0A, 0x02,
        ];
        k.cpu.bus.ram[0x200..0x200 + prog.len()].copy_from_slice(&prog);
        k.cpu.pc = 0x200;
        k.run_instructions(4000).unwrap();
        assert_eq!(k.cpu.bus.ram[0x20], code, "GETKEY");
        assert_eq!(k.cpu.bus.ram[0x21] == 0, code == 0x15, "AK");
    }
}

#[test]
fn riot_timers_run_off_the_cpu_clock() {
    let mut k = kim();
    // LDA #$FF ; STA $1747 (/1024) ; JMP *
    let prog = [0xA9, 0xFF, 0x8D, 0x47, 0x17, 0x4C, 0x05, 0x02];
    k.cpu.bus.ram[0x200..0x208].copy_from_slice(&prog);
    k.cpu.pc = 0x200;
    k.run_instructions(2).unwrap();
    assert_eq!(k.cpu.bus.riot002.timer_value, 0xFF);
    let cycles = k.run_instructions(1000).unwrap();
    assert_eq!(cycles, 3000);
    assert_eq!(u32::from(k.cpu.bus.riot002.timer_value), 0xFF - 3000 / 1024);
}
