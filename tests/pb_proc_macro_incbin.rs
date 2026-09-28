//! `.proc`/`.endproc` scopes and `.export`, macro varargs with `.paramcount`,
//! `.ifblank`/`.ifnblank`, and `.incbin` with start and size.

use mos6502::asm::{assemble, assemble_with_blobs, assemble_with_symbols};
use std::collections::HashMap;

fn bytes(src: &str) -> Vec<u8> {
    assemble(src, 0x8000).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

fn err(src: &str) -> String {
    assemble(src, 0x8000).unwrap_err().0
}

#[test]
fn proc_scopes_labels_and_constants() {
    let src = ".proc main\n  n = 3\nloop: dex\n  bne loop\n  rts\n.endproc\n\
               .proc other\nloop: nop\n  jmp loop\n.endproc\n\
               .word main, main::loop, other::loop\n.byte main::n\n";
    let out = assemble_with_symbols(src, 0x8000).unwrap();
    assert_eq!(&out.bytes[..4], [0xCA, 0xD0, 0xFD, 0x60]);
    assert_eq!(&out.bytes[4..8], [0xEA, 0x4C, 0x04, 0x80]);
    assert_eq!(&out.bytes[8..], [0x00, 0x80, 0x00, 0x80, 0x04, 0x80, 3]);
    assert_eq!(out.symbols.get("main__loop"), Some(&0x8000));
    assert_eq!(out.symbols.get("other__loop"), Some(&0x8004));
    assert!(!out.symbols.contains_key("loop"));
}

#[test]
fn nested_procs_see_outer_names_and_reach_inner_ones() {
    let src = "top = 9\n.proc outer\n  k = 1\n  .proc inner\n    v: .byte k, top\n  .endproc\n  .word inner::v, inner\n.endproc\n.word outer::inner::v\n";
    assert_eq!(bytes(src), [1, 9, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
}

#[test]
fn inner_definition_shadows_outer() {
    let src = "x = 1\n.proc p\n  x = 2\n  .byte x\n.endproc\n.byte x, p::x\n";
    assert_eq!(bytes(src), [2, 1, 2]);
}

#[test]
fn export_publishes_short_names() {
    let src =
        ".proc lib\n  .export put, size\nput: rts\n  size = 7\n.endproc\n jsr put\n .byte size\n";
    assert_eq!(bytes(src), [0x60, 0x20, 0x00, 0x80, 7]);
    assert_eq!(
        bytes(".export a\na: nop\n"),
        [0xEA],
        "top-level export is a no-op"
    );
}

#[test]
fn proc_locals_are_confined() {
    let src = ".proc a\n@l: nop\n bne @l\n.endproc\n.proc b\n@l: nop\n bne @l\n.endproc\n";
    assert_eq!(bytes(src), [0xEA, 0xD0, 0xFD, 0xEA, 0xD0, 0xFD]);
}

#[test]
fn proc_errors() {
    assert!(err(".proc a\nnop\n").contains(".proc sem .endproc"));
    assert!(err(".endproc\n").contains(".endproc sem .proc"));
    assert!(err(".proc\n.endproc\n").contains("exige um nome"));
    assert!(err(".proc 1x\n.endproc\n").contains("nome inválido"));
    assert!(err(".proc a\nx: nop\nx: nop\n.endproc\n").contains("nome repetido"));
    assert!(err(".proc a\n.export nope\n.endproc\n").contains("não definido"));
    assert!(
        err("x: nop\n.proc a\n.export x\n.endproc\n").contains("não definido"),
        "outer names are not exportable"
    );
    let mut deep = String::new();
    for i in 0..17 {
        deep += &format!(".proc p{i}\n");
    }
    assert!(err(&deep).contains("além de 16"));
}

#[test]
fn macro_varargs_and_paramcount() {
    let src = ".macro bytes first, rest...\n .byte \\first\n.ifnblank \\rest\n .byte \\rest\n.endif\n .byte .paramcount\n.endmacro\n\
               bytes 1\n bytes 1, 2, 3\n";
    assert_eq!(bytes(src), [1, 1, 1, 2, 3, 3]);
    let only = ".macro all items...\n.ifblank \\items\n .byte $FF\n.else\n .byte \\items\n.endif\n.endmacro\n all\n all 5,6\n";
    assert_eq!(bytes(only), [0xFF, 5, 6]);
}

#[test]
fn ifblank_outside_macros_and_errors() {
    assert_eq!(
        bytes(".ifblank\n.byte 1\n.endif\n.ifnblank x\n.byte 2\n.endif\n"),
        [1, 2]
    );
    assert_eq!(bytes(".ifblank x\n.byte 1\n.else\n.byte 3\n.endif\n"), [3]);
    assert!(err(".macro m a..., b\n.endmacro\n").contains("depois do variádico"));
    assert!(err(".macro m a, b...\n.endmacro\n m\n").contains("pelo menos 1"));
    assert!(err(".macro m a\n.endmacro\n m 1, 2\n").contains("espera 1"));
}

#[test]
fn incbin_start_and_size() {
    let blob: Vec<u8> = (0..16).collect();
    let mut blobs = HashMap::new();
    blobs.insert("d.bin".to_string(), blob);
    let files = HashMap::new();
    let run = |src: &str| assemble_with_blobs(src, 0x8000, &files, &blobs).map(|o| o.bytes);
    assert_eq!(
        run(".incbin \"d.bin\"\n").unwrap(),
        (0..16).collect::<Vec<u8>>()
    );
    assert_eq!(run(".incbin \"d.bin\", 0d12\n").unwrap(), [12, 13, 14, 15]);
    assert_eq!(run(".incbin \"d.bin\", 2, 3\n").unwrap(), [2, 3, 4]);
    assert_eq!(
        run("n = 4\n.incbin \"d.bin\", n * 2, n - 2\n").unwrap(),
        [8, 9]
    );
    assert_eq!(run(".incbin \"d.bin\", $10\n").unwrap(), Vec::<u8>::new());
    assert_eq!(run(".incbin d.bin, 1, 1\n").unwrap(), [1]);
    assert!(run(".incbin \"d.bin\", $11\n")
        .unwrap_err()
        .0
        .contains("passa do tamanho"));
    assert!(run(".incbin \"d.bin\", 8, 9\n")
        .unwrap_err()
        .0
        .contains("passa do tamanho"));
    assert!(run(".incbin \"d.bin\", 1, 2, 3\n")
        .unwrap_err()
        .0
        .contains("só início e tamanho"));
    assert!(run(".incbin \"d.bin\" x\n")
        .unwrap_err()
        .0
        .contains("texto depois"));
    // Every (start, size) pair of the 16-byte blob.
    for start in 0..=16u8 {
        for size in 0..=(16 - start) {
            let got = run(&format!(".incbin \"d.bin\", 0d{start}, 0d{size}\n")).unwrap();
            assert_eq!(got, (start..start + size).collect::<Vec<u8>>());
        }
    }
}
