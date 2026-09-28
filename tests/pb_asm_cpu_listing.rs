//! CPU selection (`.p02`, `.psc02`, `.pc02`, `.pwc02`, `.setcpu`) and the listing
//! file with macro expansions.

use mos6502::asm::{assemble, assemble_with_symbols, AsmCpu};
use mos6502::opcodes::{decode_variant, CpuVariant, OpType};

#[test]
fn every_opcode_class_is_ranked() {
    for op in 0..=255u8 {
        let need = AsmCpu::required_for(op);
        let official =
            decode_variant(op, CpuVariant::NMOS).is_some_and(|i| i.op_type == OpType::Official);
        let cmos_ext = decode_variant(op, CpuVariant::CMOS)
            .is_some_and(|i| i.op_type == OpType::CmosExtension);
        let nmos = decode_variant(op, CpuVariant::NMOS);
        if cmos_ext {
            assert!(
                matches!(need, AsmCpu::Sc65c02 | AsmCpu::R65c02 | AsmCpu::W65c02),
                "${op:02X} {need:?}"
            );
            assert!(AsmCpu::W65c02.accepts(op));
            assert!(!AsmCpu::Nmos6502.accepts(op) || official);
        } else if official {
            assert_eq!(need, AsmCpu::Nmos6502, "${op:02X}");
            assert!(AsmCpu::Nmos6502.accepts(op));
            assert!(AsmCpu::Nmos6502X.accepts(op));
            assert!(AsmCpu::W65c02.accepts(op));
        } else if nmos.is_some() {
            assert_eq!(need, AsmCpu::Nmos6502X, "${op:02X}");
            assert!(AsmCpu::Nmos6502X.accepts(op));
            assert!(AsmCpu::Any.accepts(op));
            assert!(!AsmCpu::Nmos6502.accepts(op));
            assert!(!AsmCpu::W65c02.accepts(op));
        }
        assert!(AsmCpu::Any.accepts(op));
    }
    assert_eq!(
        AsmCpu::required_for(0x07),
        AsmCpu::R65c02,
        "RMB0 shares $07 with SLO"
    );
    assert_eq!(AsmCpu::required_for(0x0F), AsmCpu::R65c02, "BBR0");
    assert_eq!(AsmCpu::required_for(0xCB), AsmCpu::W65c02, "WAI");
    assert_eq!(AsmCpu::required_for(0xDB), AsmCpu::W65c02, "STP");
    assert_eq!(AsmCpu::required_for(0x80), AsmCpu::Sc65c02, "BRA");
    assert_eq!(
        AsmCpu::required_for(0x64),
        AsmCpu::Sc65c02,
        "STZ zp wins over NMOS illegal"
    );
}

#[test]
fn directives_select_and_reject() {
    let cases: [(&str, &str, bool); 15] = [
        (".p02", "stz $10", false),
        (".p02", "lda ($10)", false),
        (".p02", "inc a", false),
        (".p02", "lda #1", true),
        (".p02", "lax $10", false),
        (".psc02", "stz $10", true),
        (".psc02", "rmb0 $10", false),
        (".psc02", "wai", false),
        (".pc02", "rmb0 $10", true),
        (".pc02", "bbs7 $10, $8000", true),
        (".pc02", "stp", false),
        (".pwc02", "stp", true),
        (".setcpu \"6502X\"", "lax $10", true),
        (".setcpu \"R65C02\"", "smb3 $20", true),
        (".setcpu \"W65C02\"", "wai", true),
    ];
    for (dir, ins, ok) in cases {
        let src = format!("{dir}\n {ins}\n");
        let res = assemble(&src, 0x8000);
        assert_eq!(res.is_ok(), ok, "{src}: {res:?}");
        if !ok {
            let e = res.unwrap_err().0;
            assert!(
                e.contains("exige") || e.contains("instrução ou modo inválido"),
                "{e}"
            );
        }
    }
}

#[test]
fn selection_changes_mid_file_and_data_is_never_checked() {
    let src = ".p02\n.byte $64, $10\n.pc02\n stz $10\n.p02\n lda #0\n";
    assert_eq!(
        assemble(src, 0x8000).unwrap(),
        [0x64, 0x10, 0x64, 0x10, 0xA9, 0]
    );
    assert!(assemble(".p02\n.pc02\n.p02\n bra $8000\n", 0x8000).is_err());
    assert!(
        assemble(" stz $10\n", 0x8000).is_ok(),
        "no selection accepts everything"
    );
}

#[test]
fn bad_selections() {
    assert!(assemble(".setcpu \"z80\"\n", 0)
        .unwrap_err()
        .0
        .contains("desconhecida"));
    assert!(assemble(".setcpu\n", 0).unwrap_err().0.contains("exige"));
    assert!(assemble(".p02 x\n", 0).unwrap_err().0.contains("não leva"));
}

#[test]
fn listing_file_shows_macro_expansions_under_their_call() {
    let src = ".macro two a, b\n lda #\\a\n ldx #\\b\n.endmacro\nstart: nop\n two 1, 2\n .byte 1,2,3,4,5,6\n";
    let out = assemble_with_symbols(src, 0x8000).unwrap();
    let listing = out.listing_file(src);
    let lines: Vec<&str> = listing.lines().collect();
    assert_eq!(lines[0], "  LINHA   END   BYTES        FONTE");
    assert_eq!(lines[1], "      5  8000  EA           start: nop");
    assert_eq!(lines[2], "      6                            two 1, 2");
    assert_eq!(lines[3], "      6+ 8001  A9 01        lda #1");
    assert_eq!(lines[4], "      6+ 8003  A2 02        ldx #2");
    assert_eq!(lines[5], "      7  8005  01 02 03 04  .byte 1,2,3,4,5,6");
    assert_eq!(lines[6], "         8009  05 06");
    assert_eq!(lines.len(), 7);
    assert!(out.lines.iter().filter(|l| l.expanded).count() == 2);
}

#[test]
fn listing_names_included_files() {
    let mut files = std::collections::HashMap::new();
    files.insert("inc.s".to_string(), "nop\nrts\n".to_string());
    let src = ".include \"inc.s\"\n";
    let out = mos6502::asm::assemble_with_files(src, 0x8000, &files).unwrap();
    let listing = out.listing_file(src);
    assert!(listing.contains("inc.s:1  8000  EA"), "{listing}");
    assert!(listing.contains("inc.s:2  8001  60"), "{listing}");
}
