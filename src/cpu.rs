//! Núcleo CPU MOS 6502, 65C02 e Ricoh 2A03 (NES).
//!
//! Implementação completa de todos os 256 opcodes (151 oficiais e 105 não-documentados),
//! com chaveamento de variantes de arquitetura (NMOS 6502, CMOS 65C02 e Ricoh 2A03),
//! fidelidade no manuseio de interrupções, ciclos documentados e trace.

use crate::bus::MemoryBus;
use crate::cycles::{self, Active, BusCycle, BusPhase, CycleSeq, HalfCycle, Kind};

/// `ANE` (`$8B`) computes `A = (A | ANE_MAGIC) & X & imm`. The constant is analog:
/// real NMOS chips have shown `$00`, `$EE`, `$EF`, `$FF` and values that drift with
/// temperature. `$EE` is the commonly documented value and the one this core uses.
pub const ANE_MAGIC: u8 = 0xEE;
use crate::flags::{bits, Status};
use crate::opcodes::CpuVariant;

/// Last completed instruction: fetch address, opcode byte, and cycle count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsnLog {
    pub address: u16,
    pub opcode: u8,
    pub cycles: u32,
}

/// Snapshot de registradores para dump/CLI/golden traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Registers {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: u8,
    pub cycles: u64,
}

impl std::fmt::Display for Registers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = Status(self.p);
        write!(
            f,
            "A={:02X} X={:02X} Y={:02X} SP={:02X} PC={:04X} P={:02X}[{}] CYC={}",
            self.a, self.x, self.y, self.sp, self.pc, self.p, st, self.cycles
        )
    }
}

/// Reads through `peek`, drops writes: the PHI1 probe of [`Cpu::step_phase`].
struct PeekBus<'a, B>(&'a B);

impl<B: MemoryBus> MemoryBus for PeekBus<'_, B> {
    fn read(&mut self, addr: u16) -> u8 {
        self.0.peek(addr)
    }

    fn write(&mut self, _addr: u16, _value: u8) {}

    fn peek(&self, addr: u16) -> u8 {
        self.0.peek(addr)
    }
}

/// CPU MOS 6502 / 65C02 acoplada a um [`MemoryBus`].
#[derive(Clone)]
pub struct Cpu<B: MemoryBus> {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: Status,
    pub cycles: u64,
    pub bus: B,
    /// Variante de arquitetura (NMOS, CMOS ou Ricoh 2A03).
    pub variant: CpuVariant,
    /// Se true, BRK executa seus efeitos e para a CPU.
    pub halt_on_brk: bool,
    halted: bool,
    /// Level-sensitive IRQ pin sampled only by [`Cpu::step_cycle`] on an instruction boundary.
    irq_line: bool,
    /// Edge latched by [`Cpu::pulse_nmi`], consumed by `step_cycle` (not by `step` or `nmi`).
    nmi_pending: bool,
    /// Set when cycle-stepped `CLI` clears `I`. The next instruction boundary
    /// runs one more instruction before a level IRQ is taken. NMI is not delayed.
    irq_defer: bool,
    cycle_seq: CycleSeq,
    last_insn: Option<InsnLog>,
    /// Completed `JSR`s not yet matched by `RTS`. Not part of the snapshot.
    jsr_depth: u32,
    insn_pc: u16,
    insn_op: u8,
    insn_n: u32,
    insn_irq: bool,
    /// 65C02 `WAI` executed; cleared by an IRQ level, an NMI, or reset. Not in the snapshot.
    waiting: bool,
    /// IRQ level the last cycle-stepped sequence polled at its real point (the cycle
    /// before the last; branches: before the operand fetch and before the fix-up).
    /// A line changed between two calls belongs to the next cycle, so a fresh CPU or a
    /// restored image starts false. `step` and `reset` poll the level they ran under.
    irq_poll: bool,
    /// Bit `n` set when the IRQ line was high during phase `n` of the current sequence.
    irq_hist: u16,
    /// Same polling point for the NMI latch: an edge latched only during the last
    /// cycle is recognized after the next instruction.
    nmi_poll: bool,
    /// Bit `n` set when the NMI latch was pending during phase `n`.
    nmi_hist: u16,
    /// Data half of a cycle whose PHI1 half [`Cpu::step_half_cycle`] already reported.
    phi2_pending: Option<BusCycle>,
    /// Address and R/W driven in PHI1 by [`Cpu::step_phase`]; the access waits for PHI2.
    phi1_open: Option<(u16, bool)>,
    /// 65C02 decimal `ADC`/`SBC` owes one extra cycle.
    decimal_extra: bool,
    /// Which 65C02 maker's opcode set [`CpuVariant::CMOS`] decodes. Ignored by the
    /// other variants. Defaults to [`CmosModel::Wdc`].
    pub cmos_model: CmosModel,
    /// Address of the 65C02 read-modify-write dead cycle. Defaults to
    /// [`RmwDeadCycle::Measured`].
    pub cmos_rmw_dead: RmwDeadCycle,
}

/// The two documented accounts of the 65C02 read-modify-write dead cycle disagree.
///
/// - `Measured` (default): the "6502 Dead Cycles" bus captures (6502.org) show the
///   65C02 re-reading the effective address.
/// - `WdcTable`: W65C02S data sheet (March 2000) table 5-5 lists `0,zp+1` for zp
///   (mode 3b), `0,zp+X+1` for zp,X (8b), `AA+X+1` for abs,X (10b), and `AA`
///   (unchanged) for absolute (2b). Zero-page addresses wrap within page 0.
///
/// `RMB`/`SMB` (mode 3c) re-read `0,zp` in both accounts and are not affected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RmwDeadCycle {
    #[default]
    Measured,
    WdcTable,
}

/// 65C02 makers whose opcode sets differ (VICE `techdocs/CPU/65C02.txt`, 6502.org
/// "65C02 Opcodes"). The WDC W65C02S has `BBR`/`BBS`/`RMB`/`SMB` and `WAI`/`STP`.
/// The Rockwell R65C02 has the bit instructions but not `WAI` (`$CB`) and `STP`
/// (`$DB`). The Synertek/GTE 65SC02 has neither group. A missing opcode runs as the
/// 1-byte, 1-cycle NOP that every 65C02 uses for its unused `$x3`/`$xB` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CmosModel {
    #[default]
    Wdc,
    Rockwell,
    Synertek,
}

impl CmosModel {
    /// Whether this maker's chip implements opcode `op` beyond the common 65C02 set.
    pub fn implements(self, op: u8) -> bool {
        let bit_op = matches!(op & 0x0F, 0x07 | 0x0F);
        let wai_stp = matches!(op, 0xCB | 0xDB);
        match self {
            Self::Wdc => true,
            Self::Rockwell => !wai_stp,
            Self::Synertek => !wai_stp && !bit_op,
        }
    }
}

impl<B: MemoryBus> Cpu<B> {
    /// Inicializa a CPU com variante padrão NMOS 6502.
    pub fn new(bus: B) -> Self {
        Self::with_variant(bus, CpuVariant::NMOS)
    }

