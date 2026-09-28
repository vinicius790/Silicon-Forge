//! VIA input latching (ACR bits 0/1) over every pin value, DDR and edge polarity,
//! and its survival through a machine snapshot.

use mos6502::via::Via6522;

#[test]
fn port_a_latches_on_the_active_ca1_edge() {
    for rising in [false, true] {
        let mut via = Via6522::new();
        via.write(0x03, 0x00); // all inputs
        via.write(0x0B, 0x01); // latch PA
        via.write(0x0C, u8::from(rising));
        via.set_ca1(!rising); // idle level before the active edge
        assert_eq!(via.read(0x01), 0, "nothing latched yet");
        for v in 0..=255u8 {
            via.set_port_a_in(v);
            via.set_ca1(rising); // active edge: capture v
            via.set_port_a_in(!v);
            assert_eq!(via.read(0x01), v, "{rising} {v:02X}");
            assert_eq!(via.peek(0x0F), v);
            via.set_ca1(!rising); // inactive edge: latch holds
            via.set_port_a_in(v ^ 0x55);
            assert_eq!(via.read(0x0F), v);
        }
        via.write(0x0B, 0x00);
        via.set_port_a_in(0x3C);
        assert_eq!(via.read(0x01), 0x3C, "latching off reads the pins");
    }
}

#[test]
fn port_b_latch_keeps_output_bits_live() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x02);
    for ddr in (0..=255u8).step_by(17) {
        via.write(0x02, ddr);
        for v in (0..=255u8).step_by(7) {
            via.write(0x00, !v);
            via.set_port_b_in(v);
            via.set_cb1(true);
            via.set_cb1(false); // falling edge (PCR bit 4 clear)
            via.set_port_b_in(0);
            let want = (!v & ddr) | (v & !ddr);
            assert_eq!(via.read(0x00), want, "ddr {ddr:02X} v {v:02X}");
        }
    }
    // PA latching does not affect port B and vice versa.
    let mut via = Via6522::new();
    via.write(0x0B, 0x01);
    via.set_port_b_in(0xA5);
    assert_eq!(via.read(0x00), 0xA5);
}

#[test]
fn latch_survives_state_round_trip() {
    let mut via = Via6522::new();
    via.write(0x0B, 0x03);
    via.set_port_a_in(0x81);
    via.set_port_b_in(0x42);
    via.set_ca1(false);
    via.set_cb1(false);
    via.set_port_a_in(0);
    via.set_port_b_in(0);
    let st = via.state();
    let mut other = Via6522::new();
    other.load_state(&st);
    assert_eq!((other.ira(), other.irb()), (0x81, 0x42));
}
