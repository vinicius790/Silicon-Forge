//! Emulação do MOS Technology KIM-1 (1976).
//!
//! O KIM-1 (Keyboard Input Monitor) foi o primeiro computador em placa única
//! produzido pela própria MOS Technology, desenhado por Chuck Peddle.
//! - CPU MOS 6502 original.
//! - 1 KiB de memória RAM ($0000..=$03FF).
//! - Chip RRIOT 6530 ($1700..=$17FF) com temporizador e portas de I/O.
//! - Display LED de 6 dígitos de 7 segmentos e teclado hexadecimal de 23 teclas.
//! - Suporte a modo Serial TTY para terminal.

use crate::bus::{IoDevice, MemoryBus};
use crate::cpu::Cpu;

/// Display de 6 dígitos de 7 segmentos do KIM-1.
/// 4 dígitos para endereço + 2 dígitos para dados (ex: "0200 EA").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kim1Display {
    pub address: u16,
    pub data: u8,
}

impl Kim1Display {
    pub fn new() -> Self {
        Self {
            address: 0x0200,
            data: 0x00,
        }
    }

    /// Renderiza visualmente o display de 7 segmentos no console.
    pub fn render(&self) -> String {
        format!(
            "+----------------+\n| ADDR: {:04X}  DATA: {:02X} |\n+----------------+",
            self.address, self.data
        )
    }
}

impl Default for Kim1Display {
    fn default() -> Self {
        Self::new()
    }
}

/// MOS 6530 RRIOT: I/O ports, interval timer and 64 bytes of RAM (the mask ROM is
/// not modeled; no ROM images are shipped).
///
/// Offsets (A0..A3 within the I/O window, RAM at `$40..=$7F`):
/// `0` PAD, `1` PADD, `2` PBD, `3` PBDD. Timer writes at `4..=7` (IRQ disabled) or
/// `C..=F` (IRQ enabled) load the counter with divide-by 1, 8, 64, 1024 (A1A0) and
/// clear the flag. Reading with A2 = 1: A0 = 0 reads the counter (A3 sets the IRQ
/// enable, clearing the flag), A0 = 1 reads the flag in bit 7. The counter counts
/// once per divided period; after it passes zero the flag sets and it keeps counting
/// down every cycle (free run) until the timer is read or written. Port reads give
/// the output latch on output bits and the pins on input bits; the display shows
/// what is written to port A.
///
/// PB7 doubles as the timer interrupt output (the 6530 has no IRQ pin of its own):
/// while the timer flag is set with the interrupt enabled, the PB7 pin is pulled
/// low and reads 0. The KIM-1 brings PB7 of both chips to the application
/// connector; a jumper to the 6502 IRQ line is needed for it to interrupt (see
/// [`Kim1Bus::pb7_irq_jumper`]).
///
/// # 6530 versus 6532
///
/// | | 6530 (this type) | 6532 ([`crate::pia::Riot6532`]) |
/// |---|---|---|
/// | RAM | 64 bytes, plus 1 KiB mask ROM | 128 bytes, no ROM |
/// | interrupt output | shared with PB7 | dedicated `/IRQ` pin |
/// | interrupt sources | timer | timer and PA7 edge |
/// | flag register | bit 7 timer | bit 7 timer, bit 6 PA7 edge |
/// | timer select | A2 = 1 in the I/O window | A2 = 1 with A4 = 1 |
/// | first decrement after a write | after one full interval | one clock after the write |
///
/// The last row is a modeling difference between the two types in this crate,
/// pinned by `tests/pc_riot_6530_vs_6532.rs`: a write of `n` with divider `d`
/// times out after `(n + 1) * d` clocks here and `n * d + 1` clocks on the 6532.
/// The MOS data sheets describe both timers in the same words; the 6532 timing
/// follows the Atari 2600 measurements the 6532 model is built on.
pub struct Rriot6530 {
    pub ram: [u8; 64],
    pub timer_value: u8,
    pub timer_div: u16,
    prescale: u16,
    pub timer_flag: bool,
    pub irq_enabled: bool,
    pub port_a: u8,
    pub port_b: u8,
    pub ddr_a: u8,
    pub ddr_b: u8,
    pub pins_a: u8,
    pub pins_b: u8,
    pub display: Kim1Display,
}

impl Rriot6530 {
    pub fn new() -> Self {
        Self {
            ram: [0; 64],
            timer_value: 0xFF,
            timer_div: 1,
            prescale: 1,
            timer_flag: false,
            irq_enabled: false,
            port_a: 0,
            port_b: 0,
            ddr_a: 0,
            ddr_b: 0,
            pins_a: 0xFF,
            pins_b: 0xFF,
            display: Kim1Display::new(),
        }
    }

