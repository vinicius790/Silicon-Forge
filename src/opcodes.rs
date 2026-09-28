//! Complete metadata and decoding tables for all 256 opcodes of the MOS 6502 architecture.
//!
//! Includes all 151 documented NMOS instructions, 105 undocumented/illegal opcodes
//! (such as LAX, SAX, DCP, ISC, SLO, RLA, SRE, RRA, ANC, ALR, ARR, AXS), and
//! CMOS 65C02 extensions (BRA, PHX, PHY, PLX, PLY, STZ, TRB, TSB, etc.).

/// CPU architecture variant for opcode decoding and execution behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CpuVariant {
    /// Classic NMOS MOS 6502 (NES, Apple II, Commodore VIC-20, Commodore 64).
    /// Includes classic JMP indirect bug and all 105 unofficial/illegal opcodes.
    #[default]
    NMOS,
    /// Enhanced CMOS 65C02 (Apple IIc/IIe enhanced, BBC Micro, Atari Lynx).
    /// Adds BRA, PHX/PHY/PLX/PLY, STZ, TRB/TSB; fixes JMP ($xxFF) bug.
    CMOS,
    /// Ricoh 2A03 (NES / Famicom custom core).
    /// Same as NMOS 6502, but Decimal Mode (BCD) is physically disabled in hardware.
    Ricoh2A03,
}

/// Addressing modes supported by the MOS 6502 and 65C02.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Implied / Implicit (1 byte, operand is implicit in instruction, e.g., CLC, RTS)
    Imp,
    /// Accumulator (1 byte, operates directly on register A, e.g., ASL A)
    Acc,
    /// Immediate `#$nn` (2 bytes, 8-bit constant)
    Imm,
    /// Zero Page `$nn` (2 bytes, address in 0x0000..=0x00FF)
    Zp,
    /// Zero Page,X `$nn,X` (2 bytes, zero-page address indexed by X with wrap)
    ZpX,
    /// Zero Page,Y `$nn,Y` (2 bytes, zero-page address indexed by Y with wrap)
    ZpY,
    /// Absolute `$nnnn` (3 bytes, full 16-bit address)
    Abs,
    /// Absolute,X `$nnnn,X` (3 bytes, 16-bit address indexed by X)
    AbsX,
    /// Absolute,Y `$nnnn,Y` (3 bytes, 16-bit address indexed by Y)
    AbsY,
    /// Indirect `($nnnn)` (3 bytes, 16-bit pointer lookup - JMP only)
    Ind,
    /// Indexed Indirect `($nn,X)` (2 bytes, zero page table indexed by X)
    IndX,
    /// Indirect Indexed `($nn),Y` (2 bytes, zero page pointer indexed by Y)
    IndY,
    /// Relative `$nnnn` (2 bytes, signed 8-bit offset relative to PC+2)
    Rel,
    /// Zero Page Indirect `($nn)` (2 bytes, CMOS 65C02 only, e.g., JMP ($nn), LDA ($nn))
    ZpInd,
    /// Absolute Indexed Indirect `($nnnn,X)` (3 bytes, CMOS 65C02 JMP ($nnnn,X))
    AbsIndX,
    /// Zero page and relative `$nn,label` (3 bytes, 65C02 BBR/BBS)
    ZpRel,
}

impl Mode {
    /// Instruction length in bytes for this addressing mode.
    pub const fn size(self) -> u8 {
        match self {
            Self::Imp | Self::Acc => 1,
            Self::Imm
            | Self::Zp
            | Self::ZpX
            | Self::ZpY
            | Self::IndX
            | Self::IndY
            | Self::Rel
            | Self::ZpInd => 2,
            Self::Abs | Self::AbsX | Self::AbsY | Self::Ind | Self::AbsIndX | Self::ZpRel => 3,
        }
    }
}

/// Classification of opcode legality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpType {
    /// Documented official MOS 6502 instruction.
    Official,
    /// Undocumented / illegal NMOS opcode resulting from silicon transistor matrix.
    Unofficial,
    /// 65C02 CMOS instruction extension.
    CmosExtension,
}

/// Metadata definition of an opcode.
#[derive(Debug, Clone, Copy)]
pub struct Opcode {
    pub code: u8,
    pub mnemonic: &'static str,
    pub mode: Mode,
    pub cycles: u8,
    pub op_type: OpType,
}

impl Opcode {
    pub const fn new(
        code: u8,
        mnemonic: &'static str,
        mode: Mode,
        cycles: u8,
        op_type: OpType,
    ) -> Self {
        Self {
            code,
            mnemonic,
            mode,
            cycles,
            op_type,
        }
    }
}

