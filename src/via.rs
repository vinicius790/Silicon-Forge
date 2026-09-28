//! Timer 1 of a W65C22-style VIA, one-shot only.
//!
//! Free-run reload (ACR continuous mode) is not implemented. Each [`ViaTimer1::tick`]
//! is one PHI2. The host calls `tick`; `Cpu::step` does not.

/// One-shot Timer 1.
///
/// Writing the high counter byte copies the 16-bit latch into the counter, clears the
/// T1 flag, and starts the countdown. The counter decrements once per [`ViaTimer1::tick`].
/// The tick that makes it zero sets the T1 flag and stops. Later ticks do nothing until
/// the high byte is written again.
#[derive(Debug, Clone)]
pub struct ViaTimer1 {
    latch: u16,
    counter: u16,
    running: bool,
    t1_flag: bool,
    irq_enable: bool,
}

impl Default for ViaTimer1 {
    fn default() -> Self {
        Self::new()
    }
}

impl ViaTimer1 {
    pub fn new() -> Self {
        Self {
            latch: 0,
            counter: 0,
            running: false,
            t1_flag: false,
            irq_enable: false,
        }
    }

    pub fn counter(&self) -> u16 {
        self.counter
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// T1 interrupt flag, independent of the IER mask.
    pub fn timed_out(&self) -> bool {
        self.t1_flag
    }

    /// IRQ pin: the flag is set and T1 is enabled in the IER.
    pub fn irq_asserted(&self) -> bool {
        self.t1_flag && self.irq_enable
    }

    /// One PHI2. A running counter of N reaches zero, and raises the flag, on the Nth tick.
    pub fn tick(&mut self) {
        if !self.running {
            return;
        }
        self.counter = self.counter.wrapping_sub(1);
        if self.counter == 0 {
            self.t1_flag = true;
            self.running = false;
        }
    }

    /// Register offsets match the 6522: `$04`–`$07` timer, `$0D` IFR, `$0E` IER.
    pub fn read(&mut self, offset: u8) -> u8 {
        match offset & 0x0F {
            0x04 => {
                self.t1_flag = false;
                self.counter as u8
            }
            0x05 => (self.counter >> 8) as u8,
            0x06 => self.latch as u8,
            0x07 => (self.latch >> 8) as u8,
            0x0D => {
                let mut ifr = 0;
                if self.t1_flag {
                    ifr |= 1 << 6;
                }
                if self.irq_asserted() {
                    ifr |= 1 << 7;
                }
                ifr
            }
            0x0E => {
                let mut ier = 0x80;
                if self.irq_enable {
                    ier |= 1 << 6;
                }
                ier
            }
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u8, value: u8) {
        match offset & 0x0F {
            0x04 | 0x06 => {
                self.latch = (self.latch & 0xFF00) | u16::from(value);
            }
            0x07 => {
                self.latch = (self.latch & 0x00FF) | (u16::from(value) << 8);
            }
            0x05 => {
                self.latch = (self.latch & 0x00FF) | (u16::from(value) << 8);
                self.counter = self.latch;
                self.t1_flag = false;
                self.running = true;
            }
            0x0D => {
                if value & (1 << 6) != 0 {
                    self.t1_flag = false;
                }
            }
            0x0E => {
                if value & 0x80 != 0 {
                    if value & (1 << 6) != 0 {
                        self.irq_enable = true;
                    }
                } else if value & (1 << 6) != 0 {
                    self.irq_enable = false;
                }
            }
            _ => {}
        }
    }
}

/// Architectural state of [`Via6522`] for snapshot version 2.
///
/// This is the register file the model actually keeps. It is not a transistor dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViaState {
    pub ddra: u8,
    pub ddrb: u8,
    pub ora: u8,
    pub orb: u8,
    pub pa_in: u8,
    pub pb_in: u8,
    /// Driven input masks and open-drain loads. Older images: all driven, no load.
    pub pa_driven: u8,
    pub pb_driven: u8,
    pub pa_load_low: u8,
    pub pb_load_low: u8,
    pub t1_latch: u16,
    pub t1_counter: u16,
    pub t1_running: bool,
    /// Free-run reload owed on the next tick. Older images load it as false.
    pub t1_reload: bool,
    pub t1_flag: bool,
    pub t2_latch: u16,
    pub t2_counter: u16,
    pub t2_running: bool,
    pub t2_flag: bool,
    pub ifr_sr: bool,
    pub ca1_flag: bool,
    pub cb1_flag: bool,
    pub ca1_high: bool,
    pub cb1_high: bool,
    pub pb6_high: bool,
    pub pb7: bool,
    pub sr_in: bool,
    pub ier: u8,
    pub acr: u8,
    pub pcr: u8,
    pub sr: u8,
    pub sr_left: u8,
    pub sr_out: u8,
    /// Ticks left before the T2-rate shift clock toggles CB1. Older images load 0.
    pub sr_t2_left: u16,
    /// CB1 level driven by the T2-rate shift clock. Older images load high.
    pub sr_clock_high: bool,
    /// Timer flags rolled in PHI2 and shown on the next PHI1. Older images: false.
    pub t1_flag_due: bool,
    pub t2_flag_due: bool,
    /// Input latches (ACR bits 0/1). Older images load 0.
    pub ira_latch: u8,
    pub irb_latch: u8,
}

/// W65C22-style VIA: ports, both timers, IFR/IER, and one shift-register mode.
///
/// Not a full 6522. Deliberately absent: shift-register modes other than
/// shift-out (`ACR` `110`) and shift-in (`ACR` `010`) under PHI2, shift-in
/// under CB1 (`ACR` `011`), and shift-out under CB1 (`ACR` `111`). With ACR bit 7
/// set, timer 1 drives PB7 ([`Via6522::pb7_output`]) and the port-B pin read.
///
/// CA2 and CB2 come from the PCR only. ACR bits do not select these pins.
/// CA2 is PCR bits 3–1 and CB2 is PCR bits 7–5. The three-bit code is:
/// * `000` input handshake, negative edge. The edge sets IFR bit 0 (CA2) or
///   bit 3 (CB2) only when that IER bit is already set. A read or write of the
///   data port (`$01` / `$00`, not `$0F`) clears the flag.
/// * `001` independent negative-edge input. The flag stays until an IFR write.
/// * `010` input handshake, positive edge, cleared by the same port access.
/// * `011` independent positive-edge input.
/// * `100` output handshake. A read or write of the data port pulls the pin low.
///   A PHI2 [`Via6522::tick`] does not release it. The matching CA1/CB1 falling
///   edge does. This does not increment the pulse counter. CB2 (not CA2) also
///   sets IFR bit 3 when IER bit 3 is already set.
/// * `101` pulse output, distinct from handshake. A CPU write (not a read, and
///   not `$0F`) pulls the pin low, increments the pulse counter, and the next
///   [`Via6522::tick`] releases it.
/// * `110` holds the pin low. `111` holds it high.
///   Input modes do not drive the pin; [`Via6522::ca2`] reads the level last given
///   to [`Via6522::set_ca2`]. Each tick is one PHI2. `Cpu::step` does not call it.
///
/// CA1 and CB1: a high-to-low edge sets IFR bit 1 (CA1) or bit 4 (CB1) only when
/// the matching IER bit is already set. A disabled IER bit does not set IFR.
/// The pins idle high.
///
/// Timer 2 pulse mode (ACR bit 5) counts PB6 falling edges and does not also
/// decrement on PHI2. One-shot mode (bit 5 clear) still decrements on tick.
///
/// PB7 toggles on timer 1 underflow only when ACR bits 7 and 6 are both set
/// (free-run with PB7 enabled); that is the toggle flip-flop. The pin itself is
/// [`Via6522::pb7_output`], which also covers the one-shot pulse.
///
/// Timer model, same countdown as [`ViaTimer1`]: a counter loaded with N underflows
/// on the Nth tick (the tick that makes the counter zero). A latch of 0 does not
/// underflow on the next tick, because the counter wraps to `$FFFF`.
///
/// * Timer 1 with ACR bit 6 clear is one-shot: underflow sets IFR bit 6 and stops.
/// * Timer 1 with ACR bit 6 set is free-running: underflow sets IFR bit 6 and reloads
///   the counter from the latch without stopping. Each later reload sets bit 6 again
///   after the flag has been cleared. One-shot does not.
/// * Timer 2 one-shot (ACR bit 5 clear) decrements on [`Via6522::tick`]. With bit 5
///   set, only a PB6 falling edge ([`Via6522::set_pb6`]) decrements the counter.
///   A tick in pulse mode leaves the counter alone. Underflow sets IFR bit 5 and stops.
///
/// Shift register, ACR bits 4–2:
/// * `0b110` shift out under PHI2, MSB first. A write to SR loads the byte and the
///   next eight ticks shift it out. The eighth shift sets IFR bit 2.
/// * `0b010` shift in under PHI2, MSB first. A write to SR arms eight ticks. Each
///   tick shifts the register left and inserts the bit last given to
///   [`Via6522::set_sr_in_bit`] as the new LSB. The eighth tick sets IFR bit 2.
/// * `0b111` shift out under CB1, MSB first. A write to SR loads the byte. Each
///   falling CB1 edge shifts one bit onto CB2 (the datasheet gives CB2 valid after
///   the negative CB1 transition); a rising edge does not shift. The eighth pulse
///   sets IFR bit 2; later pulses keep shifting uncounted until an SR access.
///   A PHI2 [`Via6522::tick`] does not shift this mode.
/// * `0b011` shift in under CB1, MSB first. A write to SR arms eight rising
///   edges. Each rising edge shifts the register left and inserts the bit last
///   given to [`Via6522::set_sr_in_bit`] as the new LSB. Eight rising edges
///   assemble one byte and the eighth sets IFR bit 2; later rising edges keep
///   shifting uncounted until an SR access. A falling edge does not shift. A PHI2 [`Via6522::tick`] does not shift this mode.
///   Other ACR shift modes do not clock the register.
#[derive(Debug, Clone)]
pub struct Via6522 {
    ora: u8,
    orb: u8,
    ddra: u8,
    ddrb: u8,
    pa_in: u8,
    pb_in: u8,
    /// Input bits the host drives with `pa_in`/`pb_in`; the rest float to the pull-up.
    pa_driven: u8,
    pb_driven: u8,
    /// Open-drain loads sinking pins low (see [`Via6522::set_port_a_load_low`]).
    pa_load_low: u8,
    pb_load_low: u8,
    /// Port pins captured on the last active CA1/CB1 edge (ACR bits 0/1).
    ira_latch: u8,
    irb_latch: u8,
    t1_latch: u16,
    t1_counter: u16,
    t1_running: bool,
    /// Free-run: the next tick loads the latch instead of decrementing.
    t1_reload: bool,
    t1_flag: bool,
    t2_latch: u16,
    t2_counter: u16,
    t2_running: bool,
    t2_flag: bool,
    sr_flag: bool,
    ca1_flag: bool,
    cb1_flag: bool,
    ca1_high: bool,
    cb1_high: bool,
    pb6_high: bool,
    pb7: bool,
    sr_in: bool,
    ier: u8,
    acr: u8,
    pcr: u8,
    sr: u8,
    sr_left: u8,
    sr_out: u8,
    sr_t2_left: u16,
    sr_clock_high: bool,
    /// Timer flags from a PHI2 roll, shown on the next PHI1 by [`Via6522::tick_phase`].
    t1_flag_due: bool,
    t2_flag_due: bool,
    /// Pulse-output or handshake holding the pin low until tick / CA1.
    ca2_pulse_low: bool,
    cb2_pulse_low: bool,
    ca2_pulses: u32,
    cb2_pulses: u32,
    ca2_high: bool,
    cb2_high: bool,
    ca2_flag: bool,
    cb2_flag: bool,
    /// PB7 pin level while ACR bit 7 gives it to timer 1 (true = high).
    pb7_pin: bool,
    variant: ViaVariant,
    /// Mode `101`: eighth bit out, IFR2 owed on the next rising CB1 edge.
    sr_flag_on_rise: bool,
}

/// Which 6522 part is modelled. The register map, timers, shift register and
/// handshakes behave the same on every part here; what differs is documented
/// electrical behavior that a board must account for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViaVariant {
    /// MOS/Synertek NMOS 6522: open-drain /IRQ, needs an external pull-up.
    #[default]
    Mos6522,
    /// Rockwell R6522 (NMOS): open-drain /IRQ, same as the MOS part.
    RockwellR6522,
    /// WDC W65C22N (CMOS): open-drain IRQB, drop-in for NMOS boards.
    WdcW65c22N,
    /// WDC W65C22S (CMOS): totem-pole IRQB; it cannot be wire-ORed with other
    /// open-drain interrupt sources.
    WdcW65c22S,
}

