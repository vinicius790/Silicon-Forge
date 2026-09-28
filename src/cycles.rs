//! Bus-cycle model for NMOS and the CMOS opcodes this core actually runs.
//!
//! [`crate::cpu::Cpu::step_cycle`] advances one bus cycle. The trace is a textbook
//! sequence: opcode fetch, operand bytes, effective address, the optional indexed
//! page-cross read, stores, and the read-modify-write pair (write the original byte
//! back, then write the result). It is not a visual6502 transistor netlist.
//! [`crate::cpu::Cpu::step_half_cycle`] splits each cycle into a PHI1 half (address
//! and R/W) and a PHI2 half (data); it is a view of the same cycle, not a netlist.
//!
//! Covered on NMOS and Ricoh (same matrix): all 256 bytes. The 151 official opcodes,
//! the stable unofficial groups (SLO, RLA, SRE, RRA, DCP, ISC, LAX, SAX, ANC, ALR,
//! ARR, AXS, the extra immediate `SBC`, and the NOP forms), the unstable group
//! (`ANE` with [`crate::cpu::ANE_MAGIC`], `SHA`, `SHX`, `SHY`, `TAS`/`SHS`, `LAS`),
//! plus IRQ and NMI entry.
//!
//! Covered only when [`crate::opcodes::CpuVariant::CMOS`] is selected: `BRA`, `STZ`,
//! `TRB`/`TSB`, `BIT` immediate / zp,X / abs,X, accumulator `INC`/`DEC`, `PHX`/`PHY`/
//! `PLX`/`PLY`, `(zp)` indirect, `JMP (abs,X)`, and `WAI`/`STP` (3 cycles each).
//! Decimal-mode `ADC`/`SBC` on CMOS adds one read of `PC` after the last cycle.
//!
//! `JAM`/`KIL` is cycle-stepped as a halt: one opcode read, `PC` stays on the
//! opcode, and the CPU is stopped. Further `step_cycle` calls return an error.
//! On NMOS that is `$02 $12 $22 $32 $42 $52 $62 $72 $92 $B2 $D2 $F2`. On CMOS the
//! `(zp)` bytes are real opcodes and are traced; `$02 $22 $42 $62` stay `JAM`.
//! This is not a bus-lock that keeps reading forever, and it is not visual6502.
//!
//! CMOS bytes this core does not decode as CMOS fall through to the NMOS meaning.
//!
//! [`crate::cpu::Cpu::reset`] performs seven reads and no writes. It is not a
//! `step_cycle` sequence: one call consumes the whole trace.
//!
//! `JMP ($nnnn)` is cycle-stepped on both variants. NMOS is 5 cycles and keeps the
//! page-wrap bug. CMOS is 6 cycles, with no page-wrap; the extra cycle re-reads the
//! pointer high byte. That matches `Cpu::step`'s architectural result. It is not a
//! claim about the WDC microcode.
//!
//! IRQ (`$FFFE`) and NMI (`$FFFA`) entry are 7 cycles and start on an instruction
//! boundary. The IRQ level is polled on the cycle before the last (branches: before
//! the operand fetch, and before the fix-up of a taken page-crossing branch). `Cpu::step` and `Cpu::irq` /
//! `Cpu::nmi` do not use this sequencer.

/// True when `base` and `addr` lie in different 256-byte pages.
pub fn page_cross(base: u16, addr: u16) -> bool {
    (base >> 8) != (addr >> 8)
}

/// `base` plus one when a page was crossed. Used by instruction-level cycle totals.
pub fn extra_cross(base: u8, crossed: bool) -> u8 {
    base + u8::from(crossed)
}

/// Store value and address of NMOS `SHA`/`SHX`/`SHY`/`TAS`.
///
/// The value is `src & (base_hi + 1)`. When indexing crossed a page, the chip drives
/// that value onto the high address lines, so the write lands at `value:ea_lo`.
pub fn high_byte_store(src: u8, base_hi: u8, ea: u16) -> (u16, u8) {
    let value = src & base_hi.wrapping_add(1);
    if (ea >> 8) as u8 == base_hi {
        (ea, value)
    } else {
        (u16::from_le_bytes([ea as u8, value]), value)
    }
}