/// Complete table of all 256 opcodes for the NMOS MOS 6502.
pub static OPCODES_TABLE: [Opcode; 256] = [
    // 0x00
    Opcode::new(0x00, "BRK", Mode::Imp, 7, OpType::Official),
    Opcode::new(0x01, "ORA", Mode::IndX, 6, OpType::Official),
    Opcode::new(0x02, "JAM", Mode::Imp, 0, OpType::Unofficial), // KIL
    Opcode::new(0x03, "SLO", Mode::IndX, 8, OpType::Unofficial), // ASO
    Opcode::new(0x04, "NOP", Mode::Zp, 3, OpType::Unofficial),  // 65C02: TSB
    Opcode::new(0x05, "ORA", Mode::Zp, 3, OpType::Official),
    Opcode::new(0x06, "ASL", Mode::Zp, 5, OpType::Official),
    Opcode::new(0x07, "SLO", Mode::Zp, 5, OpType::Unofficial),
    Opcode::new(0x08, "PHP", Mode::Imp, 3, OpType::Official),
    Opcode::new(0x09, "ORA", Mode::Imm, 2, OpType::Official),
    Opcode::new(0x0A, "ASL", Mode::Acc, 2, OpType::Official),
    Opcode::new(0x0B, "ANC", Mode::Imm, 2, OpType::Unofficial),
    Opcode::new(0x0C, "NOP", Mode::Abs, 4, OpType::Unofficial), // 65C02: TSB
    Opcode::new(0x0D, "ORA", Mode::Abs, 4, OpType::Official),
    Opcode::new(0x0E, "ASL", Mode::Abs, 6, OpType::Official),
    Opcode::new(0x0F, "SLO", Mode::Abs, 6, OpType::Unofficial),
    // 0x10
    Opcode::new(0x10, "BPL", Mode::Rel, 2, OpType::Official),
    Opcode::new(0x11, "ORA", Mode::IndY, 5, OpType::Official),
    Opcode::new(0x12, "JAM", Mode::Imp, 0, OpType::Unofficial), // 65C02: ORA (zp)
    Opcode::new(0x13, "SLO", Mode::IndY, 8, OpType::Unofficial),
    Opcode::new(0x14, "NOP", Mode::ZpX, 4, OpType::Unofficial), // 65C02: TRB
    Opcode::new(0x15, "ORA", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0x16, "ASL", Mode::ZpX, 6, OpType::Official),
    Opcode::new(0x17, "SLO", Mode::ZpX, 6, OpType::Unofficial),
    Opcode::new(0x18, "CLC", Mode::Imp, 2, OpType::Official),
    Opcode::new(0x19, "ORA", Mode::AbsY, 4, OpType::Official),
    Opcode::new(0x1A, "NOP", Mode::Imp, 2, OpType::Unofficial), // 65C02: INC A
    Opcode::new(0x1B, "SLO", Mode::AbsY, 7, OpType::Unofficial),
    Opcode::new(0x1C, "NOP", Mode::AbsX, 4, OpType::Unofficial), // 65C02: TRB
    Opcode::new(0x1D, "ORA", Mode::AbsX, 4, OpType::Official),
    Opcode::new(0x1E, "ASL", Mode::AbsX, 7, OpType::Official),
    Opcode::new(0x1F, "SLO", Mode::AbsX, 7, OpType::Unofficial),
    // 0x20
    Opcode::new(0x20, "JSR", Mode::Abs, 6, OpType::Official),
    Opcode::new(0x21, "AND", Mode::IndX, 6, OpType::Official),
    Opcode::new(0x22, "JAM", Mode::Imp, 0, OpType::Unofficial),
    Opcode::new(0x23, "RLA", Mode::IndX, 8, OpType::Unofficial),
    Opcode::new(0x24, "BIT", Mode::Zp, 3, OpType::Official),
    Opcode::new(0x25, "AND", Mode::Zp, 3, OpType::Official),
    Opcode::new(0x26, "ROL", Mode::Zp, 5, OpType::Official),
    Opcode::new(0x27, "RLA", Mode::Zp, 5, OpType::Unofficial),
    Opcode::new(0x28, "PLP", Mode::Imp, 4, OpType::Official),
    Opcode::new(0x29, "AND", Mode::Imm, 2, OpType::Official),
    Opcode::new(0x2A, "ROL", Mode::Acc, 2, OpType::Official),
    Opcode::new(0x2B, "ANC", Mode::Imm, 2, OpType::Unofficial),
    Opcode::new(0x2C, "BIT", Mode::Abs, 4, OpType::Official),
    Opcode::new(0x2D, "AND", Mode::Abs, 4, OpType::Official),
    Opcode::new(0x2E, "ROL", Mode::Abs, 6, OpType::Official),
    Opcode::new(0x2F, "RLA", Mode::Abs, 6, OpType::Unofficial),
    // 0x30
    Opcode::new(0x30, "BMI", Mode::Rel, 2, OpType::Official),
    Opcode::new(0x31, "AND", Mode::IndY, 5, OpType::Official),
    Opcode::new(0x32, "JAM", Mode::Imp, 0, OpType::Unofficial), // 65C02: AND (zp)
    Opcode::new(0x33, "RLA", Mode::IndY, 8, OpType::Unofficial),
    Opcode::new(0x34, "NOP", Mode::ZpX, 4, OpType::Unofficial), // 65C02: BIT zp,X
    Opcode::new(0x35, "AND", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0x36, "ROL", Mode::ZpX, 6, OpType::Official),
    Opcode::new(0x37, "RLA", Mode::ZpX, 6, OpType::Unofficial),
    Opcode::new(0x38, "SEC", Mode::Imp, 2, OpType::Official),
    Opcode::new(0x39, "AND", Mode::AbsY, 4, OpType::Official),
    Opcode::new(0x3A, "NOP", Mode::Imp, 2, OpType::Unofficial), // 65C02: DEC A
    Opcode::new(0x3B, "RLA", Mode::AbsY, 7, OpType::Unofficial),
    Opcode::new(0x3C, "NOP", Mode::AbsX, 4, OpType::Unofficial), // 65C02: BIT abs,X
    Opcode::new(0x3D, "AND", Mode::AbsX, 4, OpType::Official),
    Opcode::new(0x3E, "ROL", Mode::AbsX, 7, OpType::Official),
    Opcode::new(0x3F, "RLA", Mode::AbsX, 7, OpType::Unofficial),
    // 0x40
    Opcode::new(0x40, "RTI", Mode::Imp, 6, OpType::Official),
    Opcode::new(0x41, "EOR", Mode::IndX, 6, OpType::Official),
    Opcode::new(0x42, "JAM", Mode::Imp, 0, OpType::Unofficial),
    Opcode::new(0x43, "SRE", Mode::IndX, 8, OpType::Unofficial), // LSE
    Opcode::new(0x44, "NOP", Mode::Zp, 3, OpType::Unofficial),
    Opcode::new(0x45, "EOR", Mode::Zp, 3, OpType::Official),
    Opcode::new(0x46, "LSR", Mode::Zp, 5, OpType::Official),
    Opcode::new(0x47, "SRE", Mode::Zp, 5, OpType::Unofficial),
    Opcode::new(0x48, "PHA", Mode::Imp, 3, OpType::Official),
    Opcode::new(0x49, "EOR", Mode::Imm, 2, OpType::Official),
    Opcode::new(0x4A, "LSR", Mode::Acc, 2, OpType::Official),
    Opcode::new(0x4B, "ALR", Mode::Imm, 2, OpType::Unofficial), // ASR
    Opcode::new(0x4C, "JMP", Mode::Abs, 3, OpType::Official),
    Opcode::new(0x4D, "EOR", Mode::Abs, 4, OpType::Official),
    Opcode::new(0x4E, "LSR", Mode::Abs, 6, OpType::Official),
    Opcode::new(0x4F, "SRE", Mode::Abs, 6, OpType::Unofficial),
    // 0x50
    Opcode::new(0x50, "BVC", Mode::Rel, 2, OpType::Official),
    Opcode::new(0x51, "EOR", Mode::IndY, 5, OpType::Official),
    Opcode::new(0x52, "JAM", Mode::Imp, 0, OpType::Unofficial), // 65C02: EOR (zp)
    Opcode::new(0x53, "SRE", Mode::IndY, 8, OpType::Unofficial),
    Opcode::new(0x54, "NOP", Mode::ZpX, 4, OpType::Unofficial),
    Opcode::new(0x55, "EOR", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0x56, "LSR", Mode::ZpX, 6, OpType::Official),
    Opcode::new(0x57, "SRE", Mode::ZpX, 6, OpType::Unofficial),
    Opcode::new(0x58, "CLI", Mode::Imp, 2, OpType::Official),
    Opcode::new(0x59, "EOR", Mode::AbsY, 4, OpType::Official),
    Opcode::new(0x5A, "NOP", Mode::Imp, 2, OpType::Unofficial), // 65C02: PHY
    Opcode::new(0x5B, "SRE", Mode::AbsY, 7, OpType::Unofficial),
    Opcode::new(0x5C, "NOP", Mode::AbsX, 4, OpType::Unofficial),
    Opcode::new(0x5D, "EOR", Mode::AbsX, 4, OpType::Official),
    Opcode::new(0x5E, "LSR", Mode::AbsX, 7, OpType::Official),
    Opcode::new(0x5F, "SRE", Mode::AbsX, 7, OpType::Unofficial),
    // 0x60
    Opcode::new(0x60, "RTS", Mode::Imp, 6, OpType::Official),
    Opcode::new(0x61, "ADC", Mode::IndX, 6, OpType::Official),
    Opcode::new(0x62, "JAM", Mode::Imp, 0, OpType::Unofficial),
    Opcode::new(0x63, "RRA", Mode::IndX, 8, OpType::Unofficial),
    Opcode::new(0x64, "NOP", Mode::Zp, 3, OpType::Unofficial), // 65C02: STZ zp
    Opcode::new(0x65, "ADC", Mode::Zp, 3, OpType::Official),
    Opcode::new(0x66, "ROR", Mode::Zp, 5, OpType::Official),
    Opcode::new(0x67, "RRA", Mode::Zp, 5, OpType::Unofficial),
    Opcode::new(0x68, "PLA", Mode::Imp, 4, OpType::Official),
    Opcode::new(0x69, "ADC", Mode::Imm, 2, OpType::Official),
    Opcode::new(0x6A, "ROR", Mode::Acc, 2, OpType::Official),
    Opcode::new(0x6B, "ARR", Mode::Imm, 2, OpType::Unofficial),
    Opcode::new(0x6C, "JMP", Mode::Ind, 5, OpType::Official),
    Opcode::new(0x6D, "ADC", Mode::Abs, 4, OpType::Official),
    Opcode::new(0x6E, "ROR", Mode::Abs, 6, OpType::Official),
    Opcode::new(0x6F, "RRA", Mode::Abs, 6, OpType::Unofficial),
    // 0x70
    Opcode::new(0x70, "BVS", Mode::Rel, 2, OpType::Official),
    Opcode::new(0x71, "ADC", Mode::IndY, 5, OpType::Official),
    Opcode::new(0x72, "JAM", Mode::Imp, 0, OpType::Unofficial), // 65C02: ADC (zp)
    Opcode::new(0x73, "RRA", Mode::IndY, 8, OpType::Unofficial),
    Opcode::new(0x74, "NOP", Mode::ZpX, 4, OpType::Unofficial), // 65C02: STZ zp,X
    Opcode::new(0x75, "ADC", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0x76, "ROR", Mode::ZpX, 6, OpType::Official),
    Opcode::new(0x77, "RRA", Mode::ZpX, 6, OpType::Unofficial),
    Opcode::new(0x78, "SEI", Mode::Imp, 2, OpType::Official),
    Opcode::new(0x79, "ADC", Mode::AbsY, 4, OpType::Official),
    Opcode::new(0x7A, "NOP", Mode::Imp, 2, OpType::Unofficial), // 65C02: PLY
    Opcode::new(0x7B, "RRA", Mode::AbsY, 7, OpType::Unofficial),
    Opcode::new(0x7C, "NOP", Mode::AbsX, 4, OpType::Unofficial), // 65C02: JMP (abs,X)
    Opcode::new(0x7D, "ADC", Mode::AbsX, 4, OpType::Official),
    Opcode::new(0x7E, "ROR", Mode::AbsX, 7, OpType::Official),
    Opcode::new(0x7F, "RRA", Mode::AbsX, 7, OpType::Unofficial),
    // 0x80
    Opcode::new(0x80, "NOP", Mode::Imm, 2, OpType::Unofficial), // 65C02: BRA rel
    Opcode::new(0x81, "STA", Mode::IndX, 6, OpType::Official),
    Opcode::new(0x82, "NOP", Mode::Imm, 2, OpType::Unofficial),
    Opcode::new(0x83, "SAX", Mode::IndX, 6, OpType::Unofficial), // AXS
    Opcode::new(0x84, "STY", Mode::Zp, 3, OpType::Official),
    Opcode::new(0x85, "STA", Mode::Zp, 3, OpType::Official),
    Opcode::new(0x86, "STX", Mode::Zp, 3, OpType::Official),
    Opcode::new(0x87, "SAX", Mode::Zp, 3, OpType::Unofficial),
    Opcode::new(0x88, "DEY", Mode::Imp, 2, OpType::Official),
    Opcode::new(0x89, "NOP", Mode::Imm, 2, OpType::Unofficial), // 65C02: BIT #imm
    Opcode::new(0x8A, "TXA", Mode::Imp, 2, OpType::Official),
    Opcode::new(0x8B, "ANE", Mode::Imm, 2, OpType::Unofficial), // XAA
    Opcode::new(0x8C, "STY", Mode::Abs, 4, OpType::Official),
    Opcode::new(0x8D, "STA", Mode::Abs, 4, OpType::Official),
    Opcode::new(0x8E, "STX", Mode::Abs, 4, OpType::Official),
    Opcode::new(0x8F, "SAX", Mode::Abs, 4, OpType::Unofficial),
    // 0x90
    Opcode::new(0x90, "BCC", Mode::Rel, 2, OpType::Official),
    Opcode::new(0x91, "STA", Mode::IndY, 6, OpType::Official),
    Opcode::new(0x92, "JAM", Mode::Imp, 0, OpType::Unofficial), // 65C02: STA (zp)
    Opcode::new(0x93, "SHA", Mode::IndY, 6, OpType::Unofficial), // AHX
    Opcode::new(0x94, "STY", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0x95, "STA", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0x96, "STX", Mode::ZpY, 4, OpType::Official),
    Opcode::new(0x97, "SAX", Mode::ZpY, 4, OpType::Unofficial),
    Opcode::new(0x98, "TYA", Mode::Imp, 2, OpType::Official),
    Opcode::new(0x99, "STA", Mode::AbsY, 5, OpType::Official),
    Opcode::new(0x9A, "TXS", Mode::Imp, 2, OpType::Official),
    Opcode::new(0x9B, "SHS", Mode::AbsY, 5, OpType::Unofficial), // TAS
    Opcode::new(0x9C, "SHY", Mode::AbsX, 5, OpType::Unofficial), // 65C02: STZ abs
    Opcode::new(0x9D, "STA", Mode::AbsX, 5, OpType::Official),
    Opcode::new(0x9E, "SHX", Mode::AbsY, 5, OpType::Unofficial), // 65C02: STZ abs,X
    Opcode::new(0x9F, "SHA", Mode::AbsY, 5, OpType::Unofficial),
    // 0xA0
    Opcode::new(0xA0, "LDY", Mode::Imm, 2, OpType::Official),
    Opcode::new(0xA1, "LDA", Mode::IndX, 6, OpType::Official),
    Opcode::new(0xA2, "LDX", Mode::Imm, 2, OpType::Official),
    Opcode::new(0xA3, "LAX", Mode::IndX, 6, OpType::Unofficial),
    Opcode::new(0xA4, "LDY", Mode::Zp, 3, OpType::Official),
    Opcode::new(0xA5, "LDA", Mode::Zp, 3, OpType::Official),
    Opcode::new(0xA6, "LDX", Mode::Zp, 3, OpType::Official),
    Opcode::new(0xA7, "LAX", Mode::Zp, 3, OpType::Unofficial),
    Opcode::new(0xA8, "TAY", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xA9, "LDA", Mode::Imm, 2, OpType::Official),
    Opcode::new(0xAA, "TAX", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xAB, "LAX", Mode::Imm, 2, OpType::Unofficial), // also called LXA
    Opcode::new(0xAC, "LDY", Mode::Abs, 4, OpType::Official),
    Opcode::new(0xAD, "LDA", Mode::Abs, 4, OpType::Official),
    Opcode::new(0xAE, "LDX", Mode::Abs, 4, OpType::Official),
    Opcode::new(0xAF, "LAX", Mode::Abs, 4, OpType::Unofficial),
    // 0xB0
    Opcode::new(0xB0, "BCS", Mode::Rel, 2, OpType::Official),
    Opcode::new(0xB1, "LDA", Mode::IndY, 5, OpType::Official),
    Opcode::new(0xB2, "JAM", Mode::Imp, 0, OpType::Unofficial), // 65C02: LDA (zp)
    Opcode::new(0xB3, "LAX", Mode::IndY, 5, OpType::Unofficial),
    Opcode::new(0xB4, "LDY", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0xB5, "LDA", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0xB6, "LDX", Mode::ZpY, 4, OpType::Official),
    Opcode::new(0xB7, "LAX", Mode::ZpY, 4, OpType::Unofficial),
    Opcode::new(0xB8, "CLV", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xB9, "LDA", Mode::AbsY, 4, OpType::Official),
    Opcode::new(0xBA, "TSX", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xBB, "LAS", Mode::AbsY, 4, OpType::Unofficial),
    Opcode::new(0xBC, "LDY", Mode::AbsX, 4, OpType::Official),
    Opcode::new(0xBD, "LDA", Mode::AbsX, 4, OpType::Official),
    Opcode::new(0xBE, "LDX", Mode::AbsY, 4, OpType::Official),
    Opcode::new(0xBF, "LAX", Mode::AbsY, 4, OpType::Unofficial),
    // 0xC0
    Opcode::new(0xC0, "CPY", Mode::Imm, 2, OpType::Official),
    Opcode::new(0xC1, "CMP", Mode::IndX, 6, OpType::Official),
    Opcode::new(0xC2, "NOP", Mode::Imm, 2, OpType::Unofficial),
    Opcode::new(0xC3, "DCP", Mode::IndX, 8, OpType::Unofficial),
    Opcode::new(0xC4, "CPY", Mode::Zp, 3, OpType::Official),
    Opcode::new(0xC5, "CMP", Mode::Zp, 3, OpType::Official),
    Opcode::new(0xC6, "DEC", Mode::Zp, 5, OpType::Official),
    Opcode::new(0xC7, "DCP", Mode::Zp, 5, OpType::Unofficial),
    Opcode::new(0xC8, "INY", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xC9, "CMP", Mode::Imm, 2, OpType::Official),
    Opcode::new(0xCA, "DEX", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xCB, "SBX", Mode::Imm, 2, OpType::Unofficial), // AXS
    Opcode::new(0xCC, "CPY", Mode::Abs, 4, OpType::Official),
    Opcode::new(0xCD, "CMP", Mode::Abs, 4, OpType::Official),
    Opcode::new(0xCE, "DEC", Mode::Abs, 6, OpType::Official),
    Opcode::new(0xCF, "DCP", Mode::Abs, 6, OpType::Unofficial),
    // 0xD0
    Opcode::new(0xD0, "BNE", Mode::Rel, 2, OpType::Official),
    Opcode::new(0xD1, "CMP", Mode::IndY, 5, OpType::Official),
    Opcode::new(0xD2, "JAM", Mode::Imp, 0, OpType::Unofficial), // 65C02: CMP (zp)
    Opcode::new(0xD3, "DCP", Mode::IndY, 8, OpType::Unofficial),
    Opcode::new(0xD4, "NOP", Mode::ZpX, 4, OpType::Unofficial),
    Opcode::new(0xD5, "CMP", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0xD6, "DEC", Mode::ZpX, 6, OpType::Official),
    Opcode::new(0xD7, "DCP", Mode::ZpX, 6, OpType::Unofficial),
    Opcode::new(0xD8, "CLD", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xD9, "CMP", Mode::AbsY, 4, OpType::Official),
    Opcode::new(0xDA, "NOP", Mode::Imp, 2, OpType::Unofficial), // 65C02: PHX
    Opcode::new(0xDB, "DCP", Mode::AbsY, 7, OpType::Unofficial),
    Opcode::new(0xDC, "NOP", Mode::AbsX, 4, OpType::Unofficial),
    Opcode::new(0xDD, "CMP", Mode::AbsX, 4, OpType::Official),
    Opcode::new(0xDE, "DEC", Mode::AbsX, 7, OpType::Official),
    Opcode::new(0xDF, "DCP", Mode::AbsX, 7, OpType::Unofficial),
    // 0xE0
    Opcode::new(0xE0, "CPX", Mode::Imm, 2, OpType::Official),
    Opcode::new(0xE1, "SBC", Mode::IndX, 6, OpType::Official),
    Opcode::new(0xE2, "NOP", Mode::Imm, 2, OpType::Unofficial),
    Opcode::new(0xE3, "ISC", Mode::IndX, 8, OpType::Unofficial), // ISB
    Opcode::new(0xE4, "CPX", Mode::Zp, 3, OpType::Official),
    Opcode::new(0xE5, "SBC", Mode::Zp, 3, OpType::Official),
    Opcode::new(0xE6, "INC", Mode::Zp, 5, OpType::Official),
    Opcode::new(0xE7, "ISC", Mode::Zp, 5, OpType::Unofficial),
    Opcode::new(0xE8, "INX", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xE9, "SBC", Mode::Imm, 2, OpType::Official),
    Opcode::new(0xEA, "NOP", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xEB, "SBC", Mode::Imm, 2, OpType::Unofficial), // Unofficial USBC
    Opcode::new(0xEC, "CPX", Mode::Abs, 4, OpType::Official),
    Opcode::new(0xED, "SBC", Mode::Abs, 4, OpType::Official),
    Opcode::new(0xEE, "INC", Mode::Abs, 6, OpType::Official),
    Opcode::new(0xEF, "ISC", Mode::Abs, 6, OpType::Unofficial),
    // 0xF0
    Opcode::new(0xF0, "BEQ", Mode::Rel, 2, OpType::Official),
    Opcode::new(0xF1, "SBC", Mode::IndY, 5, OpType::Official),
    Opcode::new(0xF2, "JAM", Mode::Imp, 0, OpType::Unofficial), // 65C02: SBC (zp)
    Opcode::new(0xF3, "ISC", Mode::IndY, 8, OpType::Unofficial),
    Opcode::new(0xF4, "NOP", Mode::ZpX, 4, OpType::Unofficial),
    Opcode::new(0xF5, "SBC", Mode::ZpX, 4, OpType::Official),
    Opcode::new(0xF6, "INC", Mode::ZpX, 6, OpType::Official),
    Opcode::new(0xF7, "ISC", Mode::ZpX, 6, OpType::Unofficial),
    Opcode::new(0xF8, "SED", Mode::Imp, 2, OpType::Official),
    Opcode::new(0xF9, "SBC", Mode::AbsY, 4, OpType::Official),
    Opcode::new(0xFA, "NOP", Mode::Imp, 2, OpType::Unofficial), // 65C02: PLX
    Opcode::new(0xFB, "ISC", Mode::AbsY, 7, OpType::Unofficial),
    Opcode::new(0xFC, "NOP", Mode::AbsX, 4, OpType::Unofficial),
    Opcode::new(0xFD, "SBC", Mode::AbsX, 4, OpType::Official),
    Opcode::new(0xFE, "INC", Mode::AbsX, 7, OpType::Official),
    Opcode::new(0xFF, "ISC", Mode::AbsX, 7, OpType::Unofficial),
];

