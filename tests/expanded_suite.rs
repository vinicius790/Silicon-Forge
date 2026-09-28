use mos6502::{
    asm::{assemble, assemble_with_symbols},
    bus::{FlatMemory, IoDevice, MemoryBus, SystemBus},
    cpu::Cpu,
    flags::bits,
    opcodes::CpuVariant,
    Apple1,
};

#[test]
fn unofficial_lax_loads_both_a_and_x() {
    let mut mem = FlatMemory::new();
    // LAX #$42 (0xAB 0x42) / BRK (0x00)
    mem.load_at(0x8000, &[0xAB, 0x42, 0x00]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();

    while !cpu.is_halted() {
        cpu.step().unwrap();
    }
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.x, 0x42);
}

#[test]
fn unofficial_sax_stores_bitwise_and_of_a_and_x() {
    let mut mem = FlatMemory::new();
    // LDA #$F0, LDX #$3C, SAX $10 (0x87 0x10), BRK
    mem.load_at(0x8000, &[0xA9, 0xF0, 0xA2, 0x3C, 0x87, 0x10, 0x00]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();

    while !cpu.is_halted() {
        cpu.step().unwrap();
    }
    // 0xF0 & 0x3C = 0x30
    assert_eq!(cpu.bus.read(0x0010), 0x30);
}

#[test]
fn unofficial_dcp_decrements_and_compares() {
    let mut mem = FlatMemory::new();
    mem.data[0x0020] = 0x05;
    // LDA #$04, DCP $20 (0xC7 0x20), BRK
    mem.load_at(0x8000, &[0xA9, 0x04, 0xC7, 0x20, 0x00]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();

    while !cpu.is_halted() {
        cpu.step().unwrap();
    }
    // Mem decremented from 5 to 4; CMP with A=4 sets Zero and Carry
    assert_eq!(cpu.bus.read(0x0020), 0x04);
    assert!(cpu.p.get(bits::Z));
    assert!(cpu.p.get(bits::C));
}

#[test]
fn cmos_bra_branches_unconditionally() {
    let mut mem = FlatMemory::new();
    // BRA +2 (0x80 0x02), LDA #$99 (skipped), LDA #$42, BRK
    mem.load_at(0x8000, &[0x80, 0x02, 0xA9, 0x99, 0xA9, 0x42, 0x00]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::with_variant(mem, CpuVariant::CMOS);
    cpu.halt_on_brk = true;
    cpu.reset();

    while !cpu.is_halted() {
        cpu.step().unwrap();
    }
    assert_eq!(cpu.a, 0x42);
}

#[test]
fn cmos_bra_adds_cycle_when_branch_crosses_page() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x80fb, &[0x80, 0x03]);
    mem.set_reset_vector(0x80fb);
    let mut cpu = Cpu::with_variant(mem, CpuVariant::CMOS);
    cpu.reset();

    assert_eq!(cpu.step().unwrap(), 4);
    assert_eq!(cpu.pc, 0x8100);
}

#[test]
fn cmos_stz_stores_zero_directly() {
    let mut mem = FlatMemory::new();
    mem.data[0x0010] = 0xFF;
    // STZ $10 (0x64 0x10), BRK
    mem.load_at(0x8000, &[0x64, 0x10, 0x00]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::with_variant(mem, CpuVariant::CMOS);
    cpu.halt_on_brk = true;
    cpu.reset();

    while !cpu.is_halted() {
        cpu.step().unwrap();
    }
    assert_eq!(cpu.bus.read(0x0010), 0x00);
}

#[test]
fn cmos_phx_ply_stack_transfer() {
    let mut mem = FlatMemory::new();
    // LDX #$77, PHX (0xDA), PLY (0x7A), BRK
    mem.load_at(0x8000, &[0xA2, 0x77, 0xDA, 0x7A, 0x00]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::with_variant(mem, CpuVariant::CMOS);
    cpu.halt_on_brk = true;
    cpu.reset();

    while !cpu.is_halted() {
        cpu.step().unwrap();
    }
    assert_eq!(cpu.y, 0x77);
}

#[test]
fn assembler_supports_extended_directives() {
    let src = r#"
        .org $8000
        val = $42
        start:
            LDA #val
            STA target
            BRK
        target:
            .byte $00
        message:
            .asciiz "OK"
        table:
            .word $1234, $5678
    "#;
    let res = assemble_with_symbols(src, 0x8000).unwrap();
    assert_eq!(res.origin, 0x8000);
    assert_eq!(res.symbols.get("val"), Some(&0x42));
    assert!(res.symbols.contains_key("start"));
    assert!(res.symbols.contains_key("message"));

    // Check Intel HEX output format
    let hex = res.to_intel_hex();
    assert!(hex.starts_with(':'));
    assert!(hex.contains(":00000001FF")); // EOF
}

#[test]
fn assembler_org_gaps_preserve_addresses_in_flat_binary_and_intel_hex() {
    let result =
        assemble_with_symbols(".org $8000\n.byte $AA\n.org $8003\n.byte $BB", 0x2000).unwrap();

    assert_eq!(result.origin, 0x8000);
    assert_eq!(result.bytes, [0xAA, 0x00, 0x00, 0xBB]);
    assert_eq!(result.to_intel_hex(), ":04800000AA0000BB17\n:00000001FF\n");
}

#[test]
fn assembler_org_directive_is_case_insensitive_and_borrow_safe() {
    assert_eq!(assemble(".ORG $8000\n.byte $42", 0).unwrap(), [0x42]);
    assert_eq!(assemble("*= $8000\n.byte $42", 0).unwrap(), [0x42]);
    assert_eq!(assemble("* = $8000\n.byte $42", 0).unwrap(), [0x42]);
}

#[test]
fn assembler_rejects_backward_org_that_would_overwrite_emitted_bytes() {
    assert!(assemble_with_symbols(".org $8000\n.byte $AA, $BB\n.org $8001\n.byte $CC", 0).is_err());
}

#[test]
fn assembler_decimal_literals_require_the_decimal_prefix() {
    assert_eq!(assemble(".byte 0d10, 10", 0).unwrap(), [10, 0x10]);
    assert_eq!(assemble("LDA #0d42", 0x8000).unwrap(), [0xA9, 42]);
}

#[test]
fn assembler_keeps_semicolons_inside_strings_and_rejects_invalid_fill_values() {
    assert_eq!(
        assemble(".text \"A;B\" ; comment", 0).unwrap(),
        b"A;B".to_vec()
    );
    assert_eq!(assemble(".asciiz \"OK\"", 0).unwrap(), [b'O', b'K', 0]);
    assert!(assemble(".text \"unfinished", 0).is_err());
    assert!(assemble(".fill 2, $100", 0).is_err());
    assert!(assemble(".fill 2, $01, $02", 0).is_err());
}

#[test]
fn system_bus_with_custom_device() {
    struct DummyUart {
        last_written: u8,
    }
    impl IoDevice for DummyUart {
        fn read(&mut self, _offset: u16) -> u8 {
            0x55
        }
        fn write(&mut self, _offset: u16, value: u8) {
            self.last_written = value;
        }
        fn peek(&self, _offset: u16) -> u8 {
            0x55
        }
    }

    let mut bus = SystemBus::new();
    bus.map_device(0x4000, 0x4003, DummyUart { last_written: 0 });
    assert_eq!(bus.read(0x4000), 0x55);
    bus.write(0x4001, 0x99);
}

#[test]
fn apple1_system_boots_its_monitor() {
    let apple = Apple1::new();
    assert_eq!(apple.cpu.pc, 0xFE00); // monitor entry point
}
