//! Commodore 64 (C64) Computer Architecture Emulation.
//!
//! Emulates the hardware of the classic Commodore 64:
//! - MOS 6510 CPU memory banking via processor port `$0000` (DDR) and `$0001` (PR).
//! - PLA banking controlling BASIC ROM, KERNAL ROM, Character ROM, and I/O area ($D000..$DFFF).
//! - Integrated MOS 6581 SID sound synthesizer at `$D400..$D7FF`.
//! - VIC-II registers ($D000..$D3FF) and 1 KiB Color RAM ($D800..$DBFF).
//! - CIA 1 ($DC00..$DCFF) and CIA 2 ($DD00..$DDFF).
//! - 40x25 PETSCII screen matrix at `$0400..$07E7`.
//! - Keyboard buffer injection and terminal screen extraction.

use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::sid::Sid6581;

/// Standard 40 columns by 25 rows text screen.
pub const C64_SCREEN_COLS: usize = 40;
pub const C64_SCREEN_ROWS: usize = 25;
pub const C64_SCREEN_SIZE: usize = C64_SCREEN_COLS * C64_SCREEN_ROWS; // 1000 bytes

/// Commodore 64 System Memory Bus with PLA Banking.
pub struct C64Bus {
    /// 64 KiB main system RAM.
    pub ram: [u8; 65536],
    /// 8 KiB BASIC ROM ($A000..$BFFF).
    pub basic_rom: [u8; 8192],
    /// 8 KiB KERNAL ROM ($E000..$FFFF).
    pub kernal_rom: [u8; 8192],
    /// 4 KiB Character generator ROM ($D000..$DFFF).
    pub char_rom: [u8; 4096],
    /// 1 KiB Color RAM ($D800..$DBFF, lower 4 bits valid).
    pub color_ram: [u8; 1024],
    /// VIC-II registers ($D000..$D3FF, mirrored every 64 bytes).
    pub vic_regs: [u8; 64],
    /// CIA 1 registers ($DC00..$DCFF).
    pub cia1_regs: [u8; 16],
    pub cia1_timer_a: u16,
    pub cia1_timer_a_latch: u16,
    pub cia1_timer_a_running: bool,
    pub cia1_irq_flag: bool,
    pub cia1_irq_mask: u8,
    /// CIA 2 registers ($DD00..$DDFF).
    pub cia2_regs: [u8; 16],
    /// Integrated MOS 6581 SID Sound chip ($D400..$D7FF).
    pub sid: Sid6581,
    /// 6510 on-chip I/O direction register ($0000).
    pub port_ddr: u8,
    /// 6510 on-chip I/O data register ($0001):
    /// Bit 0: LORAM (BASIC ROM)
    /// Bit 1: HIRAM (KERNAL ROM)
    /// Bit 2: CHAREN (Character ROM vs I/O)
    pub port_data: u8,
}

impl Default for C64Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl C64Bus {
    pub fn new() -> Self {
        let mut bus = Self {
            ram: [0; 65536],
            basic_rom: [0; 8192],
            kernal_rom: [0; 8192],
            char_rom: [0; 4096],
            color_ram: [0x0E; 1024], // default light blue color
            vic_regs: [0; 64],
            cia1_regs: [0; 16],
            cia1_timer_a: 0x4025, // default PAL 60 Hz timer period (~16421 cycles)
            cia1_timer_a_latch: 0x4025,
            cia1_timer_a_running: true,
            cia1_irq_flag: false,
            cia1_irq_mask: 0x81, // timer A interrupt enabled
            cia2_regs: [0; 16],
            sid: Sid6581::new(),
            port_ddr: 0x2F,
            port_data: 0x37, // LORAM=1, HIRAM=1, CHAREN=1 (all active)
        };
        // Setup initial default screen RAM with spaces (PETSCII 0x20)
        for b in bus.ram[0x0400..0x0400 + C64_SCREEN_SIZE].iter_mut() {
            *b = 0x20;
        }
        bus
    }

    /// Load BASIC ROM image (up to 8 KiB).
    pub fn load_basic_rom(&mut self, bytes: &[u8]) {
        let len = bytes.len().min(8192);
        self.basic_rom[..len].copy_from_slice(&bytes[..len]);
    }

    /// Load KERNAL ROM image (up to 8 KiB).
    pub fn load_kernal_rom(&mut self, bytes: &[u8]) {
        let len = bytes.len().min(8192);
        self.kernal_rom[..len].copy_from_slice(&bytes[..len]);
    }

