//! SystemBus forwards the phase clock; v7 snapshots keep held timer flags and an open PHI1.

use mos6502::acia::{Acia6551, TDRE};
use mos6502::bus::{MemoryBus, SystemBus};
use mos6502::cpu::Cpu;
use mos6502::snapshot::MachineImage;
use mos6502::via::Via6522;
use mos6502::{BusPhase, FlatMemory};

#[test]
fn system_bus_clocks_every_mapped_device_per_phase() {
    let mut via = Via6522::new();
    via.write(0x04, 1);
    via.write(0x05, 0);
    let mut acia = Acia6551::new();
    acia.set_baud_period(2);
    mos6502::bus::IoDevice::write(&mut acia, 0, 0x41);
    let mut bus = SystemBus::new();
    bus.map_device(0x6000, 0x600F, via);
    bus.map_device(0x7000, 0x7003, acia);
    bus.ram_mut()[0x8000..0x8010].fill(0xEA);
    bus.ram_mut()[0xFFFC..0xFFFE].copy_from_slice(&[0x00, 0x80]);
    let mut cpu = Cpu::new(bus);
    cpu.reset();
    let mut seen = Vec::new();
    for _ in 0..5 {
        cpu.step_phase().unwrap();
        seen.push((cpu.bus.peek(0x600D) & 0x40, cpu.bus.peek(0x7001) & TDRE));
    }
    assert_eq!(
        seen,
        [(0, 0), (0, 0), (0, 0), (0, TDRE), (0x40, TDRE)],
        "T1 rolls in the 2nd PHI2 and flags in the next PHI1; the ACIA counts two PHI2s"
    );
}

fn reload(json: &str, cpu: &mut Cpu<FlatMemory>, via: &mut Via6522) {
    MachineImage::from_json(json)
        .unwrap()
        .restore_with(cpu, via, &mut Acia6551::new())
        .unwrap();
}

#[test]
fn held_timer_flags_survive_a_snapshot_and_older_images_load_none() {
    let mut via = Via6522::new();
    via.write(0x04, 0);
    via.write(0x05, 0);
    via.write(0x08, 0);
    via.write(0x09, 0);
    via.tick_phase(BusPhase::Phi2);
    assert_eq!(via.peek(0x0D) & 0x60, 0, "held until PHI1");
    let cpu = Cpu::new(FlatMemory::new());
    let json = MachineImage::capture_with(&cpu, &via, &Acia6551::new()).to_json();

    let mut back = Via6522::new();
    reload(&json, &mut Cpu::new(FlatMemory::new()), &mut back);
    back.tick_phase(BusPhase::Phi1);
    assert_eq!(back.peek(0x0D) & 0x60, 0x60);

    let old = json.replace(",\"t1_flag_due\":true,\"t2_flag_due\":true", "");
    assert_ne!(old, json);
    let mut legacy = Via6522::new();
    reload(&old, &mut Cpu::new(FlatMemory::new()), &mut legacy);
    legacy.tick_phase(BusPhase::Phi1);
    assert_eq!(legacy.peek(0x0D) & 0x60, 0);
}

fn nop_cpu() -> Cpu<FlatMemory> {
    let mut memory = FlatMemory::new();
    memory.load_at(0x8000, &[0xEA, 0xEA, 0xEA]);
    memory.load_at(0xFFFC, &[0x00, 0x80]);
    let mut cpu = Cpu::new(memory);
    cpu.reset();
    cpu
}

#[test]
fn v7_snapshot_resumes_an_open_boundary_phi1_with_its_phi2() {
    let mut cpu = nop_cpu();
    cpu.step_phase().unwrap();
    assert_eq!(cpu.open_phi1(), Some((0x8000, false)));
    let image = MachineImage::capture(&cpu).with_phase_state(&cpu);
    assert_eq!(image.version, 7);
    let json = image.to_json();
    assert!(json.contains("\"phi1_open\":{\"address\":32768,\"write\":false}"));

    let mut back = nop_cpu();
    MachineImage::from_json(&json)
        .unwrap()
        .restore(&mut back)
        .unwrap();
    assert!(back.step().is_err(), "PHI1 is open after restore");
    let phi2 = back.step_phase().unwrap();
    assert_eq!(
        (phi2.phase, phi2.address, phi2.data),
        (BusPhase::Phi2, 0x8000, Some(0xEA))
    );

    let v6 = MachineImage::capture(&cpu).with_wait_state(&cpu).to_json();
    let mut older = nop_cpu();
    MachineImage::from_json(&v6)
        .unwrap()
        .restore(&mut older)
        .unwrap();
    assert_eq!(
        older.step_phase().unwrap().phase,
        BusPhase::Phi1,
        "v6: no open PHI1"
    );

    cpu.step_phase().unwrap();
    cpu.step_phase().unwrap();
    assert_eq!(
        cpu.open_phi1(),
        None,
        "mid-instruction PHI1 is not resumable"
    );
    assert!(MachineImage::from_json(&json.replacen("\"version\":7", "\"version\":9", 1)).is_err());
}
