//! Undocumented NMOS opcodes under `.setcpu "6502X"`.

use mos6502::asm::{assemble, encode_for, unofficial_alias, AsmCpu};
use mos6502::opcodes::{Mode, OpType, OPCODES_TABLE};

#[test]
fn every_unofficial_mnemonic_encodes_under_6502x() {
    for op in OPCODES_TABLE
        .iter()
        .filter(|o| o.op_type == OpType::Unofficial)
    {
        let got = encode_for(op.mnemonic, op.mode, AsmCpu::Nmos6502X)
            .unwrap_or_else(|| panic!("{} {:?}", op.mnemonic, op.mode));
        assert_eq!(got.mode, op.mode);
        assert!(
            got.op_type == OpType::Unofficial
                || (got.op_type == OpType::Official && matches!(op.mnemonic, "SBC" | "NOP")),
            "{} {:?} -> {:?}",
            op.mnemonic,
            op.mode,
            got.op_type
        );
        if !OPCODES_TABLE.iter().any(|o| {
            o.op_type == OpType::Official && o.mnemonic == op.mnemonic && o.mode == op.mode
        }) {
            assert!(
                encode_for(op.mnemonic, op.mode, AsmCpu::Nmos6502).is_none(),
                "{} {:?}",
                op.mnemonic,
                op.mode
            );
        }
    }
}

#[test]
fn aliases_encode_to_the_same_opcodes() {
    let cases = [
        ("ASO", "SLO", Mode::Zp),
        ("LSE", "SRE", Mode::Abs),
        ("AXS", "SBX", Mode::Imm),
        ("XAA", "ANE", Mode::Imm),
        ("AHX", "SHA", Mode::AbsY),
        ("TAS", "SHS", Mode::AbsY),
        ("LXA", "LAX", Mode::Imm),
        ("DCM", "DCP", Mode::ZpX),
        ("ISB", "ISC", Mode::IndY),
        ("INS", "ISC", Mode::Abs),
        ("ASR", "ALR", Mode::Imm),
        ("KIL", "JAM", Mode::Imp),
        ("HLT", "JAM", Mode::Imp),
    ];
    for (alias, canon, mode) in cases {
        assert_eq!(unofficial_alias(alias), canon);
        assert_eq!(
            encode_for(alias, mode, AsmCpu::Nmos6502X).map(|o| o.code),
            encode_for(canon, mode, AsmCpu::Nmos6502X).map(|o| o.code),
            "{alias}/{canon} {mode:?}"
        );
    }
}

#[test]
fn assembled_under_6502x_and_rejected_under_6502() {
    let src = ".setcpu \"6502X\"\n lax $10\n sax $20\n slo ($30,x)\n rla $40,x\n sre $1234\n rra $1234,y\n dcp ($50),y\n isc $60\n anc #$80\n alr #$01\n arr #$02\n xaa #$03\n axs #$04\n jam\n";
    assert_eq!(
        assemble(src, 0x8000).unwrap(),
        [
            0xA7, 0x10, 0x87, 0x20, 0x03, 0x30, 0x37, 0x40, 0x4F, 0x34, 0x12, 0x7B, 0x34, 0x12,
            0xD3, 0x50, 0xE7, 0x60, 0x0B, 0x80, 0x4B, 0x01, 0x6B, 0x02, 0x8B, 0x03, 0xCB, 0x04,
            0x02,
        ]
    );
    assert!(assemble(".p02\n lax $10\n", 0x8000).is_err());
    assert!(assemble(".setcpu \"6502\"\n axs #$01\n", 0).is_err());
    assert_eq!(
        assemble(".setcpu \"6502X\"\n lda #1\n", 0).unwrap(),
        [0xA9, 1]
    );
}