    /// Extract text screen contents ($0400..$07E7) as an ASCII multi-line string.
    pub fn screen_text(&self) -> String {
        let mut text = String::with_capacity(C64_SCREEN_SIZE + C64_SCREEN_ROWS);
        for row in 0..C64_SCREEN_ROWS {
            let start = 0x0400 + row * C64_SCREEN_COLS;
            for col in 0..C64_SCREEN_COLS {
                let code = self.ram[start + col];
                // Convert screen code to printable ASCII
                let ch = match code {
                    0x00 => '@',
                    0x01..=0x1A => (b'A' + code - 1) as char,
                    0x20..=0x3F => code as char,
                    _ => ' ',
                };
                text.push(ch);
            }
            text.push('\n');
        }
        text
    }

    /// Read an address under PLA configuration.
    fn read_pla(&mut self, addr: u16) -> u8 {
        let loram = (self.port_data & 0x01) != 0;
        let hiram = (self.port_data & 0x02) != 0;
        let charen = (self.port_data & 0x04) != 0;

        match addr {
            0x0000 => self.port_ddr,
            0x0001 => self.port_data,
            // $A000..$BFFF: BASIC ROM if LORAM & HIRAM are set
            0xA000..=0xBFFF => {
                if loram && hiram {
                    self.basic_rom[(addr - 0xA000) as usize]
                } else {
                    self.ram[addr as usize]
                }
            }
            // $D000..$DFFF: Character ROM, I/O devices, or RAM
            0xD000..=0xDFFF => {
                if !loram && !hiram {
                    // RAM takes precedence when both ROMs are disabled
                    self.ram[addr as usize]
                } else if !charen {
                    // Character ROM visible
                    self.char_rom[(addr - 0xD000) as usize]
                } else {
                    // I/O space
                    self.read_io(addr)
                }
            }
            // $E000..$FFFF: KERNAL ROM if HIRAM is set
            0xE000..=0xFFFF => {
                if hiram {
                    self.kernal_rom[(addr - 0xE000) as usize]
                } else {
                    self.ram[addr as usize]
                }
            }
            _ => self.ram[addr as usize],
        }
    }

    /// Read I/O subsystem register ($D000..$DFFF).
    fn read_io(&mut self, addr: u16) -> u8 {
        match addr {
            0xD000..=0xD3FF => self.vic_regs[((addr - 0xD000) % 64) as usize],
            0xD400..=0xD7FF => self.sid.read_reg(((addr - 0xD400) % 32) as u8),
            0xD800..=0xDBFF => self.color_ram[(addr - 0xD800) as usize] & 0x0F,
            0xDC00..=0xDCFF => {
                let reg = (addr - 0xDC00) % 16;
                match reg {
                    0x04 => (self.cia1_timer_a & 0xFF) as u8,
                    0x05 => (self.cia1_timer_a >> 8) as u8,
                    0x0D => {
                        let ret = if self.cia1_irq_flag { 0x81 } else { 0x00 };
                        self.cia1_irq_flag = false; // reading ICR clears interrupt
                        ret
                    }
                    _ => self.cia1_regs[reg as usize],
                }
            }
            0xDD00..=0xDDFF => self.cia2_regs[((addr - 0xDD00) % 16) as usize],
            _ => 0xFF,
        }
    }

    /// Write an address under PLA configuration.
    fn write_pla(&mut self, addr: u16, val: u8) {
        let loram = (self.port_data & 0x01) != 0;
        let hiram = (self.port_data & 0x02) != 0;
        let charen = (self.port_data & 0x04) != 0;

        match addr {
            0x0000 => self.port_ddr = val,
            0x0001 => self.port_data = val,
            0xD000..=0xDFFF => {
                // If I/O space is selected, write goes to devices; otherwise directly to underlaying RAM
                if (loram || hiram) && charen {
                    self.write_io(addr, val);
                } else {
                    self.ram[addr as usize] = val;
                }
            }
            // Writes to ROM address regions always fall through to underlaying RAM on C64
            _ => self.ram[addr as usize] = val,
        }
    }

