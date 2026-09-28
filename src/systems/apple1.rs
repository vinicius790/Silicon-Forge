//! Emulador do Apple-1 Computer (1976).
//!
//! Criado por Steve Wozniak, o Apple-1 foi o primeiro produto da Apple Computer Inc.
//! Este módulo emula a placa do Apple-1, incluindo:
//! - 32 KiB de memória RAM ($0000..=$7FFF).
//! - Chip PIA 6820 ($D010..=$D013) mapeado para teclado e terminal de vídeo.
//! - Monitor original do projeto (não o WozMon) em $FE00..=$FFFF, montado do fonte.

use crate::bus::{IoDevice, SystemBus};
use crate::cpu::Cpu;
use crate::pia::Pia6821;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::rc::Rc;

/// Source of the project's own Apple-1 monitor ([`firmware/apple1_monitor.asm`]),
/// assembled at start-up into $FE00..=$FFFF. No third-party ROM image is shipped.
pub const MONITOR_SRC: &str = include_str!("../../firmware/apple1_monitor.asm");

/// The assembled monitor, exactly 512 bytes at $FE00 (one page more than the
/// original board had: it is not WozMon and does not pretend to fit its page).
pub fn monitor_image() -> Vec<u8> {
    let image = crate::asm::assemble(MONITOR_SRC, 0xFE00).expect("monitor assembles");
    debug_assert_eq!(image.len(), 512, "monitor fills $FE00..=$FFFF");
    image
}

/// Apple-1 keyboard and display on the shared [`Pia6821`]: port A bits 0..6 are the
/// keyboard (bit 7 is wired high), CA1 is the key strobe (rising edge sets CRA bit 7,
/// reading port A clears it); port B bits 0..6 go to the display and PB7 is the
/// display's busy line, read as ready (low). A write to port B data emits a character.
pub struct Apple1Pia {
    pub pia: Pia6821,
    /// Keys waiting behind the one on port A.
    pub input_buffer: VecDeque<u8>,
    /// Characters sent to the display, bit 7 removed.
    pub output_buffer: Vec<u8>,
}

impl Default for Apple1Pia {
    fn default() -> Self {
        Self::new()
    }
}

impl Apple1Pia {
    pub fn new() -> Self {
        let mut pia = Pia6821::new();
        pia.set_port_b_in(0x00);
        Self {
            pia,
            input_buffer: VecDeque::new(),
            output_buffer: Vec::new(),
        }
    }

    /// Queue keys: upper case, `\n` becomes carriage return, bit 7 set.
    pub fn queue_input(&mut self, text: &str) {
        for b in text.bytes() {
            let key = if b == b'\n' {
                0x0D
            } else {
                b.to_ascii_uppercase() & 0x7F
            };
            self.input_buffer.push_back(key | 0x80);
        }
        self.present_next();
    }

    /// Strobe the next key onto port A when the last one was taken.
    fn present_next(&mut self) {
        if self.pia.peek(1) & 0x80 != 0 {
            return;
        }
        if let Some(key) = self.input_buffer.pop_front() {
            self.pia.set_port_a_in(key);
            self.pia.set_ca1(false);
            self.pia.set_ca1(true);
        }
    }
}

impl IoDevice for Apple1Pia {
    fn read(&mut self, offset: u16) -> u8 {
        let value = self.pia.read(offset & 3);
        if offset & 3 == 0 {
            self.present_next();
        }
        value
    }

    fn write(&mut self, offset: u16, value: u8) {
        let data_b = offset & 3 == 2 && self.pia.peek(3) & 0x04 != 0;
        self.pia.write(offset & 3, value);
        if offset & 3 == 1 {
            self.present_next();
        }
        if data_b {
            self.output_buffer.push(value & 0x7F);
        }
    }

    fn peek(&self, offset: u16) -> u8 {
        self.pia.peek(offset & 3)
    }
}

struct Shared(Rc<RefCell<Apple1Pia>>);

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
}

pub struct Apple1 {
    pub cpu: Cpu<SystemBus>,
    pia: Rc<RefCell<Apple1Pia>>,
}

impl Apple1 {
    pub fn new() -> Self {
        let mut bus = SystemBus::new();
        let pia = Rc::new(RefCell::new(Apple1Pia::new()));
        bus.map_device(0xD010, 0xD013, Shared(Rc::clone(&pia)));
        bus.map_rom(0xFE00, &monitor_image());
        let mut cpu = Cpu::new(bus);
        cpu.reset();
        Self { cpu, pia }
    }