    /// Inicializa a CPU especificando a variante de hardware.
    pub fn with_variant(bus: B, variant: CpuVariant) -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            p: Status::new(),
            cycles: 0,
            bus,
            variant,
            halt_on_brk: false,
            halted: false,
            irq_line: false,
            nmi_pending: false,
            irq_defer: false,
            cycle_seq: CycleSeq::default(),
            last_insn: None,
            jsr_depth: 0,
            insn_pc: 0,
            insn_op: 0,
            insn_n: 0,
            insn_irq: false,
            waiting: false,
            irq_poll: false,
            irq_hist: 0,
            nmi_poll: false,
            nmi_hist: 0,
            phi2_pending: None,
            phi1_open: None,
            decimal_extra: false,
            cmos_model: CmosModel::Wdc,
            cmos_rmw_dead: RmwDeadCycle::Measured,
        }
    }

    /// Log written by [`Cpu::step`] and by a completed [`Cpu::step_cycle`] instruction.
    /// Interrupt sequences do not replace it.
    pub fn last_insn(&self) -> Option<InsnLog> {
        self.last_insn
    }

    /// Lê o vetor de reset em 7 ciclos, todos de leitura.
    ///
    /// Dois reads no PC atual, três reads na pilha (`$0100+SP` e os dois bytes
    /// abaixo) sem decrementar SP, depois `$FFFC` e `$FFFD`. Não há escrita.
    /// `cycles` fica em 7, não é somado. Não é uma sequência de `step_cycle`.
    pub fn reset(&mut self) {
        self.sp = 0xFD;
        self.p = Status::new();
        self.halted = false;
        self.nmi_pending = false;
        self.irq_defer = false;
        self.jsr_depth = 0;
        self.cycle_seq = CycleSeq::default();
        self.waiting = false;
        // The line was held through the seven reset cycles; reset dropped the NMI latch.
        self.irq_poll = self.irq_line;
        self.nmi_poll = false;
        self.irq_hist = 0;
        self.nmi_hist = 0;
        self.phi2_pending = None;
        self.phi1_open = None;
        let _ = self.bus.read(self.pc);
        let _ = self.bus.read(self.pc.wrapping_add(1));
        let _ = self.bus.read(0x0100 | u16::from(self.sp));
        let _ = self.bus.read(0x0100 | u16::from(self.sp.wrapping_sub(1)));
        let _ = self.bus.read(0x0100 | u16::from(self.sp.wrapping_sub(2)));
        let lo = self.bus.read(0xFFFC);
        let hi = self.bus.read(0xFFFD);
        self.pc = u16::from_le_bytes([lo, hi]);
        self.cycles = 7;
    }

    pub fn registers(&self) -> Registers {
        Registers {
            a: self.a,
            x: self.x,
            y: self.y,
            sp: self.sp,
            pc: self.pc,
            p: self.p.raw(),
            cycles: self.cycles,
        }
    }

    pub fn is_halted(&self) -> bool {
        self.halted
    }

    /// `true` after 65C02 `WAI` until an IRQ level, an NMI, or reset wakes the CPU.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// Completed `JSR` instructions that have not yet returned with `RTS`.
    pub fn jsr_depth(&self) -> u32 {
        self.jsr_depth
    }

    /// Restore the 65C02 `WAI` sleep state (snapshot v6).
    /// Address and R/W of a PHI1 that [`Cpu::step_phase`] opened on an instruction
    /// boundary (opcode fetch or `WAI` idle). `None` mid-sequence: snapshots drop the
    /// sequence, so that PHI1 cannot be resumed.
    pub fn open_phi1(&self) -> Option<(u16, bool)> {
        self.phi1_open.filter(|_| !self.cycle_seq.busy())
    }

    /// Reopen a boundary PHI1 from a snapshot; the next `step_phase` runs its PHI2.
    pub(crate) fn reopen_phi1(&mut self, open: Option<(u16, bool)>) {
        self.phi1_open = open;
    }

    pub fn set_waiting(&mut self, waiting: bool) {
        self.waiting = waiting;
    }

    /// Força o estado de parada da CPU.
    pub fn set_halted(&mut self, halted: bool) {
        self.halted = halted;
    }

    /// Executa uma única instrução; retorna ciclos gastos na instrução.
    ///
    /// After `WAI`, each call with no IRQ level and no latched NMI idles one cycle.
    /// A latched NMI is taken (7 cycles). An IRQ level is taken when `I` is clear;
    /// with `I` set the CPU wakes and runs the next instruction without the vector.
    pub fn step(&mut self) -> Result<u32, String> {
        if self.cycle_seq.busy() || self.phi2_pending.is_some() || self.phi1_open.is_some() {
            return Err(
                "bus-cycle sequence in progress; finish it with step_cycle before step".into(),
            );
        }
        if self.halted {
            return Ok(0);
        }
        // An instruction-level step runs under the current levels for all its cycles.
        self.irq_poll = self.irq_line;
        self.nmi_poll = self.nmi_pending;
        if self.waiting {
            if self.nmi_pending {
                self.nmi_pending = false;
                self.nmi();
                return Ok(7);
            }
            if !self.irq_line {
                self.cycles += 1;
                return Ok(1);
            }
            self.waiting = false;
            if !self.p.get(bits::I) {
                self.irq();
                return Ok(7);
            }
        }
        let address = self.pc;
        let raw = self.fetch();
        let op = self.model_op(raw);
        let fast = self.cmos_fast_shift(op, self.bus.peek(self.pc));
        let cyc = self.execute(op)? + u32::from(std::mem::take(&mut self.decimal_extra))
            - u32::from(fast);
        if op == 0x20 {
            self.jsr_depth = self.jsr_depth.saturating_add(1);
        } else if op == 0x60 {
            self.jsr_depth = self.jsr_depth.saturating_sub(1);
        }
        self.cycles += cyc as u64;
        self.last_insn = Some(InsnLog {
            address,
            opcode: raw,
            cycles: cyc,
        });
        Ok(cyc)
    }

    /// Roda até `max_cycles` acumulados nesta chamada.
    pub fn run_cycles(&mut self, max_cycles: u64) -> Result<u64, String> {
        let start = self.cycles;
        while self.cycles.saturating_sub(start) < max_cycles && !self.halted {
            let before = self.cycles;
            self.step()?;
            if self.cycles == before {
                break;
            }
        }
        Ok(self.cycles - start)
    }

    /// Roda até `n` instruções ou parada (halt).
    pub fn run_instructions(&mut self, n: u64) -> Result<u64, String> {
        let mut done = 0u64;
        while done < n && !self.halted {
            self.step()?;
            done += 1;
        }
        Ok(done)
    }

    #[inline]
    fn fetch(&mut self) -> u8 {
        let v = self.bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }

    #[inline]
    fn fetch_u16(&mut self) -> u16 {
        let lo = self.fetch() as u16;
        let hi = self.fetch() as u16;
        (hi << 8) | lo
    }

    #[inline]
    fn push(&mut self, v: u8) {
        self.bus.write(0x0100 | self.sp as u16, v);
        self.sp = self.sp.wrapping_sub(1);
    }

    #[inline]
    fn pull(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.bus.read(0x0100 | self.sp as u16)
    }

    fn push_u16(&mut self, v: u16) {
        self.push((v >> 8) as u8);
        self.push((v & 0xFF) as u8);
    }

    fn pull_u16(&mut self) -> u16 {
        let lo = self.pull() as u16;
        let hi = self.pull() as u16;
        (hi << 8) | lo
    }

    // ----- Endereçamento -----

    #[inline]
    fn addr_zp(&mut self) -> u16 {
        self.fetch() as u16
    }

    #[inline]
    fn addr_zpx(&mut self) -> u16 {
        self.fetch().wrapping_add(self.x) as u16
    }

    #[inline]
    fn addr_zpy(&mut self) -> u16 {
        self.fetch().wrapping_add(self.y) as u16
    }

    #[inline]
    fn addr_abs(&mut self) -> u16 {
        self.fetch_u16()
    }

    #[inline]
    fn addr_absx(&mut self) -> (u16, bool) {
        let base = self.fetch_u16();
        let addr = base.wrapping_add(self.x as u16);
        let cross = (base & 0xFF00) != (addr & 0xFF00);
        (addr, cross)
    }

    #[inline]
    fn addr_absy(&mut self) -> (u16, bool) {
        let base = self.fetch_u16();
        let addr = base.wrapping_add(self.y as u16);
        let cross = (base & 0xFF00) != (addr & 0xFF00);
        (addr, cross)
    }

    #[inline]
    fn addr_indx(&mut self) -> u16 {
        let ptr = self.fetch().wrapping_add(self.x);
        let lo = self.bus.read(ptr as u16) as u16;
        let hi = self.bus.read(ptr.wrapping_add(1) as u16) as u16;
        (hi << 8) | lo
    }

    #[inline]
    fn addr_indy(&mut self) -> (u16, bool) {
        let ptr = self.fetch();
        let lo = self.bus.read(ptr as u16) as u16;
        let hi = self.bus.read(ptr.wrapping_add(1) as u16) as u16;
        let base = (hi << 8) | lo;
        let addr = base.wrapping_add(self.y as u16);
        let cross = (base & 0xFF00) != (addr & 0xFF00);
        (addr, cross)
    }

    fn tsb_at(&mut self, addr: u16) {
        let memory = self.bus.read(addr);
        self.p.set(bits::Z, self.a & memory == 0);
        self.bus.write(addr, memory | self.a);
    }

    fn trb_at(&mut self, addr: u16) {
        let memory = self.bus.read(addr);
        self.p.set(bits::Z, self.a & memory == 0);
        self.bus.write(addr, memory & !self.a);
    }

    /// Endereçamento indireto da página zero (65C02: `($zp)`)
    #[inline]
    fn addr_zp_ind(&mut self) -> u16 {
        let ptr = self.fetch() as u16;
        let lo = self.bus.read(ptr) as u16;
        let hi = self.bus.read((ptr + 1) & 0xFF) as u16;
        (hi << 8) | lo
    }

    #[inline]
    fn branch(&mut self, take: bool) -> u32 {
        let off = self.fetch() as i8;
        if !take {
            return 2;
        }
        let old = self.pc;
        self.pc = (self.pc as i16).wrapping_add(off as i16) as u16;
        let cross = (old & 0xFF00) != (self.pc & 0xFF00);
        if cross {
            4
        } else {
            3
        }
    }

    // ----- Aritmética e Lógica -----

    #[inline]
    fn lda(&mut self, v: u8) {
        self.a = v;
        self.p.set_zn(self.a);
    }

    #[inline]
    fn ldx(&mut self, v: u8) {
        self.x = v;
        self.p.set_zn(self.x);
    }

    #[inline]
    fn ldy(&mut self, v: u8) {
        self.y = v;
        self.p.set_zn(self.y);
    }

    fn adc(&mut self, v: u8) {
        let r = match self.decimal_alu() {
            None => crate::alu::adc_binary(self.a, v, self.p.get(bits::C)),
            Some(false) => crate::alu::adc(self.a, v, self.p.get(bits::C), true),
            Some(true) => crate::alu::adc_cmos(self.a, v, self.p.get(bits::C)),
        };
        self.apply_alu(r);
    }

    fn sbc(&mut self, v: u8) {
        let r = match self.decimal_alu() {
            None => crate::alu::sbc(self.a, v, self.p.get(bits::C), false),
            Some(false) => crate::alu::sbc(self.a, v, self.p.get(bits::C), true),
            Some(true) => crate::alu::sbc_cmos(self.a, v, self.p.get(bits::C)),
        };
        self.apply_alu(r);
    }

    /// `None` in binary mode (and always on the 2A03, which has no decimal unit),
    /// `Some(is_cmos)` in decimal mode. CMOS decimal arms the extra cycle.
    fn decimal_alu(&mut self) -> Option<bool> {
        if !self.p.get(bits::D) || self.variant == CpuVariant::Ricoh2A03 {
            return None;
        }
        let cmos = self.variant == CpuVariant::CMOS;
        self.decimal_extra = cmos;
        Some(cmos)
    }

    fn apply_alu(&mut self, r: crate::alu::AdcResult) {
        self.a = r.value;
        self.p.set(bits::C, r.carry);
        self.p.set(bits::V, r.overflow);
        self.p.set(bits::N, r.negative);
        self.p.set(bits::Z, r.zero);
    }

    fn cmp_reg(&mut self, reg: u8, v: u8) {
        let r = reg.wrapping_sub(v);
        self.p.set(bits::C, reg >= v);
        self.p.set_zn(r);
    }

    #[inline]
    fn and(&mut self, v: u8) {
        self.a &= v;
        self.p.set_zn(self.a);
    }

    #[inline]
    fn ora(&mut self, v: u8) {
        self.a |= v;
        self.p.set_zn(self.a);
    }

    #[inline]
    fn eor(&mut self, v: u8) {
        self.a ^= v;
        self.p.set_zn(self.a);
    }

    #[inline]
    fn bit(&mut self, v: u8) {
        self.p.set(bits::Z, (self.a & v) == 0);
        self.p.set(bits::V, v & 0x40 != 0);
        self.p.set(bits::N, v & 0x80 != 0);
    }

    #[inline]
    fn asl_val(&mut self, v: u8) -> u8 {
        self.p.set(bits::C, v & 0x80 != 0);
        let r = v << 1;
        self.p.set_zn(r);
        r
    }

    #[inline]
    fn lsr_val(&mut self, v: u8) -> u8 {
        self.p.set(bits::C, v & 0x01 != 0);
        let r = v >> 1;
        self.p.set_zn(r);
        r
    }

    #[inline]
    fn rol_val(&mut self, v: u8) -> u8 {
        let c = if self.p.get(bits::C) { 1u8 } else { 0 };
        self.p.set(bits::C, v & 0x80 != 0);
        let r = (v << 1) | c;
        self.p.set_zn(r);
        r
    }

    #[inline]
    fn ror_val(&mut self, v: u8) -> u8 {
        let c = if self.p.get(bits::C) { 0x80u8 } else { 0 };
        self.p.set(bits::C, v & 0x01 != 0);
        let r = (v >> 1) | c;
        self.p.set_zn(r);
        r
    }

    /// Dispara interrupção de hardware IRQ (7 ciclos).
    /// Instruction-level IRQ entry. With `I` set it only wakes a `WAI`.
    pub fn irq(&mut self) {
        self.waiting = false;
        if self.p.get(bits::I) {
            return;
        }
        self.push_u16(self.pc);
        self.push(self.p.to_stack_irq());
        self.p.set(bits::I, true);
        self.pc = self.bus.read_u16(0xFFFE);
        self.cycles += 7;
    }

    /// Dispara interrupção de hardware NMI (7 ciclos, não-mascarável).
    pub fn nmi(&mut self) {
        self.waiting = false;
        self.push_u16(self.pc);
        self.push(self.p.to_stack_irq());
        self.p.set(bits::I, true);
        self.pc = self.bus.read_u16(0xFFFA);
        self.cycles += 7;
    }

    /// Linha de trace compatível com o formato nestest.
    pub fn trace_line(&self) -> String {
        let pc = self.pc;
        let op = self.bus.peek(pc);
        let b1 = self.bus.peek(pc.wrapping_add(1));
        let b2 = self.bus.peek(pc.wrapping_add(2));
        let d = crate::disasm::disasm_variant(op, b1, b2, self.variant);
        format!(
            "{:04X}  {:02X} {:02X} {:02X}  {:<14}  {}",
            pc,
            op,
            b1,
            b2,
            d.text,
            self.registers()
        )
    }

    // ----- Tabela de Despacho de Instruções (256 Opcodes) -----

    fn execute(&mut self, op: u8) -> Result<u32, String> {
        // Intercepta extensões 65C02 se ativadas
        if self.variant == CpuVariant::CMOS {
            if op & 0x0F == 0x07 {
                let zp = u16::from(self.fetch());
                let value = self.bus.read(zp);
                let _ = self.bus.read(zp);
                self.bus.write(zp, bit_op_result(op, value));
                return Ok(5);
            }
            if op & 0x0F == 0x0F {
                let zp = u16::from(self.fetch());
                let offset = self.fetch() as i8;
                let value = self.bus.read(zp);
                let _ = self.bus.read(zp);
                if !bit_branch_taken(op, value) {
                    return Ok(5);
                }
                let next = self.pc;
                self.pc = next.wrapping_add_signed(i16::from(offset));
                return Ok(if next & 0xFF00 == self.pc & 0xFF00 {
                    6
                } else {
                    7
                });
            }
            if let Some(info) = crate::opcodes::cmos_nop(op) {
                let size = info.mode.size();
                let lo = if size > 1 { self.fetch() } else { 0 };
                let hi = if size > 2 { self.fetch() } else { 0 };
                for extra in 0..u16::from(info.cycles).saturating_sub(size as u16) {
                    let address = self.cmos_nop_address(info.mode, lo, hi, extra);
                    let _ = self.bus.read(address);
                }
                return Ok(u32::from(info.cycles));
            }
            match op {
                0x80 => {
                    // BRA rel
                    return Ok(self.branch(true));
                }
                0xCB => {
                    // WAI: sleep until an IRQ level or NMI.
                    self.waiting = true;
                    return Ok(3);
                }
                0xDB => {
                    // STP: stop the clock until reset.
                    self.halted = true;
                    return Ok(3);
                }
                0x1A => {
                    // INC A
                    self.a = self.a.wrapping_add(1);
                    self.p.set_zn(self.a);
                    return Ok(2);
                }
                0x3A => {
                    // DEC A
                    self.a = self.a.wrapping_sub(1);
                    self.p.set_zn(self.a);
                    return Ok(2);
                }
                0x5A => {
                    // PHY
                    self.push(self.y);
                    return Ok(3);
                }
                0x7A => {
                    // PLY
                    self.y = self.pull();
                    self.p.set_zn(self.y);
                    return Ok(4);
                }
                0xDA => {
                    // PHX
                    self.push(self.x);
                    return Ok(3);
                }
                0xFA => {
                    // PLX
                    self.x = self.pull();
                    self.p.set_zn(self.x);
                    return Ok(4);
                }
                0x64 => {
                    // STZ zp
                    let a = self.addr_zp();
                    self.bus.write(a, 0);
                    return Ok(3);
                }
                0x74 => {
                    // STZ zp,X
                    let a = self.addr_zpx();
                    self.bus.write(a, 0);
                    return Ok(4);
                }
                0x9C => {
                    // STZ abs
                    let a = self.addr_abs();
                    self.bus.write(a, 0);
                    return Ok(4);
                }
                0x9E => {
                    // STZ abs,X
                    let (a, _) = self.addr_absx();
                    self.bus.write(a, 0);
                    return Ok(5);
                }
                0x12 => {
                    // ORA (zp)
                    let a = self.addr_zp_ind();
                    let v = self.bus.read(a);
                    self.ora(v);
                    return Ok(5);
                }
                0x32 => {
                    // AND (zp)
                    let a = self.addr_zp_ind();
                    let v = self.bus.read(a);
                    self.and(v);
                    return Ok(5);
                }
                0x52 => {
                    // EOR (zp)
                    let a = self.addr_zp_ind();
                    let v = self.bus.read(a);
                    self.eor(v);
                    return Ok(5);
                }
                0x72 => {
                    // ADC (zp)
                    let a = self.addr_zp_ind();
                    let v = self.bus.read(a);
                    self.adc(v);
                    return Ok(5);
                }
                0x92 => {
                    // STA (zp)
                    let a = self.addr_zp_ind();
                    self.bus.write(a, self.a);
                    return Ok(5);
                }
                0xB2 => {
                    // LDA (zp)
                    let a = self.addr_zp_ind();
                    let v = self.bus.read(a);
                    self.lda(v);
                    return Ok(5);
                }
                0xD2 => {
                    // CMP (zp)
                    let a = self.addr_zp_ind();
                    let v = self.bus.read(a);
                    self.cmp_reg(self.a, v);
                    return Ok(5);
                }
                0xF2 => {
                    // SBC (zp)
                    let a = self.addr_zp_ind();
                    let v = self.bus.read(a);
                    self.sbc(v);
                    return Ok(5);
                }
                0x04 => {
                    // TSB zp
                    let a = self.addr_zp();
                    self.tsb_at(a);
                    return Ok(5);
                }
                0x0C => {
                    // TSB abs
                    let a = self.addr_abs();
                    self.tsb_at(a);
                    return Ok(6);
                }
                0x14 => {
                    // TRB zp
                    let a = self.addr_zp();
                    self.trb_at(a);
                    return Ok(5);
                }
                0x1C => {
                    // TRB abs
                    let a = self.addr_abs();
                    self.trb_at(a);
                    return Ok(6);
                }
                0x34 => {
                    // BIT zp,X
                    let a = self.addr_zpx();
                    let v = self.bus.read(a);
                    self.bit(v);
                    return Ok(4);
                }
                0x3C => {
                    // BIT abs,X
                    let (a, cross) = self.addr_absx();
                    let v = self.bus.read(a);
                    self.bit(v);
                    return Ok(if cross { 5 } else { 4 });
                }
                0x89 => {
                    // BIT #imm — só Z no 65C02
                    let v = self.fetch();
                    self.p.set(bits::Z, self.a & v == 0);
                    return Ok(2);
                }
                0x7C => {
                    // JMP (abs,X) sem o bug de página do NMOS
                    let ptr = self.fetch_u16().wrapping_add(u16::from(self.x));
                    let lo = self.bus.read(ptr) as u16;
                    let hi = self.bus.read(ptr.wrapping_add(1)) as u16;
                    self.pc = (hi << 8) | lo;
                    return Ok(6);
                }
                _ => {}
            }
        }

        match op {
            // --- LDA ---
            0xA9 => {
                let v = self.fetch();
                self.lda(v);
                Ok(2)
            }
            0xA5 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.lda(v);
                Ok(3)
            }
            0xB5 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a);
                self.lda(v);
                Ok(4)
            }
            0xAD => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.lda(v);
                Ok(4)
            }
            0xBD => {
                let (a, cross) = self.addr_absx();
                let v = self.bus.read(a);
                self.lda(v);
                Ok(if cross { 5 } else { 4 })
            }
            0xB9 => {
                let (a, cross) = self.addr_absy();
                let v = self.bus.read(a);
                self.lda(v);
                Ok(if cross { 5 } else { 4 })
            }
            0xA1 => {
                let a = self.addr_indx();
                let v = self.bus.read(a);
                self.lda(v);
                Ok(6)
            }
            0xB1 => {
                let (a, cross) = self.addr_indy();
                let v = self.bus.read(a);
                self.lda(v);
                Ok(if cross { 6 } else { 5 })
            }

            // --- LDX ---
            0xA2 => {
                let v = self.fetch();
                self.ldx(v);
                Ok(2)
            }
            0xA6 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.ldx(v);
                Ok(3)
            }
            0xB6 => {
                let a = self.addr_zpy();
                let v = self.bus.read(a);
                self.ldx(v);
                Ok(4)
            }
            0xAE => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.ldx(v);
                Ok(4)
            }
            0xBE => {
                let (a, cross) = self.addr_absy();
                let v = self.bus.read(a);
                self.ldx(v);
                Ok(if cross { 5 } else { 4 })
            }

            // --- LDY ---
            0xA0 => {
                let v = self.fetch();
                self.ldy(v);
                Ok(2)
            }
            0xA4 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.ldy(v);
                Ok(3)
            }
            0xB4 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a);
                self.ldy(v);
                Ok(4)
            }
            0xAC => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.ldy(v);
                Ok(4)
            }
            0xBC => {
                let (a, cross) = self.addr_absx();
                let v = self.bus.read(a);
                self.ldy(v);
                Ok(if cross { 5 } else { 4 })
            }

            // --- STA ---
            0x85 => {
                let a = self.addr_zp();
                self.bus.write(a, self.a);
                Ok(3)
            }
            0x95 => {
                let a = self.addr_zpx();
                self.bus.write(a, self.a);
                Ok(4)
            }
            0x8D => {
                let a = self.addr_abs();
                self.bus.write(a, self.a);
                Ok(4)
            }
            0x9D => {
                let (a, _) = self.addr_absx();
                self.bus.write(a, self.a);
                Ok(5)
            }
            0x99 => {
                let (a, _) = self.addr_absy();
                self.bus.write(a, self.a);
                Ok(5)
            }
            0x81 => {
                let a = self.addr_indx();
                self.bus.write(a, self.a);
                Ok(6)
            }
            0x91 => {
                let (a, _) = self.addr_indy();
                self.bus.write(a, self.a);
                Ok(6)
            }

            // --- STX ---
            0x86 => {
                let a = self.addr_zp();
                self.bus.write(a, self.x);
                Ok(3)
            }
            0x96 => {
                let a = self.addr_zpy();
                self.bus.write(a, self.x);
                Ok(4)
            }
            0x8E => {
                let a = self.addr_abs();
                self.bus.write(a, self.x);
                Ok(4)
            }

            // --- STY ---
            0x84 => {
                let a = self.addr_zp();
                self.bus.write(a, self.y);
                Ok(3)
            }
            0x94 => {
                let a = self.addr_zpx();
                self.bus.write(a, self.y);
                Ok(4)
            }
            0x8C => {
                let a = self.addr_abs();
                self.bus.write(a, self.y);
                Ok(4)
            }

            // --- Transferências ---
            0xAA => {
                self.x = self.a;
                self.p.set_zn(self.x);
                Ok(2)
            } // TAX
            0xA8 => {
                self.y = self.a;
                self.p.set_zn(self.y);
                Ok(2)
            } // TAY
            0x8A => {
                self.a = self.x;
                self.p.set_zn(self.a);
                Ok(2)
            } // TXA
            0x98 => {
                self.a = self.y;
                self.p.set_zn(self.a);
                Ok(2)
            } // TYA
            0xBA => {
                self.x = self.sp;
                self.p.set_zn(self.x);
                Ok(2)
            } // TSX
            0x9A => {
                self.sp = self.x;
                Ok(2)
            } // TXS

            // --- Pilha ---
            0x48 => {
                self.push(self.a);
                Ok(3)
            } // PHA
            0x68 => {
                self.a = self.pull();
                self.p.set_zn(self.a);
                Ok(4)
            } // PLA
            0x08 => {
                self.push(self.p.to_stack_php());
                Ok(3)
            } // PHP
            0x28 => {
                let v = self.pull();
                self.p = Status::from_stack(v);
                Ok(4)
            } // PLP

            // --- Lógica: AND, ORA, EOR, BIT ---
            0x29 => {
                let v = self.fetch();
                self.and(v);
                Ok(2)
            }
            0x25 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.and(v);
                Ok(3)
            }
            0x35 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a);
                self.and(v);
                Ok(4)
            }
            0x2D => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.and(v);
                Ok(4)
            }
            0x3D => {
                let (a, c) = self.addr_absx();
                let v = self.bus.read(a);
                self.and(v);
                Ok(if c { 5 } else { 4 })
            }
            0x39 => {
                let (a, c) = self.addr_absy();
                let v = self.bus.read(a);
                self.and(v);
                Ok(if c { 5 } else { 4 })
            }
            0x21 => {
                let a = self.addr_indx();
                let v = self.bus.read(a);
                self.and(v);
                Ok(6)
            }
            0x31 => {
                let (a, c) = self.addr_indy();
                let v = self.bus.read(a);
                self.and(v);
                Ok(if c { 6 } else { 5 })
            }

            0x09 => {
                let v = self.fetch();
                self.ora(v);
                Ok(2)
            }
            0x05 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.ora(v);
                Ok(3)
            }
            0x15 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a);
                self.ora(v);
                Ok(4)
            }
            0x0D => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.ora(v);
                Ok(4)
            }
            0x1D => {
                let (a, c) = self.addr_absx();
                let v = self.bus.read(a);
                self.ora(v);
                Ok(if c { 5 } else { 4 })
            }
            0x19 => {
                let (a, c) = self.addr_absy();
                let v = self.bus.read(a);
                self.ora(v);
                Ok(if c { 5 } else { 4 })
            }
            0x01 => {
                let a = self.addr_indx();
                let v = self.bus.read(a);
                self.ora(v);
                Ok(6)
            }
            0x11 => {
                let (a, c) = self.addr_indy();
                let v = self.bus.read(a);
                self.ora(v);
                Ok(if c { 6 } else { 5 })
            }

            0x49 => {
                let v = self.fetch();
                self.eor(v);
                Ok(2)
            }
            0x45 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.eor(v);
                Ok(3)
            }
            0x55 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a);
                self.eor(v);
                Ok(4)
            }
            0x4D => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.eor(v);
                Ok(4)
            }
            0x5D => {
                let (a, c) = self.addr_absx();
                let v = self.bus.read(a);
                self.eor(v);
                Ok(if c { 5 } else { 4 })
            }
            0x59 => {
                let (a, c) = self.addr_absy();
                let v = self.bus.read(a);
                self.eor(v);
                Ok(if c { 5 } else { 4 })
            }
            0x41 => {
                let a = self.addr_indx();
                let v = self.bus.read(a);
                self.eor(v);
                Ok(6)
            }
            0x51 => {
                let (a, c) = self.addr_indy();
                let v = self.bus.read(a);
                self.eor(v);
                Ok(if c { 6 } else { 5 })
            }

            0x24 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.bit(v);
                Ok(3)
            }
            0x2C => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.bit(v);
                Ok(4)
            }

            // --- Aritmética: ADC / SBC ---
            0x69 => {
                let v = self.fetch();
                self.adc(v);
                Ok(2)
            }
            0x65 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.adc(v);
                Ok(3)
            }
            0x75 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a);
                self.adc(v);
                Ok(4)
            }
            0x6D => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.adc(v);
                Ok(4)
            }
            0x7D => {
                let (a, c) = self.addr_absx();
                let v = self.bus.read(a);
                self.adc(v);
                Ok(if c { 5 } else { 4 })
            }
            0x79 => {
                let (a, c) = self.addr_absy();
                let v = self.bus.read(a);
                self.adc(v);
                Ok(if c { 5 } else { 4 })
            }
            0x61 => {
                let a = self.addr_indx();
                let v = self.bus.read(a);
                self.adc(v);
                Ok(6)
            }
            0x71 => {
                let (a, c) = self.addr_indy();
                let v = self.bus.read(a);
                self.adc(v);
                Ok(if c { 6 } else { 5 })
            }

            0xE9 | 0xEB => {
                let v = self.fetch();
                self.sbc(v);
                Ok(2)
            } // 0xEB é SBC não-oficial
            0xE5 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.sbc(v);
                Ok(3)
            }
            0xF5 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a);
                self.sbc(v);
                Ok(4)
            }
            0xED => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.sbc(v);
                Ok(4)
            }
            0xFD => {
                let (a, c) = self.addr_absx();
                let v = self.bus.read(a);
                self.sbc(v);
                Ok(if c { 5 } else { 4 })
            }
            0xF9 => {
                let (a, c) = self.addr_absy();
                let v = self.bus.read(a);
                self.sbc(v);
                Ok(if c { 5 } else { 4 })
            }
            0xE1 => {
                let a = self.addr_indx();
                let v = self.bus.read(a);
                self.sbc(v);
                Ok(6)
            }
            0xF1 => {
                let (a, c) = self.addr_indy();
                let v = self.bus.read(a);
                self.sbc(v);
                Ok(if c { 6 } else { 5 })
            }

            // --- Comparações: CMP, CPX, CPY ---
            0xC9 => {
                let v = self.fetch();
                self.cmp_reg(self.a, v);
                Ok(2)
            }
            0xC5 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.cmp_reg(self.a, v);
                Ok(3)
            }
            0xD5 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a);
                self.cmp_reg(self.a, v);
                Ok(4)
            }
            0xCD => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.cmp_reg(self.a, v);
                Ok(4)
            }
            0xDD => {
                let (a, c) = self.addr_absx();
                let v = self.bus.read(a);
                self.cmp_reg(self.a, v);
                Ok(if c { 5 } else { 4 })
            }
            0xD9 => {
                let (a, c) = self.addr_absy();
                let v = self.bus.read(a);
                self.cmp_reg(self.a, v);
                Ok(if c { 5 } else { 4 })
            }
            0xC1 => {
                let a = self.addr_indx();
                let v = self.bus.read(a);
                self.cmp_reg(self.a, v);
                Ok(6)
            }
            0xD1 => {
                let (a, c) = self.addr_indy();
                let v = self.bus.read(a);
                self.cmp_reg(self.a, v);
                Ok(if c { 6 } else { 5 })
            }

            0xE0 => {
                let v = self.fetch();
                self.cmp_reg(self.x, v);
                Ok(2)
            }
            0xE4 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.cmp_reg(self.x, v);
                Ok(3)
            }
            0xEC => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.cmp_reg(self.x, v);
                Ok(4)
            }

            0xC0 => {
                let v = self.fetch();
                self.cmp_reg(self.y, v);
                Ok(2)
            }
            0xC4 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.cmp_reg(self.y, v);
                Ok(3)
            }
            0xCC => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.cmp_reg(self.y, v);
                Ok(4)
            }

            // --- Incrementos e Decrementos ---
            0xE6 => {
                let a = self.addr_zp();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.p.set_zn(v);
                Ok(5)
            }
            0xF6 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.p.set_zn(v);
                Ok(6)
            }
            0xEE => {
                let a = self.addr_abs();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.p.set_zn(v);
                Ok(6)
            }
            0xFE => {
                let (a, _) = self.addr_absx();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.p.set_zn(v);
                Ok(7)
            }

            0xC6 => {
                let a = self.addr_zp();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.p.set_zn(v);
                Ok(5)
            }
            0xD6 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.p.set_zn(v);
                Ok(6)
            }
            0xCE => {
                let a = self.addr_abs();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.p.set_zn(v);
                Ok(6)
            }
            0xDE => {
                let (a, _) = self.addr_absx();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.p.set_zn(v);
                Ok(7)
            }

            0xE8 => {
                self.x = self.x.wrapping_add(1);
                self.p.set_zn(self.x);
                Ok(2)
            } // INX
            0xCA => {
                self.x = self.x.wrapping_sub(1);
                self.p.set_zn(self.x);
                Ok(2)
            } // DEX
            0xC8 => {
                self.y = self.y.wrapping_add(1);
                self.p.set_zn(self.y);
                Ok(2)
            } // INY
            0x88 => {
                self.y = self.y.wrapping_sub(1);
                self.p.set_zn(self.y);
                Ok(2)
            } // DEY

            // --- Deslocamentos e Rotações: ASL, LSR, ROL, ROR ---
            0x0A => {
                self.a = self.asl_val(self.a);
                Ok(2)
            }
            0x06 => {
                let a = self.addr_zp();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                Ok(5)
            }
            0x16 => {
                let a = self.addr_zpx();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                Ok(6)
            }
            0x0E => {
                let a = self.addr_abs();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                Ok(6)
            }
            0x1E => {
                let (a, _) = self.addr_absx();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                Ok(7)
            }

            0x4A => {
                self.a = self.lsr_val(self.a);
                Ok(2)
            }
            0x46 => {
                let a = self.addr_zp();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                Ok(5)
            }
            0x56 => {
                let a = self.addr_zpx();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                Ok(6)
            }
            0x4E => {
                let a = self.addr_abs();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                Ok(6)
            }
            0x5E => {
                let (a, _) = self.addr_absx();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                Ok(7)
            }

            0x2A => {
                self.a = self.rol_val(self.a);
                Ok(2)
            }
            0x26 => {
                let a = self.addr_zp();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                Ok(5)
            }
            0x36 => {
                let a = self.addr_zpx();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                Ok(6)
            }
            0x2E => {
                let a = self.addr_abs();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                Ok(6)
            }
            0x3E => {
                let (a, _) = self.addr_absx();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                Ok(7)
            }

            0x6A => {
                self.a = self.ror_val(self.a);
                Ok(2)
            }
            0x66 => {
                let a = self.addr_zp();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                Ok(5)
            }
            0x76 => {
                let a = self.addr_zpx();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                Ok(6)
            }
            0x6E => {
                let a = self.addr_abs();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                Ok(6)
            }
            0x7E => {
                let (a, _) = self.addr_absx();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                Ok(7)
            }

            // --- Manipulação de Flags ---
            0x18 => {
                self.p.set(bits::C, false);
                Ok(2)
            } // CLC
            0x38 => {
                self.p.set(bits::C, true);
                Ok(2)
            } // SEC
            0x58 => {
                self.p.set(bits::I, false);
                Ok(2)
            } // CLI
            0x78 => {
                self.p.set(bits::I, true);
                Ok(2)
            } // SEI
            0xB8 => {
                self.p.set(bits::V, false);
                Ok(2)
            } // CLV
            0xD8 => {
                self.p.set(bits::D, false);
                Ok(2)
            } // CLD
            0xF8 => {
                self.p.set(bits::D, true);
                Ok(2)
            } // SED

            // --- Saltos Condicionais (Branches) ---
            0x10 => Ok(self.branch(!self.p.get(bits::N))), // BPL
            0x30 => Ok(self.branch(self.p.get(bits::N))),  // BMI
            0x50 => Ok(self.branch(!self.p.get(bits::V))), // BVC
            0x70 => Ok(self.branch(self.p.get(bits::V))),  // BVS
            0x90 => Ok(self.branch(!self.p.get(bits::C))), // BCC
            0xB0 => Ok(self.branch(self.p.get(bits::C))),  // BCS
            0xD0 => Ok(self.branch(!self.p.get(bits::Z))), // BNE
            0xF0 => Ok(self.branch(self.p.get(bits::Z))),  // BEQ

            // --- JMP / JSR / RTS / RTI / BRK ---
            0x4C => {
                self.pc = self.fetch_u16();
                Ok(3)
            } // JMP abs
            0x6C => {
                // JMP (ind)
                let ptr = self.fetch_u16();
                let lo = self.bus.read(ptr) as u16;
                let hi_addr = if self.variant == CpuVariant::CMOS {
                    ptr.wrapping_add(1)
                } else if ptr & 0x00FF == 0x00FF {
                    ptr & 0xFF00 // Bug histórico do NMOS
                } else {
                    ptr.wrapping_add(1)
                };
                let hi = self.bus.read(hi_addr) as u16;
                self.pc = (hi << 8) | lo;
                Ok(if self.variant == CpuVariant::CMOS {
                    6
                } else {
                    5
                })
            }
            0x20 => {
                // JSR
                let target = self.fetch_u16();
                let ret = self.pc.wrapping_sub(1);
                self.push_u16(ret);
                self.pc = target;
                Ok(6)
            }
            0x60 => {
                // RTS
                let addr = self.pull_u16();
                self.pc = addr.wrapping_add(1);
                Ok(6)
            }
            0x40 => {
                // RTI
                let p = self.pull();
                self.p = Status::from_stack(p);
                self.pc = self.pull_u16();
                Ok(6)
            }
            0x00 => {
                // BRK
                if self.halt_on_brk {
                    self.halted = true;
                }
                let ret = self.pc.wrapping_add(1);
                self.push_u16(ret);
                self.push(self.p.to_stack_php());
                self.p.set(bits::I, true);
                self.pc = self.bus.read_u16(0xFFFE);
                Ok(7)
            }

            // --- NOP Documentado ---
            0xEA => Ok(2),

            // ==========================================
            // --- OPCODES NÃO DOCUMENTADOS (ILEGAIS) ---
            // ==========================================

            // LAX: LDA + LDX
            0xA7 => {
                let a = self.addr_zp();
                let v = self.bus.read(a);
                self.lda(v);
                self.ldx(v);
                Ok(3)
            }
            0xB7 => {
                let a = self.addr_zpy();
                let v = self.bus.read(a);
                self.lda(v);
                self.ldx(v);
                Ok(4)
            }
            0xAF => {
                let a = self.addr_abs();
                let v = self.bus.read(a);
                self.lda(v);
                self.ldx(v);
                Ok(4)
            }
            0xBF => {
                let (a, c) = self.addr_absy();
                let v = self.bus.read(a);
                self.lda(v);
                self.ldx(v);
                Ok(if c { 5 } else { 4 })
            }
            0xA3 => {
                let a = self.addr_indx();
                let v = self.bus.read(a);
                self.lda(v);
                self.ldx(v);
                Ok(6)
            }
            0xB3 => {
                let (a, c) = self.addr_indy();
                let v = self.bus.read(a);
                self.lda(v);
                self.ldx(v);
                Ok(if c { 6 } else { 5 })
            }
            0xAB => {
                let v = self.fetch();
                self.lda(v);
                self.ldx(v);
                Ok(2)
            } // LXA / LAX #imm

            // SAX: Store A & X
            0x87 => {
                let a = self.addr_zp();
                self.bus.write(a, self.a & self.x);
                Ok(3)
            }
            0x97 => {
                let a = self.addr_zpy();
                self.bus.write(a, self.a & self.x);
                Ok(4)
            }
            0x8F => {
                let a = self.addr_abs();
                self.bus.write(a, self.a & self.x);
                Ok(4)
            }
            0x83 => {
                let a = self.addr_indx();
                self.bus.write(a, self.a & self.x);
                Ok(6)
            }

            // DCP: DEC mem, then CMP with A
            0xC7 => {
                let a = self.addr_zp();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.cmp_reg(self.a, v);
                Ok(5)
            }
            0xD7 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.cmp_reg(self.a, v);
                Ok(6)
            }
            0xCF => {
                let a = self.addr_abs();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.cmp_reg(self.a, v);
                Ok(6)
            }
            0xDF => {
                let (a, _) = self.addr_absx();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.cmp_reg(self.a, v);
                Ok(7)
            }
            0xDB => {
                let (a, _) = self.addr_absy();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.cmp_reg(self.a, v);
                Ok(7)
            }
            0xC3 => {
                let a = self.addr_indx();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.cmp_reg(self.a, v);
                Ok(8)
            }
            0xD3 => {
                let (a, _) = self.addr_indy();
                let v = self.bus.read(a).wrapping_sub(1);
                self.bus.write(a, v);
                self.cmp_reg(self.a, v);
                Ok(8)
            }

            // ISC / ISB: INC mem, then SBC from A
            0xE7 => {
                let a = self.addr_zp();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.sbc(v);
                Ok(5)
            }
            0xF7 => {
                let a = self.addr_zpx();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.sbc(v);
                Ok(6)
            }
            0xEF => {
                let a = self.addr_abs();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.sbc(v);
                Ok(6)
            }
            0xFF => {
                let (a, _) = self.addr_absx();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.sbc(v);
                Ok(7)
            }
            0xFB => {
                let (a, _) = self.addr_absy();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.sbc(v);
                Ok(7)
            }
            0xE3 => {
                let a = self.addr_indx();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.sbc(v);
                Ok(8)
            }
            0xF3 => {
                let (a, _) = self.addr_indy();
                let v = self.bus.read(a).wrapping_add(1);
                self.bus.write(a, v);
                self.sbc(v);
                Ok(8)
            }

            // SLO / ASO: ASL mem, then ORA into A
            0x07 => {
                let a = self.addr_zp();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                self.ora(v);
                Ok(5)
            }
            0x17 => {
                let a = self.addr_zpx();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                self.ora(v);
                Ok(6)
            }
            0x0F => {
                let a = self.addr_abs();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                self.ora(v);
                Ok(6)
            }
            0x1F => {
                let (a, _) = self.addr_absx();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                self.ora(v);
                Ok(7)
            }
            0x1B => {
                let (a, _) = self.addr_absy();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                self.ora(v);
                Ok(7)
            }
            0x03 => {
                let a = self.addr_indx();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                self.ora(v);
                Ok(8)
            }
            0x13 => {
                let (a, _) = self.addr_indy();
                let v = {
                    let raw = self.bus.read(a);
                    self.asl_val(raw)
                };
                self.bus.write(a, v);
                self.ora(v);
                Ok(8)
            }

            // RLA: ROL mem, then AND into A
            0x27 => {
                let a = self.addr_zp();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                self.and(v);
                Ok(5)
            }
            0x37 => {
                let a = self.addr_zpx();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                self.and(v);
                Ok(6)
            }
            0x2F => {
                let a = self.addr_abs();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                self.and(v);
                Ok(6)
            }
            0x3F => {
                let (a, _) = self.addr_absx();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                self.and(v);
                Ok(7)
            }
            0x3B => {
                let (a, _) = self.addr_absy();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                self.and(v);
                Ok(7)
            }
            0x23 => {
                let a = self.addr_indx();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                self.and(v);
                Ok(8)
            }
            0x33 => {
                let (a, _) = self.addr_indy();
                let v = {
                    let raw = self.bus.read(a);
                    self.rol_val(raw)
                };
                self.bus.write(a, v);
                self.and(v);
                Ok(8)
            }

            // SRE / LSE: LSR mem, then EOR into A
            0x47 => {
                let a = self.addr_zp();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                self.eor(v);
                Ok(5)
            }
            0x57 => {
                let a = self.addr_zpx();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                self.eor(v);
                Ok(6)
            }
            0x4F => {
                let a = self.addr_abs();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                self.eor(v);
                Ok(6)
            }
            0x5F => {
                let (a, _) = self.addr_absx();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                self.eor(v);
                Ok(7)
            }
            0x5B => {
                let (a, _) = self.addr_absy();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                self.eor(v);
                Ok(7)
            }
            0x43 => {
                let a = self.addr_indx();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                self.eor(v);
                Ok(8)
            }
            0x53 => {
                let (a, _) = self.addr_indy();
                let v = {
                    let raw = self.bus.read(a);
                    self.lsr_val(raw)
                };
                self.bus.write(a, v);
                self.eor(v);
                Ok(8)
            }

            // RRA: ROR mem, then ADC into A
            0x67 => {
                let a = self.addr_zp();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                self.adc(v);
                Ok(5)
            }
            0x77 => {
                let a = self.addr_zpx();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                self.adc(v);
                Ok(6)
            }
            0x6F => {
                let a = self.addr_abs();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                self.adc(v);
                Ok(6)
            }
            0x7F => {
                let (a, _) = self.addr_absx();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                self.adc(v);
                Ok(7)
            }
            0x7B => {
                let (a, _) = self.addr_absy();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                self.adc(v);
                Ok(7)
            }
            0x63 => {
                let a = self.addr_indx();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                self.adc(v);
                Ok(8)
            }
            0x73 => {
                let (a, _) = self.addr_indy();
                let v = {
                    let raw = self.bus.read(a);
                    self.ror_val(raw)
                };
                self.bus.write(a, v);
                self.adc(v);
                Ok(8)
            }

            // ALR / ASR: AND #imm then LSR A
            0x4B => {
                let v = self.fetch();
                self.a &= v;
                self.a = self.lsr_val(self.a);
                Ok(2)
            }

            // ANC: AND #imm then copy N to C
            0x0B | 0x2B => {
                let v = self.fetch();
                self.and(v);
                self.p.set(bits::C, self.p.get(bits::N));
                Ok(2)
            }

            0x6B => {
                let v = self.fetch();
                self.arr(v);
                Ok(2)
            }

            // SBX / AXS: (A & X) - imm -> X
            0xCB => {
                let v = self.fetch();
                let ax = self.a & self.x;
                let r = (ax as i16) - (v as i16);
                self.x = (r & 0xFF) as u8;
                self.p.set(bits::C, ax >= v);
                self.p.set_zn(self.x);
                Ok(2)
            }

            // NOPs Não-documentados (1 byte, 2 bytes ou 3 bytes)
            0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => Ok(2), // 1 byte NOP
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => {
                let _ = self.fetch();
                Ok(2)
            } // 2 bytes NOP imm
            0x04 | 0x44 | 0x64 => {
                let a = self.addr_zp();
                let _ = self.bus.read(a);
                Ok(3)
            } // NOP zp
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => {
                let a = self.addr_zpx();
                let _ = self.bus.read(a);
                Ok(4)
            } // NOP zp,X
            0x0C => {
                let a = self.addr_abs();
                let _ = self.bus.read(a);
                Ok(4)
            } // NOP abs
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => {
                let (a, c) = self.addr_absx();
                let _ = self.bus.read(a);
                Ok(if c { 5 } else { 4 })
            } // NOP abs,X

            // JAM / KIL / HLT: Trava a execução da CPU
            0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                self.halted = true;
                Ok(0)
            }

            // SHA / SHX / SHY / TAS(SHS): value = reg & (base high byte + 1). On a page
            // cross the value also replaces the high byte of the target address.
            0x93 | 0x9F => {
                let ea = if op == 0x93 {
                    self.addr_indy().0
                } else {
                    self.addr_absy().0
                };
                self.unstable_store(ea, self.y, self.a & self.x);
                Ok(if op == 0x93 { 6 } else { 5 })
            }
            0x9E => {
                let (ea, _) = self.addr_absy();
                self.unstable_store(ea, self.y, self.x);
                Ok(5)
            }
            0x9C => {
                let (ea, _) = self.addr_absx();
                self.unstable_store(ea, self.x, self.y);
                Ok(5)
            }
            0x9B => {
                self.sp = self.a & self.x;
                let (ea, _) = self.addr_absy();
                self.unstable_store(ea, self.y, self.sp);
                Ok(5)
            }
            0xBB => {
                let (a, c) = self.addr_absy();
                let v = self.bus.read(a);
                self.las(v);
                Ok(if c { 5 } else { 4 })
            }
            0x8B => {
                let v = self.fetch();
                self.ane(v);
                Ok(2)
            }
        }
    }

    fn unstable_store(&mut self, ea: u16, index: u8, src: u8) {
        let base_hi = (ea.wrapping_sub(u16::from(index)) >> 8) as u8;
        let (address, value) = cycles::high_byte_store(src, base_hi, ea);
        self.bus.write(address, value);
    }

    /// `ARR`: `A & imm` rotated right through C. Binary: C = bit 6, V = bit 6 ^ bit 5.
    /// NMOS decimal: N = old C, Z and V from the rotated value, then each nibble is
    /// BCD-fixed and C comes from the high-nibble fix ("No More Secrets"). The 2A03
    /// has no decimal mode, so it always takes the binary path.
    fn arr(&mut self, v: u8) {
        let t = self.a & v;
        let carry_in = self.p.get(bits::C);
        let r = (u8::from(carry_in) << 7) | (t >> 1);
        self.p.set_zn(r);
        if !self.p.get(bits::D) || self.variant == CpuVariant::Ricoh2A03 {
            self.a = r;
            self.p.set(bits::C, r & 0x40 != 0);
            self.p.set(bits::V, ((r >> 6) ^ (r >> 5)) & 1 != 0);
            return;
        }
        self.p.set(bits::V, (t ^ r) & 0x40 != 0);
        let (hi, lo) = (t >> 4, t & 0x0F);
        let mut a = r;
        if lo + (lo & 1) > 5 {
            a = (a & 0xF0) | (a.wrapping_add(6) & 0x0F);
        }
        let carry = hi + (hi & 1) > 5;
        if carry {
            a = a.wrapping_add(0x60);
        }
        self.p.set(bits::C, carry);
        self.a = a;
    }

    /// `LAS`: `SP & M` goes to A, X and SP.
    fn las(&mut self, v: u8) {
        self.sp &= v;
        self.a = self.sp;
        self.x = self.sp;
        self.p.set_zn(self.a);
    }

    fn ane(&mut self, v: u8) {
        self.a = (self.a | ANE_MAGIC) & self.x & v;
        self.p.set_zn(self.a);
    }

    /// Everything `step_cycle`, `step_half_cycle` and `step_phase` carry between calls,
    /// as the 36 bytes a version 8 snapshot stores:
    ///
    /// | bytes | content |
    /// | --- | --- |
    /// | 0 | bits: sequence active, IRQ line, NMI latch, `CLI` IRQ defer, sequence is an interrupt, IRQ poll, NMI poll, 65C02 decimal extra cycle |
    /// | 1 | bits: PHI1 open, its R/W, PHI2 half pending, its R/W, complete, page-cross penalty |
    /// | 2–3 | opcode address (LE) |
    /// | 4 | opcode |
    /// | 5–8 | cycles of the instruction so far (LE) |
    /// | 9–10, 11–12 | IRQ and NMI polling histories (LE) |
    /// | 13–14 | open PHI1 address (LE) |
    /// | 15–16, 17 | pending PHI2 address (LE) and data |
    /// | 18–29 | sequence state ([`Active`](crate::cycles) layout), zero when idle |
    /// | 30 | 65C02 maker: 0 not recorded (states written before it existed), 1 WDC, 2 Rockwell, 3 Synertek |
    /// | 31 | 65C02 RMW dead cycle: 0 not recorded, 1 measured, 2 WDC table |
    /// | 32–35 | reserved, zero |
    pub fn cycle_state(&self) -> Vec<u8> {
        let mut b = vec![0u8; 36];
        b[0] = cycles::pack_bits(&[
            self.cycle_seq.busy(),
            self.irq_line,
            self.nmi_pending,
            self.irq_defer,
            self.insn_irq,
            self.irq_poll,
            self.nmi_poll,
            self.decimal_extra,
        ]);
        let (phi1_addr, phi1_write) = self.phi1_open.unwrap_or((0, false));
        let pending = self.phi2_pending;
        b[1] = cycles::pack_bits(&[
            self.phi1_open.is_some(),
            phi1_write,
            pending.is_some(),
            pending.is_some_and(|c| c.write),
            pending.is_some_and(|c| c.complete),
            pending.is_some_and(|c| c.page_cross_penalty),
        ]);
        b[2..4].copy_from_slice(&self.insn_pc.to_le_bytes());
        b[4] = self.insn_op;
        b[5..9].copy_from_slice(&self.insn_n.to_le_bytes());
        b[9..11].copy_from_slice(&self.irq_hist.to_le_bytes());
        b[11..13].copy_from_slice(&self.nmi_hist.to_le_bytes());
        b[13..15].copy_from_slice(&phi1_addr.to_le_bytes());
        if let Some(cycle) = pending {
            b[15..17].copy_from_slice(&cycle.address.to_le_bytes());
            b[17] = cycle.data;
        }
        if let Some(active) = &self.cycle_seq.active {
            b[18..30].copy_from_slice(&active.to_bytes());
        }
        b[30] = match self.cmos_model {
            CmosModel::Wdc => 1,
            CmosModel::Rockwell => 2,
            CmosModel::Synertek => 3,
        };
        b[31] = match self.cmos_rmw_dead {
            RmwDeadCycle::Measured => 1,
            RmwDeadCycle::WdcTable => 2,
        };
        b
    }

    /// Restore [`Cpu::cycle_state`]. Rejects a wrong length, reserved bits, or an
    /// unknown sequence kind, and changes nothing on error.
    pub fn load_cycle_state(&mut self, b: &[u8]) -> Result<(), String> {
        if b.len() != 36 {
            return Err(format!("cycle state must be 36 bytes, got {}", b.len()));
        }
        if b[1] & 0xC0 != 0 || b[30] > 3 || b[31] > 2 || b[32..].iter().any(|&x| x != 0) {
            return Err("reserved cycle state bits set".into());
        }
        let word = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let mut raw = [0u8; 12];
        raw.copy_from_slice(&b[18..30]);
        let active = if b[0] & 1 != 0 {
            Some(Active::from_bytes(&raw)?)
        } else {
            None
        };
        self.cycle_seq = CycleSeq { active };
        self.irq_line = b[0] & 2 != 0;
        self.nmi_pending = b[0] & 4 != 0;
        self.irq_defer = b[0] & 8 != 0;
        self.insn_irq = b[0] & 16 != 0;
        self.irq_poll = b[0] & 32 != 0;
        self.nmi_poll = b[0] & 64 != 0;
        self.decimal_extra = b[0] & 128 != 0;
        self.phi1_open = (b[1] & 1 != 0).then(|| (word(13), b[1] & 2 != 0));
        self.phi2_pending = (b[1] & 4 != 0).then(|| BusCycle {
            address: word(15),
            data: b[17],
            write: b[1] & 8 != 0,
            complete: b[1] & 16 != 0,
            page_cross_penalty: b[1] & 32 != 0,
        });
        self.insn_pc = word(2);
        self.insn_op = b[4];
        self.insn_n = u32::from_le_bytes([b[5], b[6], b[7], b[8]]);
        self.irq_hist = word(9);
        self.nmi_hist = word(11);
        // Byte 30 = 0: an older state without the maker; keep the current one.
        if let Some(model) = [CmosModel::Wdc, CmosModel::Rockwell, CmosModel::Synertek]
            .get(usize::from(b[30]).wrapping_sub(1))
        {
            self.cmos_model = *model;
        }
        match b[31] {
            1 => self.cmos_rmw_dead = RmwDeadCycle::Measured,
            2 => self.cmos_rmw_dead = RmwDeadCycle::WdcTable,
            _ => {}
        }
        Ok(())
    }

    /// Drop an in-progress bus-cycle sequence and the NMI latch. Used when restoring an image.
    pub fn abandon_cycle_sequence(&mut self) {
        self.cycle_seq = CycleSeq::default();
        self.phi2_pending = None;
        self.phi1_open = None;
        self.irq_poll = false;
        self.nmi_poll = false;
        self.irq_hist = 0;
        self.nmi_hist = 0;
        self.nmi_pending = false;
        self.irq_defer = false;
    }

    /// Level of the IRQ pin. Sampled only by [`Cpu::step_cycle`] between instructions.
    /// `Cpu::step` ignores it; call [`Cpu::irq`] for an immediate instruction-level IRQ.
    pub fn set_irq_line(&mut self, asserted: bool) {
        self.irq_line = asserted;
    }

    pub fn irq_line(&self) -> bool {
        self.irq_line
    }

    /// Latch a falling-edge NMI. Consumed by the next [`Cpu::step_cycle`] at an
    /// instruction boundary, even if the I flag is set. [`Cpu::reset`] drops the latch.
    /// [`Cpu::nmi`] is the immediate instruction-level entry and does not use this latch.
    pub fn pulse_nmi(&mut self) {
        self.nmi_pending = true;
    }

    pub fn nmi_pending(&self) -> bool {
        self.nmi_pending
    }

    pub fn cycle_step_in_progress(&self) -> bool {
        self.cycle_seq.busy()
    }

    /// Advance one bus cycle for every opcode [`cycles::is_bus_cycle_stepped_variant`]
    /// accepts on this CPU's variant ([`cycles::CYCLE_STEPPED`]).
    ///
    /// `JAM`/`KIL` is one bus read: the CPU halts and `PC` stays on the opcode.
    /// IRQ is polled where the NMOS 6502 polls it: the line level during the cycle
    /// before the last decides whether the IRQ is taken after this instruction, so a
    /// line raised only for the last cycle waits one more instruction. A branch polls
    /// before its operand fetch (and, when taken across a page, before the fix-up),
    /// so a taken branch without a page cross ignores the line during its last two
    /// cycles. `I` is checked on the boundary. NMI is taken from the latch on the next
    /// boundary. `CLI` delays a pending IRQ by one instruction.
    /// After 65C02 `WAI` each call is one idle read of `PC` (no side effect, reported
    /// complete) until an IRQ level or a latched NMI wakes the CPU; with `I` set the
    /// IRQ wakes it without the vector. `STP` halts until [`Cpu::reset`].
    /// Reset is seven reads inside [`Cpu::reset`], not this sequencer.
    /// `true` when the next [`Cpu::step_cycle`] will be a bus write (stores, RMW
    /// writes, JSR/BRK/IRQ/NMI pushes, PHA/PHP). Between instructions the next cycle is
    /// an opcode fetch, a read. DMA controllers use this: they may only halt the CPU
    /// on a read cycle.
    pub fn next_cycle_writes(&self) -> bool {
        use crate::cycles::Kind;
        let Some(active) = self.cycle_seq.active.as_ref() else {
            return false;
        };
        let writes: &[u8] = match active.kind {
            Kind::ZpStore => &[2],
            Kind::AbsStore | Kind::ZpIdxStore => &[3],
            Kind::AbsXStore | Kind::ZpIndStore | Kind::BitZp => &[4],
            Kind::IndXStore | Kind::IndYStore => &[5],
            Kind::RmwZp => &[3, 4],
            Kind::RmwZpX | Kind::RmwAbs => &[4, 5],
            Kind::RmwAbsX => &[5, 6],
            Kind::RmwIndX | Kind::RmwIndY => &[6, 7],
            Kind::Jsr => &[3, 4],
            Kind::Interrupt => &[2, 3, 4],
            Kind::Push => &[2],
            _ => &[],
        };
        let rmw = matches!(
            active.kind,
            Kind::RmwZp
                | Kind::RmwZpX
                | Kind::RmwAbs
                | Kind::RmwAbsX
                | Kind::RmwIndX
                | Kind::RmwIndY
        );
        if rmw && self.variant == CpuVariant::CMOS {
            // The 65C02 reads instead of the NMOS dummy write.
            return writes.last() == Some(&active.phase);
        }
        writes.contains(&active.phase)
    }

    pub fn step_cycle(&mut self) -> Result<BusCycle, String> {
        if self.phi2_pending.is_some() {
            return Err("PHI2 half pending; finish it with step_half_cycle".into());
        }
        if self.phi1_open.is_some() {
            return Err("PHI1 open; finish the cycle with step_phase".into());
        }
        if self.halted && !self.cycle_seq.busy() {
            return Err("cpu halted".into());
        }
        if self.waiting && !self.cycle_seq.busy() {
            // WAI polls every idle cycle. The cycle that sees the line still idles; the
            // interrupt (or, with I set, the next opcode) starts on the following call.
            if self.irq_line || self.nmi_pending {
                self.waiting = false;
                self.irq_poll = self.irq_line;
                self.nmi_poll = self.nmi_pending;
            }
            self.cycles = self.cycles.wrapping_add(1);
            return Ok(bus_read(self.pc, self.bus.peek(self.pc), true));
        }
        if self.cycle_seq.active.is_none() {
            self.begin_cycle_sequence()?;
        }
        let (kind, phase) = {
            let active = self.cycle_seq.active.as_ref().unwrap();
            (active.kind, active.phase)
        };
        let bit = 1u16.checked_shl(u32::from(phase)).unwrap_or(0);
        if self.irq_line {
            self.irq_hist |= bit;
        }
        if self.nmi_pending {
            self.nmi_hist |= bit;
        }
        let cycle = self.advance_cycle_phase()?;
        self.cycles = self.cycles.wrapping_add(1);
        self.insn_n = self.insn_n.saturating_add(1);
        if cycle.complete {
            self.irq_poll = polled_at(self.irq_hist, kind, phase);
            self.nmi_poll = polled_at(self.nmi_hist, kind, phase);
        }
        if cycle.complete && !self.insn_irq {
            if self.insn_op == 0x20 {
                self.jsr_depth = self.jsr_depth.saturating_add(1);
            } else if self.insn_op == 0x60 {
                self.jsr_depth = self.jsr_depth.saturating_sub(1);
            }
            self.last_insn = Some(InsnLog {
                address: self.insn_pc,
                opcode: self.insn_op,
                cycles: self.insn_n,
            });
        }
        Ok(cycle)
    }

    /// One PHI2 half-cycle on top of [`Cpu::step_cycle`].
    ///
    /// The PHI1 half reports the address and R/W line (address valid, `data` is
    /// `None`); the PHI2 half of the same cycle reports the data byte. The bus access
    /// itself happens when the PHI1 half is produced, so device side effects keep the
    /// order of `step_cycle`. Architectural results equal `step` and `step_cycle`.
    /// `step` and `step_cycle` refuse to run between the two halves.
    pub fn step_half_cycle(&mut self) -> Result<HalfCycle, String> {
        if let Some(cycle) = self.phi2_pending.take() {
            return Ok(HalfCycle {
                phase: BusPhase::Phi2,
                address: cycle.address,
                write: cycle.write,
                data: Some(cycle.data),
                complete: cycle.complete,
            });
        }
        let cycle = self.step_cycle()?;
        self.phi2_pending = Some(cycle);
        Ok(HalfCycle {
            phase: BusPhase::Phi1,
            address: cycle.address,
            write: cycle.write,
            data: None,
            complete: false,
        })
    }

    /// Two-phase clock: PHI1 drives address and R/W without touching the bus; the
    /// access, its device side effects and the data happen in PHI2.
    ///
    /// PHI1 finds the address by running the cycle on a copy of the CPU registers over
    /// [`MemoryBus::peek`] with writes dropped: the bus is not copied and sees no access.
    /// PHI2 runs the real cycle. [`MemoryBus::phase`] clocks the bus devices before
    /// each half (before the access in PHI2).
    /// A cycle's address and R/W depend only on state before it, so they match; IRQ,
    /// NMI or device changes made between the halves are seen by the PHI2 access.
    /// Results equal [`Cpu::step_cycle`]. `step`, `step_cycle` and `step_half_cycle`
    /// refuse to run while PHI1 is open.
    pub fn step_phase(&mut self) -> Result<HalfCycle, String> {
        if let Some((address, write)) = self.phi1_open.take() {
            self.bus.phase(BusPhase::Phi2);
            let cycle = self.step_cycle()?;
            if (cycle.address, cycle.write) != (address, write) {
                return Err(format!(
                    "PHI2 access {:04X} differs from PHI1 address {address:04X}",
                    cycle.address
                ));
            }
            return Ok(HalfCycle {
                phase: BusPhase::Phi2,
                address,
                write,
                data: Some(cycle.data),
                complete: cycle.complete,
            });
        }
        let cycle = self.peek_probe().step_cycle()?;
        self.bus.phase(BusPhase::Phi1);
        self.phi1_open = Some((cycle.address, cycle.write));
        Ok(HalfCycle {
            phase: BusPhase::Phi1,
            address: cycle.address,
            write: cycle.write,
            data: None,
            complete: false,
        })
    }

    /// This CPU over a side-effect-free view of its bus. A cycle's address and R/W
    /// depend only on state before it, so one probe cycle yields them; its data is not used.
    fn peek_probe(&self) -> Cpu<PeekBus<'_, B>> {
        Cpu {
            bus: PeekBus(&self.bus),
            a: self.a,
            x: self.x,
            y: self.y,
            sp: self.sp,
            pc: self.pc,
            p: self.p,
            cycles: self.cycles,
            variant: self.variant,
            cmos_model: self.cmos_model,
            cmos_rmw_dead: self.cmos_rmw_dead,
            halt_on_brk: self.halt_on_brk,
            halted: self.halted,
            irq_line: self.irq_line,
            nmi_pending: self.nmi_pending,
            irq_defer: self.irq_defer,
            cycle_seq: self.cycle_seq.clone(),
            last_insn: self.last_insn,
            jsr_depth: self.jsr_depth,
            insn_pc: self.insn_pc,
            insn_op: self.insn_op,
            insn_n: self.insn_n,
            insn_irq: self.insn_irq,
            waiting: self.waiting,
            irq_poll: self.irq_poll,
            irq_hist: self.irq_hist,
            nmi_poll: self.nmi_poll,
            nmi_hist: self.nmi_hist,
            phi2_pending: self.phi2_pending,
            phi1_open: self.phi1_open,
            decimal_extra: self.decimal_extra,
        }
    }

    /// Run the instruction at `PC` with [`Cpu::step_cycle`] and return every bus cycle.
    ///
    /// The lines are `ADDR R|W DATA`.
    pub fn trace_bus_instruction(&mut self) -> Result<Vec<BusCycle>, String> {
        let mut trace = Vec::new();
        loop {
            let cycle = self.step_cycle()?;
            let done = cycle.complete;
            trace.push(cycle);
            if done {
                break;
            }
        }
        Ok(trace)
    }

    fn begin_cycle_sequence(&mut self) -> Result<(), String> {
        let irq_level = self.irq_poll;
        let nmi_ready = self.nmi_poll;
        self.irq_hist = 0;
        self.nmi_hist = 0;
        if self.nmi_pending && nmi_ready {
            self.nmi_pending = false;
            self.insn_irq = true;
            self.insn_n = 0;
            // The boundary is consumed by NMI. A CLI delay does not postpone NMI,
            // and it does not survive into the handler.
            self.irq_defer = false;
            let mut active = Active::start(Kind::Interrupt, 0);
            active.vector = 0xFFFA;
            // op 0 would look like BRK. Hardware NMI pushes B clear and does not skip a signature byte.
            active.brk = false;
            active.signature = false;
            self.cycle_seq.active = Some(active);
            return Ok(());
        }
        let deferred = self.irq_defer;
        if deferred {
            self.irq_defer = false;
        } else if irq_level && !self.p.get(bits::I) {
            self.insn_irq = true;
            self.insn_n = 0;
            let mut active = Active::start(Kind::Interrupt, 0);
            active.brk = false;
            active.signature = false;
            self.cycle_seq.active = Some(active);
            return Ok(());
        }
        // peek, not read: a rejected opcode must not perform a bus side effect.
        let raw = self.bus.peek(self.pc);
        let op = self.model_op(raw);
        let Some(kind) = cycles::classify(op, self.variant) else {
            if deferred {
                self.irq_defer = true;
            }
            return Err(format!(
                "opcode ${op:02X} is instruction-granular; bus-cycle stepping does not cover it"
            ));
        };
        self.insn_pc = self.pc;
        // The log keeps the byte in memory, also when a maker lacks it and runs a NOP.
        self.insn_op = raw;
        self.insn_n = 0;
        self.insn_irq = false;
        let mut active = Active::start(kind, op);
        // NMOS SHX abs,Y. On CMOS the same byte is STZ abs,X.
        active.use_y |= op == 0x9E && self.variant != CpuVariant::CMOS;
        self.cycle_seq.active = Some(active);
        Ok(())
    }

    fn advance_cycle_phase(&mut self) -> Result<BusCycle, String> {
        let kind = self.cycle_seq.active.as_ref().unwrap().kind;
        let phase = self.cycle_seq.active.as_ref().unwrap().phase;
        let cycle = match kind {
            Kind::ImmLoad => self.cyc_imm_load(phase)?,
            Kind::ZpLoad => self.cyc_zp_load(phase)?,
            Kind::AbsLoad => self.cyc_abs_load(phase)?,
            Kind::AbsXLoad => self.cyc_absx_load(phase)?,
            Kind::ZpStore => self.cyc_zp_store(phase)?,
            Kind::AbsStore => self.cyc_abs_store(phase)?,
            Kind::AbsXStore => self.cyc_absx_store(phase)?,
            Kind::Branch => self.cyc_branch(phase)?,
            Kind::Jsr => self.cyc_jsr(phase)?,
            Kind::Rts => self.cyc_rts(phase)?,
            Kind::Interrupt => self.cyc_interrupt(phase)?,
            Kind::Implied => self.cyc_implied(phase)?,
            Kind::ZpIdxRead => self.cyc_zp_idx_read(phase)?,
            Kind::ZpIdxStore => self.cyc_zp_idx_store(phase)?,
            Kind::IndXRead => self.cyc_indx_read(phase)?,
            Kind::IndYRead => self.cyc_indy_read(phase)?,
            Kind::IndXStore => self.cyc_indx_store(phase)?,
            Kind::IndYStore => self.cyc_indy_store(phase)?,
            Kind::RmwZp => self.cyc_rmw_zp(phase)?,
            Kind::RmwZpX => self.cyc_rmw_zpx(phase)?,
            Kind::RmwAbs => self.cyc_rmw_abs(phase)?,
            Kind::RmwAbsX => self.cyc_rmw_absx(phase)?,
            Kind::Push => self.cyc_push(phase)?,
            Kind::Pull => self.cyc_pull(phase)?,
            Kind::JmpAbs => self.cyc_jmp_abs(phase)?,
            Kind::JmpInd => self.cyc_jmp_ind(phase)?,
            Kind::Rti => self.cyc_rti(phase)?,
            Kind::RmwIndX => self.cyc_rmw_indx(phase)?,
            Kind::RmwIndY => self.cyc_rmw_indy(phase)?,
            Kind::ZpIndRead => self.cyc_zp_ind_read(phase)?,
            Kind::ZpIndStore => self.cyc_zp_ind_store(phase)?,
            Kind::JmpIndX => self.cyc_jmp_indx(phase)?,
            Kind::Jam => self.cyc_jam(phase)?,
            Kind::CmosNop => self.cyc_cmos_nop(phase)?,
            Kind::BitZp => self.cyc_bit_zp(phase)?,
            Kind::BitBranch => self.cyc_bit_branch(phase)?,
            Kind::Wait => self.cyc_wait(phase)?,
            Kind::DecimalExtra => {
                let address = self.pc;
                bus_read(address, self.read_at(address), true)
            }
        };
        let mut cycle = cycle;
        if cycle.complete && std::mem::take(&mut self.decimal_extra) {
            cycle.complete = false;
            self.cycle_seq.active.as_mut().unwrap().kind = Kind::DecimalExtra;
        }
        if let Some(active) = self.cycle_seq.active.as_mut() {
            active.phase = active.phase.wrapping_add(1);
        }
        if cycle.complete {
            self.cycle_seq.active = None;
        }
        Ok(cycle)
    }

    fn read_at(&mut self, address: u16) -> u8 {
        self.bus.read(address)
    }

    fn finish_load(&mut self, value: u8) {
        let op = self.cycle_seq.active.as_ref().unwrap().op;
        match op {
            0xA9 | 0xA5 | 0xB5 | 0xAD | 0xBD | 0xB9 | 0xA1 | 0xB1 => self.lda(value),
            0xA2 | 0xA6 | 0xB6 | 0xAE | 0xBE => self.ldx(value),
            0xA0 | 0xA4 | 0xB4 | 0xAC | 0xBC => self.ldy(value),
            0x09 | 0x05 | 0x15 | 0x0D | 0x1D | 0x19 | 0x01 | 0x11 => self.ora(value),
            0x29 | 0x25 | 0x35 | 0x2D | 0x3D | 0x39 | 0x21 | 0x31 => self.and(value),
            0x49 | 0x45 | 0x55 | 0x4D | 0x5D | 0x59 | 0x41 | 0x51 => self.eor(value),
            0x69 | 0x65 | 0x75 | 0x6D | 0x7D | 0x79 | 0x61 | 0x71 => self.adc(value),
            0xE9 | 0xEB | 0xE5 | 0xF5 | 0xED | 0xFD | 0xF9 | 0xE1 | 0xF1 => self.sbc(value),
            0xC9 | 0xC5 | 0xD5 | 0xCD | 0xDD | 0xD9 | 0xC1 | 0xD1 => self.cmp_reg(self.a, value),
            0xE0 | 0xE4 | 0xEC => self.cmp_reg(self.x, value),
            0xC0 | 0xC4 | 0xCC => self.cmp_reg(self.y, value),
            0x24 | 0x2C => self.bit(value),
            0xA7 | 0xB7 | 0xAF | 0xBF | 0xA3 | 0xB3 | 0xAB => {
                self.lda(value);
                self.ldx(value);
            }
            0x0B | 0x2B => {
                self.and(value);
                self.p.set(bits::C, self.p.get(bits::N));
            }
            0x4B => {
                self.a &= value;
                self.a = self.lsr_val(self.a);
            }
            0x6B => self.arr(value),
            0xCB => {
                let ax = self.a & self.x;
                let wrapped = (ax as i16) - (value as i16);
                self.x = (wrapped & 0xFF) as u8;
                self.p.set(bits::C, ax >= value);
                self.p.set_zn(self.x);
            }
            0x12 => self.ora(value),
            0x32 => self.and(value),
            0x52 => self.eor(value),
            0x72 => self.adc(value),
            0xB2 => self.lda(value),
            0xD2 => self.cmp_reg(self.a, value),
            0xF2 => self.sbc(value),
            0x8B => self.ane(value),
            0xBB => self.las(value),
            0x34 | 0x3C if self.variant == crate::opcodes::CpuVariant::CMOS => self.bit(value),
            0x89 if self.variant == crate::opcodes::CpuVariant::CMOS => {
                self.p.set(bits::Z, self.a & value == 0);
            }
            _ => {}
        }
    }

    fn store_source(&self) -> u8 {
        match self.cycle_seq.active.as_ref().unwrap().op {
            0x86 | 0x96 | 0x8E => self.x,
            0x84 | 0x94 | 0x8C => self.y,
            0x87 | 0x97 | 0x8F | 0x83 => self.a & self.x,
            0x64 | 0x74 | 0x9C | 0x9E => 0,
            _ => self.a,
        }
    }

    /// Address and byte of an indexed store's write cycle. NMOS SHA/SHX/SHY/TAS
    /// AND the source with the base high byte + 1 and, on a page cross, put that
    /// value on the high address lines. TAS first loads `SP = A & X`.
    fn indexed_store_target(&mut self) -> (u16, u8) {
        let (op, ea, hi) = {
            let active = self.cycle_seq.active.as_ref().unwrap();
            (active.op, active.ea, active.hi)
        };
        let cmos_stz = self.variant == CpuVariant::CMOS && matches!(op, 0x9C | 0x9E);
        let src = match op {
            _ if cmos_stz => return (ea, self.store_source()),
            0x93 | 0x9F => self.a & self.x,
            0x9E => self.x,
            0x9C => self.y,
            0x9B => {
                self.sp = self.a & self.x;
                self.sp
            }
            _ => return (ea, self.store_source()),
        };
        cycles::high_byte_store(src, hi, ea)
    }

    fn index_reg(&self) -> u8 {
        if self
            .cycle_seq
            .active
            .as_ref()
            .map(|a| a.use_y)
            .unwrap_or(false)
        {
            self.y
        } else {
            self.x
        }
    }

    fn fetch_pc(&mut self) -> (u16, u8) {
        let address = self.pc;
        let data = self.bus.read(address);
        self.pc = self.pc.wrapping_add(1);
        (address, data)
    }

    fn cyc_imm_load(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.finish_load(data);
                Ok(bus_read(address, data, true))
            }
            _ => Err("immediate load phase overflow".into()),
        }
    }

    fn cyc_zp_load(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().ea = data as u16;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.finish_load(data);
                Ok(bus_read(ea, data, true))
            }
            _ => Err("zero-page load phase overflow".into()),
        }
    }

    fn cyc_abs_load(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(address, data, false))
            }
            3 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.finish_load(data);
                Ok(bus_read(ea, data, true))
            }
            _ => Err("absolute load phase overflow".into()),
        }
    }

    fn cyc_absx_load(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                let base = u16::from_le_bytes([lo, data]);
                let index = self.index_reg();
                let ea = base.wrapping_add(index as u16);
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.ea = ea;
                active.crossed = cycles::page_cross(base, ea);
                active.lo = lo;
                active.hi = data;
                Ok(bus_read(address, data, false))
            }
            3 => {
                let (crossed, ea, hi, lo) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.crossed, active.ea, active.hi, active.lo)
                };
                if crossed {
                    let index = self.index_reg();
                    let fixup = self.index_dummy(hi, lo, index);
                    let data = self.read_at(fixup);
                    Ok(BusCycle {
                        address: fixup,
                        data,
                        write: false,
                        complete: false,
                        page_cross_penalty: true,
                    })
                } else {
                    let data = self.read_at(ea);
                    self.finish_load(data);
                    Ok(bus_read(ea, data, true))
                }
            }
            4 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.finish_load(data);
                Ok(bus_read(ea, data, true))
            }
            _ => Err("absolute,X load phase overflow".into()),
        }
    }

    fn cyc_zp_store(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().ea = data as u16;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.store_source();
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("zero-page store phase overflow".into()),
        }
    }

    fn cyc_abs_store(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(address, data, false))
            }
            3 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.store_source();
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("absolute store phase overflow".into()),
        }
    }

    fn cyc_absx_store(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                let base = u16::from_le_bytes([lo, data]);
                let index = self.index_reg();
                let ea = base.wrapping_add(index as u16);
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.ea = ea;
                active.crossed = cycles::page_cross(base, ea);
                active.hi = data;
                Ok(bus_read(address, data, false))
            }
            3 => {
                let (hi, lo, penalty) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.hi, active.lo, active.crossed)
                };
                let index = self.index_reg();
                let fixup = self.index_dummy(hi, lo, index);
                let data = self.read_at(fixup);
                Ok(BusCycle {
                    address: fixup,
                    data,
                    write: false,
                    complete: false,
                    page_cross_penalty: penalty,
                })
            }
            4 => {
                let (ea, data) = self.indexed_store_target();
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("absolute,X store phase overflow".into()),
        }
    }

    fn branch_taken(&self, op: u8) -> bool {
        match op {
            0x10 => !self.p.get(bits::N),
            0x30 => self.p.get(bits::N),
            0x50 => !self.p.get(bits::V),
            0x70 => self.p.get(bits::V),
            0x90 => !self.p.get(bits::C),
            0xB0 => self.p.get(bits::C),
            0xD0 => !self.p.get(bits::Z),
            0xF0 => self.p.get(bits::Z),
            0x80 => true,
            _ => false,
        }
    }

    fn cyc_branch(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                let op = self.cycle_seq.active.as_ref().unwrap().op;
                let take = self.branch_taken(op);
                let target = self.pc.wrapping_add(data as i8 as u16);
                let crossed = cycles::page_cross(self.pc, target);
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.take = take;
                active.crossed = crossed;
                active.ea = target;
                Ok(bus_read(address, data, !take))
            }
            2 => {
                let (target, crossed) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.ea, active.crossed)
                };
                // 64doc: the taken cycle reads the next opcode at PC while PCL is added.
                let address = self.pc;
                let data = self.read_at(address);
                if !crossed {
                    self.pc = target;
                }
                Ok(bus_read(address, data, !crossed))
            }
            3 => {
                let target = self.cycle_seq.active.as_ref().unwrap().ea;
                // 64doc: the fix-up cycle reads the unfixed address (old PCH, new PCL).
                let address = (self.pc & 0xFF00) | (target & 0x00FF);
                let data = self.read_at(address);
                self.pc = target;
                Ok(BusCycle {
                    address,
                    data,
                    write: false,
                    complete: true,
                    page_cross_penalty: true,
                })
            }
            _ => Err("branch phase overflow".into()),
        }
    }

    fn cyc_jsr(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                Ok(bus_read(address, data, false))
            }
            3 => {
                let address = 0x0100 | self.sp as u16;
                let data = (self.pc >> 8) as u8;
                self.bus.write(address, data);
                self.sp = self.sp.wrapping_sub(1);
                Ok(bus_write(address, data, false))
            }
            4 => {
                let address = 0x0100 | self.sp as u16;
                let data = (self.pc & 0xFF) as u8;
                self.bus.write(address, data);
                self.sp = self.sp.wrapping_sub(1);
                Ok(bus_write(address, data, false))
            }
            5 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                self.pc = u16::from_le_bytes([lo, data]);
                Ok(bus_read(address, data, true))
            }
            _ => Err("jsr phase overflow".into()),
        }
    }

    fn cyc_rts(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let address = self.pc;
                let data = self.read_at(address);
                Ok(bus_read(address, data, false))
            }
            2 => {
                // 64doc: cycle 3 reads $0100,S (discarded), then increments S.
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                self.sp = self.sp.wrapping_add(1);
                Ok(bus_read(address, data, false))
            }
            3 => {
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                self.sp = self.sp.wrapping_add(1);
                Ok(bus_read(address, data, false))
            }
            4 => {
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                self.pc = u16::from_le_bytes([lo, data]);
                Ok(bus_read(address, data, false))
            }
            5 => {
                let address = self.pc;
                let data = self.read_at(address);
                self.pc = self.pc.wrapping_add(1);
                Ok(bus_read(address, data, true))
            }
            _ => Err("rts phase overflow".into()),
        }
    }

    fn cyc_interrupt(&mut self, phase: u8) -> Result<BusCycle, String> {
        let signature = self.cycle_seq.active.as_ref().unwrap().signature;
        match phase {
            0 | 1 => {
                let address = self.pc;
                let data = self.read_at(address);
                if signature {
                    self.pc = self.pc.wrapping_add(1);
                }
                Ok(bus_read(address, data, false))
            }
            2 => {
                let address = 0x0100 | self.sp as u16;
                let data = (self.pc >> 8) as u8;
                self.bus.write(address, data);
                self.sp = self.sp.wrapping_sub(1);
                Ok(bus_write(address, data, false))
            }
            3 => {
                let address = 0x0100 | self.sp as u16;
                let data = (self.pc & 0xFF) as u8;
                self.bus.write(address, data);
                self.sp = self.sp.wrapping_sub(1);
                Ok(bus_write(address, data, false))
            }
            4 => {
                let brk = self.cycle_seq.active.as_ref().unwrap().brk;
                let data = if brk {
                    self.p.to_stack_php()
                } else {
                    self.p.to_stack_irq()
                };
                let address = 0x0100 | self.sp as u16;
                self.bus.write(address, data);
                self.sp = self.sp.wrapping_sub(1);
                self.p.set(bits::I, true);
                Ok(bus_write(address, data, false))
            }
            5 => {
                // NMI hijack: an NMI latched during the first four cycles of a BRK or
                // IRQ entry redirects the vector fetch to $FFFA. The pushed status keeps
                // BRK's B flag, and the NMI is consumed.
                if self.nmi_pending && self.nmi_hist & 0b1000 != 0 {
                    self.nmi_pending = false;
                    self.cycle_seq.active.as_mut().unwrap().vector = 0xFFFA;
                }
                let vector = self.cycle_seq.active.as_ref().unwrap().vector;
                let data = self.read_at(vector);
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(vector, data, false))
            }
            6 => {
                let vector = self.cycle_seq.active.as_ref().unwrap().vector;
                let brk = self.cycle_seq.active.as_ref().unwrap().brk;
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                let address = vector.wrapping_add(1);
                let data = self.read_at(address);
                self.pc = u16::from_le_bytes([lo, data]);
                if brk && self.halt_on_brk {
                    self.halted = true;
                }
                Ok(bus_read(address, data, true))
            }

            _ => Err("interrupt phase overflow".into()),
        }
    }

    fn apply_implied(&mut self) {
        let op = self.cycle_seq.active.as_ref().unwrap().op;
        match op {
            0xAA => {
                self.x = self.a;
                self.p.set_zn(self.x);
            }
            0xA8 => {
                self.y = self.a;
                self.p.set_zn(self.y);
            }
            0x8A => {
                self.a = self.x;
                self.p.set_zn(self.a);
            }
            0x98 => {
                self.a = self.y;
                self.p.set_zn(self.a);
            }
            0xBA => {
                self.x = self.sp;
                self.p.set_zn(self.x);
            }
            0x9A => self.sp = self.x,
            0xE8 => {
                self.x = self.x.wrapping_add(1);
                self.p.set_zn(self.x);
            }
            0xCA => {
                self.x = self.x.wrapping_sub(1);
                self.p.set_zn(self.x);
            }
            0xC8 => {
                self.y = self.y.wrapping_add(1);
                self.p.set_zn(self.y);
            }
            0x88 => {
                self.y = self.y.wrapping_sub(1);
                self.p.set_zn(self.y);
            }
            0x18 => self.p.set(bits::C, false),
            0x38 => self.p.set(bits::C, true),
            0x58 => {
                self.p.set(bits::I, false);
                // Sampled I is the value from before this instruction. IRQ waits
                // until the instruction after CLI has completed.
                self.irq_defer = true;
            }
            0x78 => self.p.set(bits::I, true),
            0xB8 => self.p.set(bits::V, false),
            0xD8 => self.p.set(bits::D, false),
            0xF8 => self.p.set(bits::D, true),
            0x0A => self.a = self.asl_val(self.a),
            0x4A => self.a = self.lsr_val(self.a),
            0x2A => self.a = self.rol_val(self.a),
            0x6A => self.a = self.ror_val(self.a),
            0xEA => {}
            0x1A if self.variant == crate::opcodes::CpuVariant::CMOS => {
                self.a = self.a.wrapping_add(1);
                self.p.set_zn(self.a);
            }
            0x3A if self.variant == crate::opcodes::CpuVariant::CMOS => {
                self.a = self.a.wrapping_sub(1);
                self.p.set_zn(self.a);
            }
            _ => {}
        }
    }

    fn rmw_result(&mut self, raw: u8) -> u8 {
        let op = self.cycle_seq.active.as_ref().unwrap().op;
        let cmos = self.variant == crate::opcodes::CpuVariant::CMOS;
        match op {
            0x06 | 0x16 | 0x0E | 0x1E | 0x07 | 0x17 | 0x0F | 0x1F | 0x1B | 0x03 | 0x13 => {
                self.asl_val(raw)
            }
            0x26 | 0x36 | 0x2E | 0x3E | 0x27 | 0x37 | 0x2F | 0x3F | 0x3B | 0x23 | 0x33 => {
                self.rol_val(raw)
            }
            0x46 | 0x56 | 0x4E | 0x5E | 0x47 | 0x57 | 0x4F | 0x5F | 0x5B | 0x43 | 0x53 => {
                self.lsr_val(raw)
            }
            0x66 | 0x76 | 0x6E | 0x7E | 0x67 | 0x77 | 0x6F | 0x7F | 0x7B | 0x63 | 0x73 => {
                self.ror_val(raw)
            }
            0xE6 | 0xF6 | 0xEE | 0xFE | 0xE7 | 0xF7 | 0xEF | 0xFF | 0xFB | 0xE3 | 0xF3 => {
                let v = raw.wrapping_add(1);
                self.p.set_zn(v);
                v
            }
            0xC6 | 0xD6 | 0xCE | 0xDE | 0xC7 | 0xD7 | 0xCF | 0xDF | 0xDB | 0xC3 | 0xD3 => {
                let v = raw.wrapping_sub(1);
                self.p.set_zn(v);
                v
            }
            0x04 | 0x0C if cmos => {
                self.p.set(bits::Z, self.a & raw == 0);
                raw | self.a
            }
            0x14 | 0x1C if cmos => {
                self.p.set(bits::Z, self.a & raw == 0);
                raw & !self.a
            }
            _ => raw,
        }
    }

    /// Second half of an illegal RMW: the accumulator op sees the value just stored.
    fn rmw_combine(&mut self, value: u8) {
        let op = self.cycle_seq.active.as_ref().unwrap().op;
        match op {
            0x07 | 0x17 | 0x0F | 0x1F | 0x1B | 0x03 | 0x13 => self.ora(value),
            0x27 | 0x37 | 0x2F | 0x3F | 0x3B | 0x23 | 0x33 => self.and(value),
            0x47 | 0x57 | 0x4F | 0x5F | 0x5B | 0x43 | 0x53 => self.eor(value),
            0x67 | 0x77 | 0x6F | 0x7F | 0x7B | 0x63 | 0x73 => self.adc(value),
            0xC7 | 0xD7 | 0xCF | 0xDF | 0xDB | 0xC3 | 0xD3 => self.cmp_reg(self.a, value),
            0xE7 | 0xF7 | 0xEF | 0xFF | 0xFB | 0xE3 | 0xF3 => self.sbc(value),
            _ => {}
        }
    }

    /// 65C02 `WAI`/`STP`: opcode fetch and two reads of the next byte, then sleep or stop.
    fn cyc_wait(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 | 2 => {
                let address = self.pc;
                let data = self.read_at(address);
                if phase == 2 {
                    if self.cycle_seq.active.as_ref().unwrap().op == 0xCB {
                        self.waiting = true;
                    } else {
                        self.halted = true;
                    }
                }
                Ok(bus_read(address, data, phase == 2))
            }
            _ => Err("wait/stop phase overflow".into()),
        }
    }

    /// Address of dummy read `extra` (0-based, after the operand) of a 65C02 NOP.
    fn cmos_nop_address(&self, mode: crate::opcodes::Mode, lo: u8, hi: u8, extra: u16) -> u16 {
        use crate::opcodes::Mode;
        match (mode, extra) {
            (Mode::Zp, _) | (Mode::ZpX, 0) => u16::from(lo),
            (Mode::ZpX, _) => u16::from(lo.wrapping_add(self.x)),
            (Mode::Abs, _) => u16::from_le_bytes([lo, hi]),
            _ => self.pc,
        }
    }

    /// Middle cycle of a read-modify-write: NMOS writes the unmodified byte back, the
    /// 65C02 reads the address again instead (no spurious write to I/O registers).
    /// 65C02 ASL/ROL/LSR/ROR abs,X take 6 cycles when `lo + X` stays in the page
    /// (INC/DEC abs,X keep 7); NMOS always takes 7.
    fn cmos_fast_shift(&self, op: u8, lo: u8) -> bool {
        self.variant == CpuVariant::CMOS
            && matches!(op, 0x1E | 0x3E | 0x5E | 0x7E)
            && u16::from(lo) + u16::from(self.x) < 0x100
    }

    fn rmw_dummy(&mut self, ea: u16, held: u8) -> BusCycle {
        if self.variant == CpuVariant::CMOS {
            let kind = self.cycle_seq.active.as_ref().map(|a| a.kind);
            let address = match (self.cmos_rmw_dead, kind) {
                (RmwDeadCycle::WdcTable, Some(Kind::RmwZp | Kind::RmwZpX)) => (ea + 1) & 0xFF,
                (RmwDeadCycle::WdcTable, Some(Kind::RmwAbsX)) => ea.wrapping_add(1),
                _ => ea,
            };
            let data = self.read_at(address);
            bus_read(address, data, false)
        } else {
            self.bus.write(ea, held);
            bus_write(ea, held, false)
        }
    }

    fn cyc_bit_zp(&mut self, phase: u8) -> Result<BusCycle, String> {
        let active = self.cycle_seq.active.as_ref().unwrap();
        let (op, ea, held) = (active.op, active.ea, active.held);
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from(data);
                Ok(bus_read(address, data, false))
            }
            2 | 3 => {
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            4 => {
                let data = bit_op_result(op, held);
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("RMB/SMB phase overflow".into()),
        }
    }

    fn cyc_bit_branch(&mut self, phase: u8) -> Result<BusCycle, String> {
        let active = self.cycle_seq.active.as_ref().unwrap();
        let (op, ea, held, lo) = (active.op, active.ea, active.held, active.lo);
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from(data);
                Ok(bus_read(address, data, false))
            }
            // W65C02S data sheet (March 2000) table 5-5, mode 12b: the zero-page
            // read and its internal re-read come before the offset fetch at PC+2.
            2 => {
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            3 => {
                let data = self.read_at(ea);
                Ok(bus_read(ea, data, false))
            }
            4 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, !bit_branch_taken(op, held)))
            }
            5 => {
                let next = self.pc;
                let target = next.wrapping_add_signed(i16::from(lo as i8));
                let data = self.read_at(next);
                let same_page = next & 0xFF00 == target & 0xFF00;
                if same_page {
                    self.pc = target;
                }
                Ok(bus_read(next, data, same_page))
            }
            6 => {
                let next = self.pc;
                let target = next.wrapping_add_signed(i16::from(lo as i8));
                let address = (next & 0xFF00) | (target & 0x00FF);
                let data = self.read_at(address);
                self.pc = target;
                let mut cycle = bus_read(address, data, true);
                cycle.page_cross_penalty = true;
                Ok(cycle)
            }
            _ => Err("BBR/BBS phase overflow".into()),
        }
    }

    fn cyc_cmos_nop(&mut self, phase: u8) -> Result<BusCycle, String> {
        let op = self.cycle_seq.active.as_ref().unwrap().op;
        let info = crate::opcodes::cmos_nop(op).ok_or("not a 65C02 NOP")?;
        let size = info.mode.size() as u8;
        let last = phase + 1 == info.cycles;
        if phase >= info.cycles {
            return Err("65C02 NOP phase overflow".into());
        }
        if phase < size {
            let (address, data) = self.fetch_pc();
            let active = self.cycle_seq.active.as_mut().unwrap();
            match phase {
                1 => active.lo = data,
                2 => active.hi = data,
                _ => {}
            }
            return Ok(bus_read(address, data, last));
        }
        let active = self.cycle_seq.active.as_ref().unwrap();
        let address =
            self.cmos_nop_address(info.mode, active.lo, active.hi, u16::from(phase - size));
        let data = self.read_at(address);
        Ok(bus_read(address, data, last))
    }

    fn cyc_jam(&mut self, phase: u8) -> Result<BusCycle, String> {
        if phase != 0 {
            return Err("jam phase overflow".into());
        }
        let address = self.pc;
        let data = self.read_at(address);
        self.halted = true;
        Ok(bus_read(address, data, true))
    }

    fn cyc_implied(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let address = self.pc;
                let data = self.read_at(address);
                self.apply_implied();
                Ok(bus_read(address, data, true))
            }
            _ => Err("implied phase overflow".into()),
        }
    }

    fn cyc_zp_idx_read(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let base = self.cycle_seq.active.as_ref().unwrap().lo;
                let index = self.index_reg();
                let dead = self.zp_dead(base);
                let data = self.read_at(dead);
                self.cycle_seq.active.as_mut().unwrap().ea = base.wrapping_add(index) as u16;
                Ok(bus_read(dead, data, false))
            }
            3 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.finish_load(data);
                Ok(bus_read(ea, data, true))
            }
            _ => Err("zero-page indexed read phase overflow".into()),
        }
    }

    fn cyc_zp_idx_store(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let base = self.cycle_seq.active.as_ref().unwrap().lo;
                let index = self.index_reg();
                let dead = self.zp_dead(base);
                let data = self.read_at(dead);
                self.cycle_seq.active.as_mut().unwrap().ea = base.wrapping_add(index) as u16;
                Ok(bus_read(dead, data, false))
            }
            3 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.store_source();
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("zero-page indexed store phase overflow".into()),
        }
    }

    fn cyc_indx_read(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo;
                let index = self.index_reg();
                let dead = self.zp_dead(zp);
                let data = self.read_at(dead);
                self.cycle_seq.active.as_mut().unwrap().hi = zp.wrapping_add(index);
                Ok(bus_read(dead, data, false))
            }
            3 => {
                let ptr = self.cycle_seq.active.as_ref().unwrap().hi;
                let data = self.read_at(ptr as u16);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ptr as u16, data, false))
            }
            4 => {
                let ptr = self.cycle_seq.active.as_ref().unwrap().hi.wrapping_add(1);
                let data = self.read_at(ptr as u16);
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(ptr as u16, data, false))
            }
            5 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.finish_load(data);
                Ok(bus_read(ea, data, true))
            }
            _ => Err("(zp,X) read phase overflow".into()),
        }
    }

    fn cyc_indx_store(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo;
                let index = self.index_reg();
                let dead = self.zp_dead(zp);
                let data = self.read_at(dead);
                self.cycle_seq.active.as_mut().unwrap().hi = zp.wrapping_add(index);
                Ok(bus_read(dead, data, false))
            }
            3 => {
                let ptr = self.cycle_seq.active.as_ref().unwrap().hi;
                let data = self.read_at(ptr as u16);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ptr as u16, data, false))
            }
            4 => {
                let ptr = self.cycle_seq.active.as_ref().unwrap().hi.wrapping_add(1);
                let data = self.read_at(ptr as u16);
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(ptr as u16, data, false))
            }
            5 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.store_source();
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("(zp,X) store phase overflow".into()),
        }
    }

    fn cyc_indy_read(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo;
                let data = self.read_at(zp as u16);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(zp as u16, data, false))
            }
            3 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo.wrapping_add(1);
                let data = self.read_at(zp as u16);
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                let base = u16::from_le_bytes([lo, data]);
                let ea = base.wrapping_add(self.y as u16);
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.hi = data;
                active.ea = ea;
                active.crossed = cycles::page_cross(base, ea);
                Ok(bus_read(zp as u16, data, false))
            }
            4 => {
                let (crossed, ea, hi, lo) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.crossed, active.ea, active.hi, active.held)
                };
                if crossed {
                    let fixup = self.indy_dummy(hi, lo);
                    let data = self.read_at(fixup);
                    Ok(BusCycle {
                        address: fixup,
                        data,
                        write: false,
                        complete: false,
                        page_cross_penalty: true,
                    })
                } else {
                    let data = self.read_at(ea);
                    self.finish_load(data);
                    Ok(bus_read(ea, data, true))
                }
            }
            5 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.finish_load(data);
                Ok(bus_read(ea, data, true))
            }
            _ => Err("(zp),Y read phase overflow".into()),
        }
    }

    fn cyc_indy_store(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo;
                let data = self.read_at(zp as u16);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(zp as u16, data, false))
            }
            3 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo.wrapping_add(1);
                let data = self.read_at(zp as u16);
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                let base = u16::from_le_bytes([lo, data]);
                let ea = base.wrapping_add(self.y as u16);
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.hi = data;
                active.ea = ea;
                active.crossed = cycles::page_cross(base, ea);
                Ok(bus_read(zp as u16, data, false))
            }
            4 => {
                let (hi, lo, penalty) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.hi, active.held, active.crossed)
                };
                let fixup = self.indy_dummy(hi, lo);
                let data = self.read_at(fixup);
                Ok(BusCycle {
                    address: fixup,
                    data,
                    write: false,
                    complete: false,
                    page_cross_penalty: penalty,
                })
            }
            5 => {
                let (ea, data) = self.indexed_store_target();
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("(zp),Y store phase overflow".into()),
        }
    }

    fn cyc_rmw_zp(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().ea = data as u16;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            3 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.cycle_seq.active.as_ref().unwrap().held;
                Ok(self.rmw_dummy(ea, data))
            }
            4 => {
                let (ea, raw) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.ea, active.held)
                };
                let data = self.rmw_result(raw);
                self.rmw_combine(data);
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("RMW zero-page phase overflow".into()),
        }
    }

    fn cyc_rmw_zpx(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let base = self.cycle_seq.active.as_ref().unwrap().lo;
                let index = self.index_reg();
                let dead = self.zp_dead(base);
                let data = self.read_at(dead);
                self.cycle_seq.active.as_mut().unwrap().ea = base.wrapping_add(index) as u16;
                Ok(bus_read(dead, data, false))
            }
            3 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            4 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.cycle_seq.active.as_ref().unwrap().held;
                Ok(self.rmw_dummy(ea, data))
            }
            5 => {
                let (ea, raw) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.ea, active.held)
                };
                let data = self.rmw_result(raw);
                self.rmw_combine(data);
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("RMW zero-page,X phase overflow".into()),
        }
    }

    fn cyc_rmw_abs(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(address, data, false))
            }
            3 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            4 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.cycle_seq.active.as_ref().unwrap().held;
                Ok(self.rmw_dummy(ea, data))
            }
            5 => {
                let (ea, raw) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.ea, active.held)
                };
                let data = self.rmw_result(raw);
                self.rmw_combine(data);
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("RMW absolute phase overflow".into()),
        }
    }

    fn cyc_rmw_absx(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                let index = self.index_reg();
                let base = u16::from_le_bytes([lo, data]);
                let ea = base.wrapping_add(index as u16);
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.ea = ea;
                active.crossed = cycles::page_cross(base, ea);
                active.hi = data;
                Ok(bus_read(address, data, false))
            }
            3 if {
                let active = self.cycle_seq.active.as_ref().unwrap();
                !active.crossed && self.cmos_fast_shift(active.op, active.lo)
            } =>
            {
                // 65C02 shifts and rotates skip the fix-up read without a page cross.
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.phase += 1;
                let ea = active.ea;
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            3 => {
                let (hi, lo, penalty) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.hi, active.lo, active.crossed)
                };
                let index = self.index_reg();
                let fixup = self.index_dummy(hi, lo, index);
                let data = self.read_at(fixup);
                Ok(BusCycle {
                    address: fixup,
                    data,
                    write: false,
                    complete: false,
                    page_cross_penalty: penalty,
                })
            }
            4 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            5 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.cycle_seq.active.as_ref().unwrap().held;
                Ok(self.rmw_dummy(ea, data))
            }
            6 => {
                let (ea, raw) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.ea, active.held)
                };
                let data = self.rmw_result(raw);
                self.rmw_combine(data);
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("RMW absolute,X phase overflow".into()),
        }
    }

    fn cyc_push(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let address = self.pc;
                let data = self.read_at(address);
                Ok(bus_read(address, data, false))
            }
            2 => {
                let op = self.cycle_seq.active.as_ref().unwrap().op;
                let data = if op == 0x08 {
                    self.p.to_stack_php()
                } else if op == 0xDA {
                    self.x
                } else if op == 0x5A {
                    self.y
                } else {
                    self.a
                };
                let address = 0x0100 | self.sp as u16;
                self.bus.write(address, data);
                self.sp = self.sp.wrapping_sub(1);
                Ok(bus_write(address, data, true))
            }
            _ => Err("push phase overflow".into()),
        }
    }

    fn cyc_pull(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let address = self.pc;
                let data = self.read_at(address);
                Ok(bus_read(address, data, false))
            }
            2 => {
                // 64doc: cycle 3 reads $0100,S (discarded), then increments S.
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                self.sp = self.sp.wrapping_add(1);
                Ok(bus_read(address, data, false))
            }
            3 => {
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                let op = self.cycle_seq.active.as_ref().unwrap().op;
                if op == 0x28 {
                    self.p = Status::from_stack(data);
                } else if op == 0xFA {
                    self.x = data;
                    self.p.set_zn(self.x);
                } else if op == 0x7A {
                    self.y = data;
                    self.p.set_zn(self.y);
                } else {
                    self.a = data;
                    self.p.set_zn(self.a);
                }
                Ok(bus_read(address, data, true))
            }
            _ => Err("pull phase overflow".into()),
        }
    }

    fn cyc_jmp_abs(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                self.pc = u16::from_le_bytes([lo, data]);
                Ok(bus_read(address, data, true))
            }
            _ => Err("jmp absolute phase overflow".into()),
        }
    }

    fn cyc_jmp_ind(&mut self, phase: u8) -> Result<BusCycle, String> {
        let cmos = self.variant == CpuVariant::CMOS;
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(address, data, false))
            }
            3 if cmos => {
                // Dead cycle: re-reads PC+2, the previous bus address (6502.org
                // "6502 Dead Cycles", verified on a WDC 65C02).
                let address = self.pc.wrapping_sub(1);
                let data = self.read_at(address);
                Ok(bus_read(address, data, false))
            }
            3 | 4 if (phase == 3) != cmos => {
                let ptr = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ptr);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ptr, data, false))
            }
            4 | 5 => {
                let ptr = self.cycle_seq.active.as_ref().unwrap().ea;
                // The 65C02 carries into the high byte; NMOS wraps within the page.
                let hi_addr = if cmos || ptr & 0x00FF != 0x00FF {
                    ptr.wrapping_add(1)
                } else {
                    ptr & 0xFF00
                };
                let data = self.read_at(hi_addr);
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                self.pc = u16::from_le_bytes([lo, data]);
                Ok(bus_read(hi_addr, data, true))
            }
            _ => Err("jmp indirect phase overflow".into()),
        }
    }

    fn cyc_rti(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let address = self.pc;
                let data = self.read_at(address);
                Ok(bus_read(address, data, false))
            }
            2 => {
                // 64doc: cycle 3 reads $0100,S (discarded), then increments S.
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                self.sp = self.sp.wrapping_add(1);
                Ok(bus_read(address, data, false))
            }
            3 => {
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                self.p = Status::from_stack(data);
                self.sp = self.sp.wrapping_add(1);
                Ok(bus_read(address, data, false))
            }
            4 => {
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                self.sp = self.sp.wrapping_add(1);
                Ok(bus_read(address, data, false))
            }
            5 => {
                let address = 0x0100 | self.sp as u16;
                let data = self.read_at(address);
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                self.pc = u16::from_le_bytes([lo, data]);
                Ok(bus_read(address, data, true))
            }
            _ => Err("rti phase overflow".into()),
        }
    }

    fn cyc_rmw_indx(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo;
                let index = self.index_reg();
                let data = self.read_at(u16::from(zp));
                self.cycle_seq.active.as_mut().unwrap().hi = zp.wrapping_add(index);
                Ok(bus_read(u16::from(zp), data, false))
            }
            3 => {
                let ptr = self.cycle_seq.active.as_ref().unwrap().hi;
                let data = self.read_at(u16::from(ptr));
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(u16::from(ptr), data, false))
            }
            4 => {
                let ptr = self.cycle_seq.active.as_ref().unwrap().hi.wrapping_add(1);
                let data = self.read_at(u16::from(ptr));
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(u16::from(ptr), data, false))
            }
            5 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            6 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.cycle_seq.active.as_ref().unwrap().held;
                Ok(self.rmw_dummy(ea, data))
            }
            7 => {
                let (ea, raw) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.ea, active.held)
                };
                let data = self.rmw_result(raw);
                self.rmw_combine(data);
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("(zp,X) RMW phase overflow".into()),
        }
    }

    fn cyc_rmw_indy(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo;
                let data = self.read_at(u16::from(zp));
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(u16::from(zp), data, false))
            }
            3 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo.wrapping_add(1);
                let data = self.read_at(u16::from(zp));
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                let base = u16::from_le_bytes([lo, data]);
                let ea = base.wrapping_add(u16::from(self.y));
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.hi = data;
                active.ea = ea;
                active.crossed = cycles::page_cross(base, ea);
                Ok(bus_read(u16::from(zp), data, false))
            }
            4 => {
                let (hi, lo, penalty) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.hi, active.held, active.crossed)
                };
                let fixup = self.indy_dummy(hi, lo);
                let data = self.read_at(fixup);
                Ok(BusCycle {
                    address: fixup,
                    data,
                    write: false,
                    complete: false,
                    page_cross_penalty: penalty,
                })
            }
            5 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            6 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.cycle_seq.active.as_ref().unwrap().held;
                Ok(self.rmw_dummy(ea, data))
            }
            7 => {
                let (ea, raw) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.ea, active.held)
                };
                let data = self.rmw_result(raw);
                self.rmw_combine(data);
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("(zp),Y RMW phase overflow".into()),
        }
    }

    fn cyc_zp_ind_read(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo;
                let data = self.read_at(u16::from(zp));
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(u16::from(zp), data, false))
            }
            3 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo.wrapping_add(1);
                let data = self.read_at(u16::from(zp));
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(u16::from(zp), data, false))
            }
            4 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.finish_load(data);
                Ok(bus_read(ea, data, true))
            }
            _ => Err("(zp) read phase overflow".into()),
        }
    }

    fn cyc_zp_ind_store(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo;
                let data = self.read_at(u16::from(zp));
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(u16::from(zp), data, false))
            }
            3 => {
                let zp = self.cycle_seq.active.as_ref().unwrap().lo.wrapping_add(1);
                let data = self.read_at(u16::from(zp));
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                self.cycle_seq.active.as_mut().unwrap().ea = u16::from_le_bytes([lo, data]);
                Ok(bus_read(u16::from(zp), data, false))
            }
            4 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.store_source();
                self.bus.write(ea, data);
                Ok(bus_write(ea, data, true))
            }
            _ => Err("(zp) store phase overflow".into()),
        }
    }

    /// Opcode as the selected 65C02 maker decodes it: one it lacks becomes `$03`,
    /// the 1-byte, 1-cycle NOP. Other variants pass through.
    fn model_op(&self, op: u8) -> u8 {
        if self.variant == CpuVariant::CMOS && !self.cmos_model.implements(op) {
            0x03
        } else {
            op
        }
    }

    /// Address of the zero-page indexing dead cycle (zp,X / zp,Y / (zp,X)). NMOS
    /// reads the unindexed zero-page address; the 65C02 re-reads the previous bus
    /// address, the operand byte at `PC - 1`.
    fn zp_dead(&self, base: u8) -> u16 {
        if self.variant == CpuVariant::CMOS {
            self.pc.wrapping_sub(1)
        } else {
            u16::from(base)
        }
    }

    /// Extra read of `(zp),Y`. NMOS reads the unfixed address; the 65C02 re-reads
    /// the previous bus address, the pointer high byte at `zp + 1`, on a page cross
    /// and on the always-present store cycle alike.
    fn indy_dummy(&self, hi: u8, lo: u8) -> u16 {
        if self.variant == CpuVariant::CMOS {
            let zp = self.cycle_seq.active.as_ref().map_or(0, |a| a.lo);
            u16::from(zp.wrapping_add(1))
        } else {
            (u16::from(hi) << 8) | u16::from(lo.wrapping_add(self.y))
        }
    }

    /// Address of the extra read of an indexed access. NMOS reads the unfixed
    /// address (operand high byte, low byte + index). On a page cross the 65C02
    /// re-reads the last instruction byte (`PC - 1`) instead, so no invalid
    /// address reaches I/O.
    fn index_dummy(&self, hi: u8, lo: u8, index: u8) -> u16 {
        let crossed = u16::from(lo) + u16::from(index) > 0xFF;
        if crossed && self.variant == CpuVariant::CMOS {
            self.pc.wrapping_sub(1)
        } else {
            (u16::from(hi) << 8) | u16::from(lo.wrapping_add(index))
        }
    }

    fn cyc_jmp_indx(&mut self, phase: u8) -> Result<BusCycle, String> {
        match phase {
            0 => {
                let (address, data) = self.fetch_pc();
                Ok(bus_read(address, data, false))
            }
            1 => {
                let (address, data) = self.fetch_pc();
                self.cycle_seq.active.as_mut().unwrap().lo = data;
                Ok(bus_read(address, data, false))
            }
            2 => {
                let (address, data) = self.fetch_pc();
                let lo = self.cycle_seq.active.as_ref().unwrap().lo;
                let base = u16::from_le_bytes([lo, data]);
                let ea = base.wrapping_add(u16::from(self.x));
                let active = self.cycle_seq.active.as_mut().unwrap();
                active.ea = ea;
                active.hi = data;
                active.crossed = cycles::page_cross(base, ea);
                Ok(bus_read(address, data, false))
            }
            3 => {
                let (hi, lo, penalty) = {
                    let active = self.cycle_seq.active.as_ref().unwrap();
                    (active.hi, active.lo, active.crossed)
                };
                let _ = (hi, lo);
                // W65C02S table 5-7 "(a,x)": cycle 4 is an internal cycle that
                // re-reads the last operand byte (PC+2).
                let fixup = self.pc.wrapping_sub(1);
                let data = self.read_at(fixup);
                Ok(BusCycle {
                    address: fixup,
                    data,
                    write: false,
                    complete: false,
                    page_cross_penalty: penalty,
                })
            }
            4 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let data = self.read_at(ea);
                self.cycle_seq.active.as_mut().unwrap().held = data;
                Ok(bus_read(ea, data, false))
            }
            5 => {
                let ea = self.cycle_seq.active.as_ref().unwrap().ea;
                let lo = self.cycle_seq.active.as_ref().unwrap().held;
                let hi_addr = ea.wrapping_add(1);
                let data = self.read_at(hi_addr);
                self.pc = u16::from_le_bytes([lo, data]);
                Ok(bus_read(hi_addr, data, true))
            }
            _ => Err("jmp (abs,X) phase overflow".into()),
        }
    }
}