    /// One CPU cycle.
    pub fn tick(&mut self) {
        if self.timer_flag {
            self.timer_value = self.timer_value.wrapping_sub(1);
            return;
        }
        self.prescale -= 1;
        if self.prescale == 0 {
            self.prescale = self.timer_div;
            if self.timer_value == 0 {
                self.timer_flag = true;
                self.timer_value = 0xFF;
            } else {
                self.timer_value -= 1;
            }
        }
    }

    pub fn irq_asserted(&self) -> bool {
        self.timer_flag && self.irq_enabled
    }

    fn port(latch: u8, ddr: u8, pins: u8) -> u8 {
        (latch & ddr) | (pins & !ddr)
    }
}

impl Default for Rriot6530 {
    fn default() -> Self {
        Self::new()
    }
}

impl IoDevice for Rriot6530 {
    fn read(&mut self, offset: u16) -> u8 {
        let value = self.peek(offset);
        if offset < 0x40 && offset & 0x05 == 0x04 {
            self.timer_flag = false;
            self.irq_enabled = offset & 0x08 != 0;
            self.prescale = self.timer_div;
        }
        value
    }

    fn write(&mut self, offset: u16, value: u8) {
        match offset {
            0x40..=0x7F => self.ram[usize::from(offset - 0x40)] = value,
            _ => match offset & 0x0F {
                0x00 => {
                    self.port_a = value;
                    self.display.data = value;
                }
                0x01 => self.ddr_a = value,
                0x02 => self.port_b = value,
                0x03 => self.ddr_b = value,
                o if o & 0x04 != 0 => {
                    self.timer_div = [1, 8, 64, 1024][usize::from(o & 3)];
                    self.prescale = self.timer_div;
                    self.timer_value = value;
                    self.timer_flag = false;
                    self.irq_enabled = o & 0x08 != 0;
                }
                _ => {}
            },
        }
    }

    fn peek(&self, offset: u16) -> u8 {
        match offset {
            0x40..=0x7F => self.ram[usize::from(offset - 0x40)],
            _ => match offset & 0x0F {
                0x00 => Self::port(self.port_a, self.ddr_a, self.pins_a),
                0x01 => self.ddr_a,
                0x02 => {
                    let pb = Self::port(self.port_b, self.ddr_b, self.pins_b);
                    if self.irq_asserted() {
                        pb & 0x7F
                    } else {
                        pb
                    }
                }
                0x03 => self.ddr_b,
                o if o & 0x05 == 0x04 => self.timer_value,
                o if o & 0x05 == 0x05 => u8::from(self.timer_flag) << 7,
                _ => 0xFF,
            },
        }
    }

    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        if phase == crate::cycles::BusPhase::Phi2 {
            self.tick();
        }
    }
}

/// KIM-1 memory map (KIM-1 User Manual, chapter 3 and appendix). Only A0–A12 are
/// decoded, so the 8 KiB image repeats through the 64 KiB space and the 6502
/// vectors at `$FFFA` come from `$1FFA`.
///
/// | range | device |
/// |---|---|
/// | `$0000–$03FF` | 1 KiB RAM |
/// | `$1700–$173F` | 6530-003 I/O and timer |
/// | `$1740–$177F` | 6530-002 I/O and timer (keypad, display, TTY) |
/// | `$1780–$17BF` | 6530-003 RAM (64 bytes) |
/// | `$17C0–$17FF` | 6530-002 RAM (64 bytes; `$17FA`/`$17FE` hold the NMI/IRQ pointers) |
/// | `$1800–$1BFF` | 6530-003 ROM (tape routines; empty here, loadable) |
/// | `$1C00–$1FFF` | 6530-002 ROM (monitor; the original one in [`MONITOR_SRC`]) |
///
/// Other addresses read `$FF` and ignore writes.
pub mod map {
    pub const SAD: u16 = 0x1740;
    pub const PADD: u16 = 0x1741;
    pub const SBD: u16 = 0x1742;
    pub const PBDD: u16 = 0x1743;
    pub const NMIV: u16 = 0x17FA;
    pub const RSTV: u16 = 0x17FC;
    pub const IRQV: u16 = 0x17FE;
    /// TTY bit delay (loop count in this monitor).
    pub const CNTL30: u16 = 0x17F2;
    /// Tape start address, end address (exclusive) and record ID.
    pub const SAL: u16 = 0x17F5;
    pub const SAH: u16 = 0x17F6;
    pub const EAL: u16 = 0x17F7;
    pub const EAH: u16 = 0x17F8;
    pub const ID: u16 = 0x17F9;
}

