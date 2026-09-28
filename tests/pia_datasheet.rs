//! 6821 control register behavior over all CR values and the C2 modes.

use mos6502::bus::IoDevice;
use mos6502::Pia6821;

#[test]
fn every_control_value_keeps_flags_read_only_and_selects_ddr_or_data() {
    for cr in 0..=255u8 {
        let mut pia = Pia6821::new();
        pia.write(0, 0x0F); // DDR A while bit 2 = 0
        pia.write(1, cr);
        assert_eq!(pia.peek(1), cr & 0x3F, "flags never written");
        if cr & 0x04 == 0 {
            assert_eq!(pia.read(0), 0x0F);
        } else {
            pia.write(0, 0xA5);
            pia.set_port_a_in(0x30);
            assert_eq!(
                pia.read(0),
                0x05 | 0x30,
                "outputs from OR, inputs from pins"
            );
        }
    }
}

#[test]
fn c1_edges_and_irq_enable_for_every_combination() {
    for cr in [0x00u8, 0x01, 0x02, 0x03] {
        for (from, to) in [(true, false), (false, true)] {
            let mut pia = Pia6821::new();
            pia.write(3, cr | 0x04);
            pia.set_cb1(from);
            pia.read(2); // clear whatever the setup edge flagged
            pia.set_cb1(to);
            let active = to == (cr & 0x02 != 0);
            assert_eq!(pia.peek(3) & 0x80 != 0, active, "cr {cr:02X} {from}->{to}");
            assert_eq!(pia.irq_b(), active && cr & 1 != 0);
            pia.read(2);
            assert_eq!(
                (pia.peek(3) & 0xC0, pia.irq_b()),
                (0, false),
                "data read clears"
            );
        }
    }
}

#[test]
fn c2_input_flag_and_its_enable() {
    let mut pia = Pia6821::new();
    pia.write(1, 0x04 | 0x10 | 0x08); // input, rising edge, IRQ enabled
    pia.set_ca2(false);
    assert!(!pia.irq_a());
    pia.set_ca2(true);
    assert!(pia.irq_a() && pia.peek(1) & 0x40 != 0);
    pia.write(1, 0x04 | 0x20 | 0x10); // switch to output: flag 2 cleared
    assert_eq!(pia.peek(1) & 0x40, 0);
}

#[test]
fn c2_output_manual_handshake_and_pulse() {
    let mut pia = Pia6821::new();
    pia.write(1, 0x04 | 0x38); // manual, bit 3 = 1
    assert!(pia.ca2());
    pia.write(1, 0x04 | 0x30); // manual, bit 3 = 0
    assert!(!pia.ca2());

    pia.write(1, 0x04 | 0x20); // handshake on read, CA1 falling ends it
    pia.write(1, 0x04 | 0x38);
    pia.write(1, 0x04 | 0x20);
    pia.read(0);
    assert!(!pia.ca2(), "read of A drops CA2");
    pia.tick();
    assert!(!pia.ca2(), "handshake waits for CA1");
    pia.set_ca1(false);
    assert!(pia.ca2(), "active CA1 edge restores CA2");

    pia.write(3, 0x04 | 0x28); // B pulse mode: write of B drops CB2 for one cycle
    pia.read(2);
    assert!(pia.cb2(), "reads do not strobe side B");
    pia.write(2, 0x55);
    assert!(!pia.cb2());
    pia.tick();
    assert!(pia.cb2());
}
