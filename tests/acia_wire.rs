//! Two ACIAs wired TxD → RxD at the bit level.

use mos6502::acia::{data_bits, frame_levels, programmed_frame, Acia6551};
use mos6502::bus::IoDevice;

fn acia(control: u8, command: u8) -> Acia6551 {
    let mut a = Acia6551::new();
    a.write(3, control);
    a.write(2, command | 0x0B);
    a
}

#[test]
fn every_byte_every_format_crosses_the_wire() {
    for len in 0..4u8 {
        for stop2 in [0u8, 0x80] {
            for parity in [0x00u8, 0x20, 0x60, 0xA0, 0xE0] {
                let control = (len << 5) | stop2;
                let mut tx = acia(control, parity);
                let mut rx = acia(control, parity);
                for _ in 0..20 {
                    rx.rxd_16x(true);
                }
                let frame_len = frame_levels(
                    &programmed_frame(0, tx.peek(2), control),
                    data_bits(control),
                )
                .len();
                for byte in 0..=255u8 {
                    tx.write(0, byte);
                    let mut clocks = 0;
                    loop {
                        let level = tx.txd_16x();
                        rx.rxd_16x(level);
                        clocks += 1;
                        if !tx.tx_line_busy() {
                            break;
                        }
                    }
                    assert_eq!(clocks, frame_len, "one frame of 16x clocks");
                    for _ in 0..3 {
                        rx.rxd_16x(tx.txd_16x());
                    }
                    let mask = (1u16 << data_bits(control)) - 1;
                    assert_eq!(
                        rx.read(1) & 0x0B,
                        0x08,
                        "{control:02X} {parity:02X} {byte:02X}"
                    );
                    assert_eq!(u16::from(rx.read(0)), u16::from(byte) & mask);
                    assert!(tx.txd_16x(), "idle high");
                }
            }
        }
    }
}

#[test]
fn cts_holds_and_break_cuts_the_frame() {
    let mut a = acia(0x00, 0x00);
    a.set_cts(true);
    a.write(0, 0x55);
    assert!((0..100).all(|_| a.txd_16x()), "CTS high: line idle");
    assert!(a.tx_pending());
    a.set_cts(false);
    assert!(!a.txd_16x(), "start bit");
    assert!(!a.tx_pending() && a.tx_line_busy());
    let cmd = a.peek(2);
    a.write(2, cmd | 0x0C);
    assert!((0..50).all(|_| !a.txd_16x()), "break holds TxD low");
    assert!(!a.tx_line_busy(), "frame abandoned");
    a.write(2, cmd);
    assert!(a.txd_16x());
}

#[test]
fn state_round_trips_at_every_16x_clock_of_a_frame() {
    use mos6502::snapshot::MachineImage;
    use mos6502::via::Via6522;
    use mos6502::{Cpu, FlatMemory};
    for control in [0x00u8, 0x60, 0x80, 0xE0] {
        for parity in [0x00u8, 0x60] {
            let make = || {
                let mut tx = acia(control, parity);
                let mut rx = acia(control, parity);
                for _ in 0..20 {
                    rx.rxd_16x(true);
                }
                tx.write(0, 0xA7);
                (tx, rx)
            };
            let (mut t0, mut r0) = make();
            let mut wire = Vec::new();
            while {
                let l = t0.txd_16x();
                r0.rxd_16x(l);
                wire.push(l);
                t0.tx_line_busy()
            } {}
            for _ in 0..3 {
                r0.rxd_16x(t0.txd_16x());
            }
            let want = (r0.read(1), r0.read(0));
            for cut in 0..wire.len() {
                let (mut tx, mut rx) = make();
                for _ in 0..cut {
                    rx.rxd_16x(tx.txd_16x());
                }
                // Through the JSON snapshot, into fresh devices.
                let cpu = Cpu::new(FlatMemory::new());
                let via = Via6522::new();
                let json_tx = MachineImage::capture_with(&cpu, &via, &tx).to_json();
                let json_rx = MachineImage::capture_with(&cpu, &via, &rx).to_json();
                let (mut tx2, mut rx2) = (Acia6551::new(), Acia6551::new());
                let (mut c, mut v) = (Cpu::new(FlatMemory::new()), Via6522::new());
                MachineImage::from_json(&json_tx)
                    .unwrap()
                    .restore_with(&mut c, &mut v, &mut tx2)
                    .unwrap();
                MachineImage::from_json(&json_rx)
                    .unwrap()
                    .restore_with(&mut c, &mut v, &mut rx2)
                    .unwrap();
                assert_eq!(tx2.state(), tx.state(), "{control:02X} cut {cut}");
                assert_eq!(rx2.state(), rx.state());
                let mut rest = Vec::new();
                for _ in cut..wire.len() {
                    let l = tx2.txd_16x();
                    rest.push(l);
                    rx2.rxd_16x(l);
                }
                assert_eq!(rest, wire[cut..], "{control:02X} {parity:02X} cut {cut}");
                for _ in 0..3 {
                    rx2.rxd_16x(tx2.txd_16x());
                }
                assert_eq!(
                    (rx2.read(1), rx2.read(0)),
                    want,
                    "{control:02X} {parity:02X} cut {cut}"
                );
            }
        }
    }
}

#[test]
fn invalid_line_state_is_a_typed_error() {
    use mos6502::snapshot::{MachineImage, SnapshotError};
    use mos6502::via::Via6522;
    use mos6502::{Cpu, FlatMemory};
    let json = MachineImage::capture_with(
        &Cpu::new(FlatMemory::new()),
        &Via6522::new(),
        &Acia6551::new(),
    )
    .to_json();
    assert!(json.contains("\"tx_line\":\"\",\"rx_line\":\"0000000000010000\""));
    let bad = json.replace(
        "\"rx_line\":\"0000000000010000\"",
        "\"rx_line\":\"0300000000010000\"",
    );
    assert!(matches!(
        MachineImage::from_json(&bad),
        Err(SnapshotError::OutOfRange {
            kind: "rx_line",
            value: 3
        })
    ));
    let bad = json.replace("\"tx_line\":\"\"", "\"tx_line\":\"012\"");
    assert!(matches!(
        MachineImage::from_json(&bad),
        Err(SnapshotError::WrongType(_))
    ));
    let old = json.replace(",\"tx_line\":\"\",\"rx_line\":\"0000000000010000\"", "");
    assert!(
        MachineImage::from_json(&old).is_ok(),
        "older images load idle"
    );
}
