//! Placa original com uma 6522 mapeada em memória. Não usa ROM de Apple, KIM,
//! NES, Commodore ou Microsoft. A ROM é montada pelo assembler deste crate.
//!
//! Mapa: RAM `$0000..=$7FFF`, VIA `$C000..=$C00F`, ROM `$E000..=$FFFF`.
//! O buraco `$8000..=$BFFF` e `$C010..=$DFFF` lê zero e ignora escrita.
//!
//! O firmware põe o timer 1 em free-run (ACR bit 6), habilita a IRQ e espera:
//! cada estouro (latch + 2 ciclos) gera uma IRQ. O handler, alcançado
//! só por `step_cycle` com a linha de IRQ vinda da VIA, inverte o bit 0 da
//! porta A. Não há meio-ciclo de PHI2: cada `run_cycles` faz um `tick` da VIA
//! e um `step_cycle`. Com `Cpu::step_phase`
//! a placa dá `tick_phase` na VIA a cada metade.

use crate::asm::assemble_with_symbols;
use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::via::Via6522;

pub const VIA_BASE: u16 = 0xC000;
pub const ROM_BASE: u16 = 0xE000;
pub const RAM_END: u16 = 0x7FFF;

pub const VIA_ROM_SOURCE: &str = r#"
.org $E000
reset:
    lda #$01
    sta $C003
    lda #$00
    sta $C001
    lda #$40
    sta $C00B       ; ACR: timer 1 free-run
    lda #$C0
    sta $C00E
    lda #$40
    sta $C004
    lda #$00
    sta $C005
    cli
loop:
    jmp loop
irq:
    lda $C001
    eor #$01
    sta $C001
    lda $C004
    rti
.org $FFFC
    .word reset
    .word irq
"#;

pub struct ViaBoard {
    ram: [u8; 0x8000],
    rom: [u8; 0x2000],
    pub via: Via6522,
}

impl ViaBoard {
    pub fn with_rom(image: &[u8]) -> Self {
        let mut rom = [0u8; 0x2000];
        let n = image.len().min(rom.len());
        rom[..n].copy_from_slice(&image[..n]);
        Self {
            ram: [0; 0x8000],
            rom,
            via: Via6522::new(),
        }
    }

    fn via_offset(&self, addr: u16) -> Option<u8> {
        if (VIA_BASE..VIA_BASE.wrapping_add(16)).contains(&addr) {
            Some((addr - VIA_BASE) as u8)
        } else {
            None
        }
    }
}

impl MemoryBus for ViaBoard {
    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        self.via.tick_phase(phase);
    }

    fn read(&mut self, addr: u16) -> u8 {
        if let Some(offset) = self.via_offset(addr) {
            return self.via.read(offset);
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
        if let Some(offset) = self.via_offset(addr) {
            self.via.write(offset, value);
            return;
        }
        if addr <= RAM_END {
            self.ram[addr as usize] = value;
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        if let Some(offset) = self.via_offset(addr) {
            return self.via.peek(offset);
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

pub struct ViaComputer {
    pub cpu: Cpu<ViaBoard>,
}

impl ViaComputer {
    pub fn new() -> Result<Self, String> {
        let image = assemble_with_symbols(VIA_ROM_SOURCE, ROM_BASE).map_err(|err| err.0)?;
        if image.origin != ROM_BASE {
            return Err(format!("origem da ROM {:04X}", image.origin));
        }
        let mut cpu = Cpu::new(ViaBoard::with_rom(&image.bytes));
        cpu.halt_on_brk = true;
        cpu.reset();
        Ok(Self { cpu })
    }

    /// One VIA tick, then one CPU bus cycle. The IRQ pin follows the VIA.
    pub fn run_cycles(&mut self, cycles: u64) -> Result<(), String> {
        for _ in 0..cycles {
            if self.cpu.is_halted() {
                return Err("CPU parada".into());
            }
            self.cpu.bus.via.tick();
            let irq = self.cpu.bus.via.irq_asserted();
            self.cpu.set_irq_line(irq);
            self.cpu.step_cycle()?;
        }
        Ok(())
    }

    /// Output latch of port A (not the pin mix).
    pub fn port_a(&self) -> u8 {
        self.cpu.bus.via.ora_latch()
    }
}