/// Monitor entry points, at the addresses the KIM-1 User Manual documents. The
/// shipped monitor is original code that implements these at the same addresses
/// (the MOS ROM is not included). The TTY routines run 8N2 frames bit by bit on
/// PB0 (out) and PA7 (in), mark = 1, with one bit lasting [`Kim1::tty_bit_cycles`];
/// the original measures the line speed into CNTH30/CNTL30, this one takes a loop
/// count from CNTL30 (`$17F2`, set to `$40` at reset). Tape is handled on the host
/// side by [`tape`]; the `$1800` tape routines are not provided.
pub mod entry {
    /// Register save (ST key / BRK through the NMI or IRQ pointer).
    pub const SAVE: u16 = 0x1C00;
    /// NMI handler: `JMP ($17FA)`.
    pub const NMIT: u16 = 0x1C1C;
    /// IRQ handler: `JMP ($17FE)`.
    pub const IRQT: u16 = 0x1C1F;
    /// Reset entry.
    pub const RST: u16 = 0x1C22;
    /// Keypad monitor main loop.
    pub const START: u16 = 0x1C4F;
    /// Carriage return and line feed on the TTY.
    pub const CRLF: u16 = 0x1E2F;
    /// A as two hex digits on the TTY.
    pub const PRTBYT: u16 = 0x1E3B;
    /// One character from the TTY (PA7) into A.
    pub const GETCH: u16 = 0x1E5A;
    /// A space on the TTY.
    pub const OUTSP: u16 = 0x1E9E;
    /// A to the TTY (PB0).
    pub const OUTCH: u16 = 0x1EA0;
    /// Key-down test: A = 0 and Z set when no key is pressed.
    pub const AK: u16 = 0x1EFE;
    /// Display POINTH, POINTL, INH on the six digits, then [`AK`].
    pub const SCANDS: u16 = 0x1F1F;
    /// Open the keypad (port A inputs, PB1–PB4 outputs).
    pub const KEYIN: u16 = 0x1F40;
    /// Key code in A: `$00–$0F` hex, `$10` AD, `$11` DA, `$12` +, `$13` GO,
    /// `$14` PC, `$15` none.
    pub const GETKEY: u16 = 0x1F6A;
    /// Hex digit to seven-segment table (16 bytes).
    pub const TABLE: u16 = 0x1FE7;
}

/// Zero-page cells the monitor uses (the KIM-1 manual's register save area).
pub mod zp {
    pub const PCL: u16 = 0xEF;
    pub const PCH: u16 = 0xF0;
    pub const PREG: u16 = 0xF1;
    pub const SPUSER: u16 = 0xF2;
    pub const ACC: u16 = 0xF3;
    pub const YREG: u16 = 0xF4;
    pub const XREG: u16 = 0xF5;
    /// TTY character shift register.
    pub const CHAR: u16 = 0xFE;
    /// This monitor's entry mode: 0 address, 1 data.
    pub const MODE: u16 = 0xF6;
    /// Scratch for nibble entry.
    pub const TMP: u16 = 0xF7;
    pub const INH: u16 = 0xF9;
    pub const POINTL: u16 = 0xFA;
    pub const POINTH: u16 = 0xFB;
}

/// Keys of the KIM-1 keypad wired to the scan matrix (ST and RS are wired to NMI
/// and reset instead). The code is what [`entry::GETKEY`] returns: row 0 holds
/// `0`–`6`, row 1 `7`–`D`, row 2 `E`, `F`, AD, DA, +, GO, PC (bits PA0–PA6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Hex(u8),
    Ad,
    Da,
    Plus,
    Go,
    Pc,
}

impl Key {
    pub fn code(self) -> u8 {
        match self {
            Key::Hex(n) => n & 0x0F,
            Key::Ad => 0x10,
            Key::Da => 0x11,
            Key::Plus => 0x12,
            Key::Go => 0x13,
            Key::Pc => 0x14,
        }
    }
}

/// Original keypad monitor for the documented entry points. Assembled at start-up.
pub const MONITOR_SRC: &str = r#"
SAD    = $1740
PADD   = $1741
SBD    = $1742
PBDD   = $1743
NMIV   = $17FA
IRQV   = $17FE
PCL    = $EF
PCH    = $F0
PREG   = $F1
SPUSER = $F2
ACC    = $F3
YREG   = $F4
XREG   = $F5
MODE   = $F6
TMP    = $F7
INH    = $F9
POINTL = $FA
POINTH = $FB
CHAR   = $FE
CNTL30 = $17F2

.org $1C00
save:               ; registers of the interrupted program
    sta ACC
    sty YREG
    stx XREG
    pla
    sta PREG
    pla
    sta PCL
    pla
    sta PCH
    tsx
    stx SPUSER
    jmp savepc
.org $1C1C
nmit:
    jmp (NMIV)
irqt:
    jmp (IRQV)
rst:                ; $1C22
    ldx #$FF
    txs
    cld
    lda #$1E
    sta PBDD
    lda #0
    sta MODE
    lda #$40        ; TTY bit delay (loop count; see Kim1::tty_bit_cycles)
    sta CNTL30
    jmp start
savepc:
    lda PCL
    sta POINTL
    lda PCH
    sta POINTH
    cld
    jmp start

.org $1C4F
start:              ; first wait until every key is let go
    ldy #0
    lda (POINTL),y
    sta INH
    jsr scands
    bne start
