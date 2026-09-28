//! String and character escapes over every byte value and every directive.

use mos6502::asm::assemble;
use mos6502::escape::{char_literal_len, char_value, decode_string};

#[test]
fn every_byte_through_hex_escapes_everywhere() {
    for b in 0..=255u8 {
        for x in ["x", "X"] {
            let e = format!("\\{x}{b:02x}");
            let src = format!(
                "l: .text \"{e}a{e}\" ; c\n.ascii \"{e}\"\n.asciiz \"{e}\"\n.pstring \"{e}\"\n.byte \"{e}\", '{e}'\nv = '{e}'\n lda #'{e}'\n.word l, v"
            );
            let want = [b, b'a', b, b, b, 0, 1, b, b, b, 0xA9, b, 0, 0x10, b, 0];
            assert_eq!(
                assemble(&src, 0x1000).unwrap_or_else(|er| panic!("{b:02X}: {er:?}")),
                want
            );
            assert_eq!(char_value(&format!("'{e}'")), Ok(b));
            // .charmap applies to escaped bytes in strings, not to char literals.
            let to = b.wrapping_add(1);
            let m = assemble(
                &format!(".charmap ${b:02X}, ${to:02X}\n.byte \"{e}\", '{e}'\n.text \"{e}\""),
                0,
            )
            .unwrap();
            assert_eq!(m, [to, b, to], "{b:02X}");
        }
    }
}

#[test]
fn named_escapes_and_quotes() {
    let src = r#".text "\"\\\n\r\t\0\'"
.byte '\'', '\\', '\"', '"', '\n', '\r', '\t', '\0', ';', ':', '='
.text "a;b:c=d\"e""#;
    let mut want = vec![b'"', b'\\', b'\n', b'\r', b'\t', 0, b'\''];
    want.extend([
        b'\'', b'\\', b'"', b'"', b'\n', b'\r', b'\t', 0, b';', b':', b'=',
    ]);
    want.extend(b"a;b:c=d\"e");
    assert_eq!(assemble(src, 0).unwrap(), want);
    assert_eq!(decode_string(r"a\x41\\").unwrap(), b"aA\\");
    assert_eq!(char_literal_len(r"'\x41'+1"), Some(6));
    assert_eq!(char_literal_len("'ab'"), None);
    assert_eq!(assemble(r#".pstring "\"\"""#, 0).unwrap(), [2, b'"', b'"']);
    assert_eq!(assemble(r#".asciiz "\0""#, 0).unwrap(), [0, 0]);
    assert_eq!(assemble(r"lda #'\n'+1", 0).unwrap(), [0xA9, 11]);
}

#[test]
fn escape_errors_are_reported() {
    for (src, needle) in [
        (r#".text "\q""#, "escape desconhecido"),
        (r#".text "\x4""#, "dois dígitos"),
        (r#".text "\xG1""#, "dois dígitos"),
        (r#".byte "ab\"#, "sem aspas"),
        (r#".text "a"b""#, "sem aspas"),
        (r"lda #'\q'", "escape desconhecido"),
        (r"lda #'\x41a'", "maior que um"),
        (
            r#".charmap 1, 2
.text "\q""#,
            "escape desconhecido",
        ),
        (
            r#".charmap 1, 2
.byte "\x1""#,
            "dois dígitos",
        ),
    ] {
        let e = assemble(src, 0).unwrap_err().0;
        assert!(e.contains(needle), "{src}: {e}");
    }
    assert_eq!(
        decode_string("a\\").unwrap_err(),
        "escape incompleto no fim"
    );
}