/// Output stage of the interrupt pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrqOutput {
    /// Pulls low when asserted, floats otherwise (wire-OR capable).
    OpenDrain,
    /// Drives high and low; wire-ORing two of these shorts the outputs.
    TotemPole,
}

impl ViaVariant {
    pub fn irq_output(self) -> IrqOutput {
        match self {
            ViaVariant::WdcW65c22S => IrqOutput::TotemPole,
            _ => IrqOutput::OpenDrain,
        }
    }

    /// CMOS parts (WDC) as opposed to the NMOS MOS/Rockwell parts.
    pub fn is_cmos(self) -> bool {
        matches!(self, ViaVariant::WdcW65c22N | ViaVariant::WdcW65c22S)
    }
}

impl Default for Via6522 {
    fn default() -> Self {
        Self::new()
    }
}

impl Via6522 {
    pub fn new() -> Self {
        Self {
            ora: 0,
            orb: 0,
            ddra: 0,
            ddrb: 0,
            pa_in: 0,
            pb_in: 0,
            pa_driven: 0,
            pb_driven: 0,
            pa_load_low: 0,
            ira_latch: 0,
            irb_latch: 0,
            pb_load_low: 0,
            t1_latch: 0,
            t1_counter: 0,
            t1_running: false,
            t1_reload: false,
            t1_flag: false,
            t2_latch: 0,
            t2_counter: 0,
            t2_running: false,
            t2_flag: false,
            sr_flag: false,
            ca1_flag: false,
            cb1_flag: false,
            ca1_high: true,
            cb1_high: true,
            pb6_high: true,
            pb7: false,
            sr_in: false,
            ier: 0,
            acr: 0,
            pcr: 0,
            sr: 0,
            sr_left: 0,
            sr_out: 0,
            sr_t2_left: 0,
            sr_clock_high: true,
            t1_flag_due: false,
            t2_flag_due: false,
            ca2_pulse_low: false,
            cb2_pulse_low: false,
            ca2_pulses: 0,
            cb2_pulses: 0,
            ca2_high: true,
            cb2_high: true,
            ca2_flag: false,
            cb2_flag: false,
            pb7_pin: true,
            variant: ViaVariant::Mos6522,
            sr_flag_on_rise: false,
        }
    }

    /// A VIA of the given part.
    pub fn with_variant(variant: ViaVariant) -> Self {
        Self {
            variant,
            ..Self::new()
        }
    }

    pub fn variant(&self) -> ViaVariant {
        self.variant
    }

    /// /IRQ pin as seen on a shared line: `Some(false)` driven low, `Some(true)`
    /// driven high (totem-pole parts only), `None` released to the pull-up.
    pub fn irq_pin_drive(&self) -> Option<bool> {
        match (self.irq_asserted(), self.variant.irq_output()) {
            (true, _) => Some(false),
            (false, IrqOutput::TotemPole) => Some(true),
            (false, IrqOutput::OpenDrain) => None,
        }
    }

