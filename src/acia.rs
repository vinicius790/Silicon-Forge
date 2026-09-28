//! Simplified MOS 6551 ACIA.
//!
//! One-byte TX and RX holding registers with the status bits a host program polls:
//! TDRE (bit 4, set when the CPU may write another byte) and RDRF (bit 3, set when a
//! byte is waiting to be read). Overrun (bit 2) is set when a second RX byte arrives
//! before the first is read, and it clears when status is read. Bit 7 (IRQ) latches
//! on a rising RDRF (DTR on, command bit 1 clear) or TDRE (DTR on, command bits 3–2
//! `01`) and clears on a status read. Hardware reset leaves command `$02` and status
//! `$10`. A programmed reset clears command bits 0–4 and keeps the parity bits.
//!
//! Command and control registers are stored and read back. Writing the control
//! register copies bits 0–3 through [`BAUD_PERIOD_BY_NIBBLE`] into the countdown
//! period. That table is a host cycle count; [`Acia6551::set_crystal`] switches to the
//! 1.8432 MHz crystal and its 16x divider, timing a whole frame.
//! [`Acia6551::set_baud_period`] still overrides the period until the next control
//! write. Each [`Acia6551::tick`] is one cycle. After that many ticks TDRE returns
//! (the written byte stays readable with [`Acia6551::pop_tx`]) and a byte queued by
//! [`Acia6551::push_rx`] raises RDRF. A period of 0 keeps the immediate holding
//! register. `Cpu::step` does not call `tick`. Optional loopback copies each
//! accepted TX byte into RX, immediately when the period is 0 and otherwise
//! through the RX countdown.
//!
//! Control-register baud nibble (bits 0–3), nominal 6551 rate, and the period
//! in [`Acia6551::tick`] cycles (bit time at a nominal 1 MHz PHI2, rounded):
//!
//! | nibble | nominal baud | period |
//! | --- | --- | --- |
//! | `0x0` | 16× external clock | 0 (immediate) |
//! | `0x1` | 50 | 20000 |
//! | `0x2` | 75 | 13333 |
//! | `0x3` | 109.92 | 9091 |
//! | `0x4` | 134.58 | 7431 |
//! | `0x5` | 150 | 6667 |
//! | `0x6` | 300 | 3333 |
//! | `0x7` | 600 | 1667 |
//! | `0x8` | 1200 | 833 |
//! | `0x9` | 1800 | 556 |
//! | `0xA` | 2400 | 417 |
//! | `0xB` | 3600 | 278 |
//! | `0xC` | 4800 | 208 |
//! | `0xD` | 7200 | 139 |
//! | `0xE` | 9600 | 104 |
//! | `0xF` | 19200 | 52 |

use std::collections::VecDeque;

use crate::bus::{FlatMemory, IoDevice, MemoryBus};
use crate::cycles::BusPhase;

pub const RDRF: u8 = 1 << 3;
pub const TDRE: u8 = 1 << 4;
pub const OVERRUN: u8 = 1 << 2;
/// Status bit 7: an enabled receiver or transmitter interrupt is pending.
pub const IRQ: u8 = 1 << 7;
/// Command register after a hardware reset: DTR off, receiver IRQ disabled (bit 1).
pub const COMMAND_AFTER_RESET: u8 = 0x02;

/// Countdown selected by the control-register baud nibble. See the module docs.
/// Nibble 0 is the external clock (period 0). This is not a crystal oscillator.
pub const BAUD_PERIOD_BY_NIBBLE: [u32; 16] = [
    0, 20_000, 13_333, 9_091, 7_431, 6_667, 3_333, 1_667, 833, 556, 417, 278, 208, 139, 104, 52,
];

/// Standard 6551 crystal, Hz.
pub const CRYSTAL_HZ: u32 = 1_843_200;

/// Internal divider from the crystal to the 16x clock, by baud nibble. Nibble 0 is the
/// external 16x clock on RxC, which this model treats as immediate.
pub const CRYSTAL_DIVIDER_BY_NIBBLE: [u32; 16] = [
    0, 2304, 1536, 1047, 857, 768, 384, 192, 96, 64, 48, 32, 24, 16, 12, 6,
];

/// Crystal periods for one whole frame: start, data, parity and stop bits, each 16
/// clocks of the 16x clock. Two stop bits are 1.5 with 5 data bits and no parity, and 1
/// with 8 data bits and parity.
pub fn crystal_frame_period(control: u8, command: u8) -> u32 {
    frame_16x_clocks(control, command) * CRYSTAL_DIVIDER_BY_NIBBLE[(control & 0x0F) as usize]
}

/// 16x clocks in one frame (the period on an external 16x clock, baud nibble 0).
pub fn frame_16x_clocks(control: u8, command: u8) -> u32 {
    let data = 8 - u32::from((control >> 5) & 0x03);
    let parity = u32::from(command & 0x20 != 0);
    16 * (1 + data + parity) + 8 * u32::from(stop_halves(control, command))
}

/// Stop bits in half bits: control bit 7 clear is 1 (2 halves); set is 2, except 1.5
/// with 5 data bits and no parity and 1 with 8 data bits and parity (6551 data sheet).
pub fn stop_halves(control: u8, command: u8) -> u8 {
    let data = 8 - ((control >> 5) & 0x03);
    match (control & 0x80 != 0, data, command & 0x20 != 0) {
        (false, _, _) => 2,
        (true, 5, false) => 3,
        (true, 8, true) => 2,
        (true, _, _) => 4,
    }
}

/// Period installed when the control register is written.
pub fn baud_period_from_control(control: u8) -> u32 {
    BAUD_PERIOD_BY_NIBBLE[(control & 0x0F) as usize]
}

