//! T2 N+1.5, 6551 crystal divider, break, RTS/DTR, modem/echo snapshot, two-phase clock.

use mos6502::acia::{crystal_frame_period, Acia6551, RamAcia, IRQ, RDRF, TDRE};
use mos6502::bus::IoDevice;
use mos6502::cpu::Cpu;
use mos6502::snapshot::MachineImage;
use mos6502::via::Via6522;
use mos6502::{BusPhase, FlatMemory};

#[test]
fn timed_t2_flags_n_plus_one_ticks_after_load_and_pulse_mode_still_at_zero() {
    let mut via = Via6522::new();
    via.write(0x08, 3);
    via.write(0x09, 0);
    for _ in 0..3 {
        via.tick();
    }
    assert_eq!(via.peek(0x0D) & 0x20, 0, "not at zero");
    via.tick();
    assert_eq!(
        via.peek(0x0D) & 0x20,
        0x20,
        "N+1 ticks, N+1.5 with the write"
    );
    assert_eq!(via.t2_counter(), 0xFFFF);

    let mut pulses = Via6522::new();
    pulses.write(0x0B, 0x20);
    pulses.write(0x08, 2);
    pulses.write(0x09, 0);
    for _ in 0..2 {
        pulses.set_pb6(true);
        pulses.set_pb6(false);
    }
    assert_eq!(pulses.peek(0x0D) & 0x20, 0x20, "pulse count flags at zero");
}

#[test]
fn crystal_frame_counts_sixteen_clocks_per_bit_through_the_divider() {
    assert_eq!(crystal_frame_period(0x1F, 0x0B), 160 * 6, "8N1 at 19200");
    assert_eq!(crystal_frame_period(0x11, 0x0B), 160 * 2304, "8N1 at 50");
    assert_eq!(
        crystal_frame_period(0x9F, 0x2B),
        176 * 6,
        "8 data + parity: 1 stop"
    );
    assert_eq!(
        crystal_frame_period(0xFF, 0x0B),
        120 * 6,
        "5N with 1.5 stop"
    );
    assert_eq!(
        crystal_frame_period(0xBF, 0x0B),
        160 * 6,
        "7N2: ten bits too"
    );

    for (cpu_hz, ticks) in [(1_843_200, 960), (1_000_000, 521)] {
        let mut acia = Acia6551::new();
        acia.set_crystal(Some(cpu_hz));
        acia.write(2, 0x0B);
        acia.write(3, 0x1F);
        acia.write(0, 0x55);
        for _ in 1..ticks {
            acia.tick();
        }
        assert_eq!(acia.status() & TDRE, 0, "{cpu_hz} Hz");
        acia.tick();
        assert_eq!(
            acia.status() & TDRE,
            TDRE,
            "{cpu_hz} Hz: 960 crystal periods"
        );
    }
}

#[test]
fn rts_dtr_outputs_and_break_follow_the_command_register() {
    let mut acia = Acia6551::new();
    assert!(!acia.rts_asserted() && !acia.dtr_asserted() && !acia.tx_break());
    acia.write(2, 0x0B);
    assert!(acia.rts_asserted() && acia.dtr_asserted() && !acia.tx_break());
    acia.write(2, 0x0F);
    assert!(acia.tx_break() && acia.rts_asserted());
    acia.write(2, 0x01);
    assert!(!acia.rts_asserted() && acia.dtr_asserted());
    acia.receive_break();
    assert_eq!(
        acia.read(1) & (RDRF | 0x02),
        RDRF | 0x02,
        "break: framing error"
    );
    assert_eq!(acia.read(0), 0x00);
}

#[test]
fn modem_levels_echo_queue_and_crystal_survive_a_snapshot() {
    let mut acia = Acia6551::new();
    acia.set_crystal(Some(1_000_000));
    acia.write(3, 0x1F);
    acia.write(2, 0x11);
    acia.push_rx(0x7E);
    acia.set_dsr(true);
    acia.set_cts(true);
    acia.tick();
    let cpu = Cpu::new(FlatMemory::new());
    let json = MachineImage::capture_with(&cpu, &Via6522::new(), &acia).to_json();

    let mut back = Acia6551::new();
    let mut cpu2 = Cpu::new(FlatMemory::new());
    MachineImage::from_json(&json)
        .unwrap()
        .restore_with(&mut cpu2, &mut Via6522::new(), &mut back)
        .unwrap();
    assert_eq!(back.state(), acia.state());
    assert_eq!(back.status() & (1 << 6), 1 << 6);
    assert_eq!(back.pop_echo(), Some(0x7E));
    back.write(0, 0x31);
    assert_eq!(back.pop_tx(), None, "CTS high restored");

    let old = &json[..json.find(",\"dcd_high\"").unwrap()];
    let mut legacy = Acia6551::new();
    MachineImage::from_json(&format!("{old}}}}}"))
        .unwrap()
        .restore_with(&mut cpu2, &mut Via6522::new(), &mut legacy)
        .unwrap();
    assert_eq!(legacy.status() & (1 << 6), 0);
    assert_eq!(legacy.pop_echo(), None);
}

fn acia_machine() -> Cpu<RamAcia> {
    let mut bus = RamAcia::new(0xD000);
    // LDA $D001 ; STA $D000
    bus.memory
        .load_at(0x8000, &[0xAD, 0x01, 0xD0, 0x8D, 0x00, 0xD0, 0xEA]);
    bus.memory.load_at(0xFFFC, &[0x00, 0x80]);
    bus.acia.write(2, 0x09);
    bus.acia.push_rx(0x41);
    let mut cpu = Cpu::new(bus);
    cpu.reset();
    cpu
}

#[test]
fn two_phase_clock_accesses_the_bus_only_in_phi2_with_step_results() {
    let mut cpu = acia_machine();
    let mut halves = Vec::new();
    for _ in 0..3 {
        halves.push(cpu.step_phase().unwrap());
        halves.push(cpu.step_phase().unwrap());
    }
    let phi1 = cpu.step_phase().unwrap();
    assert_eq!(
        (phi1.phase, phi1.address, phi1.data),
        (BusPhase::Phi1, 0xD001, None)
    );
    assert!(cpu.bus.acia.irq_asserted(), "PHI1 does not read status");
    assert!(cpu.step().is_err(), "step refuses with PHI1 open");
    let phi2 = cpu.step_phase().unwrap();
    assert_eq!(phi2.data.unwrap() & IRQ, IRQ);
    assert!(!cpu.bus.acia.irq_asserted(), "the read happens in PHI2");

    for _ in 0..3 {
        cpu.step_phase().unwrap();
        cpu.step_phase().unwrap();
    }
    let phi1 = cpu.step_phase().unwrap();
    assert_eq!((phi1.address, phi1.write), (0xD000, true));
    assert!(!cpu.bus.acia.tx_pending(), "PHI1 does not write");
    let phi2 = cpu.step_phase().unwrap();
    assert!(phi2.complete && cpu.bus.acia.tx_pending());

    let mut reference = acia_machine();
    reference.step().unwrap();
    reference.step().unwrap();
    assert_eq!(
        (cpu.a, cpu.pc, cpu.p.0),
        (reference.a, reference.pc, reference.p.0)
    );
    assert_eq!(cpu.bus.acia.state(), reference.bus.acia.state());
    assert!(halves
        .iter()
        .all(|h| h.phase != BusPhase::Phi1 || h.data.is_none()));
}
