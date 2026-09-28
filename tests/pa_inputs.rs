//! Four Score, Arkanoid (NES) and Power Pad on the controller ports, through the
//! devices and through the machine's `$4016`/`$4017`, plus their snapshot fields.

use mos6502::bus::MemoryBus;
use mos6502::input::{
    ArkanoidPaddle, Controller, FourScore, PowerPad, FOUR_SCORE_SIGNATURE, POWER_PAD_D3,
    POWER_PAD_D4,
};
use mos6502::systems::nes2a03::Nes2A03;

fn machine() -> Nes2A03 {
    Nes2A03::from_source(".org $8000\nr: jmp r\n.org $FFFC\n.word r\n.word r\n").unwrap()
}

fn strobe(nes: &mut Nes2A03) {
    nes.cpu.bus.write(0x4016, 1);
    nes.cpu.bus.write(0x4016, 0);
}

/// The documented 24-bit Four Score report of one port.
fn four_score_report(own: u8, extra: u8, port: usize) -> Vec<u8> {
    let mut v: Vec<u8> = (0..8).map(|i| (own >> i) & 1).collect();
    v.extend((0..8).map(|i| (extra >> i) & 1));
    v.extend((0..8).map(|i| u8::from(i == if port == 0 { 3 } else { 2 })));
    v.extend([1; 8]);
    v
}

#[test]
fn four_score_reports_every_pattern() {
    assert_eq!(FOUR_SCORE_SIGNATURE, [0x08, 0x04]);
    for seed in 0..64u32 {
        let b = |k: u32| (seed.wrapping_mul(2654435761).rotate_left(k * 8) & 0xFF) as u8;
        let mut nes = machine();
        nes.cpu.bus.four_score = Some(FourScore {
            extra: [
                Controller::with_buttons(b(2)),
                Controller::with_buttons(b(3)),
            ],
            count: [0; 2],
        });
        nes.cpu.bus.pads = [
            Controller::with_buttons(b(0)),
            Controller::with_buttons(b(1)),
        ];
        strobe(&mut nes);
        let p1: Vec<u8> = (0..32).map(|_| nes.cpu.bus.read(0x4016) & 1).collect();
        let p2: Vec<u8> = (0..32).map(|_| nes.cpu.bus.read(0x4017) & 1).collect();
        assert_eq!(p1, four_score_report(b(0), b(2), 0), "seed {seed}");
        assert_eq!(p2, four_score_report(b(1), b(3), 1), "seed {seed}");
        // Re-strobing restarts both sequences.
        strobe(&mut nes);
        assert_eq!(nes.cpu.bus.read(0x4016) & 1, b(0) & 1);
        assert_eq!(nes.cpu.bus.read(0x4016) & 1, (b(0) >> 1) & 1);
        assert_eq!(
            nes.cpu.bus.read(0x4016) & 0xE0,
            0x40,
            "open bus on the top bits"
        );
    }
}

#[test]
fn four_score_strobe_high_returns_button_a() {
    let mut nes = machine();
    nes.cpu.bus.four_score = Some(FourScore::default());
    nes.cpu.bus.pads[0] = Controller::with_buttons(1);
    nes.cpu.bus.write(0x4016, 1);
    for _ in 0..40 {
        assert_eq!(nes.cpu.bus.read(0x4016) & 1, 1);
    }
    assert_eq!(nes.cpu.bus.four_score.unwrap().count, [0, 0]);
}

#[test]
fn arkanoid_knob_is_nine_inverted_bits_msb_first() {
    for pos in [0u16, 1, 0x0D, 0xAD, 0x100, 0x155, 0x1AA, 0x1FF] {
        for fire in [false, true] {
            let mut nes = machine();
            nes.cpu.bus.paddle = Some(ArkanoidPaddle {
                position: pos,
                fire,
                ..Default::default()
            });
            strobe(&mut nes);
            let mut got = vec![];
            for _ in 0..14 {
                let v = nes.cpu.bus.read(0x4017);
                assert_eq!(v & 0x08 != 0, fire, "fire on D3");
                assert_eq!(v & 0x07, 0);
                got.push(((v >> 4) & 1) ^ 1);
            }
            let mut want: Vec<u8> = (0..9).rev().map(|i| ((pos >> i) & 1) as u8).collect();
            want.extend([(pos & 1) as u8; 5]);
            assert_eq!(got, want, "{pos:03X}");
        }
    }
    // The button is not latched: it follows the switch at any time.
    let mut nes = machine();
    nes.cpu.bus.paddle = Some(ArkanoidPaddle::default());
    strobe(&mut nes);
    nes.cpu.bus.paddle.as_mut().unwrap().fire = true;
    assert_ne!(nes.cpu.bus.read(0x4017) & 0x08, 0);
    // A new position is only seen after the next conversion.
    nes.cpu.bus.paddle.as_mut().unwrap().position = 0x1FF;
    assert_eq!(
        nes.cpu.bus.peek(0x4017) & 0x10,
        0x10,
        "old value (0) still shifting"
    );
    strobe(&mut nes);
    assert_eq!(nes.cpu.bus.peek(0x4017) & 0x10, 0);
    // Port 1 still holds the standard controller.
    nes.cpu.bus.pads[0] = Controller::with_buttons(1);
    strobe(&mut nes);
    assert_eq!(nes.cpu.bus.read(0x4016) & 1, 1);
}