/// Clock phase reported by [`crate::cpu::Cpu::step_half_cycle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusPhase {
    /// First half: address and R/W are valid, data is not.
    Phi1,
    /// Second half: data is valid on the bus.
    Phi2,
}

/// One half of a bus cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HalfCycle {
    pub phase: BusPhase,
    pub address: u16,
    pub write: bool,
    /// `None` in PHI1, the byte read or written in PHI2.
    pub data: Option<u8>,
    /// `true` only on the PHI2 half of the last cycle of a sequence.
    pub complete: bool,
}

/// One observed bus cycle from [`crate::cpu::Cpu::step_cycle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusCycle {
    pub address: u16,
    pub data: u8,
    /// `true` when this cycle writes `data` to `address`.
    pub write: bool,
    /// `true` on the last cycle of an instruction or interrupt sequence.
    pub complete: bool,
    /// `true` only for the extra cycle caused by a 16-bit page crossing
    /// (indexed absolute or indirect-indexed load, a taken branch, or the
    /// fix-up read of a store / RMW when the high byte actually changes).
    pub page_cross_penalty: bool,
}

impl BusCycle {
    /// `ADDR R DATA` or `ADDR W DATA`.
    pub fn trace_line(self) -> String {
        format!(
            "{:04X} {} {:02X}",
            self.address,
            if self.write { "W" } else { "R" },
            self.data
        )
    }
}

/// Human-readable coverage of [`crate::cpu::Cpu::step_cycle`].
///
/// [`is_bus_cycle_stepped`] is the NMOS/Ricoh set (official plus stable unofficial).
/// CMOS-only encodings are reported by [`is_bus_cycle_stepped_variant`]. IRQ and NMI
/// are not opcodes. Reset is seven reads inside `Cpu::reset`, not this set.
/// Not a visual6502 netlist.
pub const CYCLE_STEPPED: &str = "all 256 NMOS bytes: 151 official + stable unofficial (SLO RLA SRE RRA DCP ISC LAX SAX ANC ALR ARR AXS NOP SBC-imm) + unstable (ANE SHA SHX SHY TAS LAS) + JAM/KIL as a one-cycle halt that does not advance PC + CMOS opcodes this core runs (BRA STZ TRB TSB BIT-imm BIT-zp,X BIT-abs,X INC/DEC-A PHX PHY PLX PLY zp-ind JMP-(abs,X) WAI STP) + IRQ NMI; IRQ polled on the cycle before the last; PHI1/PHI2 halves via step_half_cycle; reset is 7 reads in Cpu::reset; not visual6502";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    ImmLoad,
    ZpLoad,
    AbsLoad,
    AbsXLoad,
    ZpStore,
    AbsStore,
    AbsXStore,
    Branch,
    Jsr,
    Rts,
    Interrupt,
    Implied,
    ZpIdxRead,
    ZpIdxStore,
    IndXRead,
    IndYRead,
    IndXStore,
    IndYStore,
    RmwZp,
    RmwZpX,
    RmwAbs,
    RmwAbsX,
    Push,
    Pull,
    JmpAbs,
    JmpInd,
    Rti,
    /// Illegal RMW `(zp,X)`: 8 cycles.
    RmwIndX,
    /// Illegal RMW `(zp),Y`: 8 cycles, fix-up read always taken.
    RmwIndY,
    /// CMOS `(zp)` read: 5 cycles.
    ZpIndRead,
    /// CMOS `(zp)` store: 5 cycles.
    ZpIndStore,
    /// CMOS `JMP (abs,X)`: 6 cycles, no page-wrap bug.
    JmpIndX,
    /// 65C02 `RMBn`/`SMBn zp`: opcode, address, read, dummy read, write (5 cycles).
    BitZp,
    /// 65C02 `BBRn`/`BBSn zp,rel`: opcode, address, offset, read, dummy read, then the
    /// taken-branch and page-cross cycles.
    BitBranch,
    /// Undefined 65C02 opcode: operand fetches, then the table's dummy reads.
    CmosNop,
    /// `JAM`/`KIL`: one read of the opcode, then the CPU halts. PC does not move.
    Jam,
    /// 65C02 `WAI`/`STP`: 3 cycles, then wait for an interrupt or stop until reset.
    Wait,
    /// 65C02 decimal `ADC`/`SBC`: one more read of `PC` after the operation.
    DecimalExtra,
}

