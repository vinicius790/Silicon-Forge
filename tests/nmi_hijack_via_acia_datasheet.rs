//! NMI hijack, polling after a fresh start or WAI, and 6522/6551 datasheet rules.

use mos6502::acia::{Acia6551, COMMAND_AFTER_RESET, IRQ, RDRF, TDRE};
use mos6502::bus::{FlatMemory, IoDevice};
use mos6502::flags::bits;
use mos6502::opcodes::CpuVariant;
use mos6502::snapshot::MachineImage;
use mos6502::via::Via6522;
use mos6502::Cpu;

fn cpu(variant: CpuVariant, code: &[u8]) -> Cpu<FlatMemory> {
    let mut mem = FlatMemory::new();
    mem.data[0x8000..0x8100].fill(0xEA);
    mem.data[0x9000..0x9010].fill(0xEA);
    mem.data[0xA000..0xA010].fill(0xEA);
    mem.data[0x8000..0x8000 + code.len()].copy_from_slice(code);
    mem.data[0xFFFE] = 0x00;
    mem.data[0xFFFF] = 0x90;
    mem.data[0xFFFA] = 0x00;
    mem.data[0xFFFB] = 0xA0;
    let mut cpu = Cpu::with_variant(mem, variant);
    cpu.pc = 0x8000;
    cpu.p.set(bits::I, false);
    cpu
}

/// Runs the BRK at $8000, pulsing NMI before cycle `at`.
fn brk_with_nmi_at(at: usize) -> Cpu<FlatMemory> {
    let mut c = cpu(CpuVariant::NMOS, &[0x00, 0x00]);
    for n in 1..=7 {
        if n == at {
            c.pulse_nmi();
        }
        c.step_cycle().unwrap();
    }
    c
}

#[test]
fn nmi_in_the_first_four_brk_cycles_hijacks_the_vector() {
    let c = brk_with_nmi_at(4);
    assert_eq!(c.pc, 0xA000);
    assert!(!c.nmi_pending(), "the hijack consumes the NMI");
    assert_ne!(
        c.bus.data[0x01FB] & bits::B,
        0,
        "BRK's B flag is still pushed"
    );
    assert_eq!(c.bus.data[0x01FC], 0x02, "BRK still pushes PC+2");

    // Cycle 5 or 6: too late to hijack, polled before BRK's last cycle.
    let mut c = brk_with_nmi_at(6);
    assert_eq!(c.pc, 0x9000);
    assert!(c.nmi_pending());
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
    assert_eq!(c.pc, 0xA000);
    // Last cycle: one handler opcode runs first.
    let mut c = brk_with_nmi_at(7);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 2);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
    assert_eq!(c.pc, 0xA000);

    // IRQ entry is hijacked the same way, with B clear.
    let mut c = cpu(CpuVariant::NMOS, &[0xEA]);
    c.set_irq_line(true);
    c.trace_bus_instruction().unwrap();
    c.step_cycle().unwrap();
    c.pulse_nmi();
    while !c.step_cycle().unwrap().complete {}
    assert_eq!(c.pc, 0xA000);
    assert_eq!(c.bus.data[0x01FB] & bits::B, 0);
}

#[test]
fn fresh_start_and_wai_poll_nmi_at_the_real_point() {
    // A pulse before the first cycle lands in the first opcode, which runs first.
    let mut c = cpu(CpuVariant::NMOS, &[0xEA]);
    c.pulse_nmi();
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 2);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
    assert_eq!(c.pc, 0xA000);

    // After reset the latch is gone, and a new pulse waits for the first opcode.
    let mut c = cpu(CpuVariant::NMOS, &[0xEA]);
    c.bus.data[0xFFFC] = 0x00;
    c.bus.data[0xFFFD] = 0x80;
    c.reset();
    c.pulse_nmi();
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 2);
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);

    // WAI: the idle cycle that sees the edge still idles; the NMI starts next.
    let mut c = cpu(CpuVariant::CMOS, &[0xCB]);
    c.trace_bus_instruction().unwrap();
    c.pulse_nmi();
    let idle = c.step_cycle().unwrap();
    assert!(idle.complete && !c.is_waiting());
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
    assert_eq!(c.pc, 0xA000);

    // After an instruction-level step the latch it ran under is recognized at once.
    let mut c = cpu(CpuVariant::NMOS, &[0xEA]);
    c.pulse_nmi();
    c.step().unwrap();
    assert_eq!(c.trace_bus_instruction().unwrap().len(), 7);
}