/// Array of 65C02 CMOS Specific Overrides
pub static CMOS_OVERRIDES: [Opcode; 29] = [
    Opcode::new(0x04, "TSB", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x0C, "TSB", Mode::Abs, 6, OpType::CmosExtension),
    Opcode::new(0x12, "ORA", Mode::ZpInd, 5, OpType::CmosExtension),
    Opcode::new(0x14, "TRB", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x1A, "INC", Mode::Acc, 2, OpType::CmosExtension),
    Opcode::new(0x1C, "TRB", Mode::Abs, 6, OpType::CmosExtension),
    Opcode::new(0x32, "AND", Mode::ZpInd, 5, OpType::CmosExtension),
    Opcode::new(0x34, "BIT", Mode::ZpX, 4, OpType::CmosExtension),
    Opcode::new(0x3A, "DEC", Mode::Acc, 2, OpType::CmosExtension),
    Opcode::new(0x3C, "BIT", Mode::AbsX, 4, OpType::CmosExtension),
    Opcode::new(0x52, "EOR", Mode::ZpInd, 5, OpType::CmosExtension),
    Opcode::new(0x5A, "PHY", Mode::Imp, 3, OpType::CmosExtension),
    Opcode::new(0x64, "STZ", Mode::Zp, 3, OpType::CmosExtension),
    Opcode::new(0x72, "ADC", Mode::ZpInd, 5, OpType::CmosExtension),
    Opcode::new(0x74, "STZ", Mode::ZpX, 4, OpType::CmosExtension),
    Opcode::new(0x7A, "PLY", Mode::Imp, 4, OpType::CmosExtension),
    Opcode::new(0x7C, "JMP", Mode::AbsIndX, 6, OpType::CmosExtension),
    Opcode::new(0x80, "BRA", Mode::Rel, 3, OpType::CmosExtension),
    Opcode::new(0x89, "BIT", Mode::Imm, 2, OpType::CmosExtension),
    Opcode::new(0x92, "STA", Mode::ZpInd, 5, OpType::CmosExtension),
    Opcode::new(0x9C, "STZ", Mode::Abs, 4, OpType::CmosExtension),
    Opcode::new(0x9E, "STZ", Mode::AbsX, 5, OpType::CmosExtension),
    Opcode::new(0xB2, "LDA", Mode::ZpInd, 5, OpType::CmosExtension),
    Opcode::new(0xCB, "WAI", Mode::Imp, 3, OpType::CmosExtension),
    Opcode::new(0xD2, "CMP", Mode::ZpInd, 5, OpType::CmosExtension),
    Opcode::new(0xDB, "STP", Mode::Imp, 3, OpType::CmosExtension),
    Opcode::new(0xDA, "PHX", Mode::Imp, 3, OpType::CmosExtension),
    Opcode::new(0xF2, "SBC", Mode::ZpInd, 5, OpType::CmosExtension),
    Opcode::new(0xFA, "PLX", Mode::Imp, 4, OpType::CmosExtension),
];