    /// Write I/O subsystem register ($D000..$DFFF).
    fn write_io(&mut self, addr: u16, val: u8) {
        match addr {
            0xD000..=0xD3FF => self.vic_regs[((addr - 0xD000) % 64) as usize] = val,
            0xD400..=0xD7FF => self.sid.write_reg(((addr - 0xD400) % 32) as u8, val),
            0xD800..=0xDBFF => self.color_ram[(addr - 0xD800) as usize] = val & 0x0F,
            0xDC00..=0xDCFF => {
                let reg = (addr - 0xDC00) % 16;
                self.cia1_regs[reg as usize] = val;
                match reg {
                    0x04 => self.cia1_timer_a_latch = (self.cia1_timer_a_latch & 0xFF00) | (val as u16),
                    0x05 => {
                        self.cia1_timer_a_latch = (self.cia1_timer_a_latch & 0x00FF) | ((val as u16) << 8);
                        self.cia1_timer_a = self.cia1_timer_a_latch;
                    }
                    0x0D => {
                        // ICR mask
                        if (val & 0x80) != 0 {
                            self.cia1_irq_mask |= val & 0x1F;
                        } else {
                            self.cia1_irq_mask &= !(val & 0x1F);
                        }
                    }
                    0x0E => {
                        // CRA: bit 0 start/stop, bit 4 force load
                        self.cia1_timer_a_running = (val & 0x01) != 0;
                        if (val & 0x10) != 0 {
                            self.cia1_timer_a = self.cia1_timer_a_latch;
                        }
                    }
                    _ => {}
                }
            }
            0xDD00..=0xDDFF => self.cia2_regs[((addr - 0xDD00) % 16) as usize] = val,
            _ => {}
        }
    }
}

impl MemoryBus for C64Bus {
    fn read(&mut self, addr: u16) -> u8 {
        self.read_pla(addr)
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.write_pla(addr, value);
    }

    fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000 => self.port_ddr,
            0x0001 => self.port_data,
            0xD400..=0xD7FF => self.sid.read_reg(((addr - 0xD400) % 32) as u8),
            _ => self.ram[addr as usize],
        }
    }
}

/// The complete Commodore 64 Computer System.
pub struct C64Computer {
    pub cpu: Cpu<C64Bus>,
}

impl Default for C64Computer {
    fn default() -> Self {
        Self::new()
    }
}

impl C64Computer {
    pub fn new() -> Self {
        let bus = C64Bus::new();
        let cpu = Cpu::new(bus);
        Self { cpu }
    }

    /// Reset computer and load reset vector from KERNAL or memory.
    pub fn reset(&mut self) {
        self.cpu.reset();
    }

    /// Run for `cycles` CPU clock cycles.
    pub fn run_cycles(&mut self, cycles: u64) {
        let target = self.cpu.cycles + cycles;
        while self.cpu.cycles < target && !self.cpu.halted {
            let before = self.cpu.cycles;
            self.cpu.step();
            let elapsed = self.cpu.cycles.saturating_sub(before).max(1);

            // Clock SID sound synthesizer
            for _ in 0..elapsed {
                self.cpu.bus.sid.clock();
            }

            // Step CIA 1 Timer A
            if self.cpu.bus.cia1_timer_a_running {
                let el = (elapsed as u16).min(self.cpu.bus.cia1_timer_a);
                if self.cpu.bus.cia1_timer_a <= el {
                    self.cpu.bus.cia1_timer_a = self.cpu.bus.cia1_timer_a_latch;
                    self.cpu.bus.cia1_irq_flag = true;
                    if (self.cpu.bus.cia1_irq_mask & 0x01) != 0 && !self.cpu.regs.p.contains(crate::flags::Status::I) {
                        self.cpu.irq();
                    }
                } else {
                    self.cpu.bus.cia1_timer_a -= el;
                }
            }
        }
    }

    /// Write character directly to 40x25 screen matrix at (col, row).
    pub fn print_char_at(&mut self, col: usize, row: usize, ch: u8, color: u8) {
        if col < C64_SCREEN_COLS && row < C64_SCREEN_ROWS {
            let offset = row * C64_SCREEN_COLS + col;
            self.cpu.bus.ram[0x0400 + offset] = ch;
            self.cpu.bus.color_ram[offset] = color & 0x0F;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_c64_banking_and_sid() {
        let mut c64 = C64Computer::new();

        // Check default banking: BASIC and KERNAL active
        c64.cpu.bus.basic_rom[0] = 0xAA;
        c64.cpu.bus.kernal_rom[0] = 0xBB;
        assert_eq!(c64.cpu.bus.read(0xA000), 0xAA);
        assert_eq!(c64.cpu.bus.read(0xE000), 0xBB);

        // Turn off BASIC ROM via LORAM bit
        c64.cpu.bus.write(0x0001, 0x36); // LORAM=0
        c64.cpu.bus.ram[0xA000] = 0x55;
        assert_eq!(c64.cpu.bus.read(0xA000), 0x55);

        // Test SID access at $D400
        c64.cpu.bus.write(0xD400, 0x34);
        c64.cpu.bus.write(0xD401, 0x12);
        assert_eq!(c64.cpu.bus.sid.voices[0].freq, 0x1234);

        // Test screen text output
        c64.print_char_at(0, 0, 0x01, 0x0E); // 'A'
        let screen = c64.cpu.bus.screen_text();
        assert!(screen.starts_with('A'));
    }
}