/// Four-register 6551 window: data, status/reset, command, control.
#[derive(Debug, Clone)]
pub struct Acia6551 {
    tx: Option<u8>,
    rx: Option<u8>,
    command: u8,
    control: u8,
    overrun: bool,
    loopback: bool,
    /// Bytes the host has already taken, kept so tests can inspect history.
    taken: VecDeque<u8>,
    /// Host-supplied cycles until TDRE/RDRF. Zero means immediate (no countdown).
    baud_period: u32,
    /// Cycles remaining before TDRE is set again after a write. Zero when idle.
    tx_left: u32,
    /// Cycles remaining before a queued RX byte raises RDRF.
    rx_left: u32,
    /// Byte waiting out the RX countdown. Not visible to the CPU yet.
    rx_waiting: Option<u8>,
    parity_error: bool,
    framing_error: bool,
    last_frame: Option<AciaFrame>,
    /// Good immediate-loopback bytes delivered since the receiver was read.
    rx_loop_unread: u8,
    /// Status bit 7 / IRQ pin. Set when RDRF rises with DTR on and command bit 1
    /// clear, or TDRE rises with DTR on and transmitter control (bits 3–2) `01`.
    /// Reading status clears it.
    irq: bool,
    /// RDRF and TDRE as last seen, for the rising-edge test of `irq`.
    seen_rdrf: bool,
    seen_tdre: bool,
    /// Modem inputs, true = pin high (inactive). Host-driven; not in the snapshot.
    dcd_high: bool,
    dsr_high: bool,
    cts_high: bool,
    /// Bytes the receiver retransmitted in echo mode, oldest first.
    echoed: VecDeque<u8>,
    /// CPU clock in Hz when the 1.8432 MHz crystal drives the countdowns.
    crystal_cpu_hz: Option<u32>,
    /// Crystal phase carried between CPU half cycles, in units of 1/(2·`cpu_hz`)
    /// crystal periods.
    crystal_acc: u64,
    /// Baud nibble 0 counts host pulses from [`Acia6551::clock_16x`].
    external_clock: bool,
    /// Bit-level receiver fed by [`Acia6551::rxd_16x`].
    line_rx: SerialRx,
    /// 16x levels of the frame being shifted out on TxD, next level last.
    line_tx: Vec<bool>,
    variant: AciaVariant,
    /// Start bits the line receiver rejected because RxD was high again at the
    /// middle of the start bit (noise shorter than half a bit).
    false_starts: u32,
    /// Last frame from the line receiver was a break: all data, parity and the
    /// stop bit low.
    rx_break: bool,
}

/// Which 6551 part is modelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AciaVariant {
    /// MOS/Rockwell/Synertek 6551 and R6551: TDRE (status bit 4) follows the
    /// transmit holding register.
    #[default]
    Classic,
    /// WDC W65C51N. WDC's datasheet documents that status bit 4 (TDRE) always
    /// reads 1 and that writing the transmit register while a byte is shifting out
    /// corrupts it, so the transmit interrupt never fires and software must time
    /// each byte itself (one frame at the programmed baud rate).
    WdcW65c51n,
}

/// Data bits, optional parity bit, and stop-bit count programmed into the 6551.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AciaFrame {
    pub data: u8,
    pub parity: Option<bool>,
    /// Stop bits in half bits (2 = 1, 3 = 1.5, 4 = 2), see [`stop_halves`].
    pub stop_halves: u8,
}

/// Frame the control and command registers would send for `data`.
pub fn programmed_frame(data: u8, command: u8, control: u8) -> AciaFrame {
    let bits = match (control >> 5) & 0x03 {
        0b00 => 8,
        0b01 => 7,
        0b10 => 6,
        _ => 5,
    };
    let data = if bits >= 8 {
        data
    } else {
        data & ((1u8 << bits) - 1)
    };
    AciaFrame {
        data,
        parity: parity_bit(data, command),
        stop_halves: stop_halves(control, command),
    }
}

fn parity_bit(data: u8, command: u8) -> Option<bool> {
    if command & 0x20 == 0 {
        return None;
    }
    Some(match command >> 6 {
        0b00 => data.count_ones() % 2 == 0,
        0b01 => data.count_ones() % 2 == 1,
        0b10 => true,
        _ => false,
    })
}

impl Default for Acia6551 {
    fn default() -> Self {
        Self::new()
    }
}

impl Acia6551 {
    pub fn new() -> Self {
        Self {
            tx: None,
            rx: None,
            command: COMMAND_AFTER_RESET,
            control: 0,
            overrun: false,
            loopback: false,
            taken: VecDeque::new(),
            baud_period: 0,
            tx_left: 0,
            rx_left: 0,
            rx_waiting: None,
            parity_error: false,
            framing_error: false,
            last_frame: None,
            rx_loop_unread: 0,
            irq: false,
            seen_rdrf: false,
            seen_tdre: true,
            dcd_high: false,
            dsr_high: false,
            cts_high: false,
            echoed: VecDeque::new(),
            crystal_cpu_hz: None,
            crystal_acc: 0,
            external_clock: false,
            line_rx: SerialRx::default(),
            line_tx: Vec::new(),
            variant: AciaVariant::Classic,
            false_starts: 0,
            rx_break: false,
        }
    }

    pub fn with_variant(variant: AciaVariant) -> Self {
        Self {
            variant,
            ..Self::new()
        }
    }

    pub fn variant(&self) -> AciaVariant {
        self.variant
    }

    /// Start edges the line receiver rejected as noise (see [`Acia6551::rxd_16x`]).
    pub fn false_starts(&self) -> u32 {
        self.false_starts
    }

    /// The last frame taken from RxD was a break (every bit including the stop bit
    /// low). Such a frame arrives as `$00` with the framing error bit, exactly as
    /// [`Acia6551::receive_break`] presents one; this flag tells the two apart
    /// from a genuine `$00` with a bad stop bit only when the data was also zero.
    pub fn rx_break_detected(&self) -> bool {
        self.rx_break
    }

    /// Baud nibble 0 takes its 16x clock from the host: each [`clock_16x`](Self::clock_16x)
    /// is one clock, a frame is [`frame_16x_clocks`], and `tick` does not count.
    /// Off keeps nibble 0 immediate.
    pub fn set_external_clock(&mut self, on: bool) {
        self.external_clock = on;
        self.baud_period = self.programmed_period();
    }

    /// One edge of the external 16x clock (only counts on baud nibble 0).
    pub fn clock_16x(&mut self) {
        if self.on_external_clock() {
            self.tick_counters();
            self.sync_irq();
        }
    }

    fn on_external_clock(&self) -> bool {
        self.external_clock && self.control & 0x0F == 0
    }

