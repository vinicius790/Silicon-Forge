//! Computador de protoboard estilo Ben Eater: 16 KiB de RAM em $0000..=$3FFF, o
//! 6522 do crate ([`crate::via::Via6522`]) em $6000..=$600F e ROM de 32 KiB em
//! $8000..=$FFFF. Um LCD HD44780 16x2 fica na porta B (dados, 8 bits) e nos bits 7
//! (E), 6 (R/W) e 5 (RS) da porta A, lido nos pinos que a VIA realmente aciona.
//!
//! O LCD trava na borda de descida de E com R/W = 0. Comandos: `$01` limpa e volta
//! ao início, `$02` só volta ao início, `$04..=$07` modo de entrada (bit 1 =
//! incrementa, decrementa caso contrário; deslocar a tela não é modelado), `$80|end`
//! define o endereço DDRAM (linha 1 em $00..=$0F, linha 2 em $40..=$4F; o resto não
//! aparece). Com R/W = 1 e RS = 0 a porta B lê o flag de ocupado (sempre livre) e o
//! endereço atual. Mais comandos (deslocamento, CGRAM) são ignorados.

use crate::bus::{IoDevice, SystemBus};
use crate::cpu::Cpu;
use crate::via::Via6522;
use std::cell::RefCell;
use std::rc::Rc;

const E: u8 = 0x80;
const RW: u8 = 0x40;
const RS: u8 = 0x20;

/// HD44780 16x2 com 80 bytes de DDRAM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lcd {
    pub ddram: [u8; 0x80],
    pub address: u8,
    pub increment: bool,
}

impl Default for Lcd {
    fn default() -> Self {
        Self {
            ddram: [b' '; 0x80],
            address: 0,
            increment: true,
        }
    }
}

impl Lcd {
    pub fn command(&mut self, cmd: u8) {
        match cmd {
            0x80..=0xFF => self.address = cmd & 0x7F,
            0x04..=0x07 => self.increment = cmd & 0x02 != 0,
            0x02 | 0x03 => self.address = 0,
            0x01 => {
                self.ddram = [b' '; 0x80];
                self.address = 0;
                self.increment = true;
            }
            _ => {}
        }
    }

    pub fn data(&mut self, byte: u8) {
        self.ddram[usize::from(self.address & 0x7F)] = byte;
        self.address = if self.increment {
            self.address.wrapping_add(1) & 0x7F
        } else {
            self.address.wrapping_sub(1) & 0x7F
        };
    }

    /// The two visible lines.
    pub fn lines(&self) -> [String; 2] {
        let line = |at: usize| String::from_utf8_lossy(&self.ddram[at..at + 16]).into_owned();
        [line(0x00), line(0x40)]
    }

    pub fn render(&self) -> String {
        let [a, b] = self.lines();
        format!("+----------------+\n|{a}|\n|{b}|\n+----------------+")
    }
}

/// VIA plus the LCD wired to its ports.
#[derive(Default)]
pub struct Board {
    pub via: Via6522,
    pub lcd: Lcd,
    last_e: bool,
}

impl Board {
    fn after_access(&mut self) {
        let a = self.via.port_a_pins();
        let e = a & E != 0;
        if std::mem::replace(&mut self.last_e, e) && !e && a & RW == 0 {
            let b = self.via.port_b_pins();
            if a & RS != 0 {
                self.lcd.data(b);
            } else {
                self.lcd.command(b);
            }
        }
        // Reading: the LCD drives port B with busy flag (0) and address, or data.
        if a & RW != 0 && e {
            let value = if a & RS == 0 {
                self.lcd.address & 0x7F
            } else {
                self.lcd.ddram[usize::from(self.lcd.address)]
            };
            self.via.set_port_b_in(value);
        }
    }

    pub fn render_lcd(&self) -> String {
        self.lcd.render()
    }
}

impl IoDevice for Board {
    fn read(&mut self, offset: u16) -> u8 {
        IoDevice::read(&mut self.via, offset)
    }

    fn write(&mut self, offset: u16, value: u8) {
        IoDevice::write(&mut self.via, offset, value);
        self.after_access();
    }

    fn peek(&self, offset: u16) -> u8 {
        IoDevice::peek(&self.via, offset)
    }

    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        self.via.phase(phase);
    }
}

/// Bus-side handle to a board the computer also keeps, so the LCD stays readable.
struct Shared(Rc<RefCell<Board>>);

impl IoDevice for Shared {
    fn read(&mut self, offset: u16) -> u8 {
        self.0.borrow_mut().read(offset)
    }

    fn write(&mut self, offset: u16, value: u8) {
        self.0.borrow_mut().write(offset, value);
    }

    fn peek(&self, offset: u16) -> u8 {
        self.0.borrow().peek(offset)
    }

    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        self.0.borrow_mut().phase(phase);
    }
}

pub struct BenEaterComputer {
    pub cpu: Cpu<SystemBus>,
    board: Rc<RefCell<Board>>,
}

impl BenEaterComputer {
    /// ROM at $8000..=$FFFF; the reset vector is $8000 unless the ROM sets it.
    pub fn new(rom_bytes: &[u8]) -> Self {
        let mut bus = SystemBus::new();
        let board = Rc::new(RefCell::new(Board::default()));
        bus.map_device(0x6000, 0x600F, Shared(Rc::clone(&board)));
        bus.map_rom(0x8000, rom_bytes);
        if rom_bytes.len() < 0x7FFE {
            bus.set_reset_vector(0x8000);
        }
        let mut cpu = Cpu::new(bus);
        cpu.reset();
        Self { cpu, board }
    }

    pub fn board(&self) -> std::cell::Ref<'_, Board> {
        self.board.borrow()
    }
}
