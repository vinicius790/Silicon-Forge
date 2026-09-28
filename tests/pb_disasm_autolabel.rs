//! Generated `L_XXXX` labels for branch, jump and call targets.

use mos6502::asm::{assemble, assemble_with_symbols};
use mos6502::disasm::{code_targets, to_source, to_source_with};
use mos6502::opcodes::CpuVariant;
use mos6502::symbols::SymbolTable;

fn image(src: &str, at: u16) -> (Vec<u8>, u16) {
    let bytes = assemble(src, at).unwrap();
    let mut mem = vec![0u8; 0x10000];
    mem[usize::from(at)..usize::from(at) + bytes.len()].copy_from_slice(&bytes);
    (mem, at + bytes.len() as u16 - 1)
}

const PROG: &str =
    "ldx #3\nloop: dex\n bne loop\n jsr sub\n jmp done\nsub: rts\ndone: jmp ($FFFC)\n";

#[test]
fn targets_get_labels_and_round_trip() {
    let (mem, end) = image(PROG, 0x8000);
    let s = to_source_with(&mem, 0x8000, end, CpuVariant::NMOS, None, None, true);
    assert!(s.named, "{}", s.text);
    for want in [
        "L_8002:\n",
        "BNE L_8002",
        "JSR L_800B",
        "L_800B:\n",
        "JMP L_800C",
        "L_800C:\n",
        "JMP ($FFFC)",
    ] {
        assert!(s.text.contains(want), "{want}\n{}", s.text);
    }
    assert_eq!(
        assemble(&s.text, 0x8000).unwrap(),
        mem[0x8000..=usize::from(end)]
    );
    let plain = to_source(&mem, 0x8000, end, CpuVariant::NMOS, None, None);
    assert!(!plain.text.contains("L_"));
}

#[test]
fn existing_symbols_win_and_outside_targets_stay_numeric() {
    let out = assemble_with_symbols(PROG, 0x8000).unwrap();
    let table = SymbolTable::from_asm(&out).unwrap();
    let (mem, end) = image(&format!("{PROG} jmp $1234\n"), 0x8000);
    let s = to_source_with(
        &mem,
        0x8000,
        end,
        CpuVariant::NMOS,
        Some(&table),
        None,
        true,
    );
    assert!(s.text.contains("BNE loop"));
    assert!(!s.text.contains("L_8002"));
    assert!(s.text.contains("JMP $1234"), "{}", s.text);
    assert_eq!(
        assemble(&s.text, 0x8000).unwrap(),
        mem[0x8000..=usize::from(end)]
    );
}

#[test]
fn code_targets_lists_only_line_starts_in_range() {
    let (mem, end) = image(PROG, 0x8000);
    let s = to_source(&mem, 0x8000, end, CpuVariant::NMOS, None, None);
    let t: Vec<u16> = code_targets(&mem, 0x8000, &s.kinds, CpuVariant::NMOS)
        .into_iter()
        .collect();
    assert_eq!(t, [0x8002, 0x800B, 0x800C]);
    // A branch into the middle of an instruction gets no label.
    let (mem, end) = image("lda $1234\n bne $8001\n", 0x8000);
    let s = to_source(&mem, 0x8000, end, CpuVariant::NMOS, None, None);
    assert!(code_targets(&mem, 0x8000, &s.kinds, CpuVariant::NMOS).is_empty());
}

#[test]
fn random_programs_round_trip_with_auto_labels() {
    let mut seed = 0xACE1u32;
    for _ in 0..30 {
        let bytes: Vec<u8> = (0..200)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (seed >> 16) as u8
            })
            .collect();
        let mut mem = vec![0u8; 0x10000];
        mem[0x6000..0x6000 + 200].copy_from_slice(&bytes);
        let s = to_source_with(
            &mem,
            0x6000,
            0x6000 + 199,
            CpuVariant::CMOS,
            None,
            None,
            true,
        );
        assert_eq!(assemble(&s.text, 0x6000).unwrap(), bytes, "{}", s.text);
    }
}
