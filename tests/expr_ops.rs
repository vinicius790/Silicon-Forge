//! Expression operators added in Part A: division, unary minus and complement.

use mos6502::expr::eval;

fn ev(s: &str) -> Result<u16, String> {
    eval(s, &|name| (name == "base").then_some(0x1234)).map_err(|e| e.0)
}

#[test]
fn division_unary_and_associativity() {
    assert_eq!(ev("$64/$0A"), Ok(10));
    assert_eq!(ev("7/2"), Ok(3), "truncates");
    assert_eq!(ev("$64/$0A/2"), Ok(5), "left associative");
    assert_eq!(ev("8-2-1"), Ok(5));
    assert_eq!(ev("2+6/2"), Ok(5), "/ binds tighter than +");
    assert_eq!(ev("-1"), Ok(0xFFFF));
    assert_eq!(ev("~base"), Ok(!0x1234));
    assert_eq!(ev("<-base"), Ok(0xCC));
    assert_eq!(ev("-(2+3)"), Ok(0u16.wrapping_sub(5)));
    assert_eq!(ev("base/0"), Err("divisão por zero".into()));
    assert!(ev("/4").is_err());
}

#[test]
fn assembler_uses_the_new_operators() {
    let code = mos6502::assemble("lda #<(-2)\nldx #$64/$0A\n", 0x0200).unwrap();
    assert_eq!(code, [0xA9, 0xFE, 0xA2, 0x0A]);
}
