//! Coverage mapped through the listing's origin fields: macros and includes.

use std::collections::HashMap;

use mos6502::asm;
use mos6502::coverage::{line_map, Coverage, FileLines};
use mos6502::{Cpu, FlatMemory, MemoryBus};

const SRC: &str = "\
.macro twice
  INX
  INX
.endmacro
start:
  twice
  JSR helper
  LDY #1
  BNE over
  twice
over:
  BRK
.include \"lib.inc\"
";

fn build() -> asm::AsmOutput {
    let mut files = HashMap::new();
    files.insert(
        "lib.inc".to_string(),
        "helper:\n  LDA #2\n  RTS\nunused:\n  RTS\ntable: .byte 1,2\n".to_string(),
    );
    asm::assemble_with_files(SRC, 0x8000, &files).unwrap()
}

fn covered(o: &asm::AsmOutput) -> Coverage {
    let mut mem = FlatMemory::new();
    mem.load_at(o.origin, &o.bytes);
    let mut cpu = Cpu::new(mem);
    cpu.pc = o.origin;
    let mut c = Coverage::new();
    while cpu.bus.peek(cpu.pc) != 0 {
        c.step(&mut cpu).unwrap();
    }
    c
}

#[test]
fn macro_lines_own_their_expansion_and_includes_get_their_file() {
    let o = build();
    let map = line_map(&o, "main.asm");
    assert_eq!(
        map,
        vec![
            FileLines {
                file: "main.asm".into(),
                lines: vec![
                    (6, vec![0x8000, 0x8001]),
                    (7, vec![0x8002]),
                    (8, vec![0x8005]),
                    (9, vec![0x8007]),
                    (10, vec![0x8009, 0x800A]),
                    (12, vec![0x800B]),
                ],
            },
            FileLines {
                file: "lib.inc".into(),
                lines: vec![(2, vec![0x800C]), (3, vec![0x800E]), (5, vec![0x800F])],
            },
        ]
    );
}

#[test]
fn lcov_per_file_counts_macro_calls_and_skipped_code() {
    let o = build();
    let c = covered(&o);
    let map = line_map(&o, "main.asm");
    assert_eq!(c.line_hits(&[0x8000, 0x8001]), 1);
    assert_eq!(c.line_hits(&[0x8009, 0x800A]), 0, "skipped macro call");
    let lcov = c.to_lcov_files("t", &map);
    assert_eq!(
        lcov,
        "TN:t\nSF:main.asm\nDA:6,1\nDA:7,1\nDA:8,1\nDA:9,1\nDA:10,0\nDA:12,0\nLF:6\nLH:4\nend_of_record\n\
TN:t\nSF:lib.inc\nDA:2,1\nDA:3,1\nDA:5,0\nLF:3\nLH:2\nend_of_record\n"
    );
}

#[test]
fn line_hits_takes_the_busiest_address() {
    let src = "  LDX #3\nloop:\n  DEX\n  BNE loop\n  BRK\n";
    let o = asm::assemble_with_symbols(src, 0x8000).unwrap();
    let c = covered(&o);
    assert_eq!(c.line_hits(&[0x8000, 0x8002]), 3);
    assert_eq!(c.line_hits(&[]), 0);
}
