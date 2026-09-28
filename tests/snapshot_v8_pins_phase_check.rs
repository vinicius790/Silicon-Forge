//! v8 mid-instruction snapshots, 6522 pin load model, PHI1 window check, tick = phases.

use mos6502::acia::RamAcia;
use mos6502::bus::{IoDevice, MemoryBus, PhaseCheckBus};
use mos6502::cpu::Cpu;
use mos6502::snapshot::MachineImage;
use mos6502::via::Via6522;
use mos6502::{BusPhase, FlatMemory};

fn machine() -> Cpu<FlatMemory> {
    let mut m = FlatMemory::new();
    // CLI; LDA #5; INC $1234; ADC $10; JMP $8001
    m.load_at(
        0x8000,
        &[
            0x58, 0xA9, 0x05, 0xEE, 0x34, 0x12, 0x65, 0x10, 0x4C, 0x01, 0x80,
        ],
    );
    m.load_at(0x9000, &[0xE8, 0x40]); // INX; RTI
    m.load_at(0xFFFC, &[0x00, 0x80, 0x00, 0x90]);
    m.data[0x10] = 0x22;
    let mut cpu = Cpu::new(m);
    cpu.reset();
    cpu
}

fn run(cpu: &mut Cpu<FlatMemory>, from: usize, to: usize) -> Vec<String> {
    (from..to)
        .map(|n| {
            cpu.set_irq_line((6..14).contains(&n));
            cpu.step_cycle().unwrap().trace_line()
        })
        .collect()
}

fn restore(json: &str) -> Cpu<FlatMemory> {
    let mut cpu = Cpu::new(FlatMemory::new());
    MachineImage::from_json(json)
        .unwrap()
        .restore(&mut cpu)
        .unwrap();
    cpu
}

#[test]
fn v8_resumes_every_cycle_cut_exactly_including_an_irq_sequence() {
    for cut in 0..40 {
        let mut original = machine();
        run(&mut original, 0, cut);
        let json = MachineImage::capture(&original)
            .with_cycle_state(&original)
            .to_json();
        let mut resumed = restore(&json);
        assert_eq!(
            run(&mut resumed, cut, 60),
            run(&mut original, cut, 60),
            "cut {cut}"
        );
        assert_eq!(
            (
                resumed.a,
                resumed.x,
                resumed.pc,
                resumed.p.0,
                resumed.bus.data[0x1234]
            ),
            (
                original.a,
                original.x,
                original.pc,
                original.p.0,
                original.bus.data[0x1234]
            )
        );
    }
}

#[test]
fn v8_resumes_between_half_cycles_and_inside_an_open_phi1() {
    let mut half = machine();
    run(&mut half, 0, 5);
    half.step_half_cycle().unwrap();
    let mut back = restore(
        &MachineImage::capture(&half)
            .with_cycle_state(&half)
            .to_json(),
    );
    assert_eq!(
        back.step_half_cycle().unwrap(),
        half.step_half_cycle().unwrap()
    );

    let mut phase = machine();
    run(&mut phase, 0, 5);
    phase.step_phase().unwrap();
    assert_eq!(
        phase.open_phi1(),
        None,
        "mid-instruction: v7 cannot keep it"
    );
    let mut back = restore(
        &MachineImage::capture(&phase)
            .with_cycle_state(&phase)
            .to_json(),
    );
    let resumed = back.step_phase().unwrap();
    assert_eq!(resumed.phase, BusPhase::Phi2);
    assert_eq!(resumed, phase.step_phase().unwrap());
}

#[test]
fn v8_rejects_9_and_bad_state_without_touching_the_cpu_and_v7_drops_the_sequence() {
    let mut cpu = machine();
    run(&mut cpu, 0, 5);
    let json = MachineImage::capture(&cpu).with_cycle_state(&cpu).to_json();
    assert!(MachineImage::from_json(&json.replacen("\"version\":8", "\"version\":9", 1)).is_err());

    let mut bad = MachineImage::from_json(&json).unwrap();
    bad.cycle_state.as_mut().unwrap()[18] = 200; // no such sequence kind
    let mut target = machine();
    assert!(bad.restore(&mut target).is_err());
    assert_eq!(target.pc, 0x8000, "registers untouched");
    assert!(target.step().is_ok(), "no sequence was half-loaded");

    let v7 = MachineImage::capture(&cpu).with_phase_state(&cpu).to_json();
    let mut older = restore(&v7);
    assert!(older.step().is_ok(), "v7 abandons the in-progress sequence");
}