/// Whether a line seen in the phases of `hist` is recognized at the real polling point
/// of a sequence whose last phase is `last`: the cycle before the last, or for a
/// branch, before the operand fetch and before the fix-up of a page-crossing branch.
fn polled_at(hist: u16, kind: Kind, last: u8) -> bool {
    let seen = |p: u8| hist & (1u16 << p) != 0;
    match (kind, last) {
        (Kind::Branch, last) => seen(0) || (last == 3 && seen(2)),
        (_, 0) => false,
        (_, last) => seen(last - 1),
    }
}

fn bus_read(address: u16, data: u8, complete: bool) -> BusCycle {
    BusCycle {
        address,
        data,
        write: false,
        complete,
        page_cross_penalty: false,
    }
}

fn bus_write(address: u16, data: u8, complete: bool) -> BusCycle {
    BusCycle {
        address,
        data,
        write: true,
        complete,
        page_cross_penalty: false,
    }
}

/// `RMBn` (`n7`, n < 8) clears bit n, `SMBn` (`(n+8)7`) sets it.
fn bit_op_result(op: u8, value: u8) -> u8 {
    let mask = 1u8 << ((op >> 4) & 7);
    if op & 0x80 == 0 {
        value & !mask
    } else {
        value | mask
    }
}

/// `BBRn` branches when bit n is clear, `BBSn` when it is set.
fn bit_branch_taken(op: u8, value: u8) -> bool {
    let set = value & (1u8 << ((op >> 4) & 7)) != 0;
    set == (op & 0x80 != 0)
}