    /// PB7 output pin while ACR bit 7 hands it to timer 1. One-shot (ACR bit 6
    /// clear): a T1C-H write drives it low and the timeout drives it high, one
    /// programmable pulse. Free-run: the T1C-H write drives it low and every
    /// timeout inverts it, a square wave of period 2(N+2). `None` while ACR bit 7
    /// is clear (PB7 is an ordinary port bit). Idle level after enabling is high.
    pub fn pb7_output(&self) -> Option<bool> {
        (self.acr & 0x80 != 0).then_some(self.pb7_pin)
    }

    /// CB2 data level during a shift-out (ACR `100`, `101`, `110`, `111`): the
    /// last bit shifted out. `None` in other modes, where CB2 follows the PCR.
    pub fn cb2_shift_data(&self) -> Option<bool> {
        matches!(self.sr_mode(), 0b100..=0b111).then_some(self.sr_out & 1 != 0)
    }

    pub fn ddra(&self) -> u8 {
        self.ddra
    }

    pub fn ddrb(&self) -> u8 {
        self.ddrb
    }

    pub fn ora_latch(&self) -> u8 {
        self.ora
    }

    pub fn orb_latch(&self) -> u8 {
        self.orb
    }

    pub fn t1_counter(&self) -> u16 {
        self.t1_counter
    }

    pub fn t1_running(&self) -> bool {
        self.t1_running
    }

    pub fn t1_flag(&self) -> bool {
        self.t1_flag
    }

    pub fn t2_counter(&self) -> u16 {
        self.t2_counter
    }

    pub fn t2_running(&self) -> bool {
        self.t2_running
    }

    pub fn t2_flag(&self) -> bool {
        self.t2_flag
    }

    pub fn sr(&self) -> u8 {
        self.sr
    }

    pub fn sr_bits_left(&self) -> u8 {
        self.sr_left
    }

    /// Bits shifted out so far in the current PHI2 shift-out, MSB first.
    pub fn sr_shifted_out(&self) -> u8 {
        self.sr_out
    }

    /// PB7 output. Toggles on timer 1 underflow only in free-run with PB7 enabled.
    pub fn pb7(&self) -> bool {
        self.pb7
    }

    fn ca2_mode(&self) -> u8 {
        (self.pcr >> 1) & 0x07
    }

    fn cb2_mode(&self) -> u8 {
        (self.pcr >> 5) & 0x07
    }

    /// CA2 pin. Output modes drive it. Input modes report [`Via6522::set_ca2`].
    pub fn ca2(&self) -> bool {
        Self::control_level(self.ca2_mode(), self.ca2_pulse_low, self.ca2_high)
    }

    /// CB2 pin. Same modes as [`Via6522::ca2`].
    pub fn cb2(&self) -> bool {
        Self::control_level(self.cb2_mode(), self.cb2_pulse_low, self.cb2_high)
    }

    /// Times a CPU write to ORA pulsed CA2 while PCR selected pulse output.
    pub fn ca2_pulses(&self) -> u32 {
        self.ca2_pulses
    }

    /// Times a CPU write to ORB pulsed CB2 while PCR selected pulse output.
    pub fn cb2_pulses(&self) -> u32 {
        self.cb2_pulses
    }

    fn control_level(mode: u8, pulse_low: bool, input_high: bool) -> bool {
        match mode {
            0b110 => false,
            0b111 => true,
            0b100 | 0b101 => !pulse_low,
            _ => input_high,
        }
    }

    fn drive_from_write(mode: u8, pulse_low: &mut bool, pulses: &mut u32) {
        match mode {
            0b100 => *pulse_low = true,
            0b101 => {
                *pulse_low = true;
                *pulses = pulses.wrapping_add(1);
            }
            _ => {}
        }
    }

    /// Host drives every port A input pin (`DDRA` bit clear) to `pins`.
    pub fn set_port_a_in(&mut self, pins: u8) {
        self.drive_port_a(0xFF, pins);
    }

    pub fn set_port_b_in(&mut self, pins: u8) {
        self.drive_port_b(0xFF, pins);
    }

    /// Pin load model. An input pin the host drives reads its level. An input pin
    /// nothing drives floats high through the pull-up and reads 1. `mask` picks the
    /// driven pins; the others are released.
    pub fn drive_port_a(&mut self, mask: u8, levels: u8) {
        self.pa_driven = mask;
        self.pa_in = levels;
    }

    pub fn drive_port_b(&mut self, mask: u8, levels: u8) {
        self.pb_driven = mask;
        self.pb_in = levels;
    }

    /// Open-drain loads (wired-AND) sinking port A pins low. On port A a low load wins
    /// over a high output: the NMOS output pulls up passively and ORA reads the pin,
    /// so the line and the read are 0. A load on an input pin also reads 0.
    pub fn set_port_a_load_low(&mut self, mask: u8) {
        self.pa_load_low = mask;
    }

    /// Open-drain loads on port B. Port B outputs are push-pull and a read of an
    /// output bit returns ORB, so a load only lowers input pins.
    pub fn set_port_b_load_low(&mut self, mask: u8) {
        self.pb_load_low = mask;
    }

    fn input_level(levels: u8, driven: u8, load_low: u8) -> u8 {
        ((levels & driven) | !driven) & !load_low
    }

    /// Port A line levels, which is also what ORA reads.
    pub fn port_a_pins(&self) -> u8 {
        ((self.ora & self.ddra) | (Self::input_level(self.pa_in, self.pa_driven, 0) & !self.ddra))
            & !self.pa_load_low
    }

    /// What an IRA read returns: the pins, or with ACR bit 0 set the pins latched on the
    /// last active CA1 edge (zero before any edge).
    pub fn ira(&self) -> u8 {
        if self.acr & 1 != 0 {
            self.ira_latch
        } else {
            self.port_a_pins()
        }
    }

    /// What an IRB read returns: output bits always read ORB; with ACR bit 1 set the
    /// input bits come from the pins latched on the last active CB1 edge.
    pub fn irb(&self) -> u8 {
        if self.acr & 2 != 0 {
            (self.orb & self.ddrb) | (self.irb_latch & !self.ddrb)
        } else {
            self.port_b_pins()
        }
    }

    /// Port B line levels: outputs drive ORB, inputs follow the load model.
    /// With ACR bit 7 set, timer 1 drives PB7 whatever DDRB bit 7 says.
    pub fn port_b_pins(&self) -> u8 {
        let pins = (self.orb & self.ddrb)
            | (Self::input_level(self.pb_in, self.pb_driven, self.pb_load_low) & !self.ddrb);
        match self.pb7_output() {
            Some(high) => (pins & 0x7F) | (u8::from(high) << 7),
            None => pins,
        }
    }

    /// RES pin: every register returns to zero (DDRs, output registers, ACR, PCR,
    /// IFR, IER, so all pins are inputs and IRQ is released) except the timer
    /// counters and latches and the shift register, which the datasheet leaves
    /// alone. The timers stop counting toward an interrupt (their flags and "running"
    /// state clear). Pins the host drives from outside keep their levels.
    pub fn reset(&mut self) {
        let old = self.state();
        let mut s = Self::new().state();
        s.t1_latch = old.t1_latch;
        s.t1_counter = old.t1_counter;
        s.t2_latch = old.t2_latch;
        s.t2_counter = old.t2_counter;
        s.sr = old.sr;
        s.pa_in = old.pa_in;
        s.pb_in = old.pb_in;
        s.pa_driven = old.pa_driven;
        s.pb_driven = old.pb_driven;
        s.pa_load_low = old.pa_load_low;
        s.pb_load_low = old.pb_load_low;
        s.ca1_high = old.ca1_high;
        s.cb1_high = old.cb1_high;
        s.pb6_high = old.pb6_high;
        self.load_state(&s);
        self.pb7_pin = true;
    }

    /// PHI2 ticks until [`Via6522::irq_asserted`] would rise with no further host
    /// input or register access, searching at most `limit` ticks; `Some(0)` when it
    /// is already asserted. For schedulers that fast-forward to the next interrupt.
    pub fn ticks_until_irq(&self, limit: u32) -> Option<u32> {
        if self.irq_asserted() {
            return Some(0);
        }
        let mut probe = self.clone();
        (1..=limit).find(|_| {
            probe.tick();
            probe.irq_asserted()
        })
    }

