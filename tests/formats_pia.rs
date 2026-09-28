//! PIA 6821, S-record/Intel HEX, KCS tape, and instruction profile.

use mos6502::asm::{assemble, assemble_with_symbols};
use mos6502::bus::{FlatMemory, IoDevice};
use mos6502::hexfmt::{from_intel_hex, from_srec, to_srec};
use mos6502::tape::{decode, encode};
use mos6502::{Cpu, Pia6821, Profile};

#[test]
fn pia_ddr_then_output_and_ca1_falling_sets_irq() {
    let mut pia = Pia6821::new();
    pia.write(1, 0x00);
    pia.write(0, 0xFF);
    pia.write(1, 0x04);
    pia.write(0, 0xA5);
    assert_eq!(pia.port_a_out(), 0xA5);
    pia.write(1, 0x05);
    pia.set_ca1(false);
    assert!(pia.irq_a());
    assert_eq!(pia.read(0), 0xA5);
    assert!(!pia.irq_a());
}

#[test]
fn srec_and_intel_hex_roundtrip_lda() {
    let bytes = assemble("lda #$42\nbrk", 0x8000).unwrap();
    let s = to_srec(&bytes, 0x8000, 0x8000);
    let img = from_srec(&s).unwrap();
    assert_eq!(img.origin, 0x8000);
    assert_eq!(img.start, Some(0x8000));
    assert_eq!(img.bytes, bytes);
    let hex = assemble_with_symbols("lda #$42\nbrk", 0x8000)
        .unwrap()
        .to_intel_hex();
    let back = from_intel_hex(&hex).unwrap();
    assert_eq!(back.origin, 0x8000);
    assert_eq!(back.bytes, bytes);
}

#[test]
fn tape_kcs_roundtrip() {
    let data = b"MOS6502";
    assert_eq!(decode(&encode(data)).unwrap(), data);
}

#[test]
fn profile_counts_lda_immediate() {
    let mut mem = FlatMemory::new();
    mem.load_at(0x8000, &[0xA9, 0x42, 0x00]);
    mem.set_reset_vector(0x8000);
    let mut cpu = Cpu::new(mem);
    cpu.halt_on_brk = true;
    cpu.reset();
    let mut p = Profile::new();
    p.run(&mut cpu, 2).unwrap();
    assert_eq!(p.opcode_counts[0xA9], 1);
    assert_eq!(cpu.a, 0x42);
    assert!(p.report().contains("op $A9"));
}
