//! `.union`, anonymous and named nested `.struct`/`.union` (ca65 layout rules).

use mos6502::asm::assemble;

fn bytes(src: &str) -> Vec<u8> {
    assemble(src, 0x8000).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

fn err(src: &str) -> String {
    assemble(src, 0x8000).unwrap_err().0
}

#[test]
fn union_members_share_offset_and_size_is_the_largest() {
    let src = ".union Val\n b .byte\n w .word\n d .dword\n arr .byte 3\n.endunion\n\
               .byte Val::b, Val::w, Val::d, Val::arr, .sizeof(Val), .sizeof(Val::w), .sizeof(Val::arr)\n";
    assert_eq!(bytes(src), [0, 0, 0, 0, 4, 2, 3]);
}

#[test]
fn union_with_base_and_tag_inside_struct() {
    let src = ".union U, $10\n a .word\n b .byte\n.endunion\n\
               .struct S\n head .byte\n u .tag U\n tail .byte\n.endstruct\n\
               .byte U::a, U::b, .sizeof(U), S::u, S::u::a, S::tail, .sizeof(S)\n";
    // `.tag` of a based union keeps the union's absolute offsets inside it.
    let out = bytes(src);
    assert_eq!(&out[..3], [0x10, 0x10, 2]);
    assert_eq!(out[3], 1);
    assert_eq!(out[5], 3);
    assert_eq!(out[6], 4);
}

#[test]
fn anonymous_union_inside_struct_is_flattened() {
    let src = ".struct Pkt\n kind .byte\n .union\n  lo .byte\n  wide .word\n .endunion\n crc .byte\n.endstruct\n\
               .byte Pkt::kind, Pkt::lo, Pkt::wide, Pkt::crc, .sizeof(Pkt)\n";
    assert_eq!(bytes(src), [0, 1, 1, 3, 4]);
}

#[test]
fn anonymous_struct_inside_union_lays_out_sequentially() {
    let src = ".union Reg\n .struct\n  l .byte\n  h .byte\n .endstruct\n x .word\n.endunion\n\
               .byte Reg::l, Reg::h, Reg::x, .sizeof(Reg)\n";
    assert_eq!(bytes(src), [0, 1, 0, 2]);
}

#[test]
fn named_nested_members_have_scoped_fields() {
    let src = ".struct Sprite\n y .byte\n pos .struct\n  px .byte\n  py .byte\n .endstruct\n alt .union\n  f .byte\n  g .dword\n .endunion\n tile .byte\n.endstruct\n\
               .byte Sprite::pos, Sprite::pos::px, Sprite::pos::py, .sizeof(Sprite::pos), Sprite::alt, Sprite::alt::g, .sizeof(Sprite::alt), Sprite::tile, .sizeof(Sprite)\n";
    assert_eq!(bytes(src), [1, 1, 2, 2, 3, 3, 4, 7, 8]);
}

#[test]
fn deep_nesting_alternates_struct_and_union() {
    let mut body = String::from(" leaf .byte\n");
    for depth in 0..8 {
        let (open, close) = if depth % 2 == 0 {
            (".union", ".endunion")
        } else {
            (".struct", ".endstruct")
        };
        body = format!(" {open}\n{body} pad{depth} .word\n {close}\n");
    }
    let src = format!(".struct Deep\n{body}.endstruct\n.byte .sizeof(Deep), Deep::leaf\n");
    let out = bytes(&src);
    assert_eq!(out[1], 0);
    // Each struct level adds a word after the inner member; each union keeps max(inner, 2).
    let mut size = 1u8;
    for depth in 0..8 {
        size = if depth % 2 == 0 {
            size.max(2)
        } else {
            size + 2
        };
    }
    assert_eq!(out[0], size);
}

#[test]
fn errors_are_reported() {
    assert!(err(".union U\n a .byte\n").contains(".union sem .endunion"));
    assert!(err(".endunion\n").contains(".endunion sem .union"));
    assert!(err(".union U\n a .byte\n.endstruct\n").contains("fecha"));
    assert!(err(".union U\n a .byte\n a .word\n.endunion\n").contains("campo repetido"));
    assert!(
        err(".struct S\n a .byte\n .union\n  a .word\n .endunion\n.endstruct\n")
            .contains("campo repetido")
    );
    assert!(err(".union U\n.endunion\n.union U\n.endunion\n").contains("repetida"));
    assert!(err(".struct S\n .struct X\n .endstruct\n.endstruct\n").contains("não leva nome"));
    let mut deep = String::from(".struct S\n");
    for _ in 0..17 {
        deep += ".union\n";
    }
    assert!(err(&deep).contains("aninhada além"));
    assert!(err(".union Big, $FFFF\n a .word\n.endunion\n").contains("64 KiB"));
}

#[test]
fn empty_union_has_size_zero() {
    assert_eq!(bytes(".union E\n.endunion\n.byte .sizeof(E)\n"), [0]);
}