    pub fn state(&self) -> ViaState {
        ViaState {
            ira_latch: self.ira_latch,
            irb_latch: self.irb_latch,
            ddra: self.ddra,
            ddrb: self.ddrb,
            ora: self.ora,
            orb: self.orb,
            pa_in: self.pa_in,
            pb_in: self.pb_in,
            pa_driven: self.pa_driven,
            pb_driven: self.pb_driven,
            pa_load_low: self.pa_load_low,
            pb_load_low: self.pb_load_low,
            t1_latch: self.t1_latch,
            t1_counter: self.t1_counter,
            t1_running: self.t1_running,
            t1_reload: self.t1_reload,
            t1_flag: self.t1_flag,
            t2_latch: self.t2_latch,
            t2_counter: self.t2_counter,
            t2_running: self.t2_running,
            t2_flag: self.t2_flag,
            ifr_sr: self.sr_flag,
            ca1_flag: self.ca1_flag,
            cb1_flag: self.cb1_flag,
            ca1_high: self.ca1_high,
            cb1_high: self.cb1_high,
            pb6_high: self.pb6_high,
            pb7: self.pb7,
            sr_in: self.sr_in,
            ier: self.ier,
            acr: self.acr,
            pcr: self.pcr,
            sr: self.sr,
            sr_left: self.sr_left,
            sr_out: self.sr_out,
            sr_t2_left: self.sr_t2_left,
            sr_clock_high: self.sr_clock_high,
            t1_flag_due: self.t1_flag_due,
            t2_flag_due: self.t2_flag_due,
        }
    }

    pub fn load_state(&mut self, state: &ViaState) {
        self.ira_latch = state.ira_latch;
        self.irb_latch = state.irb_latch;
        self.ddra = state.ddra;
        self.ddrb = state.ddrb;
        self.ora = state.ora;
        self.orb = state.orb;
        self.pa_in = state.pa_in;
        self.pb_in = state.pb_in;
        self.pa_driven = state.pa_driven;
        self.pb_driven = state.pb_driven;
        self.pa_load_low = state.pa_load_low;
        self.pb_load_low = state.pb_load_low;
        self.t1_latch = state.t1_latch;
        self.t1_counter = state.t1_counter;
        self.t1_running = state.t1_running;
        self.t1_reload = state.t1_reload;
        self.t1_flag = state.t1_flag;
        self.t2_latch = state.t2_latch;
        self.t2_counter = state.t2_counter;
        self.t2_running = state.t2_running;
        self.t2_flag = state.t2_flag;
        self.sr_flag = state.ifr_sr;
        self.ca1_flag = state.ca1_flag;
        self.cb1_flag = state.cb1_flag;
        self.ca1_high = state.ca1_high;
        self.cb1_high = state.cb1_high;
        self.pb6_high = state.pb6_high;
        self.pb7 = state.pb7;
        self.sr_in = state.sr_in;
        self.ier = state.ier & 0x7F;
        self.acr = state.acr;
        self.pcr = state.pcr;
        self.sr = state.sr;
        self.sr_left = state.sr_left;
        self.sr_out = state.sr_out;
        self.sr_t2_left = state.sr_t2_left;
        self.sr_clock_high = state.sr_clock_high;
        self.t1_flag_due = state.t1_flag_due;
        self.t2_flag_due = state.t2_flag_due;
        self.sr_flag_on_rise = false;
        self.ca2_flag = false;
        self.cb2_flag = false;
        self.ca2_high = true;
        self.cb2_high = true;
        self.ca2_pulse_low = false;
        self.cb2_pulse_low = false;
    }

    fn sr_mode(&self) -> u8 {
        (self.acr >> 2) & 0x07
    }

    fn flag_bits(&self) -> u8 {
        let mut ifr = 0;
        if self.t1_flag {
            ifr |= 1 << 6;
        }
        if self.t2_flag {
            ifr |= 1 << 5;
        }
        if self.sr_flag {
            ifr |= 1 << 2;
        }
        if self.ca1_flag {
            ifr |= 1 << 1;
        }
        if self.cb1_flag {
            ifr |= 1 << 4;
        }
        if self.ca2_flag {
            ifr |= 1 << 0;
        }
        if self.cb2_flag {
            ifr |= 1 << 3;
        }
        ifr
    }

    /// Drive CA1. Idle is high. The active edge is PCR bit 0 (0 falling, 1 rising). It
    /// sets IFR bit 1 whatever the IER says; the IER only gates the IRQ pin.
    pub fn set_ca1(&mut self, high: bool) {
        self.control_edge(true, high);
    }

    /// Drive CB1. Same rules as [`Via6522::set_ca1`] with PCR bit 4 and IFR bit 4.
    pub fn set_cb1(&mut self, high: bool) {
        self.control_edge(false, high);
    }

    fn control_edge(&mut self, ca1: bool, high: bool) {
        let previous = if ca1 { self.ca1_high } else { self.cb1_high };
        if ca1 {
            self.ca1_high = high;
        } else {
            self.cb1_high = high;
        }
        // External CB1 clock: shift in on the rising edge, shift out on the falling
        // edge (CB2 data is valid after the negative CB1 transition).
        if !ca1 && !previous && high && self.sr_mode() == 0b011 {
            self.shift_external(true);
        }
        if !ca1 && previous && !high && self.sr_mode() == 0b111 {
            self.shift_external(false);
        }
        let rising = self.pcr & if ca1 { 0x01 } else { 0x10 } != 0;
        let active = if rising {
            !previous && high
        } else {
            previous && !high
        };
        if active {
            // Input latching: the active edge captures the port pins.
            if ca1 {
                self.ira_latch = self.port_a_pins();
            } else {
                self.irb_latch = self.port_b_pins();
            }
            if ca1 {
                self.ca1_flag = true;
            } else {
                self.cb1_flag = true;
            }
            if ca1 && self.ca2_mode() == 0b100 {
                self.ca2_pulse_low = false;
            }
            if !ca1 && self.cb2_mode() == 0b100 {
                self.cb2_pulse_low = false;
            }
        }
    }

    /// Drive CA2 as an input. Ignored while the PCR selects an output mode.
    /// An active edge sets IFR bit 0; the IER only gates the IRQ pin.
    pub fn set_ca2(&mut self, high: bool) {
        self.input_edge(true, high);
    }

    /// Drive CB2 as an input. Same rules as [`Via6522::set_ca2`], IFR bit 3.
    pub fn set_cb2(&mut self, high: bool) {
        self.input_edge(false, high);
    }

    fn input_edge(&mut self, ca2: bool, high: bool) {
        let mode = if ca2 {
            self.ca2_mode()
        } else {
            self.cb2_mode()
        };
        if mode & 0b100 != 0 {
            return;
        }
        let previous = if ca2 { self.ca2_high } else { self.cb2_high };
        if ca2 {
            self.ca2_high = high;
        } else {
            self.cb2_high = high;
        }
        let positive = mode & 0b010 != 0;
        let active = if positive {
            !previous && high
        } else {
            previous && !high
        };
        if !active {
            return;
        }
        if ca2 {
            self.ca2_flag = true;
        } else {
            self.cb2_flag = true;
        }
    }

    /// Port access for handshake. Output handshake pulls the pin low.
    /// Input handshake modes (`x0` in the low two bits, PCR bit 3/7 clear) clear the flag.
    /// Independent input modes and pulse output do not.
    /// ORA/ORB read or write (not `$0F`): clears CA1/CB1 and, unless CA2/CB2 is an
    /// independent-interrupt input, CA2/CB2 (datasheet IFR clear conditions).
    fn note_port_access(&mut self, port_a: bool) {
        if port_a {
            self.ca1_flag = false;
        } else {
            self.cb1_flag = false;
        }
        let mode = if port_a {
            self.ca2_mode()
        } else {
            self.cb2_mode()
        };
        if mode & 0b100 == 0 {
            if mode & 0b001 == 0 {
                if port_a {
                    self.ca2_flag = false;
                } else {
                    self.cb2_flag = false;
                }
            }
            return;
        }
        if mode == 0b100 {
            if port_a {
                self.ca2_pulse_low = true;
            } else {
                self.cb2_pulse_low = true;
                if self.ier & (1 << 3) != 0 {
                    self.cb2_flag = true;
                }
            }
        }
    }

