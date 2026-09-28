//! 6522 T1 N+1.5 / N+2 timing and T2-rate shift modes; 6551 DTR, modem lines, echo.

use mos6502::acia::{Acia6551, IRQ, RDRF, TDRE};
use mos6502::bus::IoDevice;
use mos6502::via::Via6522;

const DCD: u8 = 1 << 5;
const DSR: u8 = 1 << 6;

/// Ticks until IFR bit 6 sets, for T1 loaded with `n`.
fn ticks_to_t1_flag(via: &mut Via6522) -> u32 {
    (1..=0x1_0010)
        .find(|_| {
            via.tick();
            via.peek(0x0D) & 0x40 != 0
        })
        .unwrap()
}

#[test]
fn t1_flags_n_plus_one_ticks_after_load_and_free_run_period_is_n_plus_two() {
    let mut once = Via6522::new();
    once.write(0x04, 3);
    once.write(0x05, 0);
    assert_eq!(
        ticks_to_t1_flag(&mut once),
        4,
        "N+1 ticks, N+1.5 with the write"
    );
    assert_eq!(once.t1_counter(), 0xFFFF);

    let mut free = Via6522::new();
    free.write(0x0B, 0x40);
    free.write(0x04, 3);
    free.write(0x05, 0);
    assert_eq!(ticks_to_t1_flag(&mut free), 4);
    free.write(0x0D, 0x40);
    assert_eq!(ticks_to_t1_flag(&mut free), 5, "N+2 per free-run period");
    free.write(0x0D, 0x40);
    assert_eq!(ticks_to_t1_flag(&mut free), 5);
}

fn t2_shift(mode: u8, sr: u8) -> Via6522 {
    let mut via = Via6522::new();
    via.write(0x08, 1); // T2 low latch N = 1: CB1 toggles every N+2 = 3 ticks
    via.write(0x0B, mode << 2);
    via.write(0x0A, sr);
    via
}

#[test]
fn shift_out_under_t2_takes_two_n_plus_two_per_bit_and_flags_after_eight() {
    let mut via = t2_shift(0b101, 0xA5);
    via.tick();
    via.tick();
    assert!(via.sr_clock_high());
    via.tick();
    assert!(!via.sr_clock_high(), "CB1 falls after N+2 ticks");
    for _ in 3..45 {
        via.tick();
    }
    assert_eq!(
        via.sr_bits_left(),
        0,
        "eighth bit out at 15(N+2) = 45 ticks"
    );
    assert_eq!(via.sr_shifted_out(), 0xA5);
    for _ in 45..47 {
        via.tick();
    }
    assert_eq!(via.peek(0x0D) & 0x04, 0, "IFR2 waits for CB1 to rise");
    via.tick();
    assert_eq!(via.peek(0x0D) & 0x04, 0x04, "IFR2 at 16(N+2) = 48 ticks");
    assert!(via.sr_clock_high());
}

#[test]
fn shift_in_under_t2_samples_on_rising_edges_and_an_sr_read_clears_and_restarts() {
    let mut via = t2_shift(0b001, 0x00);
    via.set_sr_in_bit(true);
    for _ in 0..47 {
        via.tick();
    }
    assert_eq!(via.peek(0x0D) & 0x04, 0);
    via.tick();
    assert_eq!(via.peek(0x0D) & 0x04, 0x04);
    assert_eq!(via.read(0x0A), 0xFF);
    assert_eq!(via.peek(0x0D) & 0x04, 0, "SR read clears IFR bit 2");
    via.set_sr_in_bit(false);
    for _ in 0..48 {
        via.tick();
    }
    assert_eq!(via.peek(0x0A), 0x00, "the read started eight more bits");
}

#[test]
fn free_running_shift_out_recirculates_and_never_flags() {
    let mut via = t2_shift(0b100, 0x81);
    for _ in 0..45 {
        via.tick();
    }
    assert_eq!(via.sr_shifted_out(), 0x81);
    assert_eq!(via.peek(0x0A), 0x81, "eight rotations restore the byte");
    for _ in 0..48 {
        via.tick();
    }
    assert_eq!(via.peek(0x0D) & 0x04, 0);
    assert_eq!(via.peek(0x0A), 0x81);
}

#[test]
fn acia_dtr_off_or_carrier_lost_disables_the_receiver() {
    let mut acia = Acia6551::new();
    acia.push_rx(0x41);
    assert_eq!(acia.status() & RDRF, 0, "reset leaves DTR off");
    acia.write(2, 0x09); // DTR on, receiver IRQ on
    acia.set_dcd(true);
    assert!(acia.irq_asserted(), "a DCD change interrupts");
    let status = acia.read(1);
    assert_eq!(status & (DCD | IRQ), DCD | IRQ);
    acia.push_rx(0x42);
    assert_eq!(acia.status() & RDRF, 0, "no carrier, no byte");
    acia.set_dcd(false);
    acia.set_dsr(true);
    assert_eq!(acia.read(1) & (DCD | DSR | IRQ), DSR | IRQ);
    acia.push_rx(0x43);
    assert_eq!(acia.read(0), 0x43);
}

#[test]
fn acia_cts_high_holds_the_transmitter() {
    let mut acia = Acia6551::new();
    acia.write(3, 0x1F);
    acia.set_cts(true);
    acia.write(0, 0x55);
    for _ in 0..acia.baud_period() * 2 {
        acia.tick();
    }
    assert_eq!(acia.status() & TDRE, 0);
    assert_eq!(acia.pop_tx(), None);
    acia.set_cts(false);
    for _ in 0..acia.baud_period() {
        acia.tick();
    }
    assert_eq!(acia.status() & TDRE, TDRE);
    assert_eq!(acia.pop_tx(), Some(0x55));
}

#[test]
fn acia_echo_mode_retransmits_received_bytes() {
    let mut acia = Acia6551::new();
    acia.write(2, 0x11); // DTR on, echo, transmitter control 00
    acia.push_rx(0x7E);
    assert_eq!(acia.pop_echo(), Some(0x7E));
    assert_eq!(acia.read(0), 0x7E, "the CPU still receives it");
    acia.write(2, 0x15); // echo bit with transmitter control 01: no echo
    acia.push_rx(0x7F);
    assert_eq!(acia.pop_echo(), None);
}
