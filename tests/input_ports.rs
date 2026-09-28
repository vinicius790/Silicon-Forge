//! `$4016`/`$4017`: strobe, 8-bit shift with open bus, two pads, the Zapper.

use mos6502::bus::MemoryBus;
use mos6502::input::{button, Controller, Zapper, OPEN_BUS_4016, ZAPPER_HOLD_LINES};
use mos6502::ppu::{Ppu, DOTS, WIDTH};
use mos6502::systems::nes2a03::NesBus;

#[test]
fn every_button_pattern_shifts_out_in_order_on_both_ports() {
    for b1 in 0..=255u8 {
        let b2 = b1.rotate_left(3) ^ 0x5A;
        let mut bus = NesBus::new();
        bus.pads[0].buttons = b1;
        bus.pads[1].buttons = b2;
        bus.write(0x4016, 1);
        bus.write(0x4016, 0);
        for (i, &mask) in button::ALL.iter().enumerate() {
            let r1 = bus.read(0x4016);
            let r2 = bus.read(0x4017);
            assert_eq!(
                r1,
                OPEN_BUS_4016 | u8::from(b1 & mask != 0),
                "pad 1 bit {i}"
            );
            assert_eq!(
                r2,
                OPEN_BUS_4016 | u8::from(b2 & mask != 0),
                "pad 2 bit {i}"
            );
        }
        for _ in 0..20 {
            assert_eq!(bus.read(0x4016), OPEN_BUS_4016 | 1, "after 8 reads: 1");
            assert_eq!(bus.read(0x4017), OPEN_BUS_4016 | 1);
        }
    }
}

#[test]
fn strobe_high_keeps_returning_a_and_follows_live_buttons() {
    let mut bus = NesBus::new();
    bus.write(0x4016, 1);
    for b in 0..=255u8 {
        bus.pads[0].buttons = b;
        assert_eq!(bus.read(0x4016) & 1, b & 1);
        assert_eq!(bus.read(0x4016) & 1, b & 1, "no shifting while high");
    }
}

#[test]
fn only_bit_0_of_4016_writes_is_the_strobe() {
    let mut bus = NesBus::new();
    bus.pads[0].buttons = button::B;
    bus.write(0x4016, 0xFE); // bit 0 clear
    assert!(!bus.strobe);
    bus.write(0x4016, 0x01);
    bus.write(0x4016, 0xFE);
    assert_eq!(bus.read(0x4016) & 1, 0);
    assert_eq!(bus.read(0x4016) & 1, 1, "B is second");
}

#[test]
fn ports_are_independent() {
    let mut bus = NesBus::new();
    bus.pads[0].buttons = 0xFF;
    bus.pads[1].buttons = 0x00;
    bus.write(0x4016, 1);
    bus.write(0x4016, 0);
    for _ in 0..3 {
        bus.read(0x4016);
    }
    assert_eq!(bus.read(0x4017) & 1, 0, "port 2 still at A");
    assert_eq!(
        bus.pads[1].shift, 0x80,
        "one shift in from the high serial input"
    );
}

#[test]
fn peek_does_not_clock() {
    let mut bus = NesBus::new();
    bus.pads[0].buttons = button::A;
    bus.write(0x4016, 1);
    bus.write(0x4016, 0);
    for _ in 0..5 {
        assert_eq!(bus.peek(0x4016), OPEN_BUS_4016 | 1);
    }
    assert_eq!(bus.read(0x4016) & 1, 1);
    assert_eq!(bus.peek(0x4016) & 1, 0);
}

#[test]
fn releasing_buttons_after_latch_does_not_change_the_report() {
    let mut c = Controller::with_buttons(button::START | button::RIGHT);
    c.latch();
    c.buttons = 0;
    let bits: Vec<u8> = (0..8).map(|_| c.read(false)).collect();
    assert_eq!(bits, [0, 0, 0, 1, 0, 0, 0, 1]);
}

fn ppu_at(line: u16, dot: u16, pixel: (usize, usize), index: u8) -> Ppu {
    let mut p = Ppu::new();
    p.framebuffer[pixel.1 * WIDTH + pixel.0] = index;
    p.line = line;
    p.dot = dot;
    p
}

#[test]
fn zapper_brightness_classes() {
    for i in 0..64u8 {
        let want = i >= 0x20 && i & 0x0F <= 0x0C;
        assert_eq!(Zapper::bright(i), want, "{i:02X}");
    }
}

#[test]
fn zapper_sees_light_only_after_the_beam_and_for_the_hold_window() {
    let z = Zapper {
        trigger: false,
        aim: Some((100, 50)),
    };
    let drawn = 50 * u32::from(DOTS) + 101;
    for (offset, lit) in [
        (-1i64, false),
        (0, false),
        (1, true),
        (i64::from(ZAPPER_HOLD_LINES * u32::from(DOTS)), true),
        (i64::from(ZAPPER_HOLD_LINES * u32::from(DOTS)) + 1, false),
    ] {
        let now = (i64::from(drawn) + offset) as u32;
        let p = ppu_at(
            (now / u32::from(DOTS)) as u16,
            (now % u32::from(DOTS)) as u16,
            (100, 50),
            0x30,
        );
        assert_eq!(z.light(&p), lit, "offset {offset}");
        assert_eq!(z.report(&p) & 0x08 == 0, lit, "D3 is 0 when lit");
    }
    let p = ppu_at(60, 0, (100, 50), 0x0F);
    assert!(!z.light(&p), "black never lights");
    let off = Zapper {
        trigger: false,
        aim: None,
    };
    assert!(!off.light(&ppu_at(60, 0, (100, 50), 0x30)));
    let outside = Zapper {
        trigger: false,
        aim: Some((256, 10)),
    };
    assert!(!outside.light(&ppu_at(60, 0, (0, 0), 0x30)));
}

#[test]
fn zapper_in_port_2_reports_trigger_and_light_without_shifting() {
    let mut bus = NesBus::new();
    bus.zapper = Some(Zapper {
        trigger: true,
        aim: Some((10, 10)),
    });
    bus.pads[1].buttons = 0xFF;
    bus.write(0x4016, 1);
    bus.write(0x4016, 0);
    for _ in 0..10 {
        assert_eq!(
            bus.read(0x4017),
            OPEN_BUS_4016 | 0x10 | 0x08,
            "dark, pulled, D0 = 0"
        );
    }
    bus.ppu.framebuffer[10 * WIDTH + 10] = 0x20;
    bus.ppu.line = 15;
    assert_eq!(bus.read(0x4017), OPEN_BUS_4016 | 0x10, "light seen");
    assert_eq!(bus.peek(0x4017), OPEN_BUS_4016 | 0x10);
    bus.zapper.as_mut().unwrap().trigger = false;
    assert_eq!(bus.read(0x4017), OPEN_BUS_4016);
    assert_eq!(bus.read(0x4016) & 1, 0, "port 1 pad unaffected");
}
