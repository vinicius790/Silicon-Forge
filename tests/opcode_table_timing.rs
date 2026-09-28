//! The opcode table's cycle counts (plus `page_cross_extra`) match execution for every
//! opcode of both variants, with and without a page cross.

use mos6502::opcodes::{decode_variant, page_cross_extra, Mode};
use mos6502::{Cpu, CpuVariant, FlatMemory};

#[test]
fn table_cycles_match_step_for_all_opcodes() {
    let skip = [
        "BRK", "JSR", "RTS", "RTI", "JMP", "WAI", "STP", "JAM", "KIL",
    ];
    let mut checked = 0;
    let mut wrong = Vec::new();
    for variant in [CpuVariant::NMOS, CpuVariant::CMOS] {
        for code in 0..=255u8 {
            let op = decode_variant(code, variant).unwrap();
            if skip.contains(&op.mnemonic) || matches!(op.mode, Mode::Rel | Mode::ZpRel) {
                continue;
            }
            for (index, cross) in [(0x10u8, false), (0x90, true)] {
                let mut mem = FlatMemory::new();
                mem.load_at(0x0400, &[code, 0x80, 0x12]);
                mem.load_at(0x0080, &[0x80, 0x12]);
                let mut cpu = Cpu::with_variant(mem, variant);
                cpu.pc = 0x0400;
                cpu.x = index;
                cpu.y = index;
                let Ok(n) = cpu.step() else { continue };
                if cpu.is_halted() {
                    continue;
                }
                let want =
                    u32::from(op.cycles) + u32::from(cross && page_cross_extra(code, variant));
                if n != want {
                    wrong.push(format!(
                        "{variant:?} {code:02X} {} cross={cross}: table {want}, ran {n}",
                        op.mnemonic
                    ));
                }
                checked += 1;
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(checked > 800, "{checked}");
}