    /// One 16x-clock sample of the RxD line. A complete frame (start, data LSB first,
    /// parity, first stop bit) goes through [`Acia6551::receive_framed`] with the
    /// sampled parity and stop level, so status bits 0/1 report mismatches. Word length
    /// and parity come from the control and command registers at the start bit.
    pub fn rxd_16x(&mut self, high: bool) {
        let bits = data_bits(self.control);
        let parity = self.command & 0x20 != 0;
        let was_start = self.line_rx.phase == RxPhase::Start;
        let sampled = self.line_rx.sample(high, bits, parity);
        if was_start && self.line_rx.phase == RxPhase::Idle {
            self.false_starts = self.false_starts.wrapping_add(1);
        }
        if let Some((data, par, stop)) = sampled {
            self.rx_break = data == 0 && par != Some(true) && !stop;
            self.receive_framed(data, par, stop);
        }
    }

    /// True while the bit-level receiver is inside a frame.
    /// One 16x clock of the bit-level transmitter; returns the TxD level. When the
    /// line is idle it takes the byte in the TX holding register (as
    /// [`Acia6551::pop_tx`] does, so CTS high holds it and TDRE returns at once) and
    /// shifts out start bit, data LSB first, parity and stop bits at 16 clocks per
    /// bit in the format programmed at that moment. A break (command bits 3–2
    /// `11`) drives TxD low and abandons a frame in progress. Idle is high.
    pub fn txd_16x(&mut self) -> bool {
        if self.tx_break() {
            self.line_tx.clear();
            return false;
        }
        if self.line_tx.is_empty() {
            if let Some(byte) = self.pop_tx() {
                let frame = programmed_frame(byte, self.command, self.control);
                self.line_tx = frame_levels(&frame, data_bits(self.control));
                self.line_tx.reverse();
            }
        }
        self.line_tx.pop().unwrap_or(true)
    }

    /// A frame is being shifted out on TxD.
    pub fn tx_line_busy(&self) -> bool {
        !self.line_tx.is_empty()
    }

    pub fn rx_line_busy(&self) -> bool {
        self.line_rx.phase != RxPhase::Idle
    }

    /// TxD level: low while a break is sent.
    pub fn txd_high(&self) -> bool {
        !self.tx_break()
    }

    /// Drive the countdowns from the 1.8432 MHz crystal at a CPU clock of `cpu_hz`.
    /// The period becomes a whole frame in crystal periods ([`crystal_frame_period`]),
    /// recomputed on every control or command write, and each [`tick`](Self::tick)
    /// advances `CRYSTAL_HZ / cpu_hz` crystal periods with the remainder carried.
    /// `None` restores the host cycle table.
    pub fn set_crystal(&mut self, cpu_hz: Option<u32>) {
        self.crystal_cpu_hz = cpu_hz.filter(|&hz| hz != 0);
        self.crystal_acc = 0;
        self.baud_period = self.programmed_period();
    }

    fn programmed_period(&self) -> u32 {
        if self.on_external_clock() {
            return frame_16x_clocks(self.control, self.command);
        }
        match self.crystal_cpu_hz {
            Some(_) => crystal_frame_period(self.control, self.command),
            None => baud_period_from_control(self.control),
        }
    }

    /// RTS output asserted (pin low): command bits 3–2 not `00`.
    pub fn rts_asserted(&self) -> bool {
        self.command & 0x0C != 0
    }

    /// DTR output asserted (pin low): command bit 0.
    pub fn dtr_asserted(&self) -> bool {
        self.command & 0x01 != 0
    }

    /// TxD held low as a break: command bits 3–2 `11` (transmit IRQ off, RTS low).
    pub fn tx_break(&self) -> bool {
        self.command & 0x0C == 0x0C
    }

    /// Host presents a received break (RxD low for a whole frame): a `$00` byte with
    /// the framing error bit, through the normal receive path.
    pub fn receive_break(&mut self) {
        if !self.receiver_enabled() {
            return;
        }
        self.parity_error = false;
        self.framing_error = true;
        self.queue_rx(0);
        self.sync_irq();
    }

    /// Receiver on: DTR (command bit 0) set and carrier present (DCD low).
    fn receiver_enabled(&self) -> bool {
        self.command & 0x01 != 0 && !self.dcd_high
    }

    /// Echo mode: command bit 4 with transmitter control bits 3–2 at `00`.
    fn echo_mode(&self) -> bool {
        self.command & 0x1C == 0x10
    }

    /// Drive /DCD. High means no carrier: status bit 5 sets and the receiver stops.
    /// A change raises IRQ when the receiver interrupt is enabled.
    pub fn set_dcd(&mut self, high: bool) {
        if self.dcd_high != high && self.rx_irq_enabled() {
            self.irq = true;
        }
        self.dcd_high = high;
    }

    /// Drive /DSR. High sets status bit 6. A change raises IRQ like DCD.
    pub fn set_dsr(&mut self, high: bool) {
        if self.dsr_high != high && self.rx_irq_enabled() {
            self.irq = true;
        }
        self.dsr_high = high;
    }

    /// Drive /CTS. High disables the transmitter: the byte stays in the holding
    /// register, the baud countdown holds, and TDRE does not return. No status bit.
    pub fn set_cts(&mut self, high: bool) {
        self.cts_high = high;
        self.sync_irq();
    }

    /// Byte the receiver echoed back out (echo mode), oldest first.
    pub fn pop_echo(&mut self) -> Option<u8> {
        self.echoed.pop_front()
    }

    /// IRQ output (active while status bit 7 is set).
    pub fn irq_asserted(&self) -> bool {
        self.irq
    }

    fn rx_irq_enabled(&self) -> bool {
        self.command & 0x03 == 0x01
    }

    /// Never on the W65C51N, whose stuck TDRE has no rising edge to report.
    fn tx_irq_enabled(&self) -> bool {
        self.variant != AciaVariant::WdcW65c51n && self.command & 0x0D == 0x05
    }

    /// Latch an IRQ on a rising RDRF or TDRE whose interrupt is enabled.
    fn sync_irq(&mut self) {
        let rdrf = self.rx.is_some();
        let tdre = self.tdre_ready();
        if (rdrf && !self.seen_rdrf && self.rx_irq_enabled())
            || (tdre && !self.seen_tdre && self.tx_irq_enabled())
        {
            self.irq = true;
        }
        self.seen_rdrf = rdrf;
        self.seen_tdre = tdre;
    }

