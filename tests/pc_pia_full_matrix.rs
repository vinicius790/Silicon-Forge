//! 6821 PIA: every control value of both sides against an independent model of the
//! data sheet, over a script of C1/C2 edges, data and DDR accesses and E cycles.

use mos6502::bus::IoDevice;
use mos6502::Pia6821;

#[derive(Clone, Copy, Debug)]
enum Ev {
    C1(bool),
    C2(bool),
    ReadData,
    WriteData,
    Tick,
}

struct Model {
    cr: u8,
    f1: bool,
    f2: bool,
    c1: bool,
    c2_in: bool,
    c2_out: bool,
    pulse: bool,
}

impl Model {
    fn new(cr: u8) -> Self {
        let mut m = Self {
            cr,
            f1: false,
            f2: false,
            c1: true,
            c2_in: true,
            c2_out: true,
            pulse: false,
        };
        if cr & 0x30 == 0x30 {
            m.c2_out = cr & 0x08 != 0;
        }
        m
    }
    fn output(&self) -> bool {
        self.cr & 0x20 != 0
    }
    fn irq(&self) -> bool {
        (self.f1 && self.cr & 0x01 != 0) || (self.f2 && self.cr & 0x08 != 0 && !self.output())
    }
    fn c2(&self) -> bool {
        if self.output() {
            self.c2_out
        } else {
            self.c2_in
        }
    }
    fn step(&mut self, ev: Ev, side_a: bool) {
        let data = self.cr & 0x04 != 0;
        let strobe = self.cr & 0x30 == 0x20;
        match ev {
            Ev::C1(h) => {
                if h != self.c1 && h == (self.cr & 0x02 != 0) {
                    self.f1 = true;
                    if strobe && self.cr & 0x08 == 0 {
                        self.c2_out = true;
                    }
                }
                self.c1 = h;
            }
            Ev::C2(h) => {
                if !self.output() && h != self.c2_in && h == (self.cr & 0x10 != 0) {
                    self.f2 = true;
                }
                self.c2_in = h;
            }
            Ev::ReadData => {
                if data {
                    self.f1 = false;
                    self.f2 = false;
                    if side_a && strobe {
                        self.c2_out = false;
                        self.pulse = self.cr & 0x08 != 0;
                    }
                }
            }
            Ev::WriteData => {
                if data && !side_a && strobe {
                    self.c2_out = false;
                    self.pulse = self.cr & 0x08 != 0;
                }
            }
            Ev::Tick => {
                if std::mem::take(&mut self.pulse) {
                    self.c2_out = true;
                }
            }
        }
    }
}

const SCRIPT: [Ev; 22] = [
    Ev::C1(false),
    Ev::C1(true),
    Ev::C2(false),
    Ev::C2(true),
    Ev::ReadData,
    Ev::WriteData,
    Ev::Tick,
    Ev::C1(false),
    Ev::ReadData,
    Ev::C1(true),
    Ev::WriteData,
    Ev::C1(false),
    Ev::C1(true),
    Ev::Tick,
    Ev::Tick,
    Ev::C2(false),
    Ev::ReadData,
    Ev::C2(true),
    Ev::C2(true),
    Ev::C1(true),
    Ev::WriteData,
    Ev::Tick,
];

fn apply(p: &mut Pia6821, side_a: bool, ev: Ev) {
    let base = if side_a { 0 } else { 2 };
    match ev {
        Ev::C1(h) if side_a => p.set_ca1(h),
        Ev::C1(h) => p.set_cb1(h),
        Ev::C2(h) if side_a => p.set_ca2(h),
        Ev::C2(h) => p.set_cb2(h),
        Ev::ReadData => {
            p.read(base);
        }
        Ev::WriteData => p.write(base, 0x5A),
        Ev::Tick => p.tick(),
    }
}

#[test]
fn every_control_value_on_both_sides_matches_the_model_after_every_event() {
    for side_a in [true, false] {
        for cr in 0..64u8 {
            for rot in 0..SCRIPT.len() {
                let mut pia = Pia6821::new();
                pia.write(if side_a { 1 } else { 3 }, cr);
                let mut m = Model::new(cr);
                for i in 0..SCRIPT.len() {
                    let ev = SCRIPT[(i + rot) % SCRIPT.len()];
                    apply(&mut pia, side_a, ev);
                    m.step(ev, side_a);
                    let reg = pia.peek(if side_a { 1 } else { 3 });
                    let ctx = format!(
                        "side {} cr {cr:02X} rot {rot} ev {i} {ev:?}",
                        if side_a { 'A' } else { 'B' }
                    );
                    assert_eq!(reg & 0x3F, cr, "{ctx}: control bits");
                    assert_eq!(reg & 0x80 != 0, m.f1, "{ctx}: IRQx1");
                    assert_eq!(reg & 0x40 != 0, m.f2, "{ctx}: IRQx2");
                    let irq = if side_a { pia.irq_a() } else { pia.irq_b() };
                    assert_eq!(irq, m.irq(), "{ctx}: IRQ pin");
                    let c2 = if side_a { pia.ca2() } else { pia.cb2() };
                    assert_eq!(c2, m.c2(), "{ctx}: C2 level");
                }
            }
        }
    }
}

#[test]
fn enabling_an_interrupt_with_the_flag_already_set_asserts_irq_at_once() {
    for side_a in [true, false] {
        let reg = if side_a { 1 } else { 3 };
        let mut pia = Pia6821::new();
        pia.write(reg, 0x04); // C1 falling, IRQ off
        apply(&mut pia, side_a, Ev::C1(false));
        assert!(!pia.irq_asserted());
        pia.write(reg, 0x05);
        assert!(pia.irq_asserted(), "flag kept across the CR write");
        assert_eq!(pia.irq_a(), side_a);
    }
}

#[test]
fn ddr_and_output_register_over_all_values() {
    for v in 0..=255u8 {
        let mut pia = Pia6821::new();
        pia.write(0, v); // DDRA
        pia.write(2, !v); // DDRB
        assert_eq!((pia.peek(0), pia.peek(2)), (v, !v));
        pia.write(1, 0x04);
        pia.write(3, 0x04);
        pia.write(0, 0xFF);
        pia.write(2, 0xFF);
        pia.set_port_a_in(0x00);
        pia.set_port_b_in(0x00);
        assert_eq!(pia.read(0), v, "A: outputs high, inputs from pins");
        assert_eq!(pia.read(2), !v, "B: outputs read the output register");
        assert_eq!(pia.port_a_out() & v, v);
    }
}

#[test]
fn reading_the_ddr_does_not_clear_flags() {
    let mut pia = Pia6821::new();
    pia.write(1, 0x01);
    pia.set_ca1(false);
    pia.read(0); // DDR selected
    assert!(pia.irq_a());
    pia.write(1, 0x05);
    pia.read(0);
    assert!(!pia.irq_a());
}
