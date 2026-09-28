use mos6502::{
    asm::assemble,
    disasm::{disasm, disasm_at},
    opcodes::{documented_opcodes, Mode},
    Cpu, FlatMemory,
};

#[test]
fn every_documented_opcode_roundtrips_and_executes() {
    let opcodes = documented_opcodes();
    assert_eq!(opcodes.len(), 151);
    for op in opcodes {
        let mut mem = FlatMemory::new();
        mem.load_at(0x8000, &[op.code, 0x34, 0x12]);
        let line = disasm_at(&mem.data, 0x8000);
        let bytes = assemble(&line.text, 0x8000).unwrap();
        assert_eq!(
            bytes,
            [op.code, 0x34, 0x12][..op.mode.size() as usize],
            "{}",
            line.text
        );
        mem.set_reset_vector(0x8000);
        let mut cpu = Cpu::new(mem);
        cpu.reset();
        assert!(
            cpu.step().is_ok(),
            "metadata refers to unsupported opcode {:02X}",
            op.code
        );
    }
}

#[test]
fn low_absolute_operands_keep_their_encoding() {
    for op in documented_opcodes()
        .iter()
        .filter(|op| matches!(op.mode, Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind))
    {
        let line = disasm(op.code, 0x10, 0x00);
        assert_eq!(
            assemble(&line.text, 0).unwrap(),
            [op.code, 0x10, 0x00],
            "{}",
            line.text
        );
    }
}

#[test]
fn addressing_modes_and_new_mnemonics_have_known_encodings() {
    let source = "LDX #$10\nLDY $20,X\nSTX $30,Y\nADC $1234\nLDA ($40,X)\nSTA ($50),Y\nJMP ($1234)\nASL A\nCPY #$12\nBVS $8016";
    assert_eq!(
        assemble(source, 0x8000).unwrap(),
        [
            0xa2, 0x10, 0xb4, 0x20, 0x96, 0x30, 0x6d, 0x34, 0x12, 0xa1, 0x40, 0x91, 0x50, 0x6c,
            0x34, 0x12, 0x0a, 0xc0, 0x12, 0x70, 0x01
        ]
    );
}

#[test]
fn disassembly_wraps_operand_fetch_and_resolves_branch_target() {
    let mut memory = [0; 65536];
    memory[65535] = 0x4c;
    memory[0] = 0x34;
    memory[1] = 0x12;
    assert_eq!(disasm_at(&memory, 0xffff).text, "JMP $1234");
    memory[65534] = 0xd0;
    memory[65535] = 0xfe;
    assert_eq!(disasm_at(&memory, 0xfffe).text, "BNE $FFFE");
}

#[test]
fn invalid_addressing_and_program_overflow_are_rejected() {
    for src in [
        "STA #$10",
        "LDX ($10),Y",
        "LDA ($100,X)",
        "ASL #$10",
        "LDA ($10),X",
        "BVC missing",
    ] {
        assert!(assemble(src, 0).is_err(), "accepted {src}");
    }
    assert!(assemble(&"NOP\n".repeat(65537), 0).is_err());
    assert_eq!(assemble("DB $02,$FF", 0).unwrap(), [0x02, 0xff]);
}

#[test]
fn binary_adc_sbc_exhaustive_against_integer_arithmetic() {
    use mos6502::alu::{adc, sbc};
    for a in 0..=255u16 {
        for b in 0..=255u16 {
            for carry in [false, true] {
                let sum = a + b + u16::from(carry);
                let add = adc(a as u8, b as u8, carry, false);
                assert_eq!((add.value, add.carry), (sum as u8, sum > 255));
                let signed_sum = a as u8 as i8 as i16 + b as u8 as i8 as i16 + i16::from(carry);
                assert_eq!(add.overflow, !(-128..=127).contains(&signed_sum));
                let difference = a as i16 - b as i16 - i16::from(!carry);
                let sub = sbc(a as u8, b as u8, carry, false);
                assert_eq!((sub.value, sub.carry), (difference as u8, difference >= 0));
                let signed_difference =
                    a as u8 as i8 as i16 - b as u8 as i8 as i16 - i16::from(!carry);
                assert_eq!(sub.overflow, !(-128..=127).contains(&signed_difference));
            }
        }
    }
}