    pub fn set_loopback(&mut self, enabled: bool) {
        self.loopback = enabled;
    }

    pub fn status(&self) -> u8 {
        let mut status = 0;
        if self.rx.is_some() {
            status |= RDRF;
        }
        if self.tdre_ready() {
            status |= TDRE;
        }
        if self.overrun {
            status |= OVERRUN;
        }
        if self.parity_error {
            status |= 1;
        }
        if self.framing_error {
            status |= 1 << 1;
        }
        if self.dcd_high {
            status |= 1 << 5;
        }
        if self.dsr_high {
            status |= 1 << 6;
        }
        if self.irq {
            status |= IRQ;
        }
        status
    }

    /// Cycles the host must [`tick`](Self::tick) before TDRE returns after a write.
    /// `0` disables the countdown (TDRE tracks the holding register directly).
    pub fn set_baud_period(&mut self, cycles: u32) {
        self.baud_period = cycles;
    }

    pub fn baud_period(&self) -> u32 {
        self.baud_period
    }

    /// Cycles still to go before TDRE is set for the current TX byte.
    pub fn tx_cycles_left(&self) -> u32 {
        self.tx_left
    }

    /// Cycles still to go before a queued RX byte sets RDRF.
    pub fn rx_cycles_left(&self) -> u32 {
        self.rx_left
    }

    /// Advance the baud countdown by one cycle (both phases). Not called by `Cpu::step`.
    pub fn tick(&mut self) {
        self.tick_phase(BusPhase::Phi1);
        self.tick_phase(BusPhase::Phi2);
    }

    /// One half cycle. The crystal runs through both phases (`CRYSTAL_HZ / 2·cpu_hz`
    /// periods each, remainder carried); the host cycle table counts on PHI2. On the
    /// external clock neither counts.
    pub fn tick_phase(&mut self, phase: BusPhase) {
        if self.on_external_clock() {
            return;
        }
        match self.crystal_cpu_hz {
            Some(hz) => {
                let half = 2 * u64::from(hz);
                self.crystal_acc += u64::from(CRYSTAL_HZ);
                let periods = self.crystal_acc / half;
                self.crystal_acc %= half;
                for _ in 0..periods {
                    self.tick_counters();
                }
            }
            None if phase == BusPhase::Phi2 => self.tick_counters(),
            None => {}
        }
        self.sync_irq();
    }

    fn tick_counters(&mut self) {
        if self.tx_left > 0 && !self.cts_high && !self.tx_break() {
            self.tx_left -= 1;
        }
        if self.rx_left > 0 {
            self.rx_left -= 1;
            if self.rx_left == 0 {
                if let Some(byte) = self.rx_waiting.take() {
                    self.deliver_rx(byte);
                }
            }
        }
    }

    fn tdre_ready(&self) -> bool {
        if self.variant == AciaVariant::WdcW65c51n {
            return true;
        }
        if self.tx_left > 0 {
            false
        } else if self.baud_period == 0 {
            self.tx.is_none()
        } else {
            true
        }
    }

    /// Host injects a byte toward the CPU. If RX is already full the new byte is
    /// dropped and overrun is set (the unread byte stays). With a non-zero baud
    /// period the byte waits in a shift countdown and RDRF stays clear until
    /// [`tick`](Self::tick) expires it.
    pub fn push_rx(&mut self, byte: u8) {
        self.queue_rx(byte);
        self.sync_irq();
    }

    fn queue_rx(&mut self, byte: u8) {
        if !self.receiver_enabled() {
            return;
        }
        if self.echo_mode() {
            self.echoed.push_back(byte);
        }
        if self.baud_period == 0 {
            self.deliver_rx(byte);
            return;
        }
        if self.rx.is_some() || self.rx_waiting.is_some() {
            self.overrun = true;
            return;
        }
        self.rx_waiting = Some(byte);
        self.rx_left = self.baud_period;
    }

    fn deliver_rx(&mut self, byte: u8) {
        if self.rx.is_some() {
            self.overrun = true;
            return;
        }
        self.rx = Some(byte);
    }

    /// Host takes the byte the CPU wrote, clearing the holding register (TDRE returns).
    pub fn pop_tx(&mut self) -> Option<u8> {
        if self.cts_high || self.tx_break() {
            return None;
        }
        let byte = self.tx.take()?;
        self.tx_left = 0;
        self.taken.push_back(byte);
        self.sync_irq();
        Some(byte)
    }

    pub fn tx_pending(&self) -> bool {
        self.tx.is_some()
    }

    /// Programmed reset (any write to the status register): command bits 0–4 clear,
    /// parity bits 5–7 and the control register stay, overrun and IRQ clear.
    fn reset_programming(&mut self) {
        self.tx = None;
        self.rx = None;
        self.overrun = false;
        self.irq = false;
        self.command &= 0xE0;
        self.tx_left = 0;
        self.rx_left = 0;
        self.rx_waiting = None;
        self.parity_error = false;
        self.framing_error = false;
        self.last_frame = None;
        self.rx_loop_unread = 0;
    }

    fn write_data(&mut self, value: u8) {
        self.tx = Some(value);
        self.tx_left = self.baud_period;
        if self.loopback {
            let frame = programmed_frame(value, self.command, self.control);
            self.last_frame = Some(frame);
            self.receive_framed(frame.data, frame.parity, true);
        }
    }

    /// Host presents one received frame. The data byte is masked to the programmed
    /// word length. Parity and stop level are checked against the command and control
    /// registers. Status bit 0 is the parity mismatch. Status bit 1 is a low stop level.
    pub fn receive_framed(&mut self, data: u8, parity: Option<bool>, stop_level_high: bool) {
        self.receive_frame(data, parity, stop_level_high);
        self.sync_irq();
    }

