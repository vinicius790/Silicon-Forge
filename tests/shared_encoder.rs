//! The assembler and the object assembler share one operand parser and one opcode
//! table: every documented instruction, disassembled and re-assembled through either
//! path, gives back its own bytes.

use mos6502::disasm::format_at_variant;
use mos6502::link::{assemble_object, link};
use mos6502::opcodes::{documented_opcodes, Mode, CMOS_BIT_OPS, CMOS_OVERRIDES};
use mos6502::{assemble, CpuVariant};

fn via_link(src: &str) -> Vec<u8> {
    let object = assemble_object("t", &format!(".section code\n.org $0400\n{src}\n")).unwrap();
    let linked = link(&[object], 0x0400).unwrap();
    let mut mem = vec![0u8; 0x10000];
    linked.store(&mut mem);
    let len = linked.segments.iter().map(|s| s.bytes.len()).sum::<usize>();
    mem[0x0400..0x0400 + len].to_vec()
}

#[test]
fn every_documented_opcode_round_trips_through_both_assemblers() {
    let ops = documented_opcodes()
        .iter()
        .map(|op| (op, CpuVariant::NMOS))
        .chain(
            CMOS_OVERRIDES
                .iter()
                .chain(CMOS_BIT_OPS.iter())
                .map(|op| (op, CpuVariant::CMOS)),
        );
    let mut checked = 0;
    for (op, variant) in ops {
        // Operands that keep zero page and absolute forms apart; branches land nearby.
        let (b1, b2) = match op.mode {
            Mode::Rel => (0x10, 0x00),
            Mode::ZpRel => (0x42, 0x10),
            _ => (0x42, 0x12),
        };
        let size = usize::from(op.mode.size());
        let bytes = [op.code, b1, b2][..size].to_vec();
        let line = format_at_variant(op.code, b1, b2, 0x0400, variant).text;
        assert_eq!(assemble(&line, 0x0400).unwrap(), bytes, "asm: {line}");
        let relative = matches!(op.mode, Mode::Rel | Mode::ZpRel);
        if !relative {
            assert_eq!(via_link(&line), bytes, "link: {line}");
        }
        checked += 1;
    }
    assert_eq!(checked, 151 + 29 + 32);
}

#[test]
fn object_assembler_relocates_branches_bit_branches_and_zero_page_symbols() {
    let bytes = via_link(
        "start:\n bbs7 $42,start\n bne start\n lda (ptr),y\n jmp (vec,x)\n.section data\n.org $0010\nptr:\n.byte 0\nvec:\n.byte 0",
    );
    assert_eq!(bytes[..3], [0xFF, 0x42, 0xFD], "BBS7 back to itself: -3");
    assert_eq!(bytes[3..5], [0xD0, 0xFB], "BNE start: -5");
    assert_eq!(
        bytes[5..7],
        [0xB1, 0x10],
        "zero-page symbol relocated as a byte"
    );
    assert_eq!(
        bytes[7..10],
        [0x7C, 0x11, 0x00],
        "(abs,X) symbol is two bytes"
    );
}