pub(crate) fn classify(op: u8, variant: crate::opcodes::CpuVariant) -> Option<Kind> {
    if variant == crate::opcodes::CpuVariant::CMOS {
        if let Some(kind) = classify_cmos(op) {
            return Some(kind);
        }
    }
    Some(classify_nmos(op))
}

fn classify_cmos(op: u8) -> Option<Kind> {
    if crate::opcodes::cmos_nop(op).is_some() {
        return Some(Kind::CmosNop);
    }
    match op & 0x0F {
        0x07 => return Some(Kind::BitZp),
        0x0F => return Some(Kind::BitBranch),
        _ => {}
    }
    Some(match op {
        0x80 => Kind::Branch,
        0x1A | 0x3A => Kind::Implied,
        0x5A | 0xDA => Kind::Push,
        0x7A | 0xFA => Kind::Pull,
        0x64 => Kind::ZpStore,
        0x9C => Kind::AbsStore,
        0x74 => Kind::ZpIdxStore,
        0x9E => Kind::AbsXStore,
        0x12 | 0x32 | 0x52 | 0x72 | 0xB2 | 0xD2 | 0xF2 => Kind::ZpIndRead,
        0x92 => Kind::ZpIndStore,
        0x04 | 0x14 => Kind::RmwZp,
        0x0C | 0x1C => Kind::RmwAbs,
        0x34 => Kind::ZpIdxRead,
        0x3C => Kind::AbsXLoad,
        0x7C => Kind::JmpIndX,
        0x89 => Kind::ImmLoad,
        0xCB | 0xDB => Kind::Wait,
        _ => return None,
    })
}

/// Every byte has an NMOS bus sequence.
fn classify_nmos(op: u8) -> Kind {
    match op {
        0xA9 | 0xA2 | 0xA0 | 0x09 | 0x29 | 0x49 | 0x69 | 0xE9 | 0xEB | 0xC9 | 0xE0 | 0xC0
        | 0xAB | 0x0B | 0x2B | 0x4B | 0x6B | 0xCB | 0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 | 0x8B => {
            Kind::ImmLoad
        }
        0xA5 | 0xA6 | 0xA4 | 0x05 | 0x25 | 0x45 | 0x65 | 0xE5 | 0xC5 | 0xE4 | 0xC4 | 0x24
        | 0xA7 | 0x04 | 0x44 | 0x64 => Kind::ZpLoad,
        0xAD | 0xAE | 0xAC | 0x0D | 0x2D | 0x4D | 0x6D | 0xED | 0xCD | 0xEC | 0xCC | 0x2C
        | 0xAF | 0x0C => Kind::AbsLoad,
        0xBD | 0xBC | 0x1D | 0x3D | 0x5D | 0x7D | 0xFD | 0xDD | 0xB9 | 0xBE | 0x19 | 0x39
        | 0x59 | 0x79 | 0xF9 | 0xD9 | 0xBF | 0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC | 0xBB => {
            Kind::AbsXLoad
        }
        0x84..=0x87 => Kind::ZpStore,
        0x8C..=0x8F => Kind::AbsStore,
        0x9D | 0x99 | 0x9B | 0x9C | 0x9E | 0x9F => Kind::AbsXStore,
        0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0 => Kind::Branch,
        0x20 => Kind::Jsr,
        0x60 => Kind::Rts,
        0x00 => Kind::Interrupt,
        0xB5 | 0xB4 | 0x15 | 0x35 | 0x55 | 0x75 | 0xF5 | 0xD5 | 0xB6 | 0xB7 | 0x14 | 0x34
        | 0x54 | 0x74 | 0xD4 | 0xF4 => Kind::ZpIdxRead,
        0x94..=0x97 => Kind::ZpIdxStore,
        0xA1 | 0x01 | 0x21 | 0x41 | 0x61 | 0xE1 | 0xC1 | 0xA3 => Kind::IndXRead,
        0xB1 | 0x11 | 0x31 | 0x51 | 0x71 | 0xF1 | 0xD1 | 0xB3 => Kind::IndYRead,
        0x81 | 0x83 => Kind::IndXStore,
        0x91 | 0x93 => Kind::IndYStore,
        0x06 | 0x26 | 0x46 | 0x66 | 0xE6 | 0xC6 | 0x07 | 0x27 | 0x47 | 0x67 | 0xC7 | 0xE7 => {
            Kind::RmwZp
        }
        0x16 | 0x36 | 0x56 | 0x76 | 0xF6 | 0xD6 | 0x17 | 0x37 | 0x57 | 0x77 | 0xD7 | 0xF7 => {
            Kind::RmwZpX
        }
        0x0E | 0x2E | 0x4E | 0x6E | 0xEE | 0xCE | 0x0F | 0x2F | 0x4F | 0x6F | 0xCF | 0xEF => {
            Kind::RmwAbs
        }
        0x1E | 0x3E | 0x5E | 0x7E | 0xFE | 0xDE | 0x1F | 0x3F | 0x5F | 0x7F | 0xDF | 0xFF
        | 0x1B | 0x3B | 0x5B | 0x7B | 0xDB | 0xFB => Kind::RmwAbsX,
        0x03 | 0x23 | 0x43 | 0x63 | 0xC3 | 0xE3 => Kind::RmwIndX,
        0x13 | 0x33 | 0x53 | 0x73 | 0xD3 | 0xF3 => Kind::RmwIndY,
        0x18 | 0x38 | 0x58 | 0x78 | 0xB8 | 0xD8 | 0xF8 | 0xAA | 0xA8 | 0x8A | 0x98 | 0xBA
        | 0x9A | 0xE8 | 0xCA | 0xC8 | 0x88 | 0xEA | 0x0A | 0x4A | 0x2A | 0x6A | 0x1A | 0x3A
        | 0x5A | 0x7A | 0xDA | 0xFA => Kind::Implied,
        0x48 | 0x08 => Kind::Push,
        0x68 | 0x28 => Kind::Pull,
        0x4C => Kind::JmpAbs,
        0x6C => Kind::JmpInd,
        0x40 => Kind::Rti,
        0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
            Kind::Jam
        }
    }
}