    fn receive_frame(&mut self, data: u8, parity: Option<bool>, stop_level_high: bool) {
        if !self.receiver_enabled() {
            return;
        }
        let expected = programmed_frame(data, self.command, self.control);
        let parity_error = expected.parity != parity;
        let framing_error = !stop_level_high;
        // Data sheet priority: the error bits describe the byte in the receiver
        // data register. A byte that overruns is lost, so it sets only the
        // overrun bit; parity and framing keep describing the unread byte.
        if !self.loopback && (self.rx.is_some() || self.rx_waiting.is_some()) {
            self.overrun = true;
            return;
        }
        self.parity_error = parity_error;
        self.framing_error = framing_error;
        if self.loopback && (self.parity_error || self.framing_error) {
            return;
        }
        // Two good loopback bytes: the second replaces the first in the receiver.
        // A third while those two are still unread sets overrun and leaves the
        // second byte. RDRF stays set until that register is read. A non-zero
        // baud period still uses the shift countdown and overrun rules of `push_rx`.
        if self.loopback && self.baud_period == 0 {
            if self.rx_loop_unread >= 2 {
                self.overrun = true;
                return;
            }
            self.rx = Some(expected.data);
            self.rx_loop_unread = self.rx_loop_unread.saturating_add(1);
            return;
        }
        self.queue_rx(expected.data);
    }

    /// Loopback receiver. Wrong parity sets status bit 0 and does not store the
    /// byte in the receiver data register. A frame that matches the programmed
    /// parity does. No effect unless [`set_loopback`](Self::set_loopback) is on.
    pub fn loopback_frame(&mut self, data: u8, parity: Option<bool>) {
        self.loopback_frame_with_stop(data, parity, true);
    }

    /// Loopback frame with an explicit stop level. A low stop sets status bit 1
    /// and does not store a clean byte. No effect unless loopback is on.
    pub fn loopback_frame_with_stop(
        &mut self,
        data: u8,
        parity: Option<bool>,
        stop_level_high: bool,
    ) {
        if !self.loopback {
            return;
        }
        self.receive_framed(data, parity, stop_level_high);
    }

    /// Frame built by the last loopback transmit, if any.
    pub fn last_frame(&self) -> Option<AciaFrame> {
        self.last_frame
    }
}

/// Registers kept by [`Acia6551`]. The host-side `taken` history is not part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AciaState {
    pub tx: Option<u8>,
    pub rx: Option<u8>,
    pub command: u8,
    pub control: u8,
    pub overrun: bool,
    pub loopback: bool,
    pub baud_period: u32,
    pub tx_left: u32,
    pub rx_left: u32,
    pub rx_waiting: Option<u8>,
    /// Status bit 7. Images written before it existed load it as false.
    pub irq: bool,
    /// Host-driven /DCD, /DSR, /CTS levels (`true` = high). Older images: low.
    pub dcd_high: bool,
    pub dsr_high: bool,
    pub cts_high: bool,
    /// Echo bytes not yet taken by the host. Older images: empty.
    pub echoed: Vec<u8>,
    /// Crystal mode CPU clock and carried phase. Older images: table mode.
    pub crystal_cpu_hz: Option<u32>,
    pub crystal_acc: u64,
    /// External 16x clock on baud nibble 0. Older images: off.
    pub external_clock: bool,
    /// TxD levels still to shift out (16 per bit), in send order. Older images: idle.
    pub tx_line: Vec<bool>,
    /// Bit-level receiver: phase (0 idle, 1 start, 2 bits), 16x count, bit index,
    /// shift (LE), last level, word length, parity enabled. Older images: idle.
    pub rx_line: [u8; 8],
}

impl Acia6551 {
    pub fn state(&self) -> AciaState {
        AciaState {
            tx: self.tx,
            rx: self.rx,
            command: self.command,
            control: self.control,
            overrun: self.overrun,
            loopback: self.loopback,
            baud_period: self.baud_period,
            tx_left: self.tx_left,
            rx_left: self.rx_left,
            rx_waiting: self.rx_waiting,
            irq: self.irq,
            dcd_high: self.dcd_high,
            dsr_high: self.dsr_high,
            cts_high: self.cts_high,
            echoed: self.echoed.iter().copied().collect(),
            crystal_cpu_hz: self.crystal_cpu_hz,
            crystal_acc: self.crystal_acc,
            external_clock: self.external_clock,
            tx_line: self.line_tx.iter().rev().copied().collect(),
            rx_line: self.line_rx.pack(),
        }
    }

    pub fn load_state(&mut self, state: &AciaState) {
        self.tx = state.tx;
        self.rx = state.rx;
        self.command = state.command;
        self.control = state.control;
        self.overrun = state.overrun;
        self.loopback = state.loopback;
        self.baud_period = state.baud_period;
        self.tx_left = state.tx_left;
        self.rx_left = state.rx_left;
        self.rx_waiting = state.rx_waiting;
        self.irq = state.irq;
        self.dcd_high = state.dcd_high;
        self.dsr_high = state.dsr_high;
        self.cts_high = state.cts_high;
        self.echoed = state.echoed.iter().copied().collect();
        self.crystal_cpu_hz = state.crystal_cpu_hz.filter(|&hz| hz != 0);
        self.crystal_acc = state.crystal_acc;
        self.external_clock = state.external_clock;
        self.seen_rdrf = self.rx.is_some();
        self.seen_tdre = self.tdre_ready();
        self.parity_error = false;
        self.framing_error = false;
        self.last_frame = None;
        self.rx_loop_unread = 0;
        self.taken.clear();
        self.line_tx = state.tx_line.iter().rev().copied().collect();
        self.line_rx = SerialRx::unpack(state.rx_line).unwrap_or_default();
    }

    /// Whether [`AciaState::rx_line`] decodes; [`Acia6551::load_state`] treats an
    /// invalid value as an idle receiver.
    pub fn rx_line_valid(packed: [u8; 8]) -> bool {
        SerialRx::unpack(packed).is_some()
    }
}

impl IoDevice for Acia6551 {
    fn phase(&mut self, phase: BusPhase) {
        self.tick_phase(phase);
    }

    fn read(&mut self, offset: u16) -> u8 {
        match offset & 0x03 {
            0 => {
                let byte = self.rx.unwrap_or(0);
                self.rx = None;
                self.rx_loop_unread = 0;
                if self.loopback {
                    self.overrun = false;
                }
                self.sync_irq();
                byte
            }
            1 => {
                let status = self.status();
                self.irq = false;
                self.overrun = false;
                self.parity_error = false;
                self.framing_error = false;
                status
            }
            2 => self.command,
            _ => self.control,
        }
    }