/// Rockwell/WDC 65C02 bit instructions. `RMBn`/`SMBn zp` (`n7`, `(n+8)7`) clear or set
/// bit `n` of a zero-page byte in 5 cycles. `BBRn`/`BBSn zp,rel` (`nF`, `(n+8)F`) branch
/// when bit `n` is clear or set: 5 cycles, +1 when taken, +1 more when the target is on
/// another page than the next instruction.
pub static CMOS_BIT_OPS: [Opcode; 32] = [
    Opcode::new(0x07, "RMB0", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x17, "RMB1", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x27, "RMB2", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x37, "RMB3", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x47, "RMB4", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x57, "RMB5", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x67, "RMB6", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x77, "RMB7", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x87, "SMB0", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x97, "SMB1", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0xA7, "SMB2", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0xB7, "SMB3", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0xC7, "SMB4", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0xD7, "SMB5", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0xE7, "SMB6", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0xF7, "SMB7", Mode::Zp, 5, OpType::CmosExtension),
    Opcode::new(0x0F, "BBR0", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x1F, "BBR1", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x2F, "BBR2", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x3F, "BBR3", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x4F, "BBR4", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x5F, "BBR5", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x6F, "BBR6", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x7F, "BBR7", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x8F, "BBS0", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0x9F, "BBS1", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0xAF, "BBS2", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0xBF, "BBS3", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0xCF, "BBS4", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0xDF, "BBS5", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0xEF, "BBS6", Mode::ZpRel, 5, OpType::CmosExtension),
    Opcode::new(0xFF, "BBS7", Mode::ZpRel, 5, OpType::CmosExtension),
];

