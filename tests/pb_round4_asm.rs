//! Round 4 assembler: macpack, ifconst/ifref, .repeat counter, .local, strings.

use mos6502::asm::{assemble, assemble_with_symbols};

#[test]
fn macpack_longbranch_and_generic() {
    let src = ".macpack longbranch\n.org $8000\n start: jne far\n nop\nfar: rts\n";
    let bytes = assemble(src, 0x8000).unwrap();
    assert_eq!(&bytes[..5], [0xD0, 0x03, 0x4C, 0x06, 0x80]);
    let src = ".macpack generic\n.org $00\n ptr = $10\n add16 ptr, $1234\n";
    let bytes = assemble(src, 0).unwrap();
    assert_eq!(bytes[0], 0x18);
    assert!(assemble(".macpack no_such\n", 0).is_err());
}

#[test]
fn ifconst_ifref_repeat_local_strings() {
    let src = "n = 3\n.ifconst n + 1\n.byte 1\n.else\n.byte 0\n.endif\n.ifconst missing\n.byte 9\n.endif\n\
               lda target\n.ifref target\n.byte 2\n.endif\n.ifref nowhere\n.byte 8\n.endif\n\
               .repeat 3, i\n .byte i\n.endrep\n\
               .scope\n.local lab\nlab: nop\n jmp lab\n.endscope\n\
               target: .byte .concat(\"A\", \"B\"), .left(\"xyz\", 2), .right(\"xyz\", 1), .mid(\"abcdef\", 2, 3)\n\
               .byte .sprintf(\"%02X-%s\", $AB, \"ok\")\n";
    let out = assemble_with_symbols(src, 0x8000).unwrap();
    assert!(out.bytes.starts_with(&[1, 0xAD]), "{:?}", &out.bytes[..8]);
    assert!(out.bytes.contains(&2));
    assert!(out.bytes.windows(3).any(|w| w == [0, 1, 2]));
    assert!(!out.symbols.contains_key("lab"));
    assert!(assemble(".byte .sprintf(\"%d\", 0d10)\n", 0)
        .unwrap()
        .windows(2)
        .any(|w| w == b"10"));
}

#[test]
fn local_confined_and_string_errors() {
    let src = ".scope\n.local foo\nfoo: nop\n.endscope\n jmp foo\n";
    assert!(
        assemble(src, 0x8000).is_err(),
        "outer must not see .local foo"
    );
    assert!(assemble(".byte .left(1, 2)\n", 0).is_err());
    assert!(assemble(".byte .sprintf(\"%q\", 1)\n", 0).is_err());
}
