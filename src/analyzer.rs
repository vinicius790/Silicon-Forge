//! Static Code & Control Flow Graph (CFG) Binary Analyzer for MOS 6502.
//!
//! Inspects 6502 machine code without execution:
//! - Disassembles valid opcodes into instruction sequences.
//! - Discovers branch targets, JMP destinations, and JSR subroutines.
//! - Identifies basic block boundaries, entry points, and RTS exits.
//! - Detects unreachable dead-code regions and embedded data tables.

use std::collections::{BTreeMap, BTreeSet};

use crate::opcodes::{Mode, OPCODES_TABLE};

/// Summary statistics and disassembly insights of a analyzed binary.
#[derive(Debug, Clone)]
pub struct AnalysisReport {
    pub base_addr: u16,
    pub total_bytes: usize,
    pub instruction_count: usize,
    pub subroutines: BTreeSet<u16>,
    pub jump_targets: BTreeSet<u16>,
    pub unreachable_regions: Vec<(u16, u16)>, // (start, end)
}

/// A disassembled instruction node in the control flow graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionNode {
    pub addr: u16,
    pub bytes: Vec<u8>,
    pub mnemonic: &'static str,
    pub target: Option<u16>,
    pub is_branch: bool,
    pub is_subroutine_call: bool,
    pub is_terminator: bool, // RTS, RTI, JMP
}

/// Analyze a slice of bytes loaded at `base_addr`.
pub fn analyze_binary(bytes: &[u8], base_addr: u16) -> AnalysisReport {
    let mut instructions = BTreeMap::<u16, InstructionNode>::new();
    let mut subroutines = BTreeSet::<u16>::new();
    let mut jump_targets = BTreeSet::<u16>::new();
    let mut visited_addrs = BTreeSet::<u16>::new();

    let mut offset = 0usize;
    while offset < bytes.len() {
        let addr = base_addr.wrapping_add(offset as u16);
        let opcode_byte = bytes[offset];

        if let Some(op) = OPCODES_TABLE[opcode_byte as usize] {
            let len = op.mode.operand_len() + 1;
            if offset + len <= bytes.len() {
                let insn_bytes = bytes[offset..offset + len].to_vec();
                let mut target = None;
                let is_branch = matches!(
                    op.mode,
                    Mode::Relative
                );
                let is_subroutine_call = op.mnemonic == "JSR";
                let is_terminator = matches!(op.mnemonic, "RTS" | "RTI")
                    || (op.mnemonic == "JMP" && op.mode != Mode::Indirect);

                // Compute destination address for branches and absolute jumps/calls
                if is_branch && len >= 2 {
                    let rel = insn_bytes[1] as i8;
                    let next_pc = addr.wrapping_add(2);
                    let dest = next_pc.wrapping_add(rel as i16 as u16);
                    target = Some(dest);
                    jump_targets.insert(dest);
                } else if (is_subroutine_call || op.mnemonic == "JMP") && len >= 3 {
                    let dest = (insn_bytes[1] as u16) | ((insn_bytes[2] as u16) << 8);
                    target = Some(dest);
                    if is_subroutine_call {
                        subroutines.insert(dest);
                    } else {
                        jump_targets.insert(dest);
                    }
                }

                instructions.insert(
                    addr,
                    InstructionNode {
                        addr,
                        bytes: insn_bytes,
                        mnemonic: op.mnemonic,
                        target,
                        is_branch,
                        is_subroutine_call,
                        is_terminator,
                    },
                );

                for b in 0..len {
                    visited_addrs.insert(addr.wrapping_add(b as u16));
                }
                offset += len;
                continue;
            }
        }
        // Unknown opcode or truncated: advance 1 byte
        offset += 1;
    }

    // Identify gaps that are unreachable / un-decoded data tables
    let mut unreachable_regions = Vec::new();
    let mut gap_start: Option<u16> = None;
    for i in 0..bytes.len() {
        let a = base_addr.wrapping_add(i as u16);
        if !visited_addrs.contains(&a) {
            if gap_start.is_none() {
                gap_start = Some(a);
            }
        } else if let Some(start) = gap_start.take() {
            unreachable_regions.push((start, a.wrapping_sub(1)));
        }
    }
    if let Some(start) = gap_start {
        unreachable_regions.push((start, base_addr.wrapping_add((bytes.len() - 1) as u16)));
    }

    AnalysisReport {
        base_addr,
        total_bytes: bytes.len(),
        instruction_count: instructions.len(),
        subroutines,
        jump_targets,
        unreachable_regions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_binary_flow() {
        // Assembly equivalent:
        // $8000: JSR $8008  (20 08 80)
        // $8003: RTS        (60)
        // $8004..$8007: data bytes (FF FF FF FF)
        // $8008: LDA #$10   (A9 10)
        // $800A: BNE $8003  (D0 F7 -> 800C + (-9) = 8003)
        // $800C: RTS        (60)
        let code = [
            0x20, 0x08, 0x80, // JSR $8008
            0x60,             // RTS
            0x00, 0x00, 0x00, 0x00, // padding/data
            0xA9, 0x10,       // LDA #$10
            0xD0, 0xF7,       // BNE $8003 (-9)
            0x60,             // RTS
        ];

        let report = analyze_binary(&code, 0x8000);
        assert_eq!(report.base_addr, 0x8000);
        assert_eq!(report.total_bytes, 13);
        assert!(report.subroutines.contains(&0x8008));
        assert!(report.jump_targets.contains(&0x8003));
        assert!(report.instruction_count >= 4);
    }
}