    /// Drive PB6. In pulse mode (ACR bit 5) a falling edge decrements timer 2.
    pub fn set_pb6(&mut self, high: bool) {
        let falling = self.pb6_high && !high;
        self.pb6_high = high;
        if falling && self.acr & 0x20 != 0 {
            self.decrement_t2();
        }
    }

    /// Bit sampled by shift-in (`ACR` `010` under PHI2, `011` on CB1 rising edges), MSB first.
    pub fn set_sr_in_bit(&mut self, high: bool) {
        self.sr_in = high;
    }

    /// IRQ pin: any IFR bit whose matching IER bit is set.
    pub fn irq_asserted(&self) -> bool {
        self.flag_bits() & self.ier & 0x7F != 0
    }

    /// One CB1 pulse in modes `011`/`111`. The datasheet's counter "does not stop
    /// the shifting operation": after eight pulses (IFR2 set) further pulses keep
    /// shifting data but are no longer counted until an SR access re-arms eight.
    fn shift_external(&mut self, shift_in: bool) {
        match (shift_in, self.sr_left) {
            (true, 0) => self.sr = (self.sr << 1) | u8::from(self.sr_in),
            (true, _) => self.shift_in_one(),
            (false, 0) => {
                self.sr_out = (self.sr_out << 1) | (self.sr >> 7);
                self.sr <<= 1;
            }
            (false, _) => self.shift_out_one(),
        }
    }

    fn shift_in_one(&mut self) {
        if self.sr_left == 0 {
            return;
        }
        let bit = u8::from(self.sr_in);
        self.sr = (self.sr << 1) | bit;
        self.sr_left -= 1;
        if self.sr_left == 0 {
            self.sr_flag = true;
        }
    }

    /// CB1 level of the T2-rate shift clock (ACR shift modes `001`, `100`, `101`).
    pub fn sr_clock_high(&self) -> bool {
        self.sr_clock_high
    }

    /// T2-rate shift clock. CB1 toggles every N+2 PHI2, where N is the T2 low latch;
    /// output bits change on the falling edge and input bits are taken on the rising
    /// edge, so one bit takes 2(N+2) cycles. `001` sets IFR bit 2 on the eighth rising
    /// edge; `101` sets it when CB1 returns high after the eighth bit, both 16(N+2)
    /// cycles after the SR access, and then stop with CB1 high. `100` recirculates the byte forever and never sets the flag.
    fn tick_sr_t2(&mut self, mode: u8) {
        if mode != 0b100 && self.sr_left == 0 && !self.sr_flag_on_rise {
            return;
        }
        if self.sr_t2_left > 1 {
            self.sr_t2_left -= 1;
            return;
        }
        self.sr_t2_left = (self.t2_latch & 0xFF) + 2;
        self.sr_clock_high = !self.sr_clock_high;
        match (mode, self.sr_clock_high) {
            (0b001, true) => self.shift_in_one(),
            (0b101, false) => {
                // The eighth bit leaves on a falling edge; IFR2 sets when CB1
                // returns high N+2 cycles later and CB1 then stays high, so the
                // whole byte takes 16(N+2) cycles.
                let before = self.sr_flag;
                self.shift_out_one();
                if self.sr_left == 0 && !before {
                    self.sr_flag = false;
                    self.sr_flag_on_rise = true;
                }
            }
            (0b101, true) if self.sr_flag_on_rise => {
                self.sr_flag_on_rise = false;
                self.sr_flag = true;
            }
            (0b100, false) => {
                let bit = self.sr >> 7;
                self.sr_out = (self.sr_out << 1) | bit;
                self.sr = self.sr.rotate_left(1);
            }
            _ => {}
        }
    }

    /// Start a shift after an SR access: eight bits (none counted in free-run) and a
    /// fresh T2-rate clock with CB1 high.
    fn start_shift(&mut self) {
        let mode = self.sr_mode();
        self.sr_left = if matches!(mode, 0b001 | 0b010 | 0b011 | 0b101 | 0b110 | 0b111) {
            8
        } else {
            0
        };
        self.sr_t2_left = (self.t2_latch & 0xFF) + 2;
        self.sr_clock_high = true;
        self.sr_flag_on_rise = false;
    }

    fn shift_out_one(&mut self) {
        if self.sr_left == 0 {
            return;
        }
        let bit = (self.sr >> 7) & 1;
        self.sr_out = (self.sr_out << 1) | bit;
        self.sr <<= 1;
        self.sr_left -= 1;
        if self.sr_left == 0 {
            self.sr_flag = true;
        }
    }

    /// One half cycle. PHI2 runs [`tick`](Self::tick) but holds a timer flag set by
    /// the roll to `$FFFF`; the following PHI1 shows it. After a T1C-H/T2C-H write in
    /// the PHI2 of cycle 0 the flag appears in the PHI1 of cycle N+2: the datasheet's
    /// N+1.5. Rewriting the counter high byte drops a held flag.
    pub fn tick_phase(&mut self, phase: crate::cycles::BusPhase) {
        if phase == crate::cycles::BusPhase::Phi1 {
            self.t1_flag |= std::mem::take(&mut self.t1_flag_due);
            self.t2_flag |= std::mem::take(&mut self.t2_flag_due);
            return;
        }
        let (t1, t2) = (self.t1_flag, self.t2_flag);
        self.advance_phi2();
        self.t1_flag_due |= self.t1_flag && !t1;
        self.t2_flag_due |= self.t2_flag && !t2;
        self.t1_flag = t1;
        self.t2_flag = t2;
    }

    /// One whole cycle: a PHI2 then the following PHI1, exactly
    /// `tick_phase(Phi2); tick_phase(Phi1)`. A timer flag rolled in this PHI2 shows
    /// when `tick` returns; one held by an earlier `step_phase` PHI2 shows too.
    pub fn tick(&mut self) {
        self.tick_phase(crate::cycles::BusPhase::Phi2);
        self.tick_phase(crate::cycles::BusPhase::Phi1);
    }

    /// PHI2 work: timers and, in shift-out-under-PHI2 mode, the shift register.
    fn advance_phi2(&mut self) {
        if self.ca2_mode() == 0b101 {
            self.ca2_pulse_low = false;
        }
        if self.cb2_mode() == 0b101 {
            self.cb2_pulse_low = false;
        }
        self.tick_timer(true);
        self.tick_timer(false);
        let mode = self.sr_mode();
        if matches!(mode, 0b001 | 0b100 | 0b101) {
            self.tick_sr_t2(mode);
            return;
        }
        if self.sr_left == 0 {
            return;
        }
        match mode {
            0b110 => self.shift_out_one(),
            0b010 => self.shift_in_one(),
            _ => {}
        }
    }

    /// The counters decrement on every PHI2 whether armed or not (T2 only in timed
    /// mode). `t1_running`/`t2_running` mean armed. T1 sets its flag on the tick that
    /// rolls an armed counter from 0 to `$FFFF`: N+1 ticks after loading N, which with
    /// the write cycle is the datasheet's N+1.5. Free-run spends one more tick loading
    /// the latch, so each period is N+2. A one-shot disarms and keeps counting down.
    fn tick_timer(&mut self, timer1: bool) {
        if timer1 {
            if self.t1_reload {
                self.t1_reload = false;
                self.t1_counter = self.t1_latch;
                return;
            }
            let rolled = self.t1_counter == 0;
            self.t1_counter = self.t1_counter.wrapping_sub(1);
            if rolled && self.t1_running {
                self.t1_flag = true;
                if self.acr & 0xC0 == 0xC0 {
                    self.pb7 = !self.pb7;
                }
                if self.acr & 0x40 != 0 {
                    self.pb7_pin = !self.pb7_pin;
                } else {
                    self.pb7_pin = true;
                }
                if self.acr & 0x40 != 0 {
                    self.t1_reload = true;
                } else {
                    self.t1_running = false;
                }
            }
        } else if self.acr & 0x20 == 0 {
            // Timed T2 flags on the roll to $FFFF like T1: N+1 ticks (datasheet N+1.5).
            let rolled = self.t2_counter == 0;
            self.t2_counter = self.t2_counter.wrapping_sub(1);
            if rolled && self.t2_running {
                self.t2_flag = true;
                self.t2_running = false;
            }
        }
    }

