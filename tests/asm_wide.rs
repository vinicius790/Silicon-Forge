//! .faraddr, .dword and .pstring.

use mos6502::asm::assemble;

#[test]
fn wide_words_over_every_value_pattern() {
    for v in (0..=0xFFFFu32).step_by(0x0101) {
        let src = format!(".org $1000\n.faraddr ${v:04X}, fwd\n.dword ${v:04X}\nfwd:");
        let out = assemble(&src, 0x1000).unwrap();
        let [lo, hi] = (v as u16).to_le_bytes();
        assert_eq!(out, [lo, hi, 0, 0x0A, 0x10, 0, lo, hi, 0, 0], "{v:04X}");
    }
    for bad in [".faraddr", ".dword \"x\"", ".dword nowhere"] {
        assert!(assemble(bad, 0).is_err(), "{bad}");
    }
}

#[test]
fn pstring_lengths_up_to_255() {
    for n in [0usize, 1, 2, 127, 254, 255] {
        let text = "a".repeat(n);
        let out = assemble(&format!(".pstring \"{text}\"\n.byte 7"), 0).unwrap();
        assert_eq!(out.len(), n + 2);
        assert_eq!((out[0] as usize, out[n + 1]), (n, 7));
    }
    assert!(assemble(&format!(".pstring \"{}\"", "a".repeat(256)), 0)
        .unwrap_err()
        .0
        .contains("255"));
    assert!(assemble(".pstring abc", 0).is_err());
    assert!(assemble(".pstring \"a\"b\"", 0).is_err());
}