#[test]
fn port_pins_float_high_and_open_drain_loads_follow_the_documented_model() {
    let mut via = Via6522::new();
    assert_eq!(via.read(0x01), 0xFF, "undriven inputs read the pull-up");
    via.drive_port_a(0x0F, 0x05);
    assert_eq!(
        via.read(0x01),
        0xF5,
        "driven nibble reads its level, the rest float"
    );

    via.write(0x03, 0xF0); // PA7..4 outputs
    via.write(0x01, 0xF0);
    via.set_port_a_load_low(0x80);
    assert_eq!(
        via.port_a_pins(),
        0x75,
        "PA: an external low wins over a high output"
    );
    assert_eq!(via.read(0x0F), 0x75);

    via.write(0x02, 0xF0); // PB7..4 outputs
    via.write(0x00, 0xF0);
    via.set_port_b_load_low(0x81);
    assert_eq!(
        via.read(0x00),
        0xFE,
        "PB: push-pull output and ORB read win; input bit 0 sinks"
    );
    assert_eq!(via.port_b_pins(), 0xFE);

    via.set_port_b_in(0x00);
    assert_eq!(
        via.read(0x00) & 0x0F,
        0,
        "set_port_b_in drives every input pin"
    );
}

#[test]
fn phase_window_check_rejects_accesses_in_phi1_and_step_phase_never_trips_it() {
    let mut inner = RamAcia::new(0xD000);
    inner
        .memory
        .load_at(0x8000, &[0xA9, 0x07, 0x85, 0x20, 0xAD, 0x01, 0xD0]);
    inner.memory.load_at(0xFFFC, &[0x00, 0x80]);
    inner.acia.write(2, 0x09);
    inner.acia.push_rx(1);
    let mut cpu = Cpu::new(PhaseCheckBus::new(inner));
    cpu.reset();
    for _ in 0..18 {
        cpu.step_phase().unwrap();
    }
    assert!(cpu.bus.rejected.is_empty());
    assert_eq!(cpu.bus.inner.memory.data[0x20], 0x07);
    assert!(
        !cpu.bus.inner.acia.irq_asserted(),
        "status read landed in PHI2"
    );

    cpu.bus.inner.acia.read(0);
    cpu.bus.inner.acia.push_rx(2);
    cpu.bus.phase(BusPhase::Phi1);
    cpu.bus.write(0x0021, 0x99);
    let status = cpu.bus.read(0xD001);
    assert_eq!(cpu.bus.rejected, [(0x0021, true), (0xD001, false)]);
    assert_eq!(cpu.bus.inner.memory.data[0x21], 0, "write dropped");
    assert_eq!(status & 0x80, 0x80);
    assert!(
        cpu.bus.inner.acia.irq_asserted(),
        "rejected read had no side effect"
    );
}

#[test]
fn plain_tick_equals_a_phi2_then_a_phi1() {
    let mut whole = Via6522::new();
    let mut halves = Via6522::new();
    for via in [&mut whole, &mut halves] {
        via.write(0x0B, 0x40);
        via.write(0x04, 2);
        via.write(0x05, 0);
    }
    for _ in 0..12 {
        whole.tick();
        halves.tick_phase(BusPhase::Phi2);
        halves.tick_phase(BusPhase::Phi1);
        assert_eq!(whole.state(), halves.state());
    }
    halves.tick_phase(BusPhase::Phi2);
    halves.tick_phase(BusPhase::Phi2);
    halves.tick_phase(BusPhase::Phi2);
    let mut held = halves.clone();
    held.tick();
    assert_eq!(
        held.peek(0x0D) & 0x40,
        0x40,
        "tick also shows a flag held by a PHI2"
    );
}