/// Why [`Cpu::run_until`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    /// The stop predicate held before an instruction.
    Stopped,
    /// The CPU halted (JAM, `STP`, or `BRK` with `halt_on_brk`).
    Halted,
    /// `max_cycles` were used up; the last instruction may overshoot by its length.
    CycleBudget,
}

impl<B: MemoryBus> Cpu<B> {
    /// Execute whole instructions until `stop` holds on an instruction boundary
    /// (checked before every instruction, including the first), the CPU halts, or
    /// at least `max_cycles` cycles ran. Returns the reason and the cycles used.
    pub fn run_until(
        &mut self,
        max_cycles: u64,
        mut stop: impl FnMut(&Self) -> bool,
    ) -> Result<(RunEnd, u64), String> {
        let start = self.cycles;
        loop {
            let used = self.cycles - start;
            if stop(self) {
                return Ok((RunEnd::Stopped, used));
            }
            if self.halted {
                return Ok((RunEnd::Halted, used));
            }
            if used >= max_cycles {
                return Ok((RunEnd::CycleBudget, used));
            }
            self.step()?;
        }
    }

    /// Run until `PC == target` on an instruction boundary.
    pub fn run_to_pc(&mut self, target: u16, max_cycles: u64) -> Result<(RunEnd, u64), String> {
        self.run_until(max_cycles, |c| c.pc == target)
    }

    /// Step out: run until the subroutine active now returns (its `RTS` brings the
    /// JSR depth below the current one). Nested calls inside are run through.
    pub fn run_to_return(&mut self, max_cycles: u64) -> Result<(RunEnd, u64), String> {
        let depth = self.jsr_depth;
        if depth == 0 {
            return Err("nenhuma sub-rotina ativa".into());
        }
        self.run_until(max_cycles, |c| c.jsr_depth < depth)
    }
}
