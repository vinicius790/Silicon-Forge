//! Comparison and logical operators next to the `<`/`>` byte operators.

use mos6502::expr::eval;

fn ev(s: &str) -> Result<u16, String> {
    let lookup = |n: &str| match n {
        "lbl" => Some(0x1234),
        "zero" => Some(0),
        _ => None,
    };
    eval(s, &lookup).map_err(|e| e.0)
}

fn v(s: &str) -> u16 {
    ev(s).unwrap_or_else(|e| panic!("{s}: {e}"))
}

#[test]
fn every_comparison_matches_rust_over_a_grid() {
    let vals = [
        0u16, 1, 2, 0x7F, 0x80, 0xFF, 0x100, 0x7FFF, 0x8000, 0xFFFE, 0xFFFF,
    ];
    for &a in &vals {
        for &b in &vals {
            let cases: [(&str, &str, bool); 6] = [
                ("==", ".eq", a == b),
                ("!=", ".ne", a != b),
                ("<", ".lt", a < b),
                (">", ".gt", a > b),
                ("<=", ".le", a <= b),
                (">=", ".ge", a >= b),
            ];
            for (sym, word, want) in cases {
                let want = u16::from(want);
                assert_eq!(v(&format!("${a:X} {sym} ${b:X}")), want, "{a} {sym} {b}");
                assert_eq!(v(&format!("${a:X}{sym}${b:X}")), want, "{a}{sym}{b}");
                assert_eq!(v(&format!("${a:X} {word} ${b:X}")), want, "{a} {word} {b}");
                assert_eq!(v(&format!("${a:X} {} ${b:X}", word.to_uppercase())), want);
            }
            assert_eq!(v(&format!("${a:X} <> ${b:X}")), u16::from(a != b));
            assert_eq!(v(&format!("${a:X} && ${b:X}")), u16::from(a != 0 && b != 0));
            assert_eq!(v(&format!("${a:X} || ${b:X}")), u16::from(a != 0 || b != 0));
            assert_eq!(
                v(&format!("${a:X} .and ${b:X}")),
                u16::from(a != 0 && b != 0)
            );
            assert_eq!(
                v(&format!("${a:X} .or ${b:X}")),
                u16::from(a != 0 || b != 0)
            );
            assert_eq!(
                v(&format!("${a:X} .xor ${b:X}")),
                u16::from((a != 0) != (b != 0))
            );
            if b != 0 {
                assert_eq!(v(&format!("${a:X} .mod ${b:X}")), a % b);
            }
            assert_eq!(v(&format!("${a:X} .bitand ${b:X}")), a & b);
            assert_eq!(v(&format!("${a:X} .bitor ${b:X}")), a | b);
            assert_eq!(v(&format!("${a:X} .bitxor ${b:X}")), a ^ b);
        }
        assert_eq!(v(&format!("!${a:X}")), u16::from(a == 0));
        assert_eq!(v(&format!(".not ${a:X}")), u16::from(a == 0));
        assert_eq!(v(&format!("${a:X} .shl 1")), a << 1);
        assert_eq!(v(&format!("${a:X} .shr 1")), a >> 1);
    }
}

#[test]
fn byte_operators_still_work_and_combine() {
    assert_eq!(v("<lbl"), 0x34);
    assert_eq!(v(">lbl"), 0x12);
    assert_eq!(v("lbl >> 8"), 0x12);
    assert_eq!(v("1 << 4"), 0x10);
    assert_eq!(v("<lbl == $34"), 0, "prefix covers the whole rest");
    assert_eq!(v("(<lbl) == $34"), 1);
    assert_eq!(v("$34 == <lbl"), 1, "prefix after an operator");
    assert_eq!(v("$12 == >lbl"), 1);
    assert_eq!(v("$33 < <lbl"), 1);
    assert_eq!(v("$13 > >lbl"), 1);
    assert_eq!(v("$12 >= >lbl"), 1);
    assert_eq!(v("$12 <= >lbl"), 1);
    assert_eq!(v("lbl>>8==$12"), 1);
    assert_eq!(v("1<<2<4"), 0);
    assert_eq!(v("1<<2<=4"), 1);
}

#[test]
fn precedence_and_associativity() {
    assert_eq!(v("1 + 1 == 2"), 1, "+ binds tighter than ==");
    assert_eq!(v("1 == 1 && 2 == 2"), 1);
    assert_eq!(v("0 && 1 || 1"), 1, "&& binds tighter than ||");
    assert_eq!(v("1 || 0 && 0"), 1);
    assert_eq!(v("1 .xor 1 .or 1"), 1);
    assert_eq!(v("1 .or 1 .xor 1"), 1, ".xor binds tighter than .or");
    assert_eq!(v("1 .xor 1 .and 0"), 1, ".and binds tighter than .xor");
    assert_eq!(v("3 & 1 == 1"), 3 & 1, "& binds tighter than == as in ca65");
    assert_eq!(v("2 == 2 == 1"), 1, "left associative");
    assert_eq!(v("!0 + 1"), 2, "! is a prefix");
    assert_eq!(v("!zero == 1"), 1);
    assert_eq!(v("2*-1"), 0xFFFE, "unary minus after *");
    assert_eq!(v("8-2-1"), 5);
    assert_eq!(v("0d10 .mod 3 + 1"), 2);
    assert_eq!(v("(1 == 1) * $10"), 0x10);
}

#[test]
fn errors() {
    assert!(ev("1 ==").is_err());
    assert!(ev("1 && ").is_err());
    assert!(ev("5 .mod 0").unwrap_err().contains("zero"));
    assert!(ev("a.ltb").is_err(), "word operators must stand alone");
    assert!(ev("(1 == 1").is_err());
    assert!(ev("1 / 0 || 1").is_err(), "no short circuit");
}

#[test]
fn assembler_uses_the_operators() {
    let src = "val = 5\n.if val >= 3 && val != 4\n.byte 1\n.else\n.byte 2\n.endif\n.byte val == 5, val .lt 2, <$1234 == $34\n";
    assert_eq!(mos6502::assemble(src, 0x8000).unwrap(), [1, 1, 0, 0]);
}