/// Index register is Y (absolute,Y / zero-page,Y / `(zp),Y`). Otherwise X.
pub(crate) fn indexed_by_y(op: u8) -> bool {
    matches!(
        op,
        0xB9 | 0xBE
            | 0x19
            | 0x39
            | 0x59
            | 0x79
            | 0xF9
            | 0xD9
            | 0x99
            | 0xB6
            | 0x96
            | 0xB1
            | 0x11
            | 0x31
            | 0x51
            | 0x71
            | 0xF1
            | 0xD1
            | 0x91
            | 0xB7
            | 0xBF
            | 0xB3
            | 0x97
            | 0x1B
            | 0x13
            | 0x3B
            | 0x33
            | 0x5B
            | 0x53
            | 0x7B
            | 0x73
            | 0xDB
            | 0xD3
            | 0xFB
            | 0xF3
            | 0x9B
            | 0x9F
            | 0xBB
    )
}

/// `true` when `op` has a bus-cycle trace on NMOS (and Ricoh, same matrix).
///
/// IRQ and NMI are not opcodes. CMOS-only meanings of a byte are not included;
/// use [`is_bus_cycle_stepped_variant`].
pub fn is_bus_cycle_stepped(op: u8) -> bool {
    classify(op, crate::opcodes::CpuVariant::NMOS).is_some()
}

/// `true` when `step_cycle` traces `op` on `variant`.
pub fn is_bus_cycle_stepped_variant(op: u8, variant: crate::opcodes::CpuVariant) -> bool {
    classify(op, variant).is_some()
}

/// One line per cycle: `ADDR R|W DATA`.
pub fn format_bus_trace(cycles: &[BusCycle]) -> String {
    let mut out = String::new();
    for (index, cycle) in cycles.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&cycle.trace_line());
    }
    out
}