/// Full 256-entry NMOS matrix, including illegal opcodes. Documented instructions are
/// [`documented_opcodes`].
pub const OPCODES: &[Opcode] = &OPCODES_TABLE;

/// Decode an opcode byte using standard NMOS 6502 behavior.
#[inline]
pub fn decode(code: u8) -> Option<&'static Opcode> {
    decode_variant(code, CpuVariant::NMOS)
}

/// NMOS instructions whose 65C02 timing differs: shifts and rotates abs,X take 6
/// cycles, plus one on a page cross ([`page_cross_extra`]).
pub static CMOS_TIMING: [Opcode; 4] = [
    Opcode::new(0x1E, "ASL", Mode::AbsX, 6, OpType::Official),
    Opcode::new(0x3E, "ROL", Mode::AbsX, 6, OpType::Official),
    Opcode::new(0x5E, "LSR", Mode::AbsX, 6, OpType::Official),
    Opcode::new(0x7E, "ROR", Mode::AbsX, 6, OpType::Official),
];

/// 65C02 `JMP ($nnnn)`: the page-wrap bug is fixed with a sixth, dead cycle that
/// re-reads PC+2 (6 cycles against the NMOS 5).
pub static CMOS_JMP_IND: Opcode = Opcode::new(0x6C, "JMP", Mode::Ind, 6, OpType::Official);