    /// Pulse-counting decrement: the flag sets when the count reaches zero.
    fn decrement_t2(&mut self) {
        self.t2_counter = self.t2_counter.wrapping_sub(1);
        if self.t2_counter == 0 && self.t2_running {
            self.t2_flag = true;
            self.t2_running = false;
        }
    }

    /// Same map as [`Via6522::read`] but does not clear interrupt flags.
    pub fn peek(&self, offset: u8) -> u8 {
        match offset & 0x0F {
            0x00 => self.irb(),
            0x01 | 0x0F => self.ira(),
            0x02 => self.ddrb,
            0x03 => self.ddra,
            0x04 => self.t1_counter as u8,
            0x05 => (self.t1_counter >> 8) as u8,
            0x06 => self.t1_latch as u8,
            0x07 => (self.t1_latch >> 8) as u8,
            0x08 => self.t2_counter as u8,
            0x09 => (self.t2_counter >> 8) as u8,
            0x0A => self.sr,
            0x0B => self.acr,
            0x0C => self.pcr,
            0x0D => {
                let flags = self.flag_bits();
                if flags & self.ier & 0x7F != 0 {
                    flags | 0x80
                } else {
                    flags
                }
            }
            0x0E => 0x80 | self.ier,
            _ => 0,
        }
    }

    /// Register offsets `$0`–`$F` match the 6522.
    pub fn read(&mut self, offset: u8) -> u8 {
        match offset & 0x0F {
            0x00 => {
                self.note_port_access(false);
                self.irb()
            }
            0x01 => {
                self.note_port_access(true);
                self.ira()
            }
            0x0F => self.ira(),
            0x02 => self.ddrb,
            0x03 => self.ddra,
            0x04 => {
                self.t1_flag = false;
                self.t1_counter as u8
            }
            0x05 => (self.t1_counter >> 8) as u8,
            0x06 => self.t1_latch as u8,
            0x07 => (self.t1_latch >> 8) as u8,
            0x08 => {
                self.t2_flag = false;
                self.t2_counter as u8
            }
            0x09 => (self.t2_counter >> 8) as u8,
            0x0A => {
                // Datasheet: an SR read clears IFR bit 2; in shift-in modes it also
                // starts the next eight bits.
                let value = self.sr;
                self.sr_flag = false;
                if matches!(self.sr_mode(), 0b001..=0b011) {
                    self.start_shift();
                }
                value
            }
            0x0B => self.acr,
            0x0C => self.pcr,
            0x0D => {
                let flags = self.flag_bits();
                if flags & self.ier & 0x7F != 0 {
                    flags | 0x80
                } else {
                    flags
                }
            }
            0x0E => 0x80 | self.ier,
            _ => 0,
        }
    }

    pub fn write(&mut self, offset: u8, value: u8) {
        match offset & 0x0F {
            0x00 => {
                self.orb = value;
                self.note_port_access(false);
                let mode = self.cb2_mode();
                Self::drive_from_write(mode, &mut self.cb2_pulse_low, &mut self.cb2_pulses);
            }
            0x01 => {
                self.ora = value;
                self.note_port_access(true);
                let mode = self.ca2_mode();
                Self::drive_from_write(mode, &mut self.ca2_pulse_low, &mut self.ca2_pulses);
            }
            0x0F => self.ora = value,
            0x02 => self.ddrb = value,
            0x03 => self.ddra = value,
            0x04 | 0x06 => {
                self.t1_latch = (self.t1_latch & 0xFF00) | u16::from(value);
            }
            0x05 => {
                self.t1_latch = (self.t1_latch & 0x00FF) | (u16::from(value) << 8);
                self.t1_counter = self.t1_latch;
                self.t1_flag = false;
                self.t1_flag_due = false;
                self.t1_running = true;
                self.t1_reload = false;
                self.pb7_pin = false;
            }
            0x07 => {
                self.t1_latch = (self.t1_latch & 0x00FF) | (u16::from(value) << 8);
            }
            0x08 => {
                self.t2_latch = (self.t2_latch & 0xFF00) | u16::from(value);
            }
            0x09 => {
                self.t2_latch = (self.t2_latch & 0x00FF) | (u16::from(value) << 8);
                self.t2_counter = self.t2_latch;
                self.t2_flag = false;
                self.t2_flag_due = false;
                self.t2_running = true;
            }
            0x0A => {
                self.sr = value;
                self.sr_flag = false;
                self.sr_out = 0;
                self.start_shift();
            }
            0x0B => self.acr = value,
            0x0C => self.pcr = value,
            0x0D => {
                if value & (1 << 6) != 0 {
                    self.t1_flag = false;
                }
                if value & (1 << 5) != 0 {
                    self.t2_flag = false;
                }
                if value & (1 << 2) != 0 {
                    self.sr_flag = false;
                }
                if value & (1 << 1) != 0 {
                    self.ca1_flag = false;
                }
                if value & (1 << 4) != 0 {
                    self.cb1_flag = false;
                }
                if value & (1 << 0) != 0 {
                    self.ca2_flag = false;
                }
                if value & (1 << 3) != 0 {
                    self.cb2_flag = false;
                }
            }
            0x0E => {
                let mask = value & 0x7F;
                if value & 0x80 != 0 {
                    self.ier |= mask;
                } else {
                    self.ier &= !mask;
                }
            }
            _ => {}
        }
    }
}

/// Register window for [`crate::bus::SystemBus`]: offsets wrap to 16 registers and
/// [`IoDevice::phase`](crate::bus::IoDevice::phase) runs [`Via6522::tick_phase`].
impl crate::bus::IoDevice for Via6522 {
    fn read(&mut self, offset: u16) -> u8 {
        Via6522::read(self, (offset & 0x0F) as u8)
    }

    fn write(&mut self, offset: u16, value: u8) {
        Via6522::write(self, (offset & 0x0F) as u8, value);
    }

    fn peek(&self, offset: u16) -> u8 {
        Via6522::peek(self, (offset & 0x0F) as u8)
    }

    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        self.tick_phase(phase);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_fires_once_and_stays_idle_until_rearmed() {
        let mut timer = ViaTimer1::new();
        timer.write(0x0E, 0xC0); // enable T1
        assert!(timer.read(0x0E) & 0x40 != 0);

        timer.write(0x04, 0x03);
        timer.write(0x05, 0x00);
        assert_eq!(timer.counter(), 3);
        assert!(timer.running());
        assert!(!timer.timed_out());

        timer.tick();
        timer.tick();
        assert_eq!(timer.counter(), 1);
        assert!(!timer.irq_asserted());

        timer.tick();
        assert_eq!(timer.counter(), 0);
        assert!(!timer.running());
        assert!(timer.timed_out());
        assert!(timer.irq_asserted());
        assert_eq!(timer.read(0x0D) & 0xC0, 0xC0);

        // Reading the low counter clears the flag. Further ticks must not set it again.
        let _ = timer.read(0x04);
        assert!(!timer.timed_out());
        for _ in 0..8 {
            timer.tick();
        }
        assert!(!timer.timed_out());
        assert!(!timer.irq_asserted());
        assert_eq!(timer.counter(), 0);

        // Writing the latch high byte reloads and restarts.
        timer.write(0x06, 0x01);
        timer.write(0x05, 0x00);
        assert_eq!(timer.counter(), 1);
        assert!(timer.running());
        timer.tick();
        assert!(timer.timed_out());
        assert!(!timer.running());
    }

