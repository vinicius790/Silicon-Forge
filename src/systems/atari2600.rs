//! Atari 2600 (VCS) Video Computer System Architecture.
//!
//! Emulates the hardware of the Atari 2600:
//! - MOS 6507 CPU (6502 core with 13-bit address space: $0000..$1FFF).
//! - Television Interface Adapter (TIA) at $0000..$007F.
//! - MOS 6532 RIOT: 128 bytes of RAM at $0080..$00FF, Timers & I/O at $0280..$0297.
//! - Cartridge ROM space: $1000..$1FFF (2 KiB / 4 KiB).

use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::tia::Tia2600;

/// Atari 2600 memory bus (8 KiB addressable space).
pub struct Atari2600Bus {
    /// 128 bytes RIOT RAM ($0080..$00FF).
    pub riot_ram: [u8; 128],
    /// Up to 4 KiB Cartridge ROM ($1000..$1FFF).
    pub rom: [u8; 4096],
    /// TIA chip.
    pub tia: Tia2600,
    /// RIOT Timer register.
    pub riot_timer: u8,
    pub riot_timer_divider: u16,
}

impl Default for Atari2600Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Atari2600Bus {
    pub fn new() -> Self {
        Self {
            riot_ram: [0; 128],
            rom: [0; 4096],
            tia: Tia2600::new(),
            riot_timer: 0,
            riot_timer_divider: 1,
        }
    }

    /// Load 2K or 4K cartridge ROM bytes.
    pub fn load_rom(&mut self, bytes: &[u8]) {
        if bytes.len() <= 2048 {
            // Mirror 2K ROM into 4K window
            let len = bytes.len().min(2048);
            self.rom[..len].copy_from_slice(&bytes[..len]);
            self.rom[2048..2048 + len].copy_from_slice(&bytes[..len]);
        } else {
            let len = bytes.len().min(4096);
            self.rom[..len].copy_from_slice(&bytes[..len]);
        }
    }
}

impl MemoryBus for Atari2600Bus {
    fn read(&mut self, addr: u16) -> u8 {
        // A12 selects Cartridge ROM ($1000..$1FFF)
        let a = addr & 0x1FFF;
        if (a & 0x1000) != 0 {
            self.rom[(a & 0x0FFF) as usize]
        } else if (a & 0x0200) != 0 {
            // RIOT I/O and Timer ($0280..$0297)
            if (a & 0x04) != 0 {
                self.riot_timer
            } else {
                0xFF // Console switches (SWCHB)
            }
        } else if (a & 0x0080) != 0 {
            // RIOT RAM ($0080..$00FF)
            self.riot_ram[(a & 0x007F) as usize]
        } else {
            // TIA registers ($0000..$007F)
            self.tia.read_reg((a & 0x0F) as u8)
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        let a = addr & 0x1FFF;
        if (a & 0x1000) != 0 {
            // ROM is read-only (standard carts)
        } else if (a & 0x0200) != 0 {
            // RIOT I/O and Timer writes ($0284..$0297)
            match a & 0x17 {
                0x14 => { self.riot_timer = value; self.riot_timer_divider = 1; }
                0x15 => { self.riot_timer = value; self.riot_timer_divider = 8; }
                0x16 => { self.riot_timer = value; self.riot_timer_divider = 64; }
                0x17 => { self.riot_timer = value; self.riot_timer_divider = 1024; }
                _ => {}
            }
        } else if (a & 0x0080) != 0 {
            // RIOT RAM ($0080..$00FF)
            self.riot_ram[(a & 0x007F) as usize] = value;
        } else {
            // TIA write
            self.tia.write_reg((a & 0x3F) as u8, value);
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        let a = addr & 0x1FFF;
        if (a & 0x1000) != 0 {
            self.rom[(a & 0x0FFF) as usize]
        } else if (a & 0x0080) != 0 {
            self.riot_ram[(a & 0x007F) as usize]
        } else {
            self.tia.read_reg((a & 0x0F) as u8)
        }
    }
}

/// The complete Atari 2600 VCS console.
pub struct Atari2600 {
    pub cpu: Cpu<Atari2600Bus>,
}

impl Default for Atari2600 {
    fn default() -> Self {
        Self::new()
    }
}

impl Atari2600 {
    pub fn new() -> Self {
        let bus = Atari2600Bus::new();
        let cpu = Cpu::new(bus);
        Self { cpu }
    }

    /// Reset console and jump through reset vector at $FFFC ($1FFC in 13-bit space).
    pub fn reset(&mut self) {
        self.cpu.reset();
    }

    /// Run for `cycles` CPU clock cycles, stepping the TIA video clock in sync.
    pub fn run_cycles(&mut self, cycles: u64) {
        let target = self.cpu.cycles + cycles;
        while self.cpu.cycles < target && !self.cpu.halted {
            // If WSYNC is active, CPU is frozen until end of current scanline
            if self.cpu.bus.tia.wsync_active {
                self.cpu.cycles += 1;
                self.cpu.bus.tia.clock_cpu_cycle();
                continue;
            }

            let before = self.cpu.cycles;
            self.cpu.step();
            let elapsed = self.cpu.cycles.saturating_sub(before).max(1);

            for _ in 0..elapsed {
                self.cpu.bus.tia.clock_cpu_cycle();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atari2600_memory_mapping() {
        let mut console = Atari2600::new();
        // Test RIOT RAM
        console.cpu.bus.write(0x80, 0x42);
        assert_eq!(console.cpu.bus.read(0x80), 0x42);

        // Test ROM mapping at $1000 and reset vector at $1FFC
        let mut cart = [0xEA; 4096]; // NOPs
        cart[0x0FFC] = 0x00;
        cart[0x0FFD] = 0xF0; // Start at $F000 (maps to $1000)
        console.cpu.bus.load_rom(&cart);
        assert_eq!(console.cpu.bus.read(0x1000), 0xEA);
        assert_eq!(console.cpu.bus.read(0x1FFC), 0x00);
        assert_eq!(console.cpu.bus.read(0x1FFD), 0xF0);
    }
}