wait:
    ldy #0
    lda (POINTL),y
    sta INH
    jsr scands
    jsr getkey
    cmp #$15
    beq wait
    jsr dokey
    jmp start

dokey:
    cmp #$10
    bcc hex
    beq ad
    cmp #$11
    beq da
    cmp #$12
    beq plus
    cmp #$13
    beq go
    lda PCL         ; $14 PC: back to the saved program counter
    sta POINTL
    lda PCH
    sta POINTH
    rts
ad:
    lda #0
    sta MODE
    rts
da:
    lda #1
    sta MODE
    rts
plus:
    inc POINTL
    bne plusend
    inc POINTH
plusend:
    rts
go:
    ldx SPUSER
    txs
    ldx XREG
    ldy YREG
    lda ACC
    jmp (POINTL)
hex:                ; shift the key's nibble into the address or the data byte
    asl a
    asl a
    asl a
    asl a
    sta TMP
    ldx #4
    lda MODE
    bne data
addr:
    asl TMP
    rol POINTL
    rol POINTH
    dex
    bne addr
    rts
data:
    ldy #0
    lda (POINTL),y
dloop:
    asl TMP
    rol a
    dex
    bne dloop
    sta (POINTL),y
    rts

two:                ; both digits of A, X = digit select code
    pha
    lsr a
    lsr a
    lsr a
    lsr a
    jsr seg
    pla
    and #$0F
seg:
    tay
    lda table,y
    stx SBD         ; select first, so the pattern lands on this digit only
    sta SAD
    ldy #$20
delay:
    dey
    bne delay
    sty SAD         ; blank before the next digit
    inx
    inx
    rts

delay1:             ; one TTY bit time: CNTL30 loops of 5 cycles
    ldy CNTL30
dl:
    dey
    bne dl
    rts
pad16:
    nop
    nop
    rts
hexout:             ; low nibble of A as one ASCII hex digit
    cmp #$0A
    bcc hexd
    adc #6
hexd:
    adc #$30
    jmp outch

.org $1E2F
crlf:
    lda #$0D
    jsr outch
    lda #$0A
    jmp outch

.org $1E3B
prtbyt:             ; A as two hex digits, A kept
    pha
    pha
    lsr a
    lsr a
    lsr a
    lsr a
    jsr hexout
    pla
    and #$0F
    jsr hexout
    pla
    rts

.org $1E5A
getch:              ; one 8N1 character from PA7 (mark = 1) into A, X and Y kept
    txa
    pha
    tya
    pha
gwait:
    bit SAD
    bmi gwait       ; idle mark
    lda CNTL30      ; to the middle of the start bit
    lsr a
    tay
ghalf:
    dey
    bne ghalf
    ldx #8
gbit:
    jsr delay1
    jsr pad16       ; same 47 + 5 * CNTL30 cycles per bit as OUTCH
    lda SAD
    asl a           ; PA7 into carry
    ror CHAR        ; LSB first
    dex
    bne gbit
    jsr delay1      ; let the stop bit pass
    pla
    tay
    pla
    tax
    lda CHAR
    rts

.org $1E9E
outsp:
    lda #$20        ; falls into OUTCH
.org $1EA0
outch:              ; A out on PB0 as 8N2 (start 0, LSB first, two mark stops); A, X, Y kept
    pha
    sta CHAR
    txa
    pha
    tya
    pha
    lda SBD         ; mark in the latch before PB0 turns output: no glitch
    ora #$01
    sta SBD
    lda PBDD
    ora #$01
    sta PBDD
    ldx #11
    clc             ; start bit
obit:
    lda SBD
    and #$FE
    sta TMP
    lda #0
    rol a           ; carry into bit 0, same time for 0 and 1
    ora TMP
    sta SBD
    jsr delay1
    sec             ; mark shifts in behind the data: the stop bits
    ror CHAR
    dex
    bne obit
    pla
    tay
    pla
    tax
    pla
    rts

.org $1EFE
ak:
    jsr getkey
    eor #$15        ; 0 and Z when no key
    rts

.org $1F1F
scands:
    lda #$7F
    sta PADD
    lda #$1E
    sta PBDD
    ldx #$08        ; 74145 input 4: leftmost digit
    lda POINTH
    jsr two
    lda POINTL
    jsr two
    lda INH
    jsr two
    jmp ak

.org $1F40
keyin:
    lda #0
    sta PADD
    lda #$1E
    sta PBDD
    rts

.org $1F6A
getkey:
    jsr keyin
    ldx #0          ; 74145 inputs 0, 1, 2 select the key rows
    ldy #0
row:
    stx SBD
    lda SAD
    ora #$80
    eor #$FF        ; pressed keys as 1 bits
    bne found
    tya
    clc
    adc #7
    tay
    inx
    inx
    cpx #6
    bne row
    lda #$15
    rts
found:
    lsr a
    bcs done
    iny
    bne found
done:
    tya
    rts