/// Whether crossing a page while indexing adds a cycle to the table count: reads
/// through abs,X, abs,Y and (zp),Y (including the NMOS undocumented reads and the
/// CMOS `(zp)`-free forms), and on the 65C02 the abs,X shifts. Stores and other
/// read-modify-writes always pay the fix-up cycle, so they never add one.
pub fn page_cross_extra(code: u8, variant: CpuVariant) -> bool {
    let Some(op) = decode_variant(code, variant) else {
        return false;
    };
    if variant == CpuVariant::CMOS && CMOS_TIMING.iter().any(|t| t.code == code) {
        return true;
    }
    if !matches!(op.mode, Mode::AbsX | Mode::AbsY | Mode::IndY) {
        return false;
    }
    let reads = [
        "LDA", "LDX", "LDY", "EOR", "AND", "ORA", "ADC", "SBC", "CMP", "BIT", "LAX", "LAS", "NOP",
    ];
    reads.contains(&op.mnemonic)
}

/// Decode an opcode byte taking CPU variant into account.
pub fn decode_variant(code: u8, variant: CpuVariant) -> Option<&'static Opcode> {
    if variant == CpuVariant::CMOS {
        for op in &CMOS_OVERRIDES {
            if op.code == code {
                return Some(op);
            }
        }
        if let Some(op) = CMOS_TIMING.iter().find(|op| op.code == code) {
            return Some(op);
        }
        if code == CMOS_JMP_IND.code {
            return Some(&CMOS_JMP_IND);
        }
        if let Some(op) = CMOS_BIT_OPS.iter().find(|op| op.code == code) {
            return Some(op);
        }
        if let Some(op) = cmos_nop(code) {
            return Some(op);
        }
    }
    Some(&OPCODES_TABLE[code as usize])
}

