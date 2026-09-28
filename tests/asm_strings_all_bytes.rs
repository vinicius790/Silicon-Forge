//! Every printable byte inside strings, in every string directive, with and
//! without `.charmap`, with labels and comments on the same line.

use mos6502::asm::assemble;

#[test]
fn every_printable_byte_in_every_string_directive() {
    for b in 0x20u8..=0x7E {
        if b == b'"' {
            continue;
        }
        // A backslash is written as its escape.
        let c = if b == b'\\' {
            "\\\\".to_string()
        } else {
            (b as char).to_string()
        };
        let src = format!(
            "l{b}: .text \"{c}x{c}\" ; comment {c}\n.ascii \"{c}\"\n.asciiz \"{c}\"\n.pstring \"{c}\"\n.byte \"{c}\", 1 ; {c}\nv{b} = '{c}'\n.byte v{b}\nm{b}: .byte '{c}', \"{c}\"\n.word l{b}"
        );
        let want = [b, b'x', b, b, b, 0, 1, b, b, 1, b, b, b, 0x00, 0x10];
        assert_eq!(
            assemble(&src, 0x1000).unwrap_or_else(|e| panic!("{c:?}: {e:?}")),
            want,
            "{c:?}"
        );
        let to = b ^ 0x80;
        let mapped = format!(".charmap ${b:02X}, ${to:02X}\n{src}");
        let mut want = want;
        for (i, w) in want.iter_mut().enumerate() {
            // String bytes map; numbers and character literals do not.
            let in_string = matches!(i, 0 | 2 | 3 | 4 | 7 | 8 | 12) || (i == 1 && b == b'x');
            if in_string && *w == b {
                *w = to;
            }
        }
        assert_eq!(
            assemble(&mapped, 0x1000).unwrap_or_else(|e| panic!("map {c:?}: {e:?}")),
            want,
            "map {c:?}"
        );
    }
}

#[test]
fn separators_as_character_literals_everywhere() {
    for c in [',', ';', ':', '=', ' ', '\'', '$', '%'] {
        let src = format!(
            "a = '{c}'\n.byte '{c}', a\n.word '{c}'\n.dbyt '{c}'\n.fill 2, '{c}'\n lda #'{c}'"
        );
        let b = c as u8;
        assert_eq!(
            assemble(&src, 0).unwrap_or_else(|e| panic!("{c:?}: {e:?}")),
            [b, b, b, 0, 0, b, b, b, 0xA9, b],
            "{c:?}"
        );
    }
}