.org $1FE7
table:
    .byte $3F, $06, $5B, $4F, $66, $6D, $7D, $07
    .byte $7F, $6F, $77, $7C, $39, $5E, $79, $71
.org $1FFA
    .word nmit, rst, irqt
"#;

/// The assembled monitor: 1 KiB for `$1C00–$1FFF`, unused bytes `$00`.
pub fn monitor_rom() -> Vec<u8> {
    let out =
        crate::asm::assemble_with_symbols(MONITOR_SRC, 0x1C00).expect("KIM-1 monitor assembles");
    assert_eq!(out.origin, 0x1C00, "monitor origin");
    let mut rom = vec![0u8; 0x400];
    rom[..out.bytes.len()].copy_from_slice(&out.bytes);
    rom
}

/// Address-decoded KIM-1 bus with the two RRIOTs, the keypad matrix and the LEDs.
pub struct Kim1Bus {
    pub ram: [u8; 0x400],
    /// 6530-002: keypad, display, TTY.
    pub riot002: Rriot6530,
    /// 6530-003: user ports (application connector).
    pub riot003: Rriot6530,
    pub rom002: Vec<u8>,
    pub rom003: Vec<u8>,
    /// Pressed keys by code (`0x00..=0x14`).
    pub keys: [bool; 21],
    /// Last segment pattern (`a`–`g` in bits 0–6) driven to each of the six digits.
    pub segments: [u8; 6],
    /// Times each digit was strobed with segments lit.
    pub strobes: [u32; 6],
    /// TTY input level on PA7 (high = idle mark).
    pub tty_in: bool,
    /// CPU cycles clocked so far.
    pub cycles: u64,
    /// PB7 → IRQ jumpers on the application connector: `[6530-002, 6530-003]`.
    /// Off as shipped.
    pub pb7_irq_jumper: [bool; 2],
    /// TTY output: every change of the PB0 pin with the cycle it happened at.
    pub tty_out: Vec<(u64, bool)>,
    /// Scheduled PA7 levels for the TTY input, by cycle.
    tty_in_queue: std::collections::VecDeque<(u64, bool)>,
}

impl Kim1Bus {
    pub fn new() -> Self {
        Self {
            ram: [0; 0x400],
            riot002: Rriot6530::new(),
            riot003: Rriot6530::new(),
            rom002: monitor_rom(),
            rom003: vec![0xFF; 0x400],
            keys: [false; 21],
            segments: [0; 6],
            strobes: [0; 6],
            tty_in: true,
            cycles: 0,
            pb7_irq_jumper: [false; 2],
            tty_out: Vec::new(),
            tty_in_queue: std::collections::VecDeque::new(),
        }
    }

    /// 74145 decoder input: PB1–PB4 as driven by the 6530-002 (inputs read high).
    fn select(&self) -> u8 {
        let r = &self.riot002;
        let pb = (r.port_b & r.ddr_b) | (r.pins_b & !r.ddr_b);
        (pb >> 1) & 0x0F
    }

    /// Port A pins of the 6530-002: the selected key row pulls PA0–PA6 low for each
    /// pressed key; PA7 is the TTY input.
    fn refresh_pins(&mut self) {
        let sel = self.select();
        let mut pins = 0x7F;
        if sel < 3 {
            for bit in 0..7 {
                if self.keys[usize::from(sel) * 7 + bit] {
                    pins &= !(1 << bit);
                }
            }
        }
        if self.tty_in {
            pins |= 0x80;
        }
        self.riot002.pins_a = pins;
    }

    /// Digits 0–5 sit on 74145 outputs 4–9; a digit shows PA0–PA6 as driven.
    fn latch_display(&mut self) {
        let sel = self.select();
        if (4..=9).contains(&sel) {
            let r = &self.riot002;
            let seg = r.port_a & r.ddr_a & 0x7F;
            let d = usize::from(sel - 4);
            if seg != 0 {
                self.segments[d] = seg;
                self.strobes[d] += 1;
            }
        }
    }

    /// The six digits decoded back through the monitor's segment table (`?` for a
    /// pattern that is not a hex digit, space for dark).
    pub fn display_text(&self) -> String {
        const TABLE: [u8; 16] = [
            0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F, 0x77, 0x7C, 0x39, 0x5E,
            0x79, 0x71,
        ];
        self.segments
            .iter()
            .map(|&s| match TABLE.iter().position(|&t| t == s) {
                Some(n) => char::from_digit(n as u32, 16).unwrap().to_ascii_uppercase(),
                None if s == 0 => ' ',
                None => '?',
            })
            .collect()
    }

    pub fn tick(&mut self) {
        self.riot002.tick();
        self.riot003.tick();
        self.cycles += 1;
        while let Some(&(at, level)) = self.tty_in_queue.front() {
            if at > self.cycles {
                break;
            }
            self.tty_in = level;
            self.tty_in_queue.pop_front();
        }
    }