    fn write(&mut self, offset: u16, value: u8) {
        match offset & 0x03 {
            0 => self.write_data(value),
            1 => self.reset_programming(),
            2 => {
                // Entering break cuts the byte being sent: it never reaches the line.
                if value & 0x0C == 0x0C && !self.tx_break() {
                    self.tx = None;
                    self.tx_left = 0;
                }
                // Enabling an interrupt while its condition already holds raises it.
                self.command = value;
                self.seen_rdrf = false;
                self.seen_tdre = false;
                if self.crystal_cpu_hz.is_some() || self.on_external_clock() {
                    self.baud_period = self.programmed_period();
                }
            }
            _ => {
                self.control = value;
                self.baud_period = self.programmed_period();
            }
        }
        self.sync_irq();
    }

    fn peek(&self, offset: u16) -> u8 {
        match offset & 0x03 {
            0 => self.rx.unwrap_or(0),
            1 => self.status(),
            2 => self.command,
            _ => self.control,
        }
    }
}

/// 64 KiB RAM with an ACIA decoded at `base..=base+3`.
///
/// The device wins over RAM in that window. The CPU can `STA` the data register and
/// the host reads it back with [`Acia6551::pop_tx`] on the public `acia` field.
#[derive(Clone)]
pub struct RamAcia {
    pub memory: FlatMemory,
    pub acia: Acia6551,
    pub base: u16,
}

impl RamAcia {
    pub fn new(base: u16) -> Self {
        Self {
            memory: FlatMemory::new(),
            acia: Acia6551::new(),
            base,
        }
    }

    fn in_acia(&self, addr: u16) -> bool {
        let end = self.base.wrapping_add(3);
        addr >= self.base && addr <= end && self.base <= end
    }
}

impl MemoryBus for RamAcia {
    fn phase(&mut self, phase: BusPhase) {
        self.acia.tick_phase(phase);
    }