/// Undefined 65C02 opcodes, which are documented NOPs (no register or flag change):
///
/// | opcodes | bytes | cycles | bus reads after the operand |
/// |---|---|---|---|
/// | `x3`, `xB` (except `CB`, `DB`) | 1 | 1 | none |
/// | `02 22 42 62 82 C2 E2` | 2 | 2 | none |
/// | `44` | 2 | 3 | zero page |
/// | `54 D4 F4` | 2 | 4 | dummy zero page, then zero page + X |
/// | `DC FC` | 3 | 4 | absolute |
/// | `5C` | 3 | 8 | absolute, five times |
///
/// `x7`/`xF` are the bit instructions in [`CMOS_BIT_OPS`] (Rockwell/WDC parts).
pub fn cmos_nop(code: u8) -> Option<&'static Opcode> {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<Opcode>> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..=255u8)
            .filter_map(|c| {
                let (mode, cycles) = match c {
                    0xCB | 0xDB => return None,
                    _ if matches!(c & 0x0F, 0x03 | 0x0B) => (Mode::Imp, 1),
                    0x02 | 0x22 | 0x42 | 0x62 | 0x82 | 0xC2 | 0xE2 => (Mode::Imm, 2),
                    0x44 => (Mode::Zp, 3),
                    0x54 | 0xD4 | 0xF4 => (Mode::ZpX, 4),
                    0xDC | 0xFC => (Mode::Abs, 4),
                    0x5C => (Mode::Abs, 8),
                    _ => return None,
                };
                Some(Opcode::new(c, "NOP", mode, cycles, OpType::Unofficial))
            })
            .collect()
    });
    table.iter().find(|op| op.code == code)
}

