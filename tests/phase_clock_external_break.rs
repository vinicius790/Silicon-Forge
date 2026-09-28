//! Per-phase VIA/ACIA ticks under step_phase, 6551 external 16x clock, break, no bus copy.

use mos6502::acia::{frame_16x_clocks, Acia6551, RamAcia, TDRE};
use mos6502::bus::{IoDevice, MemoryBus};
use mos6502::cpu::Cpu;
use mos6502::snapshot::MachineImage;
use mos6502::via::Via6522;
use mos6502::{BusPhase, FlatMemory};

fn half(via: &mut Via6522, phase: BusPhase) -> bool {
    via.tick_phase(phase);
    via.peek(0x0D) & 0x60 == 0x60
}

#[test]
fn via_timer_flags_appear_in_the_phi1_after_the_roll_n_plus_one_and_a_half() {
    let mut via = Via6522::new();
    via.write(0x04, 3);
    via.write(0x05, 0); // PHI2 of cycle 0
    via.write(0x08, 3);
    via.write(0x09, 0);
    for cycle in 1..=4 {
        assert!(!half(&mut via, BusPhase::Phi1), "cycle {cycle} PHI1");
        assert!(!half(&mut via, BusPhase::Phi2), "cycle {cycle} PHI2");
    }
    assert_eq!(via.t1_counter(), 0xFFFF, "rolled in the PHI2 of cycle N+1");
    assert!(
        half(&mut via, BusPhase::Phi1),
        "both flags in the PHI1 of cycle N+2"
    );

    let mut held = Via6522::new();
    held.write(0x04, 0);
    held.write(0x05, 0);
    held.tick_phase(BusPhase::Phi2);
    held.write(0x05, 0); // reload before the flag shows
    held.tick_phase(BusPhase::Phi1);
    assert_eq!(
        held.peek(0x0D) & 0x40,
        0,
        "a T1C-H write drops the held flag"
    );
}

#[test]
fn acia_counts_on_phi2_in_table_mode_and_through_both_phases_on_the_crystal() {
    let mut acia = Acia6551::new();
    acia.set_baud_period(4);
    acia.write(0, 0x41);
    acia.tick_phase(BusPhase::Phi1);
    assert_eq!(acia.tx_cycles_left(), 4);
    acia.tick_phase(BusPhase::Phi2);
    assert_eq!(acia.tx_cycles_left(), 3);

    let mut crystal = Acia6551::new();
    crystal.set_crystal(Some(921_600)); // one crystal period per half cycle
    crystal.write(3, 0x1F);
    crystal.write(0, 0x41);
    crystal.tick_phase(BusPhase::Phi1);
    crystal.tick_phase(BusPhase::Phi2);
    assert_eq!(crystal.tx_cycles_left(), 960 - 2);
}

#[test]
fn step_phase_clocks_the_acia_on_the_bus_before_each_half() {
    let mut bus = RamAcia::new(0xD000);
    bus.memory
        .load_at(0x8000, &[0x8D, 0x00, 0xD0, 0xEA, 0xEA, 0xEA]);
    bus.memory.load_at(0xFFFC, &[0x00, 0x80]);
    bus.acia.set_baud_period(3);
    let mut cpu = Cpu::new(bus);
    cpu.reset();
    for _ in 0..8 {
        cpu.step_phase().unwrap();
    }
    assert_eq!(cpu.bus.acia.tx_cycles_left(), 3, "STA landed in its PHI2");
    let mut left = Vec::new();
    for _ in 0..6 {
        cpu.step_phase().unwrap();
        left.push(cpu.bus.acia.tx_cycles_left());
    }
    assert_eq!(left, [3, 2, 2, 1, 1, 0]);
    assert_eq!(cpu.bus.acia.status() & TDRE, TDRE);
}