    fn read(&mut self, addr: u16) -> u8 {
        if self.in_acia(addr) {
            self.acia.read(addr.wrapping_sub(self.base))
        } else {
            self.memory.read(addr)
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if self.in_acia(addr) {
            self.acia.write(addr.wrapping_sub(self.base), value);
        } else {
            self.memory.write(addr, value);
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        if self.in_acia(addr) {
            self.acia.peek(addr.wrapping_sub(self.base))
        } else {
            self.memory.peek(addr)
        }
    }
}

/// Run `cpu` for exactly `cycles` bus cycles, ticking the 6551 once per cycle.
///
/// Uses [`crate::cpu::Cpu::step_cycle`], not [`crate::cpu::Cpu::step`]. `step` still
/// does not touch the baud countdown. The opcode stream must be cycle-steppable.
pub fn run_cycles_with_acia(cpu: &mut crate::cpu::Cpu<RamAcia>, cycles: u64) -> Result<(), String> {
    for _ in 0..cycles {
        cpu.step_cycle()?;
        cpu.bus.acia.tick();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::assemble;
    use crate::cpu::Cpu;

    #[test]
    fn tdre_clears_on_write_and_returns_when_the_host_takes_the_byte() {
        let mut acia = Acia6551::new();
        assert_eq!(acia.status() & TDRE, TDRE);
        assert_eq!(acia.status() & RDRF, 0);
        acia.write(0, b'A');
        assert_eq!(acia.status() & TDRE, 0);
        assert!(acia.tx_pending());
        assert_eq!(acia.pop_tx(), Some(b'A'));
        assert_eq!(acia.pop_tx(), None);
        assert_eq!(acia.status() & TDRE, TDRE);
    }

    #[test]
    fn rdrf_and_overrun_follow_the_holding_register() {
        let mut acia = Acia6551::new();
        acia.write(2, 0x0B); // DTR on: the receiver is enabled
        acia.push_rx(0x11);
        acia.push_rx(0x22);
        assert_eq!(acia.peek(1) & (RDRF | OVERRUN), RDRF | OVERRUN);
        assert_eq!(acia.read(0), 0x11);
        assert_eq!(acia.read(0), 0);
        assert_eq!(acia.status() & RDRF, 0);
        let status = acia.read(1);
        assert_eq!(status & OVERRUN, OVERRUN);
        assert_eq!(acia.read(1) & OVERRUN, 0);
    }

    #[test]
    fn loopback_copies_tx_into_rx_and_soft_reset_clears_both() {
        let mut acia = Acia6551::new();
        acia.set_loopback(true);
        acia.write(3, 0x10); // nibble 0: external clock, period stays 0
        acia.write(2, 0x0B);
        acia.write(0, 0x5A);
        assert_eq!(acia.read(0), 0x5A);
        assert_eq!(acia.read(2), 0x0B);
        acia.write(0, 0x01);
        acia.write(1, 0x00);
        assert_eq!(acia.status() & TDRE, TDRE);
        assert_eq!(acia.pop_tx(), None);
        assert_eq!(acia.read(2), 0);
        assert_eq!(acia.read(3), 0x10);
        acia.write(3, 0x1F);
        acia.write(1, 0x00);
        assert_eq!(acia.read(3), 0x1F, "soft reset keeps the control register");
        assert_eq!(acia.baud_period(), BAUD_PERIOD_BY_NIBBLE[0x0F]);
    }

    #[test]
    fn assembled_program_stores_a_byte_the_host_can_read() {
        let src = "LDA #$41\nSTA $A000\nBRK\n";
        let program = assemble(src, 0x8000).unwrap();
        let mut bus = RamAcia::new(0xA000);
        bus.memory.load_at(0x8000, &program);
        bus.memory.set_reset_vector(0x8000);
        let mut cpu = Cpu::new(bus);
        cpu.halt_on_brk = true;
        cpu.reset();
        cpu.run_instructions(8).unwrap();
        assert!(cpu.is_halted());
        assert!(cpu.bus.acia.tx_pending());
        assert_eq!(cpu.bus.acia.pop_tx(), Some(0x41));
    }

    #[test]
    fn baud_period_counts_cycles_until_tdre_and_rdrf() {
        let mut acia = Acia6551::new();
        acia.write(2, 0x0B); // DTR on: the receiver is enabled
        acia.set_baud_period(4);
        assert_eq!(acia.baud_period(), 4);
        assert_eq!(acia.status() & TDRE, TDRE);

        acia.write(0, 0x41);
        assert_eq!(acia.status() & TDRE, 0);
        assert_eq!(acia.tx_cycles_left(), 4);
        acia.tick();
        acia.tick();
        acia.tick();
        assert_eq!(acia.tx_cycles_left(), 1);
        assert_eq!(acia.status() & TDRE, 0);
        acia.tick();
        assert_eq!(acia.tx_cycles_left(), 0);
        assert_eq!(
            acia.status() & TDRE,
            TDRE,
            "TDRE returns when the countdown expires"
        );
        assert_eq!(acia.pop_tx(), Some(0x41));

        acia.push_rx(0x11);
        assert_eq!(acia.status() & RDRF, 0);
        assert_eq!(acia.rx_cycles_left(), 4);
        for _ in 0..3 {
            acia.tick();
        }
        assert_eq!(acia.status() & RDRF, 0);
        acia.tick();
        assert_eq!(acia.rx_cycles_left(), 0);
        assert_eq!(acia.status() & RDRF, RDRF);
        assert_eq!(acia.read(0), 0x11);

        // A second byte during the countdown sets overrun and does not replace the first.
        acia.push_rx(0x22);
        acia.push_rx(0x33);
        assert_eq!(acia.peek(1) & OVERRUN, OVERRUN);
        assert_eq!(acia.status() & RDRF, 0);
        for _ in 0..4 {
            acia.tick();
        }
        assert_eq!(acia.read(0), 0x22);
    }

    #[test]
    fn control_nibble_sets_the_period_and_step_does_not_tick_it() {
        let mut acia = Acia6551::new();
        acia.write(3, 0x1E);
        assert_eq!(acia.peek(3), 0x1E);
        assert_eq!(acia.baud_period(), BAUD_PERIOD_BY_NIBBLE[0x0E]);
        acia.write(3, 0x11);
        assert_eq!(acia.baud_period(), 20_000);
        acia.write(0, 0x41);
        assert_eq!(acia.tx_cycles_left(), 20_000);
        let mut bus = RamAcia::new(0xA000);
        bus.acia.write(3, 0x16); // 300 baud -> 3333
        bus.memory.data[0x4000] = 0xEA;
        let mut cpu = Cpu::new(bus);
        cpu.pc = 0x4000;
        let before = cpu.bus.acia.baud_period();
        cpu.step().unwrap();
        assert_eq!(cpu.bus.acia.baud_period(), before);
        assert_eq!(cpu.bus.acia.tx_cycles_left(), 0);
        assert_eq!(before, BAUD_PERIOD_BY_NIBBLE[0x06]);
    }

    #[test]
    fn loopback_checks_programmed_baud_parity_and_stop_bits() {
        let mut acia = Acia6551::new();
        acia.set_loopback(true);
        acia.write(3, 0x8E);
        assert_eq!(acia.baud_period(), BAUD_PERIOD_BY_NIBBLE[0x0E]);
        assert_eq!(
            programmed_frame(0xA5, 0x6B, 0x8E).stop_halves,
            2,
            "8 bits + parity: 1 stop"
        );
        acia.write(3, 0x80);
        assert_eq!(acia.baud_period(), 0, "nibble 0 stays immediate");
        acia.write(2, 0x6B);
        acia.write(0, 0xA5);
        let frame = acia.last_frame().unwrap();
        assert_eq!(frame.data, 0xA5);
        assert_eq!(frame.stop_halves, 2);
        assert_eq!(frame.parity, Some(0xA5u8.count_ones() % 2 == 1));
        assert_eq!(acia.peek(1) & 0x03, 0, "a generated frame matches");
        assert_eq!(acia.read(0), 0xA5);

        acia.write(2, 0x2B);
        acia.write(0, 0xA5);
        assert_eq!(
            acia.last_frame().unwrap().parity,
            Some(0xA5u8.count_ones() % 2 == 0)
        );
        acia.write(2, 0xAB);
        acia.write(0, 0x00);
        assert_eq!(acia.last_frame().unwrap().parity, Some(true));
        acia.write(2, 0xEB);
        acia.write(0, 0xFF);
        assert_eq!(acia.last_frame().unwrap().parity, Some(false));

        let _ = acia.read(0);
        acia.write(3, 0xA0);
        acia.write(2, 0x0B);
        acia.write(0, 0xFF);
        assert_eq!(acia.last_frame().unwrap().data, 0x7F);
        assert_eq!(acia.read(0), 0x7F);

        acia.write(3, 0x00);
        acia.write(2, 0x2B);
        acia.receive_framed(0x01, Some(true), true);
        assert_eq!(acia.peek(1) & 0x01, 0x01, "odd parity rejects a mark bit");
        let _ = acia.read(1);
        acia.receive_framed(0x01, Some(false), false);
        assert_eq!(acia.peek(1) & 0x03, 0x02);
    }
}

/// Data bits for control register bits 6–5 (`00` 8 … `11` 5).
pub fn data_bits(control: u8) -> u8 {
    8 - ((control >> 5) & 3)
}

/// The line levels of `frame` at 16 samples per bit: start (low), data LSB first,
/// optional parity, then the stop bits (high, 8 samples per half bit).
pub fn frame_levels(frame: &AciaFrame, bits: u8) -> Vec<bool> {
    let mut out = vec![false; 16];
    for i in 0..bits {
        out.extend(std::iter::repeat((frame.data >> i) & 1 != 0).take(16));
    }
    if let Some(p) = frame.parity {
        out.extend(std::iter::repeat(p).take(16));
    }
    out.extend(std::iter::repeat(true).take(8 * usize::from(frame.stop_halves)));
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RxPhase {
    #[default]
    Idle,
    Start,
    Bits,
}

/// 16x-oversampling receiver: a falling edge starts a frame, the start bit is
/// confirmed at its middle (sample 8, a glitch shorter than that is ignored), and each
/// later bit is taken at its middle. At power up the line is taken as idle (mark,
/// high), as an RS-232 receiver sees it, so a first low sample is a start edge.
/// Snapshots older than the `rx_line` field load with the last level low.
#[derive(Debug, Clone)]
struct SerialRx {
    phase: RxPhase,
    count: u8,
    index: u8,
    shift: u16,
    last: bool,
    bits: u8,
    parity: bool,
}

impl Default for SerialRx {
    fn default() -> Self {
        Self {
            phase: RxPhase::Idle,
            count: 0,
            index: 0,
            shift: 0,
            last: true,
            bits: 0,
            parity: false,
        }
    }
}

impl SerialRx {
    fn pack(&self) -> [u8; 8] {
        let phase = match self.phase {
            RxPhase::Idle => 0,
            RxPhase::Start => 1,
            RxPhase::Bits => 2,
        };
        let [lo, hi] = self.shift.to_le_bytes();
        [
            phase,
            self.count,
            self.index,
            lo,
            hi,
            u8::from(self.last),
            self.bits,
            u8::from(self.parity),
        ]
    }

    /// `None` for a phase above 2, a flag other than 0/1, or more than 8 data bits.
    fn unpack(b: [u8; 8]) -> Option<Self> {
        let phase = match b[0] {
            0 => RxPhase::Idle,
            1 => RxPhase::Start,
            2 => RxPhase::Bits,
            _ => return None,
        };
        let flag = |v: u8| (v <= 1).then_some(v == 1);
        (b[6] <= 8).then_some(())?;
        Some(Self {
            phase,
            count: b[1],
            index: b[2],
            shift: u16::from_le_bytes([b[3], b[4]]),
            last: flag(b[5])?,
            bits: b[6],
            parity: flag(b[7])?,
        })
    }

    fn sample(&mut self, high: bool, bits: u8, parity: bool) -> Option<(u8, Option<bool>, bool)> {
        let falling = self.last && !high;
        self.last = high;
        match self.phase {
            RxPhase::Idle => {
                if falling {
                    self.phase = RxPhase::Start;
                    self.count = 1;
                    self.bits = bits;
                    self.parity = parity;
                }
                None
            }
            RxPhase::Start => {
                self.count += 1;
                if self.count == 8 {
                    if high {
                        self.phase = RxPhase::Idle; // false start
                    } else {
                        self.phase = RxPhase::Bits;
                        self.count = 0;
                        self.index = 0;
                        self.shift = 0;
                    }
                }
                None
            }
            RxPhase::Bits => {
                self.count += 1;
                if self.count < 16 {
                    return None;
                }
                self.count = 0;
                let total = self.bits + u8::from(self.parity);
                if self.index < total {
                    self.shift |= u16::from(high) << self.index;
                    self.index += 1;
                    return None;
                }
                // Middle of the first stop bit.
                self.phase = RxPhase::Idle;
                let data = (self.shift & ((1 << self.bits) - 1)) as u8;
                let par = self.parity.then(|| (self.shift >> self.bits) & 1 != 0);
                Some((data, par, high))
            }
        }
    }
}

/// Parity mode, command bits 7–5 (bit 5 enables; bits 7–6 choose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Odd,
    Even,
    /// Parity bit always 1 (transmit), not checked on receive.
    Mark,
    /// Parity bit always 0 (transmit), not checked on receive.
    Space,
}

/// Transmitter control, command bits 3–2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxControl {
    /// `00`: TX IRQ off, RTS high (off).
    IrqOffRtsHigh,
    /// `01`: TX IRQ on, RTS low.
    IrqOnRtsLow,
    /// `10`: TX IRQ off, RTS low.
    IrqOffRtsLow,
    /// `11`: TX IRQ off, RTS low, TxD held at break.
    Break,
}

/// Control and command registers decoded per the 6551 data sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AciaConfig {
    /// Bits 3–0: 0 selects the external 16x clock, 1–15 the internal generator.
    pub baud_nibble: u8,
    /// Bit 4: receiver clocked by the baud generator (`true`) or by RxC.
    pub rx_clock_internal: bool,
    pub data_bits: u8,
    /// Stop bits in halves (2, 3 or 4).
    pub stop_halves: u8,
    /// Command bit 0: DTR asserted (receiver and interrupts enabled).
    pub dtr: bool,
    /// Command bit 1 clear: receiver IRQ enabled.
    pub rx_irq: bool,
    pub tx: TxControl,
    /// Command bit 4: receiver echo.
    pub echo: bool,
    pub parity: Parity,
}

/// Standard rates of the internal generator with the 1.8432 MHz crystal, nibble
/// 1–15 (nibble 0 is the external clock).
pub const BAUD_RATES: [f64; 16] = [
    0.0, 50.0, 75.0, 109.92, 134.58, 150.0, 300.0, 600.0, 1200.0, 1800.0, 2400.0, 3600.0, 4800.0,
    7200.0, 9600.0, 19200.0,
];

impl AciaConfig {
    pub fn decode(control: u8, command: u8) -> Self {
        Self {
            baud_nibble: control & 0x0F,
            rx_clock_internal: control & 0x10 != 0,
            data_bits: data_bits(control),
            stop_halves: stop_halves(control, command),
            dtr: command & 0x01 != 0,
            rx_irq: command & 0x02 == 0,
            tx: match (command >> 2) & 3 {
                0 => TxControl::IrqOffRtsHigh,
                1 => TxControl::IrqOnRtsLow,
                2 => TxControl::IrqOffRtsLow,
                _ => TxControl::Break,
            },
            echo: command & 0x10 != 0,
            parity: match command >> 5 {
                1 => Parity::Odd,
                3 => Parity::Even,
                5 => Parity::Mark,
                7 => Parity::Space,
                _ => Parity::None,
            },
        }
    }

    /// Nominal bit rate from the crystal; `None` on the external clock.
    pub fn baud(&self) -> Option<f64> {
        (self.baud_nibble != 0).then(|| BAUD_RATES[usize::from(self.baud_nibble)])
    }

    /// Bits on the line per frame (start, data, parity, stop), in halves.
    pub fn frame_halves(&self) -> u32 {
        2 * (1 + u32::from(self.data_bits) + u32::from(self.parity != Parity::None))
            + u32::from(self.stop_halves)
    }
}

impl Acia6551 {
    pub fn config(&self) -> AciaConfig {
        AciaConfig::decode(self.control, self.command)
    }
}