    pub fn step_cycles(&mut self, cycles: u64) -> Result<u64, String> {
        self.cpu.run_cycles(cycles)
    }

    pub fn type_text(&mut self, text: &str) {
        self.pia.borrow_mut().queue_input(text);
    }

    /// Take everything the display received so far, carriage returns as `\n`.
    pub fn take_output(&mut self) -> String {
        let bytes = std::mem::take(&mut self.pia.borrow_mut().output_buffer);
        bytes
            .iter()
            .map(|&b| if b == 0x0D { '\n' } else { b as char })
            .collect()
    }

    /// Type `line` and run until the keyboard queue drains, then `settle` more cycles.
    pub fn enter(&mut self, line: &str, settle: u64) -> Result<String, String> {
        self.type_text(line);
        self.type_text("\n");
        let mut budget = 2_000_000u64;
        while !self.pia.borrow().input_buffer.is_empty()
            || self.pia.borrow().pia.peek(1) & 0x80 != 0
        {
            self.cpu.run_cycles(1000)?;
            budget = budget
                .checked_sub(1000)
                .ok_or("monitor não consumiu a entrada")?;
        }
        self.cpu.run_cycles(settle)?;
        Ok(self.take_output())
    }

    pub fn run_interactive(&mut self) -> io::Result<()> {
        println!("=== Apple-1 (monitor próprio em $FE00) — Ctrl+C sai ===");
        let to_io = |e: String| io::Error::other(e);
        self.cpu.run_cycles(5000).map_err(to_io)?;
        print!("{}", self.take_output());
        let stdin = io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            if stdin.read_line(&mut line)? == 0 {
                break;
            }
            let out = self.enter(line.trim_end(), 20_000).map_err(to_io)?;
            print!("{out}");
            io::stdout().flush()?;
        }
        Ok(())
    }
}

impl Default for Apple1 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::MemoryBus;

    #[test]
    fn wozmon_echoes_writes_and_examines_memory() {
        let mut apple = Apple1::new();
        apple.step_cycles(5000).unwrap();
        assert!(apple.take_output().starts_with('\\'), "prompt");
        apple.enter("0300: A9 42", 50_000).unwrap();
        assert_eq!(
            (apple.cpu.bus.peek(0x0300), apple.cpu.bus.peek(0x0301)),
            (0xA9, 0x42)
        );
        let out = apple.enter("0300.0301", 50_000).unwrap();
        assert!(out.contains("0300: A9 42"), "{out:?}");
    }

    #[test]
    fn range_crosses_lines_and_run_jumps() {
        let mut apple = Apple1::new();
        apple.step_cycles(5000).unwrap();
        apple.take_output();
        let syms = crate::asm::assemble_with_symbols(MONITOR_SRC, 0xFE00)
            .unwrap()
            .symbols;
        let echo = syms["echo"];
        let line = format!(
            "0300: A9 C1 20 {:02X} {:02X} 4C 00 FE",
            echo & 0xFF,
            echo >> 8
        );
        apple.enter(&line, 50_000).unwrap();
        let out = apple.enter("02FE.0309", 80_000).unwrap();
        assert!(out.contains("02FE: "), "{out:?}");
        let row = format!(
            "\n0300: A9 C1 20 {:02X} {:02X} 4C 00 FE",
            echo & 0xFF,
            echo >> 8
        );
        assert!(
            out.contains(&row),
            "8 per line from an aligned address: {out:?}"
        );
        let out = apple.enter("0300 R", 80_000).unwrap();
        assert!(
            out.contains("\nA"),
            "program printed A, then restarted the monitor: {out:?}"
        );
    }

    #[test]
    fn monitor_is_the_projects_own_512_bytes() {
        let image = monitor_image();
        assert_eq!(image.len(), 512);
        assert_eq!(&image[0x1FC..0x1FE], &[0x00, 0xFE], "reset vector");
    }

    #[test]
    fn keys_wait_for_the_previous_one_to_be_read() {
        let mut pia = Apple1Pia::new();
        pia.write(1, 0x04);
        pia.queue_input("ab\n");
        assert_eq!(pia.read(1) & 0x80, 0x80);
        assert_eq!(pia.read(0), b'A' | 0x80);
        assert_eq!(
            pia.read(0),
            b'B' | 0x80,
            "next key presented after the read"
        );
        assert_eq!(pia.read(0), 0x8D);
        assert_eq!(pia.read(1) & 0x80, 0);
    }
}
