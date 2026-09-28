//! Assembler extensions: `.define`, `.ifdef`/`.ifndef`/`.elseif`, `.strlen`,
//! `.struct`/`.enum`/`.sizeof`, `::`, anonymous labels, `.res` and the cycle listing.

use mos6502::asm::{assemble, assemble_with_symbols, AsmOutput};

fn asm(src: &str) -> Vec<u8> {
    assemble(src, 0x0200).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

fn err(src: &str) -> String {
    assemble(src, 0x0200).unwrap_err().0
}

fn sym(out: &AsmOutput, name: &str) -> u16 {
    out.symbols[name]
}

#[test]
fn define_plain_and_with_parameters() {
    assert_eq!(asm(".define VAL $42\nlda #VAL\n"), [0xA9, 0x42]);
    assert_eq!(
        asm(".define ADD(a, b) a+b\nlda #ADD(1, 2)\nldx #ADD((1+1), 3*2)\n"),
        [0xA9, 3, 0xA2, 8]
    );
    // Nested: a define using another define.
    assert_eq!(asm(".define A1 2\n.define A2 A1*3\nlda #A2\n"), [0xA9, 6]);
    // Not replaced inside strings, after '.', after '$' or in longer words.
    assert_eq!(
        asm(".define AB 7\n.byte \"AB\", $AB, AB\nABC = 1\nlda #ABC\n"),
        [b'A', b'B', 0xAB, 7, 0xA9, 1]
    );
    // Case-sensitive.
    assert!(err(".define Val 1\nlda #val\n").contains("VAL"));
    // Zero-parameter call.
    assert_eq!(asm(".define NOP2() nop\nNOP2()\n"), [0xEA]);
}

#[test]
fn define_errors_and_undefine() {
    assert!(err(".define X 1\n.define X 2\n").contains(".define repetido"));
    assert!(err(".define F(a) a\nlda #F\n").contains("exige argumentos"));
    assert!(err(".define F(a) a\nlda #F(1,2)\n").contains("espera 1"));
    assert!(err(".define F(a,a) a\n").contains("repetido"));
    assert!(err(".define R R+1\nlda #R\n").contains("níveis"));
    assert!(err(".undefine Q\n").contains("não definido"));
    assert_eq!(
        asm(".define X 1\nlda #X\n.undefine X\n.define X 2\nlda #X\n"),
        [0xA9, 1, 0xA9, 2]
    );
    assert!(err(".define 9x 1\n").contains("nome inválido"));
}

#[test]
fn ifdef_ifndef_track_source_order() {
    let src = ".ifdef DEBUG\n.byte 1\n.else\n.byte 2\n.endif\n";
    assert_eq!(asm(src), [2]);
    assert_eq!(asm(&format!(".define DEBUG 1\n{src}")), [1]);
    assert_eq!(asm(&format!("debug = 1\n{src}")), [1]);
    assert_eq!(asm(&format!("debug: nop\n{src}")), [0xEA, 1]);
    // A definition after the test does not count.
    assert_eq!(asm(&format!("{src}DEBUG = 1\n")), [2]);
    assert_eq!(asm(".ifndef X\n.byte 9\n.endif\n"), [9]);
    // A definition in an inactive branch is not seen.
    assert_eq!(
        asm(".if 0\nFOO = 1\n.endif\n.ifdef FOO\n.byte 1\n.else\n.byte 0\n.endif\n"),
        [0]
    );
    // An undefine removes it.
    assert_eq!(
        asm(".define D\n.undefine D\n.ifdef D\n.byte 1\n.endif\nnop\n"),
        [0xEA]
    );
    assert!(err(".ifdef\n.endif\n").contains("sem nome"));
    assert!(err(".ifdef a b\n.endif\n").contains("único nome"));
}

#[test]
fn elseif_picks_exactly_one_arm() {
    for v in 0u8..8 {
        let src = format!(
            "v = {v}\n.if v & 4\n.byte $A4\n.elseif v & 2\n.byte $A2\n.elseif v & 1\n.byte $A1\n.else\n.byte $A0\n.endif\n"
        );
        let want = if v & 4 != 0 {
            0xA4
        } else if v & 2 != 0 {
            0xA2
        } else if v & 1 != 0 {
            0xA1
        } else {
            0xA0
        };
        assert_eq!(asm(&src), [want], "v={v}");
    }
    // Inactive arms are not evaluated (an undefined name there is fine).
    assert_eq!(asm(".if 1\n.byte 1\n.elseif nope\n.byte 2\n.endif\n"), [1]);
    assert!(err(".if 0\n.else\n.elseif 1\n.endif\n").contains("depois de .else"));
    assert!(err(".elseif 1\n").contains("sem .if"));
    assert!(err(".if 0\n.elseif\n.endif\n").contains("sem expressão"));
}

#[test]
fn nested_if_does_not_steal_the_outer_else() {
    // The inner .if ends at its own .endif; the .else belongs to the outer block.
    assert_eq!(
        asm(".if 1\n.if 0\n.byte 1\n.endif\n.else\n.byte 2\n.endif\n.byte 3\n"),
        [3]
    );
    assert_eq!(
        asm(".if 0\n.if 1\n.byte 1\n.endif\n.else\n.byte 2\n.endif\n"),
        [2]
    );
}

#[test]
fn if_sees_earlier_constants() {
    assert_eq!(
        asm("N = 3\n.if N*2 - 6\n.byte 0\n.else\n.byte 1\n.endif\n.rept N\nnop\n.endr\n"),
        [1, 0xEA, 0xEA, 0xEA]
    );
}

#[test]
fn strlen_counts_decoded_bytes() {
    assert_eq!(asm("lda #.strlen(\"hello\")\n"), [0xA9, 5]);
    assert_eq!(asm("lda #.STRLEN(\"\")\n"), [0xA9, 0]);
    assert_eq!(asm("lda #.strlen(\"a\\nb\")\n"), [0xA9, 3]);
    assert_eq!(
        asm(".define MSG \"abcd\"\nlda #.strlen(MSG)+1\n"),
        [0xA9, 5]
    );
    assert_eq!(
        asm(".pstring \".strlen\"\n"),
        [7, b'.', b's', b't', b'r', b'l', b'e', b'n']
    );
    assert_eq!(asm(".if .strlen(\"xy\") - 1\n.byte 1\n.endif\n"), [1]);
    for n in 0..=40usize {
        let s = "z".repeat(n);
        assert_eq!(asm(&format!(".byte .strlen(\"{s}\")\n")), [n as u8]);
    }
    assert!(err("lda #.strlen(abc)\n").contains("entre aspas"));
    assert!(err("lda #.strlen \"a\"\n").contains("exige"));
    assert!(err("lda #.strlen(\"a\"\n").contains("sem ')'"));
}

#[test]
fn struct_offsets_and_sizeof() {
    let out = assemble_with_symbols(
        ".struct Point\nx .word\ny .word\nflags .byte\n.endstruct\n\
         .struct Sprite\npos .tag Point\n.res 3\ntiles .byte 4\nnext .addr\nfar .faraddr\nlong .dword\n.endstruct\n\
         lda obj+Point::y\nldx #.sizeof(Point)\nldy #.sizeof(Sprite)\nlda #Sprite::pos::flags\nlda #.sizeof(Sprite::tiles)\nobj: .res .sizeof(Sprite), $FF\n",
        0x0200,
    )
    .unwrap();
    assert_eq!(sym(&out, "point__x"), 0);
    assert_eq!(sym(&out, "point__y"), 2);
    assert_eq!(sym(&out, "point__flags"), 4);
    assert_eq!(sym(&out, "sprite__pos"), 0);
    assert_eq!(sym(&out, "sprite__pos__y"), 2);
    assert_eq!(sym(&out, "sprite__tiles"), 8);
    assert_eq!(sym(&out, "sprite__next"), 12);
    assert_eq!(sym(&out, "sprite__far"), 14);
    assert_eq!(sym(&out, "sprite__long"), 17);
    let obj = sym(&out, "obj");
    assert_eq!(
        &out.bytes[..3],
        &[0xAD, (obj + 2) as u8, ((obj + 2) >> 8) as u8]
    );
    assert_eq!(&out.bytes[3..11], &[0xA2, 5, 0xA0, 21, 0xA9, 4, 0xA9, 4]);
    assert_eq!(out.bytes.len() as u16 - (obj - 0x0200), 21);
    assert!(out.bytes[usize::from(obj - 0x0200)..]
        .iter()
        .all(|&b| b == 0xFF));
}

#[test]
fn struct_base_offset_count_and_errors() {
    let out = assemble_with_symbols(
        ".struct S, 4\na .byte 3\nb .word 2\n.endstruct\nlda #.sizeof(S)\n",
        0,
    )
    .unwrap();
    assert_eq!(sym(&out, "s__a"), 4);
    assert_eq!(sym(&out, "s__b"), 7);
    assert_eq!(out.bytes, [0xA9, 7]);
    assert!(err(".struct S\na .byte\na .byte\n.endstruct\n").contains("campo repetido"));
    assert!(err(".struct S\na .float\n.endstruct\n").contains("desconhecido"));
    assert!(err(".struct S\na .byte\n").contains("sem .endstruct"));
    assert!(err(".endstruct\n").contains("sem .struct"));
    assert!(err(".struct S\nt .tag Q\n.endstruct\n").contains("desconhecida"));
    assert!(err(".struct S\n.endstruct\n.struct s\n.endstruct\n").contains("repetida"));
    assert!(err("lda #.sizeof(Nope)\n").contains("desconhecido"));
    assert!(err(".struct S\na .res\n.endstruct\n").contains("tamanho"));
    assert!(err(".struct S\nbig .res $8000\nmore .res $8001\n.endstruct\n").contains("64 KiB"));
    // .tag with a count multiplies the size but drops nested names.
    let out = assemble_with_symbols(
        ".struct P\nx .byte\ny .byte\n.endstruct\n.struct L\nps .tag P 3\nend .byte\n.endstruct\n",
        0,
    )
    .unwrap();
    assert_eq!(sym(&out, "l__end"), 6);
    assert!(!out.symbols.contains_key("l__ps__x"));
}

#[test]
fn enum_members() {
    let out = assemble_with_symbols(
        ".enum Color\nBlack\nRed\nGreen = 10\nBlue\nMask = Green | Red\n.endenum\n.enum\nFIRST\nSECOND\n.endenum\nlda #Color::Blue\n",
        0,
    )
    .unwrap();
    assert_eq!(
        [
            "color__black",
            "color__red",
            "color__green",
            "color__blue",
            "color__mask"
        ]
        .map(|n| sym(&out, n)),
        [0, 1, 0x10, 0x11, 0x11]
    );
    assert_eq!((sym(&out, "first"), sym(&out, "second")), (0, 1));
    assert_eq!(out.bytes, [0xA9, 0x11]);
    assert!(err(".enum E\nA\nA\n.endenum\n").contains("repetido"));
    assert!(err(".enum E\n1x\n.endenum\n").contains("inválido"));
    assert!(err(".enum E\nA = $FFFF\nB\n.endenum\n").contains("$FFFF"));
    assert!(err(".enum E\nA\n").contains("sem .endenum"));
    assert!(err(".endenum\n").contains("sem .enum"));
}

#[test]
fn scope_access_is_left_alone_in_strings() {
    assert_eq!(asm(".byte \"a::b\"\n"), b"a::b");
    assert_eq!(asm("lda #':'\n"), [0xA9, b':']);
}

#[test]
fn anonymous_labels_back_and_forward() {
    let out = assemble_with_symbols(
        "ldx #3\n: dex\nbne :-\nbeq :+\nnop\n: jmp :--\n:\n.word :-, :---\n",
        0x0200,
    )
    .unwrap();
    // $0200 ldx #3 ; $0202 dex ; $0203 bne -3 ; $0205 beq +1 ; $0207 nop ; $0208 jmp $0202 ; $020B .word $020B,$0202
    assert_eq!(
        out.bytes,
        [0xA2, 3, 0xCA, 0xD0, 0xFD, 0xF0, 0x01, 0xEA, 0x4C, 0x02, 0x02, 0x0B, 0x02, 0x02, 0x02]
    );
    assert!(err("bne :-\n").contains("anônimo inexistente"));
    assert!(err(": nop\nbne :+\n").contains("anônimo inexistente"));
    assert!(err(": nop\nbne :--\n").contains("anônimo inexistente"));
}

#[test]
fn anonymous_labels_every_distance() {
    for n in 1..=8usize {
        let mut src = String::new();
        for _ in 0..8 {
            src.push_str(": nop\n");
        }
        src.push_str(&format!("jmp :{}\n", "-".repeat(n)));
        let bytes = asm(&src);
        let target = 0x0200 + (8 - n) as u16;
        assert_eq!(&bytes[9..], &target.to_le_bytes(), "back {n}");
        let mut fwd = format!("jmp :{}\n", "+".repeat(n));
        for _ in 0..8 {
            fwd.push_str(": nop\n");
        }
        let bytes = asm(&fwd);
        assert_eq!(
            &bytes[1..3],
            &(0x0203 + (n - 1) as u16).to_le_bytes(),
            "fwd {n}"
        );
    }
}

#[test]
fn anonymous_labels_keep_local_scope_and_macros() {
    assert_eq!(
        asm("main:\n@l: nop\n: nop\nbne @l\n"),
        [0xEA, 0xEA, 0xD0, 0xFC]
    );
    assert_eq!(
        asm(".macro spin\n: dex\nbne :-\n.endm\nspin\nspin\n"),
        [0xCA, 0xD0, 0xFD, 0xCA, 0xD0, 0xFD]
    );
    assert_eq!(
        asm(".rept 2\n: nop\nbne :-\n.endr\n"),
        [0xEA, 0xD0, 0xFD, 0xEA, 0xD0, 0xFD]
    );
}

#[test]
fn res_reserves_with_optional_fill() {
    assert_eq!(asm(".res 3\n.res 2, $AA\n"), [0, 0, 0, 0xAA, 0xAA]);
    assert_eq!(asm(".res 0\nnop\n"), [0xEA]);
}

#[test]
fn listing_with_cycles() {
    let out = assemble_with_symbols(
        "start: lda #1\nlda $1234,x\nsta $1234,x\nbne start\n.byte 1\nX = 3\njmp ($FFFC)\nbra start\nnop ; c\n",
        0x0200,
    )
    .unwrap();
    let cyc: Vec<String> = out
        .lines
        .iter()
        .map(|l| AsmOutput::line_cycles(l).map_or("-".into(), |c| c.to_string()))
        .collect();
    assert_eq!(cyc, ["2", "4+p", "5", "2+t", "-", "-", "5", "3+t", "2"]);
    assert_eq!(out.base_cycles(), 2 + 4 + 5 + 2 + 5 + 3 + 2);
    let text = out.listing_with_cycles();
    assert!(
        text.contains("0200: A9 01        2     start: lda #1"),
        "{text}"
    );
    assert!(
        text.contains("0202: BD 34 12     4+p   lda $1234,x"),
        "{text}"
    );
}

#[test]
fn listing_cycles_match_opcode_table_for_every_documented_opcode() {
    use mos6502::opcodes::{decode, Mode};
    for code in 0u16..=255 {
        let Some(op) = decode(code as u8) else {
            continue;
        };
        let operand = match op.mode {
            Mode::Imp => String::new(),
            Mode::Acc => "a".into(),
            Mode::Imm => "#1".into(),
            Mode::Zp => "$10".into(),
            Mode::ZpX => "$10,x".into(),
            Mode::ZpY => "$10,y".into(),
            Mode::Abs => "$1234".into(),
            Mode::AbsX => "$1234,x".into(),
            Mode::AbsY => "$1234,y".into(),
            Mode::Ind => "($1234)".into(),
            Mode::IndX => "($10,x)".into(),
            Mode::IndY => "($10),y".into(),
            Mode::Rel => "*+2".into(),
            _ => continue,
        };
        let src = format!("{} {operand}\n", op.mnemonic);
        let Ok(out) = assemble_with_symbols(&src, 0x0200) else {
            continue;
        };
        if out.bytes.first() != Some(&op.code) {
            continue;
        }
        let c = AsmOutput::line_cycles(&out.lines[0]).unwrap();
        assert_eq!(c.base, op.cycles, "{src}");
        assert_eq!(c.branch, op.mode == Mode::Rel, "{src}");
    }
}