#[test]
fn external_16x_clock_times_a_frame_on_baud_nibble_zero() {
    let mut acia = Acia6551::new();
    acia.set_external_clock(true);
    acia.write(2, 0x0B);
    acia.write(3, 0x00);
    acia.write(0, 0x5A);
    assert_eq!(acia.tx_cycles_left(), frame_16x_clocks(0x00, 0x0B));
    for _ in 0..1000 {
        acia.tick();
    }
    assert_eq!(
        acia.status() & TDRE,
        0,
        "tick does not drive the external clock"
    );
    for _ in 1..160 {
        acia.clock_16x();
    }
    assert_eq!(acia.status() & TDRE, 0);
    acia.clock_16x();
    assert_eq!(acia.status() & TDRE, TDRE, "160 clocks for 8N1");

    let cpu = Cpu::new(FlatMemory::new());
    let json = MachineImage::capture_with(&cpu, &Via6522::new(), &acia).to_json();
    let mut back = Acia6551::new();
    MachineImage::from_json(&json)
        .unwrap()
        .restore_with(
            &mut Cpu::new(FlatMemory::new()),
            &mut Via6522::new(),
            &mut back,
        )
        .unwrap();
    back.write(0, 0x5B);
    assert_eq!(back.tx_cycles_left(), 160, "external clock restored");

    let mut internal = Acia6551::new();
    internal.write(0, 0x5A);
    assert_eq!(
        internal.tx_cycles_left(),
        0,
        "off: nibble 0 has no countdown"
    );
}

#[test]
fn break_cuts_the_byte_in_flight_and_holds_the_line_until_it_ends() {
    let mut acia = Acia6551::new();
    acia.write(2, 0x0B);
    acia.write(3, 0x1F);
    acia.set_baud_period(52);
    acia.write(0, 0x55);
    for _ in 0..10 {
        acia.tick();
    }
    acia.write(2, 0x0F);
    assert!(!acia.txd_high() && !acia.tx_pending(), "the byte is cut");
    acia.write(0, 0x66);
    for _ in 0..500 {
        acia.tick();
    }
    assert_eq!(acia.pop_tx(), None, "line held low");
    assert_eq!(acia.status() & TDRE, 0);
    acia.write(2, 0x0B);
    assert!(acia.txd_high());
    for _ in 0..52 {
        acia.tick();
    }
    assert_eq!(acia.status() & TDRE, TDRE);
    assert_eq!(acia.pop_tx(), Some(0x66), "0x55 never reached the line");
}

/// Not `Clone`: `step_phase` must work without copying the bus.
struct Counting {
    memory: FlatMemory,
    reads: u32,
    phases: Vec<BusPhase>,
}

impl MemoryBus for Counting {
    fn read(&mut self, addr: u16) -> u8 {
        self.reads += 1;
        self.memory.read(addr)
    }
    fn write(&mut self, addr: u16, value: u8) {
        self.memory.write(addr, value);
    }
    fn peek(&self, addr: u16) -> u8 {
        self.memory.peek(addr)
    }
    fn phase(&mut self, phase: BusPhase) {
        self.phases.push(phase);
    }
}

#[test]
fn step_phase_probes_through_peek_without_touching_or_copying_the_bus() {
    let mut memory = FlatMemory::new();
    memory.load_at(0x8000, &[0xA9, 0x42, 0x85, 0x10]);
    memory.load_at(0xFFFC, &[0x00, 0x80]);
    let mut cpu = Cpu::new(Counting {
        memory,
        reads: 0,
        phases: Vec::new(),
    });
    cpu.reset();
    let base = cpu.bus.reads;
    let phi1 = cpu.step_phase().unwrap();
    assert_eq!(
        (phi1.address, cpu.bus.reads),
        (0x8000, base),
        "PHI1 reads nothing"
    );
    let phi2 = cpu.step_phase().unwrap();
    assert_eq!((phi2.data, cpu.bus.reads), (Some(0xA9), base + 1));
    for _ in 0..8 {
        cpu.step_phase().unwrap();
    }
    assert_eq!((cpu.a, cpu.bus.memory.data[0x10]), (0x42, 0x42));
    assert_eq!(cpu.bus.phases.len(), 10);
    assert!(cpu
        .bus
        .phases
        .chunks(2)
        .all(|p| p == [BusPhase::Phi1, BusPhase::Phi2]));
}
