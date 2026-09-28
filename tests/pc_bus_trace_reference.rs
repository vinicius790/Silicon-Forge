//! Exhaustive per-cycle bus trace check against an independent reference model.
//!
//! The reference below is written from the documented cycle tables (MOS 64doc
//! "6510 Instruction Timing" for NMOS; for the 65C02, the WDC W65C02S data sheet
//! cycle counts plus the 6502.org "6502 Dead Cycles" table by Drass and Dr Jefyll,
//! measured on a WDC 65C02: every 65C02 dead cycle re-reads the previous bus
//! address, except abs,X/Y stores and INC/DEC abs,X without a page cross, which
//! read the final address), not from `opcodes.rs`. The current WDC data sheet has
//! no per-cycle table, so the BBR/BBS fetch order, the 5C idle-read address and the
//! WAI/STP bus cycles follow this core and are marked as such below. Every opcode byte of both cores is run in three
//! scenarios (no page cross, page cross, and for branches not-taken / taken /
//! taken across a page) and each cycle's address and R/W must match.
//! NMOS `JAM` is a one-cycle halt in this core (documented in `lib.rs`) and is
//! checked only for that. For SHA/SHX/SHY/TAS across a page the write address is
//! the unstable `H & value` form, so only R/W is compared on that final cycle.

use mos6502::{bits, Cpu, CpuVariant, FlatMemory};

