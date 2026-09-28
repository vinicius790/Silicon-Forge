//! 6551 register decoding over all 65 536 control/command pairs, checked against
//! the frame helpers and the live chip.

use mos6502::acia::{
    data_bits, frame_16x_clocks, Acia6551, AciaConfig, Parity, TxControl, BAUD_RATES,
    CRYSTAL_DIVIDER_BY_NIBBLE,
};
use mos6502::bus::IoDevice;

#[test]
fn every_register_pair() {
    for control in 0..=255u8 {
        for command in 0..=255u8 {
            let c = AciaConfig::decode(control, command);
            assert_eq!(c.data_bits, 8 - ((control >> 5) & 3));
            assert_eq!(c.data_bits, data_bits(control));
            assert_eq!(
                8 * c.frame_halves(),
                frame_16x_clocks(control, command),
                "{control:02X} {command:02X}"
            );
            assert_eq!(c.parity == Parity::None, command & 0x20 == 0);
            assert_eq!(c.tx == TxControl::Break, command & 0x0C == 0x0C);
            assert_eq!(
                (c.dtr, c.rx_irq, c.echo),
                (command & 1 != 0, command & 2 == 0, command & 0x10 != 0)
            );
            let stop = match (control & 0x80 != 0, c.data_bits, c.parity != Parity::None) {
                (false, _, _) => 2,
                (true, 5, false) => 3,
                (true, 8, true) => 2,
                _ => 4,
            };
            assert_eq!(c.stop_halves, stop);
        }
        let c = AciaConfig::decode(control, 0);
        match c.baud() {
            None => assert_eq!(control & 0x0F, 0),
            Some(b) => {
                // 1.8432 MHz / 16 / divider, within the data sheet's rounding.
                let derived = 1_843_200.0
                    / 16.0
                    / f64::from(CRYSTAL_DIVIDER_BY_NIBBLE[usize::from(control & 0x0F)]);
                assert!(
                    (derived - b).abs() / b < 0.005,
                    "{control:02X}: {derived} vs {b}"
                );
            }
        }
    }
    assert_eq!(BAUD_RATES[15], 19200.0);
}

#[test]
fn live_chip_reports_its_config() {
    let mut a = Acia6551::new();
    a.write(3, 0x1E);
    a.write(2, 0x69);
    let c = a.config();
    assert_eq!(
        (c.baud(), c.rx_clock_internal, c.data_bits),
        (Some(9600.0), true, 8)
    );
    assert_eq!(
        (c.parity, c.tx, c.dtr, c.rx_irq),
        (Parity::Even, TxControl::IrqOffRtsLow, true, true)
    );
}