#[test]
fn via_port_access_clears_ca1_cb1_and_pcr_picks_the_edge() {
    let mut via = Via6522::new();
    via.set_ca1(false);
    via.set_cb1(false);
    assert_eq!(via.peek(0x0D), 0x12, "IFR without IER; no IRQ bit");
    let _ = via.read(0x0F);
    assert_eq!(via.peek(0x0D) & 0x02, 0x02, "$0F leaves CA1");
    let _ = via.read(0x01);
    assert_eq!(via.peek(0x0D) & 0x02, 0, "ORA read clears CA1");
    via.write(0x00, 0x00);
    assert_eq!(via.peek(0x0D) & 0x10, 0, "ORB write clears CB1");

    via.write(0x0C, 0x11); // CA1 and CB1 on rising edges
    via.set_ca1(true);
    via.set_cb1(true);
    assert_eq!(via.peek(0x0D) & 0x12, 0x12);
    via.write(0x0D, 0x12);
    via.set_ca1(false);
    via.set_cb1(false);
    assert_eq!(via.peek(0x0D) & 0x12, 0, "falling edge is inactive now");
}

#[test]
fn via_counters_keep_decrementing_after_time_out_without_a_second_flag() {
    let mut via = Via6522::new();
    via.write(0x04, 0x02);
    via.write(0x05, 0x00);
    via.write(0x08, 0x02);
    via.write(0x09, 0x00);
    via.tick();
    via.tick();
    via.tick();
    assert_eq!(via.peek(0x0D) & 0x60, 0x60);
    via.write(0x0D, 0x60);
    assert_eq!((via.t1_counter(), via.t2_counter()), (0xFFFF, 0xFFFF));
    for _ in 0..0x1_0000 {
        via.tick();
    }
    assert_eq!(via.peek(0x0D) & 0x60, 0, "a one-shot wrap does not re-fire");
    // Unarmed counters count too.
    let mut idle = Via6522::new();
    idle.tick();
    assert_eq!(idle.t1_counter(), 0xFFFF);
}

#[test]
fn acia_reset_state_irq_bit_and_programmed_reset() {
    let mut acia = Acia6551::new();
    assert_eq!(acia.read(2), COMMAND_AFTER_RESET);
    assert_eq!(acia.read(1), TDRE, "hardware reset status is $10");

    // Receiver IRQ needs DTR (bit 0) and bit 1 clear.
    acia.push_rx(0x41);
    assert!(
        !acia.irq_asserted(),
        "reset command disables the receiver IRQ"
    );
    let _ = acia.read(0);
    acia.write(2, 0x09);
    acia.push_rx(0x42);
    assert!(acia.irq_asserted());
    assert_eq!(acia.read(1) & (IRQ | RDRF), IRQ | RDRF);
    assert!(!acia.irq_asserted(), "status read clears bit 7");
    assert_eq!(acia.read(1) & IRQ, 0);

    // Transmitter IRQ: control bits 3-2 = 01 and DTR; raised when TDRE returns.
    let mut tx = Acia6551::new();
    tx.write(3, 0x1F);
    tx.write(2, 0x05);
    let _ = tx.read(1);
    tx.write(0, 0x55);
    assert!(!tx.irq_asserted());
    for _ in 0..tx.baud_period() {
        tx.tick();
    }
    assert!(tx.irq_asserted());

    // Programmed reset keeps parity bits 5-7 and the control register.
    let mut acia = Acia6551::new();
    acia.write(3, 0x1E);
    acia.write(2, 0x6B);
    acia.write(1, 0x00);
    assert_eq!(acia.read(2), 0x60);
    assert_eq!(acia.read(3), 0x1E);
    assert!(!acia.irq_asserted());
}

#[test]
fn acia_irq_survives_a_snapshot_and_old_images_load_it_false() {
    let mut acia = Acia6551::new();
    acia.write(2, 0x09);
    acia.push_rx(0x42);
    let cpu = cpu(CpuVariant::NMOS, &[]);
    let json = MachineImage::capture_with(&cpu, &Via6522::new(), &acia).to_json();
    assert!(json.contains("\"irq\":true"));
    let image = MachineImage::from_json(&json).unwrap();
    let mut back = Acia6551::new();
    back.load_state(&image.acia);
    assert!(back.irq_asserted());

    let old = json.replace(",\"irq\":true", "");
    let image = MachineImage::from_json(&old).unwrap();
    assert!(!image.acia.irq);
}