#[derive(Clone, Copy, PartialEq, Debug)]
enum M {
    Imp,
    Imm,
    Zp,
    Zpx,
    Zpy,
    Abs,
    Abx,
    Aby,
    Izx,
    Izy,
    Izp,
    Rel,
    Brk,
    Jsr,
    Rti,
    Rts,
    Jmp,
    Jmpi,
    Jmpix,
    Push,
    Pull,
    Jam,
    Nop1,
    Nop5c,
    Bbx,
    Wai,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum K {
    R,
    W,
    Rmw,
}

const PC: u16 = 0x0300;
const SP: u8 = 0xFD;

fn nmos_mode(op: u8) -> M {
    use M::*;
    let row = op >> 4;
    let col = op & 15;
    let odd = row & 1 == 1;
    match op {
        0x00 => return Brk,
        0x20 => return Jsr,
        0x40 => return Rti,
        0x60 => return Rts,
        0x4C => return Jmp,
        0x6C => return Jmpi,
        0x08 | 0x48 => return Push,
        0x28 | 0x68 => return Pull,
        0x96 | 0x97 | 0xB6 | 0xB7 => return Zpy,
        0x9E | 0x9F | 0xBE | 0xBF => return Aby,
        _ => {}
    }
    match col {
        0x0 if odd => Rel,
        0x0 => Imm,
        0x1 | 0x3 if odd => Izy,
        0x1 | 0x3 => Izx,
        0x2 if odd || matches!(row, 0x0 | 0x2 | 0x4 | 0x6) => Jam,
        0x2 => Imm,
        0x4..=0x7 if odd => Zpx,
        0x4..=0x7 => Zp,
        0x8 | 0xA => Imp,
        0x9 | 0xB if odd => Aby,
        0x9 | 0xB => Imm,
        _ if odd => Abx,
        _ => Abs,
    }
}

fn nmos_kind(op: u8) -> K {
    let row = op >> 4;
    let col = op & 15;
    if matches!(op, 0x9B | 0x9C | 0x9E | 0x9F | 0x93) {
        return K::W;
    }
    match row {
        0x8 | 0x9 => {
            if col & 1 == 1 || matches!(col, 0x4 | 0x6 | 0xC | 0xE) {
                K::W
            } else {
                K::R
            }
        }
        0xA | 0xB => K::R,
        _ => match col {
            0x3 | 0x6 | 0x7 | 0xE | 0xF => K::Rmw,
            0xB if row & 1 == 1 => K::Rmw,
            _ => K::R,
        },
    }
}

fn cmos_mode(op: u8) -> M {
    use M::*;
    let col = op & 15;
    let row = op >> 4;
    match op {
        0x80 => return Rel,
        0x7C => return Jmpix,
        0x5C => return Nop5c,
        0x5A | 0xDA => return Push,
        0x7A | 0xFA => return Pull,
        0x1A | 0x3A => return Imp,
        0x89 => return Imm,
        0x9E => return Abx,
        0x9C | 0xDC | 0xFC | 0x0C | 0x1C => return Abs,
        0x14 | 0x04 | 0x64 | 0x44 => return Zp,
        0x34 | 0x74 | 0x54 | 0xD4 | 0xF4 => return Zpx,
        0x3C => return Abx,
        _ => {}
    }
    match col {
        0x3 | 0xB if op != 0xCB && op != 0xDB => Nop1,
        0xB => Wai,
        0x7 => Zp,
        0xF => Bbx,
        0x2 if row & 1 == 1 => Izp,
        0x2 => Imm,
        _ => match nmos_mode(op) {
            Jam => Imm,
            m => m,
        },
    }
}

fn cmos_kind(op: u8) -> K {
    match op {
        0x64 | 0x74 | 0x9C | 0x9E | 0x92 => K::W,
        0x04 | 0x0C | 0x14 | 0x1C => K::Rmw,
        0x89 | 0x34 | 0x3C | 0x44 | 0x54 | 0xD4 | 0xF4 | 0xDC | 0xFC => K::R,
        _ if op & 15 == 7 => K::Rmw,
        _ => match nmos_kind(op) {
            K::Rmw if op & 15 == 3 || op & 15 == 0xB || op & 15 == 0xF => K::R,
            k => k,
        },
    }
}

fn branch_taken(op: u8, p: u8) -> bool {
    if op == 0x80 {
        return true;
    }
    let flag = match op >> 6 {
        0 => bits::N,
        1 => bits::V,
        2 => bits::C,
        _ => bits::Z,
    };
    (p & flag != 0) == (op & 0x20 != 0)
}

struct Sc {
    x: u8,
    y: u8,
    b1: u8,
    b2: u8,
    p: u8,
}

fn mem_for(op: u8, sc: &Sc) -> FlatMemory {
    let mut m = FlatMemory::new();
    for i in 0..256usize {
        m.data[i] = if i % 2 == 0 { 0x40 } else { 0x02 };
    }
    m.data[0x10] = 0x20;
    m.data[0x11] = 0x02;
    m.data[0x01FE] = 0x7F;
    m.data[0x01FF] = 0x04;
    m.data[0x0100] = 0x05;
    m.load_at(PC, &[op, sc.b1, sc.b2]);
    m.load_at(0xFFFE, &[0x00, 0x05]);
    m.data[0x0500] = 0xEA;
    m
}

fn is_adc_sbc(op: u8) -> bool {
    let row = op >> 4;
    let col = op & 15;
    matches!(row, 0x6 | 0x7 | 0xE | 0xF)
        && (matches!(col, 0x1 | 0x5 | 0x9 | 0xD) || (col == 0x2 && row & 1 == 1))
}

fn reference(cmos: bool, op: u8, sc: &Sc, mem: &FlatMemory) -> Vec<(u16, bool)> {
    let mut v = reference_core(cmos, op, sc, mem);
    // 65C02 decimal ADC/SBC: one extra dead cycle on the previous bus address.
    if cmos && sc.p & bits::D != 0 && is_adc_sbc(op) {
        let last = *v.last().unwrap();
        v.push(last);
    }
    v
}

fn reference_core(cmos: bool, op: u8, sc: &Sc, mem: &FlatMemory) -> Vec<(u16, bool)> {
    let (mode, kind) = if cmos {
        (cmos_mode(op), cmos_kind(op))
    } else {
        (nmos_mode(op), nmos_kind(op))
    };
    let rd = |a: u16| mem.data[a as usize];
    let st = |n: u8| 0x0100 | SP.wrapping_sub(n) as u16;
    let su = |n: u8| 0x0100 | SP.wrapping_add(n) as u16;
    let p1 = PC + 1;
    let p2 = PC + 2;
    let b1 = sc.b1;
    let base = u16::from_le_bytes([sc.b1, sc.b2]);
    let mut v = vec![(PC, false), (p1, false)];
    // Access at the effective address for a given kind.
    let fin = |v: &mut Vec<(u16, bool)>, ea: u16| match kind {
        K::R => v.push((ea, false)),
        K::W => v.push((ea, true)),
        K::Rmw => {
            v.push((ea, false));
            v.push((ea, !cmos));
            v.push((ea, true));
        }
    };
    let rmw_shift = matches!(op, 0x1E | 0x3E | 0x5E | 0x7E);
    match mode {
        M::Imp | M::Imm => {}
        M::Nop1 => v.truncate(1),
        M::Jam => v.truncate(1),
        M::Zp => {
            if op & 15 == 7 && cmos {
                v.push((b1 as u16, false));
                v.push((b1 as u16, false));
                v.push((b1 as u16, true));
            } else {
                fin(&mut v, b1 as u16)
            }
        }
        M::Zpx | M::Zpy => {
            let i = if mode == M::Zpx { sc.x } else { sc.y };
            // Dead cycle: NMOS reads the unindexed zero-page address, the 65C02
            // re-reads the previous bus address (the operand byte).
            // The undocumented 65C02 zp,X NOPs (54 D4 F4) keep the zero-page read in
            // this core (tests/cmos_nops.rs); no measured trace is published for them.
            let nop = cmos && matches!(op, 0x54 | 0xD4 | 0xF4);
            v.push((if cmos && !nop { p1 } else { b1 as u16 }, false));
            fin(&mut v, b1.wrapping_add(i) as u16);
        }
        M::Abs => {
            v.push((p2, false));
            fin(&mut v, base);
        }
        M::Abx | M::Aby => {
            let i = if mode == M::Abx { sc.x } else { sc.y };
            v.push((p2, false));
            let ea = base.wrapping_add(i as u16);
            let cross = ea & 0xFF00 != base & 0xFF00;
            let unfixed = (base & 0xFF00) | (ea & 0xFF);
            let dummy = if cmos { p2 } else { unfixed };
            let always = kind != K::R && !(cmos && kind == K::Rmw && rmw_shift && !cross);
            if cross || always {
                v.push((if cmos && !cross { unfixed } else { dummy }, false));
            }
            fin(&mut v, ea);
        }
        M::Izx => {
            v.push((if cmos { p1 } else { b1 as u16 }, false));
            let z = b1.wrapping_add(sc.x);
            v.push((z as u16, false));
            v.push((z.wrapping_add(1) as u16, false));
            let ea = u16::from_le_bytes([rd(z as u16), rd(z.wrapping_add(1) as u16)]);
            fin(&mut v, ea);
        }
        M::Izy | M::Izp => {
            v.push((b1 as u16, false));
            v.push((b1.wrapping_add(1) as u16, false));
            let ptr = u16::from_le_bytes([rd(b1 as u16), rd(b1.wrapping_add(1) as u16)]);
            if mode == M::Izp {
                fin(&mut v, ptr);
            } else {
                let ea = ptr.wrapping_add(sc.y as u16);
                let cross = ea & 0xFF00 != ptr & 0xFF00;
                if cross || kind != K::R {
                    // 65C02: the previous bus address, the pointer high byte.
                    let d = if cmos {
                        b1.wrapping_add(1) as u16
                    } else {
                        (ptr & 0xFF00) | (ea & 0xFF)
                    };
                    v.push((d, false));
                }
                fin(&mut v, ea);
            }
        }
        M::Rel => {
            if branch_taken(op, sc.p) {
                v.push((p2, false));
                let t = p2.wrapping_add(b1 as i8 as u16);
                if t & 0xFF00 != p2 & 0xFF00 {
                    let d = (p2 & 0xFF00) | (t & 0xFF);
                    v.push((d, false));
                }
            }
        }
        M::Bbx => {
            // W65C02S data sheet table 5-5, mode 12b: zp read, internal re-read,
            // then the offset at PC+2.
            v.push((b1 as u16, false));
            v.push((b1 as u16, false));
            v.push((p2, false));
            let bit = (op >> 4) & 7;
            let set = rd(b1 as u16) & (1 << bit) != 0;
            if set == (op & 0x80 != 0) {
                let next = PC + 3;
                v.push((next, false));
                let t = next.wrapping_add(sc.b2 as i8 as u16);
                if t & 0xFF00 != next & 0xFF00 {
                    v.push(((next & 0xFF00) | (t & 0xFF), false));
                }
            }
        }
        M::Brk => {
            v.push((st(0), true));
            v.push((st(1), true));
            v.push((st(2), true));
            v.push((0xFFFE, false));
            v.push((0xFFFF, false));
        }
        M::Jsr => {
            v.push((st(0), false));
            v.push((st(0), true));
            v.push((st(1), true));
            v.push((p2, false));
        }
        M::Rts => {
            v.push((st(0), false));
            v.push((su(1), false));
            v.push((su(2), false));
            let ret = u16::from_le_bytes([rd(su(1)), rd(su(2))]);
            v.push((ret, false));
        }
        M::Rti => {
            v.push((st(0), false));
            v.push((su(1), false));
            v.push((su(2), false));
            v.push((su(3), false));
        }
        M::Push => v.push((st(0), true)),
        M::Pull => {
            v.push((st(0), false));
            v.push((su(1), false));
        }
        M::Jmp => v.push((p2, false)),
        M::Wai => v.push((p1, false)),
        M::Jmpi => {
            v.push((p2, false));
            if cmos {
                // Six cycles; cycle 4 is dead and re-reads PC+2 (the previous bus address).
                v.push((p2, false));
                v.push((base, false));
                v.push((base.wrapping_add(1), false));
            } else {
                v.push((base, false));
                v.push(((base & 0xFF00) | (base.wrapping_add(1) & 0xFF), false));
            }
        }
        M::Jmpix => {
            v.push((p2, false));
            v.push((p2, false));
            let ptr = base.wrapping_add(sc.x as u16);
            v.push((ptr, false));
            v.push((ptr.wrapping_add(1), false));
        }
        M::Nop5c => {
            v.push((p2, false));
            // Eight cycles, reads only; the five idle reads hold the operand address.
            for _ in 0..5 {
                v.push((base, false));
            }
        }
    }
    v
}

fn scenarios(op: u8, cmos: bool) -> Vec<Sc> {
    let rel = if cmos {
        cmos_mode(op) == M::Rel
    } else {
        nmos_mode(op) == M::Rel
    };
    let p_off = bits::U;
    let all = bits::U | bits::N | bits::V | bits::C | bits::Z;
    if rel {
        vec![
            Sc {
                x: 0,
                y: 0,
                b1: 0x10,
                b2: 0,
                p: p_off,
            },
            Sc {
                x: 0,
                y: 0,
                b1: 0x10,
                b2: 0,
                p: all,
            },
            Sc {
                x: 0,
                y: 0,
                b1: 0xF0,
                b2: 0,
                p: p_off,
            },
            Sc {
                x: 0,
                y: 0,
                b1: 0xF0,
                b2: 0,
                p: all,
            },
        ]
    } else {
        vec![
            Sc {
                x: 0x01,
                y: 0x01,
                b1: 0x10,
                b2: 0x02,
                p: p_off,
            },
            Sc {
                x: 0xF8,
                y: 0xF8,
                b1: 0x10,
                b2: 0x02,
                p: p_off,
            },
            Sc {
                x: 0x01,
                y: 0x01,
                b1: 0x40,
                b2: 0x10,
                p: p_off,
            },
            Sc {
                x: 0x01,
                y: 0x01,
                b1: 0x40,
                b2: 0xF0,
                p: p_off,
            },
        ]
    }
}

fn run(variant: CpuVariant) -> Vec<String> {
    let cmos = variant == CpuVariant::CMOS;
    let mut bad = Vec::new();
    for op in 0..=255u8 {
        for sc in scenarios(op, cmos) {
            let mem = mem_for(op, &sc);
            let want = reference(cmos, op, &sc, &mem);
            let mut cpu = Cpu::with_variant(mem.clone(), variant);
            cpu.pc = PC;
            cpu.x = sc.x;
            cpu.y = sc.y;
            cpu.sp = SP;
            cpu.p.0 = sc.p;
            let mut got = Vec::new();
            for _ in 0..12 {
                let Ok(c) = cpu.step_cycle() else { break };
                got.push((c.address, c.write));
                if c.complete || cpu.is_halted() {
                    break;
                }
            }
            let unstable = !cmos && matches!(op, 0x93 | 0x9B | 0x9C | 0x9E | 0x9F);
            let ok = got.len() == want.len()
                && got.iter().zip(&want).enumerate().all(|(i, (g, w))| {
                    if unstable && i + 1 == want.len() {
                        g.1 == w.1
                    } else {
                        g == w
                    }
                });
            if !ok {
                bad.push(format!(
                    "{op:02X} x={:02X} b1={:02X} b2={:02X} p={:02X}\n  want {want:04X?}\n  got  {got:04X?}",
                    sc.x, sc.b1, sc.b2, sc.p
                ));
            }
        }
    }
    bad
}

#[test]
fn nmos_every_opcode_matches_the_documented_cycle_table() {
    let bad = run(CpuVariant::NMOS);
    assert!(
        bad.is_empty(),
        "{} mismatches:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn cmos_every_opcode_matches_the_documented_cycle_table() {
    let bad = run(CpuVariant::CMOS);
    assert!(
        bad.is_empty(),
        "{} mismatches:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn reference_cycle_counts_match_the_documented_base_counts() {
    // 64doc base counts for a few anchor opcodes, no cross, branch not taken.
    let sc = Sc {
        x: 1,
        y: 1,
        b1: 0x10,
        b2: 0x02,
        p: bits::U,
    };
    for (op, n) in [
        (0x00u8, 7usize),
        (0x20, 6),
        (0x60, 6),
        (0x40, 6),
        (0x6C, 5),
        (0x1E, 7),
        (0x9D, 5),
        (0x91, 6),
        (0xB1, 5),
        (0x03, 8),
        (0x13, 8),
        (0x1B, 7),
        (0xBB, 4),
    ] {
        assert_eq!(
            reference(false, op, &sc, &mem_for(op, &sc)).len(),
            n,
            "{op:02X}"
        );
    }
    for (op, n) in [
        (0x6Cu8, 6usize),
        (0x7C, 6),
        (0x5C, 8),
        (0x1E, 6),
        (0xFE, 7),
        (0x8F, 5),
        (0x0F, 6),
        (0x07, 5),
        (0x03, 1),
        (0xCB, 3),
    ] {
        assert_eq!(
            reference(true, op, &sc, &mem_for(op, &sc)).len(),
            n,
            "{op:02X}"
        );
    }
}

/// `opcodes.rs` cycle counts and page-cross flags, generated from the reference.
fn table_mismatches(variant: CpuVariant) -> Vec<String> {
    use mos6502::opcodes::{decode_variant, page_cross_extra, Mode};
    let cmos = variant == CpuVariant::CMOS;
    let mut bad = Vec::new();
    for op in 0..=255u8 {
        let mode = if cmos { cmos_mode(op) } else { nmos_mode(op) };
        if mode == M::Jam {
            continue;
        }
        let info = decode_variant(op, variant).unwrap();
        let sc = &scenarios(op, cmos)[0];
        let base = if matches!(mode, M::Bbx) {
            // Not taken: BBS on a clear bit 0 style; compute with a fresh scenario.
            let s = Sc {
                x: 1,
                y: 1,
                b1: 0x10,
                b2: 0x02,
                p: bits::U,
            };
            let mem = mem_for(op, &s);
            let taken = (mem.data[0x10] & (1 << ((op >> 4) & 7)) != 0) == (op & 0x80 != 0);
            reference(cmos, op, &s, &mem).len() - usize::from(taken)
        } else if mode == M::Rel {
            // Table count is the not-taken time (BRA: always taken, its minimum).
            scenarios(op, cmos)
                .iter()
                .map(|s| reference(cmos, op, s, &mem_for(op, s)).len())
                .min()
                .unwrap()
        } else {
            reference(cmos, op, sc, &mem_for(op, sc)).len()
        };
        if usize::from(info.cycles) != base {
            bad.push(format!(
                "{op:02X} {}: table {} reference {base}",
                info.mnemonic, info.cycles
            ));
        }
        if matches!(info.mode, Mode::AbsX | Mode::AbsY | Mode::IndY) {
            let cross = &scenarios(op, cmos)[1];
            let extra = reference(cmos, op, cross, &mem_for(op, cross)).len() - base;
            if (extra == 1) != page_cross_extra(op, variant) {
                bad.push(format!(
                    "{op:02X} {}: page-cross extra {extra}",
                    info.mnemonic
                ));
            }
        }
    }
    bad
}

#[test]
fn opcode_table_cycles_and_page_cross_flags_match_the_reference_nmos() {
    let bad = table_mismatches(CpuVariant::NMOS);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn opcode_table_cycles_and_page_cross_flags_match_the_reference_cmos() {
    let bad = table_mismatches(CpuVariant::CMOS);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn cycles_rs_classifies_every_traced_opcode() {
    use mos6502::{is_bus_cycle_stepped_variant, BusCycle};
    for variant in [CpuVariant::NMOS, CpuVariant::CMOS] {
        for op in 0..=255u8 {
            assert!(
                is_bus_cycle_stepped_variant(op, variant),
                "{variant:?} {op:02X}"
            );
        }
    }
    let _ = std::mem::size_of::<BusCycle>();
}
