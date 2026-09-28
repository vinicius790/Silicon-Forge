//! Preprocessor: repetition counts, every `.if` nesting combination, macro depth and
//! every structural error.

use mos6502::assemble;

#[test]
fn rept_emits_exactly_n_copies_for_0_to_64() {
    for n in 0..=64usize {
        let bytes = assemble(&format!(".rept 0d{n}\n nop\n.endr\n brk"), 0x0400).unwrap();
        assert_eq!(bytes.len(), n + 1, "{n}");
        assert!(bytes[..n].iter().all(|&b| b == 0xEA));
    }
    let nested = assemble(".rept 3\n.rept 4\n inx\n.endr\n.endr", 0).unwrap();
    assert_eq!(nested, vec![0xE8; 12]);
    assert!(assemble(".rept 0d4097\n nop\n.endr", 0).is_err());
    assert!(assemble(".rept 0d4096\n.endr", 0).is_ok());
}

#[test]
fn every_three_level_if_combination_selects_one_path() {
    for mask in 0..8u8 {
        let (a, b, c) = (mask & 1, (mask >> 1) & 1, (mask >> 2) & 1);
        let src = format!(
            ".if {a}\n.if {b}\n lda #1\n.else\n lda #2\n.endif\n.else\n.if {c}\n lda #3\n.else\n lda #4\n.endif\n.endif"
        );
        let want = match (a, b, c) {
            (1, 1, _) => 1,
            (1, 0, _) => 2,
            (0, _, 1) => 3,
            _ => 4,
        };
        assert_eq!(assemble(&src, 0).unwrap(), vec![0xA9, want], "{mask}");
    }
}

#[test]
fn macros_nest_to_depth_four_and_stop_at_five() {
    let chain = |depth: usize| {
        let mut src = String::from(".macro m0\n nop\n.endmacro\n");
        for i in 1..depth {
            src.push_str(&format!(".macro m{i}\n m{}\n.endmacro\n", i - 1));
        }
        src.push_str(&format!(" m{}\n", depth - 1));
        assemble(&src, 0)
    };
    for depth in 1..=4 {
        assert_eq!(chain(depth).unwrap(), vec![0xEA], "{depth}");
    }
    assert!(chain(5).is_err());
    assert!(
        assemble(".macro r\n r\n.endmacro\n r", 0).is_err(),
        "self call stops"
    );
}

#[test]
fn structural_errors_name_the_problem() {
    for (src, needle) in [
        (".endr", ".endr sem .rept"),
        (".else", ".else sem .if"),
        (".endif", ".endif sem .if"),
        (".rept 2\n nop", ".rept sem .endr"),
        (".if 1\n nop", ".if sem .endif"),
        (".endmacro", ".endmacro sem .macro"),
        (".macro a\n nop", ".macro sem .endmacro"),
        (
            ".macro a\n.endmacro\n.macro a\n.endmacro",
            "macro duplicada: a",
        ),
        (".endscope", ".endscope sem .scope"),
        (".scope\n nop", ".scope sem .endscope"),
        (".include \"x\"", ".include não encontrado"),
    ] {
        let err = assemble(src, 0).unwrap_err().to_string();
        assert!(err.contains(needle), "{src:?}: {err}");
    }
}

#[test]
fn preprocessor_errors_carry_the_source_line() {
    for (src, want) in [
        (" nop\n nop\n.endr", "linha 3: .endr sem .rept"),
        (" nop\n.else", "linha 2: .else sem .if"),
        ("\n\n\n.endif", "linha 4: .endif sem .if"),
        (" nop\n.macro a\n nop", "linha 2: .macro sem .endmacro"),
        ("\n.endmacro", "linha 2: .endmacro sem .macro"),
        ("\n.scope\n nop", "linha 2: .scope sem .endscope"),
        ("\n\n.endscope", "linha 3: .endscope sem .scope"),
        ("x:\n@a: nop\n@a: nop", "linha 3: label local duplicado: @a"),
        ("\n.include \"none\"", "linha 2: .include não encontrado"),
        (".rept 0d4097\n.endr", "linha 1: .rept maior que 4096"),
    ] {
        let err = assemble(src, 0).unwrap_err().to_string();
        assert!(err.starts_with(want), "{src:?}: {err}");
    }
    // Lines of an included file are named by file and line.
    let mut files = std::collections::HashMap::new();
    files.insert("inc".to_string(), " nop\n.endr\n".to_string());
    let err = mos6502::asm::assemble_with_files(" nop\n.include \"inc\"", 0, &files).unwrap_err();
    assert!(
        err.to_string().starts_with("linha inc:2: .endr sem .rept"),
        "{err}"
    );
    // Tags never leak into output or listings.
    let out = mos6502::asm::assemble_with_symbols("a: lda #1 ; c\n .byte \"x;y\"", 0).unwrap();
    assert!(out.listing().chars().all(|c| c != '\u{1}'));
}

#[test]
fn macro_body_errors_name_the_call_and_the_definition_line() {
    let src = ".macro bad\n nop\n.endr\n.endmacro\n nop\n bad";
    let err = assemble(src, 0).unwrap_err().to_string();
    assert!(
        err.starts_with("linha 6: (macro, linha 3) .endr sem .rept"),
        "{err}"
    );
    let nested = ".macro inner\n.else\n.endmacro\n.macro outer\n inner\n.endmacro\n\n outer";
    let err = assemble(nested, 0).unwrap_err().to_string();
    assert!(
        err.starts_with("linha 8: (macro, linha 2) .else sem .if"),
        "{err}"
    );
    let arity = ".macro two a,b\n lda #a\n.endmacro\n two 1";
    let err = assemble(arity, 0).unwrap_err().to_string();
    assert!(err.starts_with("linha 4: macro two espera 2"), "{err}");
}
