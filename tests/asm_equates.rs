//! `=` / `.equ` constants under $100 defined on an earlier line give zero-page
//! operands; uses before the definition stay absolute; both passes agree.

use mos6502::assemble;

#[test]
fn earlier_small_constants_are_zero_page_for_every_value() {
    for v in 0..=0x1FFu16 {
        let src = format!("ptr = ${v:X}\n lda ptr\n sta ptr,x\n ldx ptr,y\n lda (ptr),y\n nop");
        let out = assemble(&src, 0x0400);
        if v >= 0x100 {
            assert!(
                out.unwrap_err().to_string().contains("8 bits"),
                "(zp),y with {v:X}"
            );
            let [lo, hi] = v.to_le_bytes();
            let abs = assemble(&format!("ptr = ${v:X}\n lda ptr"), 0).unwrap();
            assert_eq!(abs, vec![0xAD, lo, hi]);
            continue;
        }
        let out = out.unwrap();
        {
            assert_eq!(
                out,
                vec![0xA5, v as u8, 0x95, v as u8, 0xB6, v as u8, 0xB1, v as u8, 0xEA],
                "{v:X}"
            );
        }
    }
}

#[test]
fn forward_uses_stay_absolute_and_expressions_follow_the_same_rule() {
    let fwd = assemble(" lda ptr\nptr = $10\n lda ptr", 0).unwrap();
    assert_eq!(fwd, vec![0xAD, 0x10, 0x00, 0xA5, 0x10]);
    let expr = assemble(
        "base = $20\n lda base+2\n lda <base\n jmp end\nend: lda end",
        0x0200,
    )
    .unwrap();
    assert_eq!(
        expr,
        vec![0xA5, 0x22, 0xA5, 0x20, 0x4C, 0x07, 0x02, 0xAD, 0x07, 0x02]
    );
    // Code labels stay absolute even when their address is under $100.
    let low = assemble(" jmp l\nl: lda l", 0x0010).unwrap();
    assert_eq!(low, vec![0x4C, 0x13, 0x00, 0xAD, 0x13, 0x00]);
    // Instructions without a zero-page form still use absolute.
    assert_eq!(
        assemble("v = $10\n jmp v", 0).unwrap(),
        vec![0x4C, 0x10, 0x00]
    );
}