    #[test]
    fn ports_honor_ddr_and_input_pins() {
        let mut via = Via6522::new();
        via.write(0x03, 0x0F);
        via.write(0x01, 0xA5);
        via.set_port_a_in(0xF0);
        assert_eq!(via.read(0x01), 0xF5);
        assert_eq!(via.ora_latch(), 0xA5);

        via.write(0x02, 0xF0);
        via.write(0x00, 0x3C);
        via.set_port_b_in(0x0F);
        assert_eq!(via.read(0x00), 0x3F);
        assert_eq!(via.ddrb(), 0xF0);
        assert_eq!(via.ddra(), 0x0F);
    }

    #[test]
    fn timer1_one_shot_sets_ifr_and_stops() {
        let mut via = Via6522::new();
        via.write(0x0B, 0x00);
        via.write(0x0E, 0xC0);
        via.write(0x04, 0x02);
        via.write(0x05, 0x00);
        assert!(via.t1_running());
        via.tick();
        via.tick();
        assert_eq!(via.t1_counter(), 0);
        assert!(!via.t1_flag(), "zero alone is not the time-out");
        assert_eq!(via.read(0x0D) & 0x40, 0);
        via.tick();
        assert!(!via.t1_running());
        assert_eq!(via.t1_counter(), 0xFFFF);
        assert_eq!(via.read(0x0D) & 0xC0, 0xC0);
        via.tick();
        // Datasheet: the counter keeps decrementing after a one-shot time-out.
        assert_eq!(via.t1_counter(), 0xFFFE);
        assert!(!via.t1_running());
        let _ = via.read(0x04);
        assert_eq!(via.read(0x0D) & 0x40, 0);
    }

    #[test]
    fn timer1_free_run_reloads_and_keeps_the_flag_until_cleared() {
        let mut via = Via6522::new();
        via.write(0x0B, 0x40);
        via.write(0x0E, 0xC0);
        via.write(0x06, 0x01);
        via.write(0x05, 0x00);
        via.tick();
        via.tick();
        assert!(via.t1_running());
        assert_eq!(via.t1_counter(), 0xFFFF);
        assert_eq!(via.read(0x0D) & 0xC0, 0xC0);
        let _ = via.read(0x04);
        assert_eq!(via.read(0x0D) & 0x40, 0);
        via.tick();
        assert_eq!(via.t1_counter(), 1, "reload one tick after the roll");
        via.tick();
        via.tick();
        assert!(via.t1_running());
        assert_eq!(via.read(0x0D) & 0x40, 0x40);
        via.write(0x0E, 0x40);
        assert_eq!(via.read(0x0D) & 0x80, 0);
        assert!(via.t1_flag());
    }

    #[test]
    fn timer2_one_shot_sets_its_ifr_bit_and_does_not_reload() {
        let mut via = Via6522::new();
        via.write(0x0B, 0x00);
        via.write(0x0E, 0xA0);
        via.write(0x08, 0x01);
        via.write(0x09, 0x00);
        assert!(via.t2_running());
        via.tick();
        assert!(!via.t2_flag(), "timed T2 flags on the roll to $FFFF");
        via.tick();
        assert!(via.t2_flag());
        assert!(!via.t2_running());
        assert_eq!(via.t2_counter(), 0xFFFF);
        assert_eq!(via.read(0x0D) & 0xA0, 0xA0);
        via.tick();
        assert_eq!(via.t2_counter(), 0xFFFE);
        assert!(!via.t2_running());
    }

    #[test]
    fn shift_out_under_phi2_emits_eight_bits_and_sets_ifr() {
        let mut via = Via6522::new();
        via.write(0x0B, 0b110 << 2);
        via.write(0x0E, 0x84);
        via.write(0x0A, 0xA5);
        assert_eq!(via.sr_bits_left(), 8);
        for _ in 0..7 {
            via.tick();
        }
        assert_eq!(via.sr_bits_left(), 1);
        assert_eq!(via.read(0x0D) & 0x04, 0);
        via.tick();
        assert_eq!(via.sr_bits_left(), 0);
        assert_eq!(via.sr_shifted_out(), 0xA5);
        assert_eq!(via.sr(), 0);
        assert_eq!(via.read(0x0D) & 0x84, 0x84);
        via.tick();
        assert_eq!(via.sr_shifted_out(), 0xA5);
    }

    #[test]
    fn ca1_and_cb1_falling_edges_follow_ier() {
        let mut via = Via6522::new();
        via.set_ca1(false);
        via.set_cb1(false);
        // Datasheet: the flag is set whatever the IER; only the IRQ bit 7 stays low.
        assert_eq!(via.peek(0x0D), 0x12, "disabled IER still sets IFR");
        assert!(!via.irq_asserted());
        via.write(0x0D, 0x12);

        via.set_ca1(true);
        via.set_cb1(true);
        via.write(0x0E, 0x80 | 0x02);
        via.set_ca1(true);
        assert_eq!(via.peek(0x0D) & 0x02, 0);
        via.set_ca1(false);
        assert_eq!(via.peek(0x0D) & 0x02, 0x02);
        assert_eq!(via.peek(0x0D) & 0x10, 0);

        via.write(0x0E, 0x80 | 0x10);
        via.set_cb1(false);
        assert_eq!(via.peek(0x0D) & 0x10, 0x10);
        via.write(0x0D, 0x12);
        assert_eq!(via.peek(0x0D) & 0x12, 0);
    }

    #[test]
    fn timer2_pulse_mode_counts_three_pb6_edges() {
        let mut via = Via6522::new();
        via.write(0x0B, 0x20);
        via.write(0x0E, 0xA0);
        via.write(0x08, 0x03);
        via.write(0x09, 0x00);
        via.tick();
        assert_eq!(via.t2_counter(), 3, "pulse mode does not count PHI2");
        for expected in [2u16, 1, 0] {
            via.set_pb6(true);
            via.set_pb6(false);
            assert_eq!(via.t2_counter(), expected);
        }
        assert!(via.t2_flag());
        assert!(!via.t2_running());
        assert_eq!(via.read(0x0D) & 0x20, 0x20);
    }

    #[test]
    fn pb7_toggles_twice_on_free_run_underflow() {
        let mut via = Via6522::new();
        via.write(0x0B, 0xC0);
        via.write(0x06, 0x01);
        via.write(0x05, 0x00);
        assert!(!via.pb7());
        via.tick();
        via.tick();
        assert!(via.pb7());
        assert!(via.t1_running());
        for _ in 0..3 {
            via.tick();
        }
        assert!(!via.pb7(), "free-run period is N+2");
        assert!(via.t1_flag());
    }

    #[test]
    fn shift_in_under_phi2_is_msb_first_and_sets_ifr() {
        let mut via = Via6522::new();
        via.write(0x0B, 0b010 << 2);
        via.write(0x0E, 0x84);
        via.write(0x0A, 0x00);
        for bit in [1, 0, 1, 0, 0, 1, 0, 1] {
            via.set_sr_in_bit(bit == 1);
            via.tick();
        }
        assert_eq!(via.sr(), 0xA5);
        assert_eq!(via.sr_bits_left(), 0);
        assert_eq!(via.read(0x0D) & 0x84, 0x84);
        via.tick();
        assert_eq!(via.sr(), 0xA5);
    }