    /// IRQ line: a RRIOT pulling PB7 low through a fitted jumper.
    pub fn irq(&self) -> bool {
        (self.pb7_irq_jumper[0] && self.riot002.irq_asserted())
            || (self.pb7_irq_jumper[1] && self.riot003.irq_asserted())
    }

    fn pb0(&self) -> bool {
        let r = &self.riot002;
        Rriot6530::port(r.port_b, r.ddr_b, r.pins_b) & 1 != 0
    }

    fn track_tty_out(&mut self) {
        let level = self.pb0();
        let last = self.tty_out.last().is_none_or(|&(_, l)| l);
        if level != last {
            self.tty_out.push((self.cycles, level));
        }
    }

    /// Queue one 8N2 frame on PA7 starting `delay` cycles from now.
    pub fn tty_queue_byte(&mut self, byte: u8, bit_cycles: u64, delay: u64) {
        let start = self
            .tty_in_queue
            .back()
            .map_or(self.cycles, |&(t, _)| t)
            .max(self.cycles)
            + delay;
        let mut t = start;
        let mut bits = vec![false];
        bits.extend((0..8).map(|i| (byte >> i) & 1 != 0));
        bits.extend([true, true]);
        for b in bits {
            self.tty_in_queue.push_back((t, b));
            t += bit_cycles;
        }
    }

