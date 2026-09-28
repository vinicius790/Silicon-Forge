//! Computador de terminal original: 32 KiB de RAM, ROM montada pelo assembler
//! do crate e uma 6551. Não usa ROM de Apple, KIM, NES ou Microsoft.
//!
//! Mapa: RAM `$0000..=$7FFF`, ACIA `$D000..=$D003`, ROM `$E000..=$FFFF`
//! (o buraco `$8000..=$CFFF` lê zero e ignora escrita).

use crate::acia::Acia6551;
use crate::asm::assemble_with_symbols;
use crate::bus::IoDevice;
use crate::bus::MemoryBus;
use crate::cpu::Cpu;

pub const RAM_END: u16 = 0x7FFF;
pub const ACIA_BASE: u16 = 0xD000;
pub const ROM_BASE: u16 = 0xE000;
/// Linha editada pela ROM: bytes em `$0200`, comprimento em `$10`.
pub const LINE_BUF: u16 = 0x0200;
pub const LINE_LEN: u16 = 0x0010;
/// Capacidade do editor de linha da ROM.
pub const LINE_MAX: usize = 64;

/// Falhas ao dirigir o terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermError {
    /// Erro da CPU (opcode JAM etc.).
    Cpu(String),
    /// A CPU já executou BRK depois do CR.
    Halted,
    /// `step_limit` instruções sem a ROM voltar a esperar entrada.
    Timeout { byte: u8 },
}

impl std::fmt::Display for TermError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cpu(m) => write!(f, "CPU: {m}"),
            Self::Halted => f.write_str("CPU parou antes de ecoar o byte"),
            Self::Timeout { byte } => write!(f, "tempo esgotado processando {byte:02X}"),
        }
    }
}

impl std::error::Error for TermError {}

/// Fonte da ROM de eco. Também está em `examples/term_echo.asm`.
pub const ECHO_ROM_SOURCE: &str = include_str!("../../examples/term_echo.asm");

pub struct TermBus {
    ram: [u8; 0x8000],
    rom: [u8; 0x2000],
    pub acia: Acia6551,
}

impl TermBus {
    pub fn with_rom(image: &[u8]) -> Self {
        let mut rom = [0u8; 0x2000];
        let n = image.len().min(rom.len());
        rom[..n].copy_from_slice(&image[..n]);
        Self {
            ram: [0; 0x8000],
            rom,
            acia: Acia6551::new(),
        }
    }

    fn acia_offset(&self, addr: u16) -> Option<u16> {
        if (ACIA_BASE..=ACIA_BASE.wrapping_add(3)).contains(&addr) {
            Some(addr.wrapping_sub(ACIA_BASE))
        } else {
            None
        }
    }
}

impl MemoryBus for TermBus {
    fn read(&mut self, addr: u16) -> u8 {
        if let Some(offset) = self.acia_offset(addr) {
            return self.acia.read(offset);
        }
        if addr <= RAM_END {
            self.ram[addr as usize]
        } else if addr >= ROM_BASE {
            self.rom[(addr - ROM_BASE) as usize]
        } else {
            0
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if let Some(offset) = self.acia_offset(addr) {
            self.acia.write(offset, value);
            return;
        }
        if addr <= RAM_END {
            self.ram[addr as usize] = value;
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        if let Some(offset) = self.acia_offset(addr) {
            return self.acia.peek(offset);
        }
        if addr <= RAM_END {
            self.ram[addr as usize]
        } else if addr >= ROM_BASE {
            self.rom[(addr - ROM_BASE) as usize]
        } else {
            0
        }
    }
}

/// Máquina pronta: ROM de eco montada, reset e a inicialização da 6551 já executados.
pub struct TermComputer {
    pub cpu: Cpu<TermBus>,
    getc: u16,
}

impl TermComputer {
    pub fn new() -> Result<Self, String> {
        let image = assemble_with_symbols(ECHO_ROM_SOURCE, ROM_BASE).map_err(|err| err.0)?;
        if image.origin != ROM_BASE {
            return Err(format!("origem da ROM {:04X}", image.origin));
        }
        let mut cpu = Cpu::new(TermBus::with_rom(&image.bytes));
        cpu.halt_on_brk = true;
        cpu.reset();
        // LDA #$0B / STA $D002: the firmware turns DTR on before the first byte.
        cpu.run_instructions(2)?;
        let getc = image.symbols.get("getc").copied().ok_or("ROM sem getc")?;
        Ok(Self { cpu, getc })
    }

    /// Entrega `byte` e executa até a ROM voltar a `getc` com o RX vazio (ou parar
    /// no BRK). Devolve todos os bytes transmitidos nesse intervalo, em ordem.
    pub fn send(&mut self, byte: u8, step_limit: u32) -> Result<Vec<u8>, TermError> {
        if self.cpu.is_halted() {
            return Err(TermError::Halted);
        }
        self.cpu.bus.acia.push_rx(byte);
        let mut out = Vec::new();
        let mut consumed = false;
        for _ in 0..step_limit {
            self.cpu.step().map_err(TermError::Cpu)?;
            out.extend(self.cpu.bus.acia.pop_tx());
            if self.cpu.is_halted() {
                return Ok(out);
            }
            // RDRF clear: the ROM has read the byte; the next getc entry waits for more.
            if self.cpu.bus.acia.peek(1) & 0x08 == 0 {
                consumed = true;
            }
            if consumed && self.cpu.pc == self.getc {
                return Ok(out);
            }
        }
        Err(TermError::Timeout { byte })
    }

    /// Digita `bytes` um a um; devolve o eco concatenado.
    pub fn type_bytes(&mut self, bytes: &[u8], step_limit: u32) -> Result<Vec<u8>, TermError> {
        let mut out = Vec::new();
        for &b in bytes {
            out.extend(self.send(b, step_limit)?);
        }
        Ok(out)
    }

    /// Linha atual do editor da ROM (`$0200`, comprimento em `$10`).
    pub fn line(&self) -> Vec<u8> {
        let len = usize::from(self.cpu.bus.peek(LINE_LEN));
        debug_assert!(len <= LINE_MAX, "a ROM nunca passa de {LINE_MAX}");
        (0..len as u16)
            .map(|i| self.cpu.bus.peek(LINE_BUF + i))
            .collect()
    }

    /// Entrega `byte` na 6551 e executa até o eco aparecer no TX, ou `step_limit`.
    pub fn echo_byte(&mut self, byte: u8, step_limit: u32) -> Result<u8, String> {
        self.cpu.bus.acia.push_rx(byte);
        for _ in 0..step_limit {
            if self.cpu.is_halted() {
                return Err("CPU parou antes de ecoar o byte".into());
            }
            self.cpu.step()?;
            if let Some(sent) = self.cpu.bus.acia.pop_tx() {
                return Ok(sent);
            }
        }
        Err("tempo esgotado esperando o eco".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echoes_a_line_ending_in_cr() {
        let mut machine = TermComputer::new().unwrap();
        assert_eq!(machine.cpu.pc, ROM_BASE + 5);
        assert_eq!(machine.cpu.bus.acia.peek(2), 0x0B);
        let line = b"Hi\r";
        let mut echoed = Vec::new();
        for &byte in line {
            echoed.push(machine.echo_byte(byte, 10_000).unwrap());
        }
        assert_eq!(echoed, line);
        // Depois do CR o firmware cai no BRK.
        for _ in 0..20 {
            if machine.cpu.is_halted() {
                break;
            }
            machine.cpu.step().unwrap();
        }
        assert!(machine.cpu.is_halted());
    }
}
