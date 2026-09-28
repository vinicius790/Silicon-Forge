//! Standard controllers on $4016/$4017, read by real 6502 code.

use mos6502::bus::MemoryBus;
use mos6502::systems::nes2a03::{button, Controller, Nes2A03, OPEN_BUS_4016};

const READER: &str = "
.org $8000
reset: lda #1
 sta $4016
 lda #0
 sta $4016
 ldx #0
loop: lda $4016
 sta $0300,x
 lda $4017
 sta $0310,x
 inx
 cpx #10
 bne loop
 lda #1
 sta $4016
 lda $4016
 sta $0320
 lda $4016
 sta $0321
done: jmp done
.org $FFFC
.word reset
";

#[test]
fn every_button_combination_on_both_ports() {
    for buttons in 0..=255u8 {
        let mut nes = Nes2A03::from_source(READER).unwrap();
        nes.cpu.bus.pads[0].buttons = buttons;
        nes.cpu.bus.pads[1].buttons = !buttons;
        nes.run_cycles(800).unwrap();
        for i in 0..10u16 {
            let bit = |b: u8| if i < 8 { (b >> i) & 1 } else { 1 };
            assert_eq!(
                nes.cpu.bus.peek(0x0300 + i),
                OPEN_BUS_4016 | bit(buttons),
                "{buttons:02X} {i}"
            );
            assert_eq!(nes.cpu.bus.peek(0x0310 + i), OPEN_BUS_4016 | bit(!buttons));
        }
        // Strobe high: every read is A, and stays A.
        let a = OPEN_BUS_4016 | (buttons & button::A);
        assert_eq!((nes.cpu.bus.peek(0x0320), nes.cpu.bus.peek(0x0321)), (a, a));
    }
}

#[test]
fn shift_register_semantics() {
    let mut pad = Controller {
        buttons: button::START | button::RIGHT,
        shift: 0,
    };
    pad.latch();
    let bits: Vec<u8> = (0..12).map(|_| pad.read(false)).collect();
    assert_eq!(bits, [0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 1, 1]);
    // Changing buttons after the latch does not affect the report until relatched.
    pad.buttons = button::A;
    pad.latch();
    pad.buttons = 0;
    assert_eq!(pad.read(false), 1);
    // During strobe the live A button is reported.
    pad.buttons = button::A;
    assert_eq!(pad.read(true), 1);
    pad.buttons = 0;
    assert_eq!(pad.read(true), 0);
    // Peek has no side effects.
    let mut nes = Nes2A03::from_source(READER).unwrap();
    nes.cpu.bus.pads[0].buttons = 0xFF;
    nes.cpu.bus.write(0x4016, 1);
    nes.cpu.bus.write(0x4016, 0);
    for _ in 0..5 {
        assert_eq!(nes.cpu.bus.peek(0x4016), OPEN_BUS_4016 | 1);
    }
    assert_eq!(nes.cpu.bus.pads[0].shift, 0xFF);
}

#[test]
fn controllers_are_in_the_save_state_and_v2_still_loads() {
    let mut a = Nes2A03::from_source(READER).unwrap();
    a.cpu.bus.pads[0].buttons = 0x5A;
    a.cpu.bus.pads[1].buttons = 0xC3;
    a.run_cycles(300).unwrap();
    let s = a.save_state();
    let mut b = Nes2A03::from_source(READER).unwrap();
    b.load_state(&s).unwrap();
    assert_eq!(
        (b.cpu.bus.pads, b.cpu.bus.strobe),
        (a.cpu.bus.pads, a.cpu.bus.strobe)
    );
    a.run_cycles(500).unwrap();
    b.run_cycles(500).unwrap();
    assert_eq!(a.save_state(), b.save_state());
    let v2 = a.save_state_version(2);
    let mut c = Nes2A03::from_source(READER).unwrap();
    c.cpu.bus.pads[0].buttons = 9;
    c.load_state(&v2).unwrap();
    assert_eq!(c.cpu.bus.pads, [Controller::default(); 2]);
    assert_eq!(v2.len() + 5, a.save_state_version(3).len());
}