    /// Decode the PB0 edges into 8N1 characters, sampling the middle of each bit.
    pub fn tty_decode(&self, bit_cycles: u64) -> Vec<u8> {
        let level_at = |t: u64| {
            self.tty_out
                .iter()
                .take_while(|&&(at, _)| at <= t)
                .last()
                .is_none_or(|&(_, l)| l)
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.tty_out.len() {
            let (start, level) = self.tty_out[i];
            if level {
                i += 1;
                continue;
            }
            let mut byte = 0u8;
            for bit in 0..8 {
                if level_at(start + bit_cycles * (bit + 1) + bit_cycles / 2) {
                    byte |= 1 << bit;
                }
            }
            out.push(byte);
            let stop = start + bit_cycles * 9 + bit_cycles / 2;
            while i < self.tty_out.len() && (self.tty_out[i].0 <= stop || self.tty_out[i].1) {
                i += 1;
            }
        }
        out
    }
}

impl Default for Kim1Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBus for Kim1Bus {
    fn read(&mut self, addr: u16) -> u8 {
        let a = addr & 0x1FFF;
        match a {
            0x1740..=0x177F => {
                self.refresh_pins();
                self.riot002.read(a & 0x3F)
            }
            0x1700..=0x173F => self.riot003.read(a & 0x3F),
            _ => self.peek(addr),
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        let a = addr & 0x1FFF;
        match a {
            0x0000..=0x03FF => self.ram[usize::from(a)] = value,
            0x1700..=0x173F => self.riot003.write(a & 0x3F, value),
            0x1740..=0x177F => {
                self.riot002.write(a & 0x3F, value);
                self.latch_display();
                self.track_tty_out();
            }
            0x1780..=0x17BF => self.riot003.write(0x40 | (a & 0x3F), value),
            0x17C0..=0x17FF => self.riot002.write(0x40 | (a & 0x3F), value),
            _ => {}
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        let a = addr & 0x1FFF;
        match a {
            0x0000..=0x03FF => self.ram[usize::from(a)],
            0x1700..=0x173F => self.riot003.peek(a & 0x3F),
            0x1740..=0x177F => self.riot002.peek(a & 0x3F),
            0x1780..=0x17BF => self.riot003.peek(0x40 | (a & 0x3F)),
            0x17C0..=0x17FF => self.riot002.peek(0x40 | (a & 0x3F)),
            0x1800..=0x1BFF => self.rom003[usize::from(a - 0x1800)],
            0x1C00..=0x1FFF => self.rom002[usize::from(a - 0x1C00)],
            _ => 0xFF,
        }
    }
}

/// Complete KIM-1: CPU, the decoded bus and the front-panel keys.
pub struct Kim1 {
    pub cpu: Cpu<Kim1Bus>,
}

impl Kim1 {
    /// Power on and press RS: the CPU starts at the monitor's reset entry.
    pub fn new() -> Self {
        let mut cpu = Cpu::new(Kim1Bus::new());
        cpu.reset();
        Self { cpu }
    }

    /// Run `n` instructions, clocking both RRIOT timers once per CPU cycle. A
    /// jumpered PB7 interrupt is taken at the instruction boundary when I is clear.
    pub fn run_instructions(&mut self, n: u64) -> Result<u64, String> {
        let mut total = 0;
        for _ in 0..n {
            if self.cpu.bus.irq() && !self.cpu.p.get(crate::flags::bits::I) {
                let before = self.cpu.cycles;
                self.cpu.irq();
                for _ in before..self.cpu.cycles {
                    self.cpu.bus.tick();
                }
                total += self.cpu.cycles - before;
                continue;
            }
            let cycles = self.cpu.step()?;
            for _ in 0..cycles {
                self.cpu.bus.tick();
            }
            total += u64::from(cycles);
        }
        Ok(total)
    }

    pub fn press(&mut self, key: Key) {
        self.cpu.bus.keys[usize::from(key.code())] = true;
    }

    pub fn release_all(&mut self) {
        self.cpu.bus.keys = [false; 21];
    }

    /// Press `key`, run until the monitor has seen it, release, and run on until it
    /// is back waiting (the monitor acts once per press).
    pub fn tap(&mut self, key: Key) -> Result<(), String> {
        self.press(key);
        self.run_instructions(3000)?;
        self.release_all();
        self.run_instructions(3000)?;
        Ok(())
    }

    /// Cycles per TTY bit: OUTCH's per-bit loop, 47 cycles plus 5 per count of
    /// CNTL30 (`$00` counts 256). GETCH's loop is padded to the same length.
    pub fn tty_bit_cycles(&self) -> u64 {
        let n = match self.cpu.bus.peek(map::CNTL30) {
            0 => 256,
            n => u64::from(n),
        };
        47 + 5 * n
    }

    /// Run until `cycles` CPU cycles have passed.
    pub fn run_cycles(&mut self, cycles: u64) -> Result<(), String> {
        let end = self.cpu.bus.cycles + cycles;
        while self.cpu.bus.cycles < end {
            self.run_instructions(1)?;
        }
        Ok(())
    }

    /// Write a tape record of `start..end` with `id`, as the documented KIM-1
    /// character stream ([`tape::encode`]).
    pub fn save_tape(&self, id: u8, start: u16, end: u16) -> Vec<u8> {
        let data: Vec<u8> = (start..end).map(|a| self.cpu.bus.peek(a)).collect();
        tape::encode(id, start, &data)
    }

    /// Load the first matching record the way LOADT selects it: the ID in `$17F9`
    /// must match, `$00` takes any record at its tape address, `$FF` takes any
    /// record at the address in SAL/SAH (`$17F5`). Sets SAL/SAH and EAL/EAH (end,
    /// exclusive) to where it landed. Returns the loaded record.
    pub fn load_tape(&mut self, stream: &[u8]) -> Result<tape::Record, String> {
        let want = self.cpu.bus.peek(map::ID);
        let records = tape::decode_all(stream)?;
        let rec = records
            .into_iter()
            .find(|r| want == 0 || want == 0xFF || r.id == want)
            .ok_or_else(|| format!("no record with ID {want:02X}"))?;
        let start = if want == 0xFF {
            u16::from_le_bytes([self.cpu.bus.peek(map::SAL), self.cpu.bus.peek(map::SAH)])
        } else {
            rec.start
        };
        let mut a = start;
        for &b in &rec.data {
            self.cpu.bus.write(a, b);
            a = a.wrapping_add(1);
        }
        let [sl, sh] = start.to_le_bytes();
        let [el, eh] = a.to_le_bytes();
        for (cell, v) in [
            (map::SAL, sl),
            (map::SAH, sh),
            (map::EAL, el),
            (map::EAH, eh),
        ] {
            self.cpu.bus.write(cell, v);
        }
        Ok(rec)
    }

    /// RS key: hardware reset.
    pub fn rs(&mut self) {
        self.cpu.reset();
    }

    /// ST key: NMI, which the monitor sends through the pointer at `$17FA`.
    pub fn st(&mut self) {
        self.cpu.nmi();
    }
}

impl Default for Kim1 {
    fn default() -> Self {
        Self::new()
    }
}

/// KIM-1 audio tape format, from the monitor's DUMPT/LOADT (KIM-1 User Manual,
/// "kim1_mon" listing).
///
/// A record is 100 SYN (`$16`), `*` (`$2A`), then as two ASCII hex characters
/// each: ID, SAL, SAH and the bytes from SA up to EA − 1; then `/` (`$2F`), the
/// checksum low and high (two hex characters each) and two EOT (`$04`). The
/// 16-bit checksum is the sum of SAL, SAH and the data bytes (not the ID).
/// Characters go out least significant bit first. A bit is three 2.484 ms tones:
/// 3700 Hz first, 3700 Hz (`0`) or 2400 Hz (`1`) in the middle, 2400 Hz last
/// (134.2 bit/s).
pub mod tape {
    pub const SYN: u8 = 0x16;
    pub const SOD: u8 = 0x2A;
    pub const EOD: u8 = 0x2F;
    pub const EOT: u8 = 0x04;
    pub const SYN_COUNT: usize = 100;
    /// Length of one of the three tones of a bit, in seconds.
    pub const TONE_SECONDS: f64 = 0.002484;
    pub const HIGH_HZ: f64 = 3700.0;
    pub const LOW_HZ: f64 = 2400.0;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Record {
        pub id: u8,
        pub start: u16,
        pub data: Vec<u8>,
        pub checksum: u16,
    }

    fn hex(n: u8) -> u8 {
        b"0123456789ABCDEF"[usize::from(n & 15)]
    }

    fn unhex(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }

    pub fn checksum(start: u16, data: &[u8]) -> u16 {
        let [l, h] = start.to_le_bytes();
        [l, h]
            .iter()
            .chain(data)
            .fold(0u16, |s, &b| s.wrapping_add(u16::from(b)))
    }

    /// The character stream of one record.
    pub fn encode(id: u8, start: u16, data: &[u8]) -> Vec<u8> {
        let mut out = vec![SYN; SYN_COUNT];
        out.push(SOD);
        let [sl, sh] = start.to_le_bytes();
        let [cl, ch] = checksum(start, data).to_le_bytes();
        let byte = |out: &mut Vec<u8>, b: u8| out.extend([hex(b >> 4), hex(b)]);
        for b in [id, sl, sh].iter().chain(data) {
            byte(&mut out, *b);
        }
        out.push(EOD);
        byte(&mut out, cl);
        byte(&mut out, ch);
        out.extend([EOT, EOT]);
        out
    }

    /// Every record in a stream. LOADT synchronizes on SYN and then waits for `*`;
    /// a non-hex character inside a record or a wrong checksum is an error.
    pub fn decode_all(stream: &[u8]) -> Result<Vec<Record>, String> {
        let mut recs = Vec::new();
        let mut i = 0;
        loop {
            while i < stream.len() && stream[i] != SYN {
                i += 1;
            }
            while i < stream.len() && stream[i] == SYN {
                i += 1;
            }
            if i >= stream.len() {
                break;
            }
            if stream[i] != SOD {
                continue;
            }
            i += 1;
            let mut bytes = Vec::new();
            loop {
                let c = *stream.get(i).ok_or("record ends before '/'")?;
                if c == EOD {
                    i += 1;
                    break;
                }
                let pair = stream.get(i..i + 2).ok_or("odd hex character count")?;
                match (unhex(pair[0]), unhex(pair[1])) {
                    (Some(h), Some(l)) => bytes.push((h << 4) | l),
                    _ => return Err(format!("non-hex character at {i}")),
                }
                i += 2;
            }
            if bytes.len() < 3 {
                return Err("record shorter than ID and start address".into());
            }
            let mut sum = [0u8; 2];
            for s in &mut sum {
                let pair = stream.get(i..i + 2).ok_or("checksum cut off")?;
                *s = match (unhex(pair[0]), unhex(pair[1])) {
                    (Some(h), Some(l)) => (h << 4) | l,
                    _ => return Err("non-hex checksum".into()),
                };
                i += 2;
            }
            let start = u16::from_le_bytes([bytes[1], bytes[2]]);
            let data = bytes[3..].to_vec();
            let stored = u16::from_le_bytes(sum);
            if stored != checksum(start, &data) {
                return Err(format!("checksum {stored:04X} does not match"));
            }
            recs.push(Record {
                id: bytes[0],
                start,
                data,
                checksum: stored,
            });
        }
        Ok(recs)
    }

    /// Bits of a character stream, LSB first.
    pub fn bits(stream: &[u8]) -> Vec<bool> {
        stream
            .iter()
            .flat_map(|&c| (0..8).map(move |i| (c >> i) & 1 != 0))
            .collect()
    }

    /// Square-wave audio of a character stream.
    pub fn to_audio(stream: &[u8], rate: u32) -> Vec<i16> {
        let tone = (TONE_SECONDS * f64::from(rate)).round() as usize;
        let mut out = Vec::new();
        let mut phase = 0.0f64;
        for bit in bits(stream) {
            for hz in [HIGH_HZ, if bit { LOW_HZ } else { HIGH_HZ }, LOW_HZ] {
                for _ in 0..tone {
                    out.push(if phase < 0.5 { 12000 } else { -12000 });
                    phase = (phase + hz / f64::from(rate)).fract();
                }
            }
        }
        out
    }

    /// Recover the character stream from [`to_audio`] output by timing the middle
    /// tone of each bit: the zero-crossing interval tells 3700 Hz from 2400 Hz.
    pub fn from_audio(samples: &[i16], rate: u32) -> Vec<u8> {
        let tone = (TONE_SECONDS * f64::from(rate)).round() as usize;
        let n = samples.len() / (3 * tone);
        let mut bits = Vec::with_capacity(n);
        for b in 0..n {
            let mid = &samples[b * 3 * tone + tone..b * 3 * tone + 2 * tone];
            let crossings = mid.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
            // 3700 Hz gives ~18 crossings per tone, 2400 Hz ~12.
            let threshold = TONE_SECONDS * (HIGH_HZ + LOW_HZ);
            bits.push((crossings as f64) < threshold);
        }
        bits.chunks_exact(8)
            .map(|c| {
                c.iter()
                    .enumerate()
                    .fold(0u8, |a, (i, &b)| a | (u8::from(b) << i))
            })
            .collect()
    }
}
