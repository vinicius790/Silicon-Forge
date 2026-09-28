//! ACIA status peek, VIA IER clear, `.byte` low/high operators, and backward branch disassembly.

use mos6502::acia::{Acia6551, OVERRUN};
use mos6502::asm::assemble;
use mos6502::disasm::disasm_at;
use mos6502::{IoDevice, Via6522};

#[test]
fn acia_status_peek_keeps_overrun_until_read() {
    let mut acia = Acia6551::new();
    acia.write(2, 0x0B); // DTR on: the receiver is enabled
    acia.push_rx(0x11);
    acia.push_rx(0x22);
    assert_eq!(acia.peek(1) & OVERRUN, OVERRUN);
    assert_eq!(
        acia.peek(1) & OVERRUN,
        OVERRUN,
        "peek must not clear overrun"
    );
    assert_eq!(acia.read(1) & OVERRUN, OVERRUN);
    assert_eq!(acia.peek(1) & OVERRUN, 0, "status read clears overrun");
    assert_eq!(acia.read(0), 0x11);
}

#[test]
fn via_ier_write_without_bit7_clears_only_named_bits() {
    let mut via = Via6522::new();
    via.write(0x0E, 0xE0);
    assert_eq!(via.read(0x0E), 0xE0);
    via.write(0x0E, 0x40);
    assert_eq!(via.read(0x0E), 0xA0, "T1 cleared, T2 kept, bit 7 reads 1");
}

#[test]
fn byte_directive_splits_word_with_low_high_operators() {
    assert_eq!(assemble(".byte <$1234, >$1234\n", 0).unwrap(), [0x34, 0x12]);
}

#[test]
fn backward_branch_disassembles_to_absolute_target() {
    let mut mem = vec![0u8; 0x10000];
    mem[0x8010] = 0xD0;
    mem[0x8011] = 0xEE;
    let line = disasm_at(&mem, 0x8010);
    assert_eq!(line.size, 2);
    assert!(line.text.contains("$8000"), "{}", line.text);
}