    #[test]
    fn ca2_and_cb2_output_modes_pulse_on_a_write_to_the_output_register() {
        let mut via = Via6522::new();
        assert!(via.ca2());
        assert!(via.cb2());
        via.write(0x0C, 0x0C); // CA2 manual low (PCR bits 3–1 = 110)
        assert!(!via.ca2());
        via.write(0x0C, 0x0E); // CA2 manual high (111)
        assert!(via.ca2());
        via.write(0x01, 0x55);
        assert_eq!(via.ca2_pulses(), 0, "manual high does not pulse");

        via.write(0x0C, 0x0A); // CA2 pulse (101)
        via.write(0x01, 0x55);
        assert_eq!(via.ca2_pulses(), 1);
        assert!(!via.ca2(), "pulse holds CA2 low until the next PHI2");
        via.write(0x0F, 0x01);
        assert_eq!(via.ca2_pulses(), 1, "$0F does not handshake CA2");
        via.tick();
        assert!(via.ca2());

        via.write(0x0C, 0xA0); // CB2 pulse (bits 7–5 = 101)
        via.write(0x00, 0x01);
        assert_eq!(via.cb2_pulses(), 1);
        assert!(!via.cb2());
        via.tick();
        assert!(via.cb2());
    }

    #[test]
    fn ca2_cb2_handshake_is_not_a_pulse() {
        let mut via = Via6522::new();
        via.write(0x0C, 0x08);
        assert!(via.ca2());
        via.write(0x01, 0x01);
        assert!(!via.ca2());
        assert_eq!(via.ca2_pulses(), 0, "output handshake is not pulse mode");
        via.tick();
        assert!(!via.ca2(), "PHI2 does not release output handshake");
        via.set_ca1(false);
        assert!(via.ca2());
        let _ = via.read(0x01);
        assert!(!via.ca2(), "reading ORA asserts output handshake");
        via.write(0x0F, 0x00);
        assert!(!via.ca2(), "$0F does not touch CA2");

        let mut via = Via6522::new();
        via.set_ca2(false);
        assert_eq!(via.peek(0x0D), 0x01, "IER gates only the IRQ bit");
        via.write(0x0D, 0x01);
        via.set_ca2(true);
        via.write(0x0E, 0x81);
        via.set_ca2(false);
        assert_eq!(via.peek(0x0D) & 0x01, 0x01);
        let _ = via.read(0x01);
        assert_eq!(via.peek(0x0D) & 0x01, 0, "input handshake clears on ORA");

        via.write(0x0C, 0x02);
        via.set_ca2(true);
        via.set_ca2(false);
        assert_eq!(via.peek(0x0D) & 0x01, 0x01);
        let _ = via.read(0x01);
        assert_eq!(via.peek(0x0D) & 0x01, 0x01, "independent CA2 survives ORA");
        via.write(0x0D, 0x01);
        assert_eq!(via.peek(0x0D) & 0x01, 0);

        via.write(0x0C, 0x80);
        via.write(0x00, 0x22);
        assert!(!via.cb2());
        assert_eq!(via.cb2_pulses(), 0);
        via.tick();
        assert!(!via.cb2());
        via.set_cb1(false);
        assert!(via.cb2());

        via.write(0x0C, 0x40);
        via.write(0x0E, 0x88);
        via.set_cb2(false);
        assert_eq!(
            via.peek(0x0D) & 0x08,
            0,
            "positive mode ignores a falling edge"
        );
        via.set_cb2(true);
        assert_eq!(via.peek(0x0D) & 0x08, 0x08);
        via.write(0x00, 0x00);
        assert_eq!(
            via.peek(0x0D) & 0x08,
            0,
            "CB2 input handshake clears on ORB"
        );
    }
}

/// Shift-register mode, ACR bits 4–2 (datasheet table "Shift Register Control").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftMode {
    Disabled,
    InUnderT2,
    InUnderPhi2,
    InUnderCb1,
    OutFreeRunT2,
    OutUnderT2,
    OutUnderPhi2,
    OutUnderCb1,
}

/// Decoded Auxiliary Control Register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Acr {
    /// Bit 7: timer 1 drives PB7.
    pub t1_pb7: bool,
    /// Bit 6: timer 1 continuous (free-run) instead of one-shot.
    pub t1_free_run: bool,
    /// Bit 5: timer 2 counts PB6 falling edges instead of PHI2.
    pub t2_counts_pb6: bool,
    pub shift: ShiftMode,
    /// Bit 1 / bit 0: port B / port A input latching on CB1 / CA1.
    pub pb_latch: bool,
    pub pa_latch: bool,
}

impl Acr {
    const SHIFT: [ShiftMode; 8] = [
        ShiftMode::Disabled,
        ShiftMode::InUnderT2,
        ShiftMode::InUnderPhi2,
        ShiftMode::InUnderCb1,
        ShiftMode::OutFreeRunT2,
        ShiftMode::OutUnderT2,
        ShiftMode::OutUnderPhi2,
        ShiftMode::OutUnderCb1,
    ];

    pub fn decode(v: u8) -> Self {
        Self {
            t1_pb7: v & 0x80 != 0,
            t1_free_run: v & 0x40 != 0,
            t2_counts_pb6: v & 0x20 != 0,
            shift: Self::SHIFT[usize::from((v >> 2) & 7)],
            pb_latch: v & 0x02 != 0,
            pa_latch: v & 0x01 != 0,
        }
    }

    pub fn encode(&self) -> u8 {
        let shift = Self::SHIFT
            .iter()
            .position(|&m| m == self.shift)
            .expect("every mode is listed") as u8;
        (u8::from(self.t1_pb7) << 7)
            | (u8::from(self.t1_free_run) << 6)
            | (u8::from(self.t2_counts_pb6) << 5)
            | (shift << 2)
            | (u8::from(self.pb_latch) << 1)
            | u8::from(self.pa_latch)
    }
}

/// CA2 / CB2 control, PCR bits 3–1 / 7–5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum C2Mode {
    /// Input; the flag sets on the active edge and clears on an ORA/ORB access.
    Input { positive: bool },
    /// Input whose flag an ORA/ORB access does not clear.
    IndependentInput { positive: bool },
    /// Output low after an ORA/ORB access (read for CA2, write for CB2) until the
    /// next active C1 edge.
    Handshake,
    /// Output low for one cycle after the access.
    Pulse,
    /// Output held at this level.
    Manual { high: bool },
}

/// Decoded Peripheral Control Register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pcr {
    /// CA1 / CB1 active edge: `true` rising, `false` falling.
    pub ca1_positive: bool,
    pub ca2: C2Mode,
    pub cb1_positive: bool,
    pub cb2: C2Mode,
}

fn c2_decode(bits: u8) -> C2Mode {
    match bits & 7 {
        0 => C2Mode::Input { positive: false },
        1 => C2Mode::IndependentInput { positive: false },
        2 => C2Mode::Input { positive: true },
        3 => C2Mode::IndependentInput { positive: true },
        4 => C2Mode::Handshake,
        5 => C2Mode::Pulse,
        6 => C2Mode::Manual { high: false },
        _ => C2Mode::Manual { high: true },
    }
}

fn c2_encode(m: C2Mode) -> u8 {
    match m {
        C2Mode::Input { positive } => u8::from(positive) << 1,
        C2Mode::IndependentInput { positive } => 1 | (u8::from(positive) << 1),
        C2Mode::Handshake => 4,
        C2Mode::Pulse => 5,
        C2Mode::Manual { high } => 6 | u8::from(high),
    }
}

impl Pcr {
    pub fn decode(v: u8) -> Self {
        Self {
            ca1_positive: v & 0x01 != 0,
            ca2: c2_decode(v >> 1),
            cb1_positive: v & 0x10 != 0,
            cb2: c2_decode(v >> 5),
        }
    }

    pub fn encode(&self) -> u8 {
        u8::from(self.ca1_positive)
            | (c2_encode(self.ca2) << 1)
            | (u8::from(self.cb1_positive) << 4)
            | (c2_encode(self.cb2) << 5)
    }
}

impl Via6522 {
    pub fn acr(&self) -> Acr {
        Acr::decode(self.peek(0x0B))
    }

    pub fn pcr(&self) -> Pcr {
        Pcr::decode(self.peek(0x0C))
    }

    /// Interrupt sources that are both flagged and enabled, IFR bit order
    /// (0 CA2, 1 CA1, 2 SR, 3 CB2, 4 CB1, 5 T2, 6 T1).
    pub fn pending_sources(&self) -> u8 {
        self.peek(0x0D) & self.peek(0x0E) & 0x7F
    }
}
