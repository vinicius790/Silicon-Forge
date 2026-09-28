//! 6551 frame format over every control and command value.

use mos6502::acia::{frame_16x_clocks, programmed_frame, stop_halves};

#[test]
fn every_control_and_command_pair_gives_a_consistent_frame() {
    for control in 0..=255u8 {
        for command in 0..=255u8 {
            let bits = 8 - u32::from((control >> 5) & 3);
            let parity = command & 0x20 != 0;
            let halves = stop_halves(control, command);
            let expected = match (control & 0x80 != 0, bits, parity) {
                (false, ..) => 2,
                (true, 5, false) => 3,
                (true, 8, true) => 2,
                _ => 4,
            };
            assert_eq!(halves, expected, "{control:02X}/{command:02X}");
            assert_eq!(
                frame_16x_clocks(control, command),
                16 * (1 + bits + u32::from(parity)) + 8 * u32::from(halves)
            );
            for data in [0x00u8, 0x5A, 0xA5, 0xFF] {
                let frame = programmed_frame(data, command, control);
                assert_eq!(u32::from(frame.data), u32::from(data) & ((1 << bits) - 1));
                assert_eq!(frame.stop_halves, halves);
                let ones = frame.data.count_ones();
                let want = parity.then_some(match command >> 6 {
                    0 => ones % 2 == 0,
                    1 => ones % 2 == 1,
                    2 => true,
                    _ => false,
                });
                assert_eq!(frame.parity, want);
                if let Some(p) = frame.parity {
                    let total = ones + u32::from(p);
                    match command >> 6 {
                        0 => assert_eq!(total % 2, 1, "odd parity"),
                        1 => assert_eq!(total % 2, 0, "even parity"),
                        _ => {}
                    }
                }
            }
        }
    }
}
