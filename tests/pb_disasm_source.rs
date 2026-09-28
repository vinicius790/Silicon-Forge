//! Reassemblable disassembly: symbols, coverage-driven code/data, round trip.

use mos6502::asm::{assemble, assemble_with_symbols};
use mos6502::coverage::Coverage;
use mos6502::disasm::{to_source, ByteKind};
use mos6502::opcodes::CpuVariant;
use mos6502::symbols::SymbolTable;
use mos6502::{Cpu, FlatMemory};

fn mem_with(at: u16, bytes: &[u8]) -> Vec<u8> {
    let mut mem = vec![0u8; 0x10000];
    mem[usize::from(at)..usize::from(at) + bytes.len()].copy_from_slice(bytes);
    mem
}

fn round_trip(mem: &[u8], start: u16, end: u16, src: &str) {
    let back = assemble(src, start).unwrap_or_else(|e| panic!("{e}\n{src}"));
    assert_eq!(back, mem[usize::from(start)..=usize::from(end)], "{src}");
}

#[test]
fn every_opcode_with_varied_operands_round_trips_for_both_variants() {
    for variant in [CpuVariant::NMOS, CpuVariant::CMOS] {
        for op in 0..=255u8 {
            for (b1, b2) in [
                (0x00, 0x00),
                (0x10, 0x00),
                (0x34, 0x12),
                (0xFF, 0xFF),
                (0x80, 0x7F),
            ] {
                let mem = mem_with(0x8000, &[op, b1, b2]);
                let s = to_source(&mem, 0x8000, 0x8002, variant, None, None);
                round_trip(&mem, 0x8000, 0x8002, &s.text);
                assert_eq!(s.kinds.len(), 3);
            }
        }
    }
}

#[test]
fn pseudo_random_buffers_round_trip() {
    let mut seed = 0x1234_5678u32;
    for _ in 0..40 {
        let bytes: Vec<u8> = (0..300)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (seed >> 16) as u8
            })
            .collect();
        let mem = mem_with(0x9000, &bytes);
        let s = to_source(&mem, 0x9000, 0x9000 + 299, CpuVariant::NMOS, None, None);
        round_trip(&mem, 0x9000, 0x9000 + 299, &s.text);
    }
}

#[test]
fn symbols_name_operands_and_labels_and_still_round_trip() {
    let src = "io = $D000\nstart: ldx #0\nloop: lda table,x\n sta io\n inx\n cpx #3\n bne loop\n jmp start\ntable: .byte 1,2,3\n";
    let out = assemble_with_symbols(src, 0x8000).unwrap();
    let mem = mem_with(0x8000, &out.bytes);
    let table = SymbolTable::from_asm(&out).unwrap();
    let end = 0x8000 + out.bytes.len() as u16 - 1;
    let s = to_source(&mem, 0x8000, end, CpuVariant::NMOS, Some(&table), None);
    assert!(s.named, "{}", s.text);
    for want in [
        "io = $D000",
        "loop:\n",
        "LDA table,X",
        "STA io",
        "BNE loop",
        "JMP start",
        "table:\n",
    ] {
        assert!(s.text.contains(want), "{want} in\n{}", s.text);
    }
    round_trip(&mem, 0x8000, end, &s.text);
}

#[test]
fn coverage_separates_code_from_data() {
    let src = "start: lda data\n jmp done\ndata: .byte $EA,$A9,$FF\ndone: brk\n";
    let out = assemble_with_symbols(src, 0x0400).unwrap();
    let mut flat = FlatMemory::new();
    flat.load_at(0x0400, &out.bytes);
    let mut cpu = Cpu::new(flat);
    cpu.pc = 0x0400;
    let mut cov = Coverage::new();
    for _ in 0..3 {
        cov.step(&mut cpu).unwrap();
    }
    let mem = mem_with(0x0400, &out.bytes);
    let end = 0x0400 + out.bytes.len() as u16 - 1;
    let s = to_source(&mem, 0x0400, end, CpuVariant::NMOS, None, Some(&cov));
    use ByteKind::*;
    assert_eq!(
        s.kinds,
        [Code, Operand, Operand, Code, Operand, Operand, Data, Data, Data, Code]
    );
    assert!(s.text.contains(".byte $EA,$A9,$FF"), "{}", s.text);
    round_trip(&mem, 0x0400, end, &s.text);
    // Without coverage the data bytes decode as NOP / LDA #$FF.
    let linear = to_source(&mem, 0x0400, end, CpuVariant::NMOS, None, None);
    assert_eq!(linear.kinds[6], Code);
    assert_eq!(linear.kinds[7], Code);
}

#[test]
fn unexecuted_range_is_all_data_and_truncated_tail_is_data() {
    let mem = mem_with(0x2000, &[0xA9, 0x01, 0x8D, 0x00]);
    let cov = Coverage::new();
    let s = to_source(&mem, 0x2000, 0x2003, CpuVariant::NMOS, None, Some(&cov));
    assert!(s.kinds.iter().all(|k| *k == ByteKind::Data));
    round_trip(&mem, 0x2000, 0x2003, &s.text);
    let s = to_source(&mem, 0x2000, 0x2003, CpuVariant::NMOS, None, None);
    assert_eq!(
        s.kinds,
        [
            ByteKind::Code,
            ByteKind::Operand,
            ByteKind::Data,
            ByteKind::Code
        ],
        "the trailing $00 is a one-byte BRK"
    );
}

#[test]
fn label_inside_an_instruction_forces_data() {
    let mem = mem_with(0x3000, &[0xAD, 0x34, 0x12, 0x60]);
    let mut table = SymbolTable::new();
    table.insert("mid", 0x3001).unwrap();
    let s = to_source(&mem, 0x3000, 0x3003, CpuVariant::NMOS, Some(&table), None);
    assert!(s.named);
    assert!(s.text.contains("mid:\n"), "{}", s.text);
    round_trip(&mem, 0x3000, 0x3003, &s.text);
}