/// Documented NMOS opcodes (151), in ascending opcode order.
///
/// Illegal opcodes stay in [`OPCODES_TABLE`] and still execute, but they are not part of
/// this slice. The assembler encodes this set plus [`CMOS_OVERRIDES`].
pub fn documented_opcodes() -> &'static [Opcode] {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Vec<Opcode>> = OnceLock::new();
    CACHE.get_or_init(|| {
        OPCODES_TABLE
            .iter()
            .copied()
            .filter(|op| op.op_type == OpType::Official)
            .collect()
    })
}

/// Find an opcode by mnemonic and addressing mode.
///
/// Official NMOS encodings win, then 65C02 extensions. Illegal opcodes are intentionally
/// not assembled: several of them reuse mnemonics such as `NOP`, so a text form would be
/// ambiguous. They remain executable and disassemblable.
pub fn encode(mnemonic: &str, mode: Mode) -> Option<&'static Opcode> {
    for op in OPCODES_TABLE.iter() {
        if op.op_type == OpType::Official
            && op.mnemonic.eq_ignore_ascii_case(mnemonic)
            && op.mode == mode
        {
            return Some(op);
        }
    }
    CMOS_OVERRIDES
        .iter()
        .chain(CMOS_BIT_OPS.iter())
        .find(|op| op.mnemonic.eq_ignore_ascii_case(mnemonic) && op.mode == mode)
}