#[test]
fn power_pad_shift_order_every_single_sensor() {
    assert_eq!(POWER_PAD_D3, [2, 1, 5, 9, 6, 10, 11, 7]);
    assert_eq!(POWER_PAD_D4, [4, 3, 12, 8]);
    for sensor in 0..=12u16 {
        let buttons = if sensor == 0 { 0 } else { 1 << (sensor - 1) };
        let mut nes = machine();
        nes.cpu.bus.power_pad = Some(PowerPad {
            buttons,
            ..Default::default()
        });
        strobe(&mut nes);
        let reads: Vec<u8> = (0..12).map(|_| nes.cpu.bus.read(0x4017)).collect();
        for (i, v) in reads.iter().enumerate() {
            let d3 = (v >> 3) & 1;
            let d4 = (v >> 4) & 1;
            let want3 = if i < 8 {
                u8::from(u16::from(POWER_PAD_D3[i]) == sensor)
            } else {
                1
            };
            let want4 = if i < 4 {
                u8::from(u16::from(POWER_PAD_D4[i]) == sensor)
            } else {
                1
            };
            assert_eq!((d3, d4), (want3, want4), "sensor {sensor} read {i}");
        }
    }
    let mut p = PowerPad {
        buttons: 0xFFF,
        ..Default::default()
    };
    p.latch();
    for _ in 0..20 {
        assert_eq!(p.read(false), 0x18, "all pressed, then 1s");
    }
}

#[test]
fn devices_in_snapshots() {
    let mut a = machine();
    a.cpu.bus.four_score = Some(FourScore {
        extra: [
            Controller::with_buttons(0x5A),
            Controller::with_buttons(0xC3),
        ],
        count: [0; 2],
    });
    a.cpu.bus.paddle = Some(ArkanoidPaddle {
        position: 0x12C,
        fire: true,
        ..Default::default()
    });
    strobe(&mut a);
    for _ in 0..11 {
        a.cpu.bus.read(0x4016);
    }
    for _ in 0..3 {
        a.cpu.bus.read(0x4017);
    }
    let s = a.save_state();
    let mut b = machine();
    b.cpu.bus.four_score = Some(FourScore::default());
    b.cpu.bus.paddle = Some(ArkanoidPaddle::default());
    b.load_state(&s).unwrap();
    assert_eq!(b.cpu.bus.four_score, a.cpu.bus.four_score);
    assert_eq!(b.cpu.bus.paddle, a.cpu.bus.paddle);
    for port in [0x4016u16, 0x4017] {
        for _ in 0..30 {
            assert_eq!(a.cpu.bus.read(port), b.cpu.bus.read(port));
        }
    }
    // A snapshot naming other devices is refused and leaves the machine alone.
    let mut c = machine();
    let before = c.save_state();
    assert!(c.load_state(&s).is_err());
    assert_eq!(c.save_state(), before);
    let mut p = machine();
    p.cpu.bus.power_pad = Some(PowerPad {
        buttons: 0x801,
        ..Default::default()
    });
    strobe(&mut p);
    p.cpu.bus.read(0x4017);
    let ps = p.save_state();
    let mut q = machine();
    q.cpu.bus.power_pad = Some(PowerPad::default());
    q.load_state(&ps).unwrap();
    assert_eq!(q.cpu.bus.power_pad, p.cpu.bus.power_pad);
    // Version 6 has no device fields and still loads.
    let v6 = a.save_state_version(6);
    let mut d = machine();
    d.cpu.bus.four_score = Some(FourScore::default());
    d.cpu.bus.paddle = Some(ArkanoidPaddle::default());
    d.load_state(&v6).unwrap();
}
