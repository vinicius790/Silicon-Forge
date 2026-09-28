//! `.charmap` over every byte value and every string directive.

use mos6502::asm::assemble;

#[test]
fn every_byte_maps_in_every_directive() {
    for from in 0x20u8..=0x7E {
        if matches!(from, b'"' | b'a' | b'b') {
            continue;
        }
        let to = from ^ 0xC5;
        // A backslash is written as its escape.
        let c = if from == b'\\' {
            "\\\\".to_string()
        } else {
            (from as char).to_string()
        };
        let src = format!(
            ".charmap ${from:02X}, ${to:02X}\nl: .text \"{c}a\"\n.ascii \"{c}\"\n.asciiz \"{c}\"\n.pstring \"{c}{c}\"\n.byte 1, \"{c}b\", 2\n.charmap\n.text \"{c}\""
        );
        let a = b'a';
        assert_eq!(
            assemble(&src, 0).unwrap_or_else(|e| panic!("{from:02X} {e:?}")),
            [to, a, to, to, 0, 2, to, to, 1, to, b'b', 2, from],
            "{from:02X}"
        );
    }
}

#[test]
fn later_mappings_override_and_labels_survive() {
    let src = ".charmap 'A', 1\n.charmap 'A', 2\n.charmap 'B', 'A'\nmsg: .asciiz \"AB\"\n.word msg\n.text \"\"\n.byte 9";
    assert_eq!(assemble(src, 0x300).unwrap(), [2, b'A', 0, 0x00, 0x03, 9]);
    // Identity map: source untouched, strings keep normal handling.
    assert_eq!(assemble(".text \"hi\"", 0).unwrap(), b"hi");
}

#[test]
fn errors_name_the_line() {
    for (src, needle) in [
        ("nop\n.charmap 1", "linha 2: .charmap exige"),
        (".charmap 256, 1", "fora da faixa"),
        (".charmap 1, $100", "fora da faixa"),
        (".charmap 'a', 1\n.text \"ab", "linha 2: string sem aspas"),
        (".charmap 'a', 1\n.byte 1, \"a", "string sem aspas"),
        (".charmap 'a', 1\n.byte \"\"", "string vazia"),
    ] {
        let e = assemble(src, 0).unwrap_err().0;
        assert!(e.contains(needle), "{src}: {e}");
    }
    let long = format!(".charmap 'a', 1\n.pstring \"{}\"", "a".repeat(256));
    assert!(assemble(&long, 0).unwrap_err().0.contains("255"));
}