#[derive(Debug, Clone)]
pub(crate) struct Active {
    pub kind: Kind,
    pub phase: u8,
    pub op: u8,
    pub lo: u8,
    pub hi: u8,
    pub ea: u16,
    pub crossed: bool,
    pub take: bool,
    /// IRQ/BRK `$FFFE` or NMI `$FFFA`. Also reused as a pointer address by `JMP` indirect.
    pub vector: u16,
    /// Push status with the B flag set (BRK). Hardware IRQ/NMI push B clear.
    pub brk: bool,
    /// BRK fetches a signature byte and pushes PC+2. IRQ/NMI do not advance PC.
    pub signature: bool,
    /// Indexed modes that add Y instead of X.
    pub use_y: bool,
    /// Byte latched from a data or pointer read (RMW original value, pointer low).
    pub held: u8,
}

impl Active {
    pub(crate) fn start(kind: Kind, op: u8) -> Self {
        Self {
            kind,
            phase: 0,
            op,
            lo: 0,
            hi: 0,
            ea: 0,
            crossed: false,
            take: false,
            vector: 0xFFFE,
            brk: op == 0x00,
            signature: op == 0x00,
            use_y: indexed_by_y(op),
            held: 0,
        }
    }
}

/// Bit `i` of the result is `bits[i]`.
pub(crate) fn pack_bits(bits: &[bool]) -> u8 {
    bits.iter()
        .enumerate()
        .fold(0, |acc, (i, &bit)| acc | (u8::from(bit) << i))
}

/// Every [`Kind`], in declaration order: the snapshot code of a kind is its index.
pub(crate) const KINDS: [Kind; 35] = [
    Kind::ImmLoad,
    Kind::ZpLoad,
    Kind::AbsLoad,
    Kind::AbsXLoad,
    Kind::ZpStore,
    Kind::AbsStore,
    Kind::AbsXStore,
    Kind::Branch,
    Kind::Jsr,
    Kind::Rts,
    Kind::Interrupt,
    Kind::Implied,
    Kind::ZpIdxRead,
    Kind::ZpIdxStore,
    Kind::IndXRead,
    Kind::IndYRead,
    Kind::IndXStore,
    Kind::IndYStore,
    Kind::RmwZp,
    Kind::RmwZpX,
    Kind::RmwAbs,
    Kind::RmwAbsX,
    Kind::Push,
    Kind::Pull,
    Kind::JmpAbs,
    Kind::JmpInd,
    Kind::Rti,
    Kind::RmwIndX,
    Kind::RmwIndY,
    Kind::ZpIndRead,
    Kind::ZpIndStore,
    Kind::JmpIndX,
    Kind::Jam,
    Kind::Wait,
    Kind::DecimalExtra,
];

impl Active {
    /// Snapshot bytes: kind, phase, op, lo, hi, ea (LE), flags (crossed, take, brk,
    /// signature, use_y as bits 0–4), vector (LE), held. 12 bytes.
    pub(crate) fn to_bytes(&self) -> [u8; 12] {
        let kind = KINDS.iter().position(|k| *k == self.kind).unwrap_or(0) as u8;
        let flags = pack_bits(&[
            self.crossed,
            self.take,
            self.brk,
            self.signature,
            self.use_y,
        ]);
        let [ea_lo, ea_hi] = self.ea.to_le_bytes();
        let [v_lo, v_hi] = self.vector.to_le_bytes();
        [
            kind, self.phase, self.op, self.lo, self.hi, ea_lo, ea_hi, flags, v_lo, v_hi,
            self.held, 0,
        ]
    }

    pub(crate) fn from_bytes(b: &[u8; 12]) -> Result<Self, String> {
        let kind = *KINDS
            .get(usize::from(b[0]))
            .ok_or_else(|| format!("unknown cycle kind {}", b[0]))?;
        if b[7] & !0x1F != 0 || b[11] != 0 {
            return Err("bad cycle flags".into());
        }
        Ok(Self {
            kind,
            phase: b[1],
            op: b[2],
            lo: b[3],
            hi: b[4],
            ea: u16::from_le_bytes([b[5], b[6]]),
            crossed: b[7] & 1 != 0,
            take: b[7] & 2 != 0,
            brk: b[7] & 4 != 0,
            signature: b[7] & 8 != 0,
            use_y: b[7] & 16 != 0,
            vector: u16::from_le_bytes([b[8], b[9]]),
            held: b[10],
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CycleSeq {
    pub active: Option<Active>,
}

impl CycleSeq {
    pub(crate) fn busy(&self) -> bool {
        self.active.is_some()
    }
}
