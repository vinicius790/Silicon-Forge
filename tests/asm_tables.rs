//! Split tables (.lobytes/.hibytes), big-endian words (.dbyt) and .assert.

use mos6502::asm::{assemble, assemble_with_symbols};

#[test]
fn split_tables_over_every_word_pattern() {
    let values: Vec<u16> = (0..=255u16)
        .map(|i| i.wrapping_mul(0x0101) ^ (i << 3))
        .collect();
    let list = values
        .iter()
        .map(|v| format!("${v:04X}"))
        .collect::<Vec<_>>()
        .join(",");
    let src = format!(".org $1000\nlo: .lobytes {list}\nhi: .hibytes {list}\nbe: .dbyt {list}\n");
    let out = assemble_with_symbols(&src, 0x1000).unwrap();
    let b = &out.bytes;
    for (i, v) in values.iter().enumerate() {
        assert_eq!(b[i], *v as u8);
        assert_eq!(b[256 + i], (*v >> 8) as u8);
        assert_eq!(&b[512 + 2 * i..514 + 2 * i], &v.to_be_bytes());
    }
    assert_eq!(out.symbols["hi"], 0x1100);
    assert_eq!(out.symbols["be"], 0x1200);
}

#[test]
fn split_tables_take_forward_labels_and_expressions() {
    let src =
        "lo: .lobytes a, b, b+1\nhi: .hibytes a, b, b+1\n.org $2345\na: nop\n.org $3456\nb: nop";
    let bytes = assemble(src, 0x1000).unwrap();
    assert_eq!(&bytes[..6], &[0x45, 0x56, 0x57, 0x23, 0x34, 0x34]);
    // Typical jump table use: lda lo,x / lda hi,x.
    for bad in [".lobytes", ".hibytes \"ab\"", ".dbyt"] {
        assert!(assemble(bad, 0).unwrap_err().0.contains("valores"), "{bad}");
    }
}

#[test]
fn assert_checks_final_values_with_messages() {
    // The value is the expression; zero fails (2 - 2 below), non-zero passes.
    assert!(assemble("start: nop\nnop\nend:\n.assert end-start-2", 0x8000).is_err());
    assert!(assemble("start: nop\nnop\nend:\n.assert end-start", 0x8000).is_ok());
    assert!(
        assemble(".assert fwd\nfwd = 1", 0).is_ok(),
        "forward symbols resolve"
    );
    let err = assemble("x = 0\n.assert x", 0).unwrap_err().0;
    assert!(err.contains(".assert falhou: x"), "{err}");
    let err = assemble("x = 0\n.assert x, \"tabela grande demais\"", 0)
        .unwrap_err()
        .0;
    assert!(err.contains("tabela grande demais"), "{err}");
    assert!(assemble(".assert", 0).unwrap_err().0.contains("exige"));
    // Emits nothing.
    assert_eq!(
        assemble("nop\n.assert 1\nnop", 0).unwrap(),
        vec![0xEA, 0xEA]
    );
}
