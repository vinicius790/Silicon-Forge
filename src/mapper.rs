//! Cartridge PRG mappers for the 2A03 console, as models of the bank-switching logic.
//! The images come from the project's assembler; no game data is involved.
//!
//! * [`Nrom`]: 16 KiB mirrored at `$8000` and `$C000`, or 32 KiB flat; optional 8 KiB
//!   PRG RAM at `$6000`. Writes to `$8000`–`$FFFF` are ignored.
//! * [`Uxrom`]: 16 KiB banks. A write anywhere in `$8000`–`$FFFF` selects the bank at
//!   `$8000`; `$C000` is fixed to the last bank. The board has bus conflicts: the
//!   value latched is the written byte AND the ROM byte at that address.
//! * [`Mmc1`]: five-write serial port. A write with bit 7 set clears the shift register
//!   and sets PRG mode 3. Otherwise bit 0 shifts in, LSB first; the fifth write stores
//!   the five bits into the register chosen by address bits 14–13 (`$8000` control,
//!   `$A000`/`$C000` CHR, kept but unused here, `$E000` PRG bank). Control bits 3–2:
//!   `0`/`1` switch 32 KiB (bank number ignoring bit 0), `2` fixes the first bank at
//!   `$8000` and switches `$C000`, `3` switches `$8000` and fixes the last at `$C000`.
//!   PRG bank bit 4 clear enables the PRG RAM. A serial write on the CPU cycle right
//!   after another is ignored, as on the chip: the double write of a read-modify-write
//!   instruction (`INC $8000`) counts once. The machine advances the cycle count with
//!   [`Cartridge::clock`]; without it every write is accepted.

use std::fmt;

/// Errors restoring a `.sav` image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveRamError {
    NoSaveRam,
    WrongSize { expected: usize, found: usize },
}

impl fmt::Display for SaveRamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSaveRam => write!(f, "cartucho sem RAM de save"),
            Self::WrongSize { expected, found } => {
                write!(f, "save de {found} bytes, esperado {expected}")
            }
        }
    }
}

impl std::error::Error for SaveRamError {}

/// Copy a `.sav` image into `ram` after checking its size.
pub fn restore_ram(ram: &mut [u8], data: &[u8]) -> Result<(), SaveRamError> {
    if ram.len() != data.len() {
        return Err(SaveRamError::WrongSize {
            expected: ram.len(),
            found: data.len(),
        });
    }
    ram.copy_from_slice(data);
    Ok(())
}

/// Destination of a PPU access chosen by [`Cartridge::ppu_target`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuTarget {
    /// CHR through [`Cartridge::chr_offset`] for `$0000`–`$1FFF`, header/mapper
    /// mirroring for nametables.
    Default,
    /// Byte offset into CHR memory.
    Chr(usize),
    /// Byte offset into the console's 2 KiB nametable RAM.
    Ciram(usize),
    /// Memory inside the mapper (MMC5 ExRAM, fill mode): reads return this byte,
    /// writes go to [`Cartridge::ppu_write`].
    Mapper(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapperError {
    /// PRG size must be a non-zero multiple of 16 KiB (NROM: exactly 16 or 32 KiB).
    BadPrgSize(usize),
    TooManyBanks(usize),
}

impl fmt::Display for MapperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadPrgSize(n) => write!(f, "tamanho de PRG inválido: {n} bytes"),
            Self::TooManyBanks(n) => write!(f, "bancos demais para o mapper: {n}"),
        }
    }
}

impl std::error::Error for MapperError {}

/// CPU side of a cartridge: `$4020`–`$FFFF`.
pub trait Cartridge {
    fn read(&mut self, addr: u16) -> u8 {
        self.peek(addr)
    }
    fn peek(&self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, value: u8);
    /// One CPU cycle elapsed. Mappers without timing ignore it.
    fn clock(&mut self) {}
    /// Offset into CHR memory for PPU address `$0000`–`$1FFF` (bank switching).
    fn chr_offset(&self, addr: u16) -> usize {
        usize::from(addr)
    }
    /// Mapper-controlled nametable mirroring; `None` keeps the header's.
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        None
    }
    /// Where a PPU access to `$0000`–`$2FFF` lands when the mapper overrides the
    /// usual CHR banking and nametable mirroring (Namco 163 CIRAM/CHR mapping).
    fn ppu_target(&self, _addr: u16) -> PpuTarget {
        PpuTarget::Default
    }
    /// The PPU put `addr` on its address bus (MMC3 watches A12).
    fn ppu_address(&mut self, _addr: u16) {}
    /// The PPU performed a rendering read of `addr` (after [`Cartridge::ppu_address`]).
    fn ppu_read(&mut self, _addr: u16) {}
    /// PPU write that [`Cartridge::ppu_target`] sent to [`PpuTarget::Mapper`].
    fn ppu_write(&mut self, _addr: u16, _value: u8) {}
    /// The CPU wrote PPU register `$2000 + (addr & 7)` (MMC5 snoops PPUCTRL/PPUMASK).
    fn snoop_ppu_register(&mut self, _addr: u16, _value: u8) {}
    /// Expansion audio output in units of the APU mixer ([`crate::apu::mix`]); 0 when
    /// the board has none.
    fn audio(&self) -> f32 {
        0.0
    }
    /// Battery-backed RAM as stored in a `.sav` file, if the board has any.
    fn save_ram(&self) -> Option<Vec<u8>> {
        None
    }
    /// Restore battery RAM from a `.sav` image of [`Cartridge::save_ram`]'s size.
    fn load_save_ram(&mut self, _data: &[u8]) -> Result<(), SaveRamError> {
        Err(SaveRamError::NoSaveRam)
    }
    /// Cartridge IRQ output.
    fn irq(&self) -> bool {
        false
    }
    /// Mapper tag in save states (iNES mapper number; 255 for a cartridge that
    /// stores nothing).
    fn state_kind(&self) -> u8 {
        255
    }
    /// Append the mapper's registers, PRG RAM and a PRG ROM fingerprint.
    fn save_state(&self, _w: &mut StateWriter) {}
    /// Inverse of [`Cartridge::save_state`]; the PRG ROM must match.
    fn load_state(&mut self, _r: &mut StateReader) -> Result<(), NesStateError> {
        Ok(())
    }
}

use crate::nes_state::{NesStateError, StateReader, StateWriter};

fn bank_index(
    r: &mut StateReader,
    banks: usize,
    what: &'static str,
) -> Result<usize, NesStateError> {
    let v = r.u32(what)? as usize;
    if v >= banks.max(1) {
        return Err(NesStateError::BadValue {
            what,
            value: v as u64,
        });
    }
    Ok(v)
}

const BANK: usize = 0x4000;

fn check_banks(prg: &[u8], max: usize) -> Result<usize, MapperError> {
    if prg.is_empty() || prg.len() % BANK != 0 {
        return Err(MapperError::BadPrgSize(prg.len()));
    }
    let banks = prg.len() / BANK;
    if banks > max {
        return Err(MapperError::TooManyBanks(banks));
    }
    Ok(banks)
}

fn ram_index(addr: u16) -> Option<usize> {
    (0x6000..=0x7FFF)
        .contains(&addr)
        .then(|| usize::from(addr - 0x6000))
}

pub struct Nrom {
    prg: Vec<u8>,
    ram: Option<Box<[u8; 0x2000]>>,
}

impl Nrom {
    pub fn new(prg: Vec<u8>, with_ram: bool) -> Result<Self, MapperError> {
        if prg.len() != BANK && prg.len() != 2 * BANK {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        Ok(Self {
            prg,
            ram: with_ram.then(|| Box::new([0; 0x2000])),
        })
    }
}

impl Cartridge for Nrom {
    fn save_ram(&self) -> Option<Vec<u8>> {
        self.ram.as_ref().map(|r| r.to_vec())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        match self.ram.as_mut() {
            Some(r) => restore_ram(&mut r[..], data),
            None => Err(SaveRamError::NoSaveRam),
        }
    }
    fn state_kind(&self) -> u8 {
        0
    }
    fn save_state(&self, w: &mut StateWriter) {
        w.u64(crate::nes_state::fnv64(&self.prg));
        w.bool(self.ram.is_some());
        if let Some(ram) = &self.ram {
            w.bytes(&ram[..]);
        }
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        if r.bool("PRG RAM")? != self.ram.is_some() {
            return Err(NesStateError::RomMismatch("PRG RAM"));
        }
        if let Some(ram) = self.ram.as_mut() {
            r.fill(&mut ram[..], "PRG RAM")?;
        }
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        match (addr, ram_index(addr)) {
            (_, Some(i)) => self.ram.as_ref().map_or(0, |ram| ram[i]),
            (0x8000.., _) => self.prg[usize::from(addr - 0x8000) % self.prg.len()],
            _ => 0,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if let (Some(i), Some(ram)) = (ram_index(addr), self.ram.as_mut()) {
            ram[i] = value;
        }
    }
}

pub struct Uxrom {
    prg: Vec<u8>,
    banks: usize,
    pub bank: usize,
    /// UNROM/UOROM boards AND the written byte with ROM (NES 2.0 submapper 2, and
    /// the default for submapper 0); submapper 1 boards have no conflicts.
    pub bus_conflicts: bool,
}

impl Uxrom {
    pub fn new(prg: Vec<u8>) -> Result<Self, MapperError> {
        let banks = check_banks(&prg, 16)?;
        Ok(Self {
            prg,
            banks,
            bank: 0,
            bus_conflicts: true,
        })
    }
}

impl Cartridge for Uxrom {
    fn state_kind(&self) -> u8 {
        2
    }
    fn save_state(&self, w: &mut StateWriter) {
        w.u64(crate::nes_state::fnv64(&self.prg));
        w.u32(self.bank as u32);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        self.bank = bank_index(r, self.banks, "UxROM bank")?;
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        let offset = usize::from(addr & 0x3FFF);
        match addr {
            0x8000..=0xBFFF => self.prg[self.bank * BANK + offset],
            0xC000.. => self.prg[(self.banks - 1) * BANK + offset],
            _ => 0,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if addr >= 0x8000 {
            let latched = if self.bus_conflicts {
                value & self.peek(addr)
            } else {
                value
            };
            self.bank = usize::from(latched & 0x0F) % self.banks;
        }
    }
}

pub struct Mmc1 {
    prg: Vec<u8>,
    banks: usize,
    ram: Box<[u8; 0x2000]>,
    shift: u8,
    writes: u8,
    pub control: u8,
    pub chr: [u8; 2],
    pub prg_bank: u8,
    cycle: u64,
    last_serial: Option<u64>,
}

impl Mmc1 {
    pub fn new(prg: Vec<u8>) -> Result<Self, MapperError> {
        let banks = check_banks(&prg, 16)?;
        Ok(Self {
            prg,
            banks,
            ram: Box::new([0; 0x2000]),
            shift: 0,
            writes: 0,
            control: 0x0C,
            chr: [0; 2],
            prg_bank: 0,
            cycle: 0,
            last_serial: None,
        })
    }

    fn ram_enabled(&self) -> bool {
        self.prg_bank & 0x10 == 0
    }

    /// 16 KiB bank mapped at `$8000` (`high` false) or `$C000`.
    pub fn bank_at(&self, high: bool) -> usize {
        let bank = usize::from(self.prg_bank & 0x0F);
        let last = self.banks - 1;
        let chosen = match ((self.control >> 2) & 3, high) {
            (0 | 1, false) => bank & !1,
            (0 | 1, true) => bank | 1,
            (2, false) => 0,
            (2, true) => bank,
            (_, false) => bank,
            (_, true) => last,
        };
        chosen % self.banks
    }
}

impl Cartridge for Mmc1 {
    fn save_ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.to_vec())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        restore_ram(&mut self.ram[..], data)
    }
    fn state_kind(&self) -> u8 {
        1
    }
    fn save_state(&self, w: &mut StateWriter) {
        w.u64(crate::nes_state::fnv64(&self.prg));
        w.bytes(&self.ram[..]);
        w.u8(self.shift);
        w.u8(self.writes);
        w.u8(self.control);
        w.u8(self.chr[0]);
        w.u8(self.chr[1]);
        w.u8(self.prg_bank);
        w.u64(self.cycle);
        w.bool(self.last_serial.is_some());
        w.u64(self.last_serial.unwrap_or(0));
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        self.shift = r.small(0x1F, "MMC1 shift")?;
        self.writes = r.small(4, "MMC1 writes")?;
        self.control = r.small(0x1F, "MMC1 control")?;
        self.chr = [r.small(0x1F, "MMC1 CHR")?, r.small(0x1F, "MMC1 CHR")?];
        self.prg_bank = r.small(0x1F, "MMC1 PRG")?;
        self.cycle = r.u64("MMC1 cycle")?;
        let some = r.bool("MMC1 last write")?;
        let at = r.u64("MMC1 last write")?;
        self.last_serial = some.then_some(at);
        Ok(())
    }
    fn clock(&mut self) {
        self.cycle += 1;
    }

    /// Control bit 4 clear: one 8 KiB bank (`chr[0]` without bit 0); set: two 4 KiB banks.
    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        if self.control & 0x10 == 0 {
            usize::from(self.chr[0] & 0x1E) * 0x1000 + a
        } else {
            usize::from(self.chr[a >> 12]) * 0x1000 + (a & 0xFFF)
        }
    }

    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        use crate::ppu::Mirroring::*;
        Some([SingleLow, SingleHigh, Vertical, Horizontal][usize::from(self.control & 3)])
    }

    fn peek(&self, addr: u16) -> u8 {
        match (addr, ram_index(addr)) {
            (_, Some(i)) if self.ram_enabled() => self.ram[i],
            (0x8000.., _) => {
                let bank = self.bank_at(addr >= 0xC000);
                self.prg[bank * BANK + usize::from(addr & 0x3FFF)]
            }
            _ => 0,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if let Some(i) = ram_index(addr) {
            if self.ram_enabled() {
                self.ram[i] = value;
            }
            return;
        }
        if addr < 0x8000 {
            return;
        }
        let consecutive = self.last_serial.is_some_and(|last| last + 1 == self.cycle);
        self.last_serial = Some(self.cycle);
        if consecutive {
            return;
        }
        if value & 0x80 != 0 {
            self.shift = 0;
            self.writes = 0;
            self.control |= 0x0C;
            return;
        }
        self.shift |= (value & 1) << self.writes;
        self.writes += 1;
        if self.writes == 5 {
            let data = self.shift;
            match (addr >> 13) & 3 {
                0 => self.control = data,
                1 => self.chr[0] = data,
                2 => self.chr[1] = data,
                _ => self.prg_bank = data,
            }
            self.shift = 0;
            self.writes = 0;
        }
    }
}

/// AxROM (mapper 7): one 32 KiB bank at `$8000`–`$FFFF`, chosen by bits 2–0 of any
/// write to `$8000`–`$FFFF`; bit 4 selects the single-screen nametable page (kept in
/// [`Axrom::page`], unused without a PPU). ANROM has no bus conflicts, so the written
/// byte is latched as is. Bank numbers wrap modulo the bank count.
pub struct Axrom {
    prg: Vec<u8>,
    banks: usize,
    pub bank: usize,
    pub page: u8,
    /// AMROM/AOROM variants conflict (NES 2.0 submapper 2); ANROM does not (default).
    pub bus_conflicts: bool,
}

impl Axrom {
    pub fn new(prg: Vec<u8>) -> Result<Self, MapperError> {
        if prg.is_empty() || prg.len() % (2 * BANK) != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks = prg.len() / (2 * BANK);
        if banks > 8 {
            return Err(MapperError::TooManyBanks(banks));
        }
        Ok(Self {
            prg,
            banks,
            bank: 0,
            page: 0,
            bus_conflicts: false,
        })
    }
}

impl Cartridge for Axrom {
    fn state_kind(&self) -> u8 {
        7
    }
    fn save_state(&self, w: &mut StateWriter) {
        w.u64(crate::nes_state::fnv64(&self.prg));
        w.u32(self.bank as u32);
        w.u8(self.page);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        self.bank = bank_index(r, self.banks, "AxROM bank")?;
        self.page = r.small(1, "AxROM page")?;
        Ok(())
    }
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        Some(if self.page == 0 {
            crate::ppu::Mirroring::SingleLow
        } else {
            crate::ppu::Mirroring::SingleHigh
        })
    }

    fn peek(&self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        self.prg[self.bank * 2 * BANK + usize::from(addr - 0x8000)]
    }

    fn write(&mut self, addr: u16, value: u8) {
        if addr >= 0x8000 {
            let value = if self.bus_conflicts {
                value & self.peek(addr)
            } else {
                value
            };
            self.bank = usize::from(value & 7) % self.banks;
            self.page = (value >> 4) & 1;
        }
    }
}

/// Discrete-logic latch boards with bus conflicts (the written byte is ANDed with the
/// ROM byte at the target address before it is latched):
/// * [`Latch::cnrom`] (mapper 3): PRG fixed like NROM (16 KiB mirrored or 32 KiB);
///   the latch picks an 8 KiB CHR bank (kept in [`Latch::chr_bank`], no PPU here).
/// * [`Latch::gxrom`] (mapper 66): bits 5–4 pick a 32 KiB PRG bank, bits 1–0 the CHR bank.
/// * [`Latch::color_dreams`] (mapper 11) and [`Latch::bnrom`] (mapper 34, CHR RAM).
/// * [`Latch::cnrom`] honours NES 2.0 submapper 1 (no conflicts) via `bus_conflicts`.
///
/// Bank numbers wrap modulo the bank count present.
pub struct Latch {
    prg: Vec<u8>,
    prg_banks: usize,
    chr_banks: usize,
    /// iNES mapper number: 3 CNROM, 66 GxROM, 11 Color Dreams, 34 BNROM.
    kind: u8,
    pub prg_bank: usize,
    pub chr_bank: usize,
    /// Written byte ANDed with ROM. True on every board here; NES 2.0 submapper 1 of
    /// mapper 3 (no conflicts) clears it.
    pub bus_conflicts: bool,
}

impl Latch {
    pub fn cnrom(prg: Vec<u8>, chr_banks: usize) -> Result<Self, MapperError> {
        if prg.len() != BANK && prg.len() != 2 * BANK {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        Self::build(prg, 1, chr_banks, 3)
    }

    pub fn gxrom(prg: Vec<u8>, chr_banks: usize) -> Result<Self, MapperError> {
        if prg.is_empty() || prg.len() % (2 * BANK) != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks = prg.len() / (2 * BANK);
        Self::build(prg, banks, chr_banks, 66)
    }

    /// Color Dreams (mapper 11): any write to `$8000`–`$FFFF` (with bus conflicts)
    /// selects the 32 KiB PRG bank from bits 1–0 and the 8 KiB CHR bank from bits
    /// 7–4. Up to 128 KiB PRG and 128 KiB CHR.
    pub fn color_dreams(prg: Vec<u8>, chr_banks: usize) -> Result<Self, MapperError> {
        if prg.is_empty() || prg.len() % (2 * BANK) != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks = prg.len() / (2 * BANK);
        Self::build(prg, banks, chr_banks, 11)
    }

    /// BNROM (mapper 34 without CHR ROM): a write to `$8000`–`$FFFF` (with bus
    /// conflicts) selects the 32 KiB PRG bank; CHR is 8 KiB RAM, not banked.
    pub fn bnrom(prg: Vec<u8>) -> Result<Self, MapperError> {
        if prg.is_empty() || prg.len() % (2 * BANK) != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks = prg.len() / (2 * BANK);
        Self::build(prg, banks, 1, 34)
    }

    fn build(
        prg: Vec<u8>,
        prg_banks: usize,
        chr_banks: usize,
        kind: u8,
    ) -> Result<Self, MapperError> {
        let (max_prg, max_chr) = match kind {
            11 => (4, 16),
            34 => (256, 1),
            _ => (4, 4),
        };
        if prg_banks > max_prg || chr_banks > max_chr {
            return Err(MapperError::TooManyBanks(prg_banks.max(chr_banks)));
        }
        Ok(Self {
            prg,
            prg_banks,
            chr_banks: chr_banks.max(1),
            kind,
            prg_bank: 0,
            chr_bank: 0,
            bus_conflicts: true,
        })
    }
}

impl Cartridge for Latch {
    fn state_kind(&self) -> u8 {
        self.kind
    }
    fn save_state(&self, w: &mut StateWriter) {
        w.u64(crate::nes_state::fnv64(&self.prg));
        w.u32(self.prg_bank as u32);
        w.u32(self.chr_bank as u32);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        self.prg_bank = bank_index(r, self.prg_banks, "PRG bank")?;
        self.chr_bank = bank_index(r, self.chr_banks, "CHR bank")?;
        Ok(())
    }
    fn chr_offset(&self, addr: u16) -> usize {
        self.chr_bank * 0x2000 + usize::from(addr & 0x1FFF)
    }

    fn peek(&self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let offset = usize::from(addr - 0x8000);
        if self.kind == 3 {
            self.prg[offset % self.prg.len()]
        } else {
            self.prg[self.prg_bank * 2 * BANK + offset]
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if addr < 0x8000 {
            return;
        }
        let v = if self.bus_conflicts {
            value & self.peek(addr)
        } else {
            value
        };
        let (prg, chr) = match self.kind {
            3 => (0, v & 3),
            66 => ((v >> 4) & 3, v & 3),
            11 => (v & 3, v >> 4),
            _ => (v, 0),
        };
        self.prg_bank = usize::from(prg) % self.prg_banks;
        self.chr_bank = usize::from(chr) % self.chr_banks;
    }
}

/// MMC3 (mapper 4, TxROM), Sharp-style IRQ counter.
///
/// * `$8000` even: bank select (bits 2–0 register R0–R7, bit 6 PRG mode, bit 7 CHR
///   inversion); `$8001` odd: data for the selected register.
/// * `$A000` even: mirroring (0 vertical, 1 horizontal; ignored with four-screen);
///   `$A001` odd: PRG RAM protect (bit 7 enable, bit 6 deny writes).
/// * `$C000` even: IRQ latch; `$C001` odd: clear the counter so the next clock reloads.
/// * `$E000` even: disable IRQ and acknowledge; `$E001` odd: enable IRQ.
///
/// PRG in 8 KiB banks. Mode 0: `$8000` R6, `$A000` R7, `$C000` second-last, `$E000`
/// last. Mode 1 swaps `$8000` and `$C000`. CHR in 1 KiB units: R0/R1 are 2 KiB banks
/// (bit 0 ignored) at `$0000`/`$0800`, R2–R5 1 KiB at `$1000`–`$1C00`; inversion swaps
/// the halves. The counter clocks on rising edges of PPU A12 reported through
/// [`Cartridge::ppu_address`] (the PPU reports one per rendered line), filtered as on
/// the chip: an edge counts only after A12 stayed low for at least [`Mmc3::A12_FILTER`]
/// CPU cycles ([`Cartridge::clock`]), so quick `$2006`/`$2007` toggles are ignored. At zero, or after `$C001`, it reloads from the
/// latch, else it decrements; reaching zero with IRQs enabled asserts IRQ (Sharp
/// behaviour: latch 0 then fires on every clock).
pub struct Mmc3 {
    prg: Vec<u8>,
    banks8: usize,
    ram: Box<[u8; 0x2000]>,
    pub select: u8,
    pub regs: [u8; 8],
    pub horizontal: bool,
    pub four_screen: bool,
    pub ram_protect: u8,
    pub latch: u8,
    pub counter: u8,
    reload: bool,
    pub irq_enabled: bool,
    pub irq_pending: bool,
    a12: bool,
    low_cycles: u32,
    /// Rising edges counted (diagnostics).
    pub edges: u64,
    /// Older MMC3A/NEC IRQ behaviour (NES 2.0 submapper 4): a clock that leaves the
    /// counter at zero asserts IRQ only if it decremented a non-zero counter or the
    /// `$C001` reload flag was set; reloading a latch of 0 by itself stays silent.
    pub alt_irq: bool,
    /// Board/ASIC variant sharing the MMC3 core.
    pub board: Mmc3Board,
    /// MMC6: 1 KiB internal RAM at `$7000`–`$7FFF` (two 512-byte halves, mirrored).
    pub mmc6_ram: Box<[u8; 0x400]>,
    /// TQROM: the 8 KiB CHR RAM beside the CHR ROM.
    pub chr_ram: Box<[u8; 0x2000]>,
}

/// Boards built on the MMC3 core.
///
/// * `Mmc3`: TxROM (mapper 4).
/// * `Mmc6`: HKROM (mapper 4 submapper 1). `$8000` bit 5 enables the 1 KiB RAM;
///   while it is clear `$A001` is held at 0. `$A001` bits 7/6 enable reading/writing
///   `$7200`–`$73FF`, bits 5/4 `$7000`–`$71FF`; a half is writable only while also
///   readable. With neither half readable `$7000`–`$7FFF` is open bus; with one, the
///   other reads 0. `$6000`–`$6FFF` is unmapped.
/// * `TxSrom` (mapper 118): CIRAM A10 is bit 7 of the CHR bank that the MMC3
///   selects for the nametable address with A13 ignored (`$2000` → the `$0000`
///   bank), so the CHR registers also pick each nametable; `$A000` has no effect.
/// * `Tqrom` (mapper 119): bit 6 of a CHR bank selects the 8 KiB CHR RAM (1) or
///   the CHR ROM (0); the remaining bits index the selected chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mmc3Board {
    Mmc3,
    Mmc6,
    TxSrom,
    Tqrom,
}

impl Mmc3 {
    /// The same core on another board.
    pub fn with_board(
        prg: Vec<u8>,
        four_screen: bool,
        board: Mmc3Board,
    ) -> Result<Self, MapperError> {
        let mut m = Self::new(prg, four_screen)?;
        m.board = board;
        if board == Mmc3Board::Mmc6 {
            m.ram_protect = 0;
        }
        Ok(m)
    }

    fn mmc6_enabled(&self) -> bool {
        self.select & 0x20 != 0
    }

    /// MMC6 `$7000`–`$7FFF` read: `None` is open bus.
    pub fn mmc6_read(&self, addr: u16) -> Option<u8> {
        let p = self.ram_protect;
        let (lo, hi) = (p & 0x20 != 0, p & 0x80 != 0);
        if !self.mmc6_enabled() || !(lo || hi) {
            return None;
        }
        let high_half = addr & 0x200 != 0;
        let readable = if high_half { hi } else { lo };
        Some(if readable {
            self.mmc6_ram[usize::from(addr & 0x3FF)]
        } else {
            0
        })
    }

    /// CPU cycles A12 must stay low before a rising edge clocks the counter.
    pub const A12_FILTER: u32 = 3;

    pub fn new(prg: Vec<u8>, four_screen: bool) -> Result<Self, MapperError> {
        if prg.is_empty() || prg.len() % 0x2000 != 0 || prg.len() < 0x4000 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks8 = prg.len() / 0x2000;
        if banks8 > 256 {
            return Err(MapperError::TooManyBanks(banks8));
        }
        Ok(Self {
            prg,
            banks8,
            ram: Box::new([0; 0x2000]),
            select: 0,
            regs: [0, 2, 4, 5, 6, 7, 0, 1],
            horizontal: false,
            four_screen,
            ram_protect: 0x80,
            latch: 0,
            counter: 0,
            reload: false,
            irq_enabled: false,
            irq_pending: false,
            a12: false,
            low_cycles: Self::A12_FILTER,
            edges: 0,
            alt_irq: false,
            board: Mmc3Board::Mmc3,
            mmc6_ram: Box::new([0; 0x400]),
            chr_ram: Box::new([0; 0x2000]),
        })
    }

    /// 8 KiB bank number mapped at `$8000 + slot * $2000`.
    pub fn prg_bank(&self, slot: usize) -> usize {
        let last = self.banks8 - 1;
        let mode = self.select & 0x40 != 0;
        let bank = match (slot, mode) {
            (0, false) | (2, true) => usize::from(self.regs[6] & 0x3F),
            (1, _) => usize::from(self.regs[7] & 0x3F),
            (0, true) | (2, false) => last - 1,
            _ => last,
        };
        bank % self.banks8
    }

    /// 1 KiB CHR bank at `slot * $400`.
    pub fn chr_bank(&self, slot: usize) -> usize {
        let slot = if self.select & 0x80 != 0 {
            slot ^ 4
        } else {
            slot
        };
        match slot {
            0 | 1 => usize::from(self.regs[0] & 0xFE) + slot,
            2 | 3 => usize::from(self.regs[1] & 0xFE) + slot - 2,
            _ => usize::from(self.regs[slot - 2]),
        }
    }

    fn clock_counter(&mut self) {
        self.edges += 1;
        let (was, forced) = (self.counter, self.reload);
        if self.counter == 0 || self.reload {
            self.counter = self.latch;
            self.reload = false;
        } else {
            self.counter -= 1;
        }
        let fires = !self.alt_irq || was != 0 || forced;
        if self.counter == 0 && self.irq_enabled && fires {
            self.irq_pending = true;
        }
    }
}

impl Cartridge for Mmc3 {
    /// MMC6 boards keep their battery on the internal 1 KiB.
    fn save_ram(&self) -> Option<Vec<u8>> {
        Some(if self.board == Mmc3Board::Mmc6 {
            self.mmc6_ram.to_vec()
        } else {
            self.ram.to_vec()
        })
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        if self.board == Mmc3Board::Mmc6 {
            restore_ram(&mut self.mmc6_ram[..], data)
        } else {
            restore_ram(&mut self.ram[..], data)
        }
    }
    fn state_kind(&self) -> u8 {
        match self.board {
            Mmc3Board::TxSrom => 118,
            Mmc3Board::Tqrom => 119,
            _ => 4,
        }
    }
    fn save_state(&self, w: &mut StateWriter) {
        w.u64(crate::nes_state::fnv64(&self.prg));
        w.bytes(&self.ram[..]);
        w.u8(self.select);
        w.bytes(&self.regs);
        for f in [self.horizontal, self.four_screen] {
            w.bool(f);
        }
        w.u8(self.ram_protect);
        w.u8(self.latch);
        w.u8(self.counter);
        for f in [self.reload, self.irq_enabled, self.irq_pending, self.a12] {
            w.bool(f);
        }
        w.u32(self.low_cycles);
        w.u64(self.edges);
        if w.version >= 7 {
            match self.board {
                Mmc3Board::Mmc6 => w.bytes(&self.mmc6_ram[..]),
                Mmc3Board::Tqrom => w.bytes(&self.chr_ram[..]),
                _ => {}
            }
        }
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        self.select = r.u8("MMC3 select")?;
        r.fill(&mut self.regs, "MMC3 registers")?;
        self.horizontal = r.bool("MMC3 mirroring")?;
        self.four_screen = r.bool("MMC3 four-screen")?;
        self.ram_protect = r.u8("MMC3 RAM protect")?;
        self.latch = r.u8("MMC3 latch")?;
        self.counter = r.u8("MMC3 counter")?;
        self.reload = r.bool("MMC3 reload")?;
        self.irq_enabled = r.bool("MMC3 IRQ enable")?;
        self.irq_pending = r.bool("MMC3 IRQ")?;
        self.a12 = r.bool("MMC3 A12")?;
        self.low_cycles = r.u32("MMC3 A12 low")?;
        self.edges = r.u64("MMC3 edges")?;
        if r.version >= 7 {
            match self.board {
                Mmc3Board::Mmc6 => r.fill(&mut self.mmc6_ram[..], "MMC6 RAM")?,
                Mmc3Board::Tqrom => r.fill(&mut self.chr_ram[..], "TQROM CHR RAM")?,
                _ => {}
            }
        }
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        if self.board == Mmc3Board::Mmc6 && (0x6000..0x8000).contains(&addr) {
            return if addr >= 0x7000 {
                self.mmc6_read(addr).unwrap_or(0)
            } else {
                0
            };
        }
        match (addr, ram_index(addr)) {
            (_, Some(i)) if self.ram_protect & 0x80 != 0 => self.ram[i],
            (0x8000.., _) => {
                let slot = usize::from((addr - 0x8000) >> 13);
                self.prg[self.prg_bank(slot) * 0x2000 + usize::from(addr & 0x1FFF)]
            }
            _ => 0,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if self.board == Mmc3Board::Mmc6 {
            match addr {
                0x7000..=0x7FFF => {
                    let shift = if addr & 0x200 != 0 { 6 } else { 4 };
                    let bits = (self.ram_protect >> shift) & 3;
                    if self.mmc6_enabled() && bits == 3 {
                        self.mmc6_ram[usize::from(addr & 0x3FF)] = value;
                    }
                    return;
                }
                0x6000..=0x6FFF => return,
                _ if addr & 0xE001 == 0xA001 => {
                    if self.mmc6_enabled() {
                        self.ram_protect = value & 0xF0;
                    }
                    return;
                }
                _ if addr & 0xE001 == 0x8000 => {
                    self.select = value;
                    if !self.mmc6_enabled() {
                        self.ram_protect = 0;
                    }
                    return;
                }
                _ => {}
            }
        }
        if let Some(i) = ram_index(addr) {
            if self.ram_protect & 0xC0 == 0x80 {
                self.ram[i] = value;
            }
            return;
        }
        if addr < 0x8000 {
            return;
        }
        match addr & 0xE001 {
            0x8000 => self.select = value,
            0x8001 => self.regs[usize::from(self.select & 7)] = value,
            0xA000 => self.horizontal = value & 1 != 0,
            0xA001 => self.ram_protect = value,
            0xC000 => self.latch = value,
            0xC001 => {
                self.counter = 0;
                self.reload = true;
            }
            0xE000 => {
                self.irq_enabled = false;
                self.irq_pending = false;
            }
            _ => self.irq_enabled = true,
        }
    }

    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        self.chr_bank(a >> 10) * 0x400 + (a & 0x3FF)
    }

    fn ppu_target(&self, addr: u16) -> PpuTarget {
        match self.board {
            Mmc3Board::TxSrom if addr >= 0x2000 => {
                let page = (self.chr_bank(usize::from(addr & 0x0FFF) >> 10) >> 7) & 1;
                PpuTarget::Ciram(page * 0x400 + usize::from(addr & 0x3FF))
            }
            Mmc3Board::Tqrom if addr < 0x2000 => {
                let bank = self.chr_bank(usize::from(addr) >> 10);
                if bank & 0x40 != 0 {
                    PpuTarget::Mapper(self.chr_ram[(bank & 7) * 0x400 + usize::from(addr & 0x3FF)])
                } else {
                    PpuTarget::Chr((bank & 0x3F) * 0x400 + usize::from(addr & 0x3FF))
                }
            }
            _ => PpuTarget::Default,
        }
    }

    fn ppu_write(&mut self, addr: u16, value: u8) {
        if self.board == Mmc3Board::Tqrom && addr < 0x2000 {
            let bank = self.chr_bank(usize::from(addr) >> 10);
            self.chr_ram[(bank & 7) * 0x400 + usize::from(addr & 0x3FF)] = value;
        }
    }

    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        use crate::ppu::Mirroring::*;
        Some(if self.four_screen {
            FourScreen
        } else if self.horizontal {
            Horizontal
        } else {
            Vertical
        })
    }

    fn clock(&mut self) {
        if !self.a12 {
            self.low_cycles = self.low_cycles.saturating_add(1);
        }
    }

    fn ppu_address(&mut self, addr: u16) {
        let high = addr & 0x1000 != 0;
        if high && !self.a12 && self.low_cycles >= Self::A12_FILTER {
            self.clock_counter();
        }
        if !high && self.a12 {
            self.low_cycles = 0;
        }
        self.a12 = high;
    }

    fn irq(&self) -> bool {
        self.irq_pending
    }
}

fn checked<const N: usize>(
    v: [u8; N],
    max: u8,
    what: &'static str,
) -> Result<[u8; N], NesStateError> {
    match v.iter().find(|&&b| b > max) {
        Some(&b) => Err(NesStateError::BadValue {
            what,
            value: u64::from(b),
        }),
        None => Ok(v),
    }
}

fn prg_ram_state(w: &mut StateWriter, prg: &[u8], ram: &[u8]) {
    w.u64(crate::nes_state::fnv64(prg));
    w.bytes(ram);
}

/// MMC2 (mapper 9, PxROM) and MMC4 (mapper 10, FxROM): CHR switching driven by the
/// PPU's own pattern fetches.
///
/// * `$A000`–`$AFFF`: PRG bank. MMC2: 8 KiB at `$8000` (bits 3–0), the last three
///   8 KiB banks fixed at `$A000`–`$FFFF`. MMC4: 16 KiB at `$8000` (bits 3–0), the
///   last 16 KiB fixed at `$C000`; MMC4 boards carry 8 KiB PRG RAM at `$6000`.
/// * `$B000`/`$C000`: 4 KiB CHR bank at `$0000` used while latch 0 holds `$FD`/`$FE`.
/// * `$D000`/`$E000`: 4 KiB CHR bank at `$1000` for latch 1 `$FD`/`$FE` (bits 4–0).
/// * `$F000`: bit 0 mirroring (0 vertical, 1 horizontal).
///
/// Latch 0 becomes `$FD` when the PPU reads `$0FD8` and `$FE` on `$0FE8`; latch 1
/// becomes `$FD` on `$1FD8`–`$1FDF` and `$FE` on `$1FE8`–`$1FEF`. MMC4 uses the
/// 8-byte ranges for both latches. The switch happens after the triggering fetch,
/// so the tile `$FD`/`$FE` itself still comes from the old bank. The latches power
/// up in an unspecified state; the model starts both at `$FE`.
pub struct Mmc24 {
    prg: Vec<u8>,
    mmc4: bool,
    ram: Box<[u8; 0x2000]>,
    pub prg_bank: u8,
    /// `[$0000 FD, $0000 FE, $1000 FD, $1000 FE]`.
    pub chr: [u8; 4],
    /// `true` while the latch holds `$FE`.
    pub latch: [bool; 2],
    pub horizontal: bool,
}

impl Mmc24 {
    pub fn mmc2(prg: Vec<u8>) -> Result<Self, MapperError> {
        if prg.len() < 0x8000 || prg.len() % 0x2000 != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        Self::build(prg, false, 16)
    }

    pub fn mmc4(prg: Vec<u8>) -> Result<Self, MapperError> {
        check_banks(&prg, 16)?;
        if prg.len() < 2 * BANK {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        Self::build(prg, true, 16)
    }

    fn build(prg: Vec<u8>, mmc4: bool, max: usize) -> Result<Self, MapperError> {
        let unit = if mmc4 { BANK } else { 0x2000 };
        if prg.len() / unit > max {
            return Err(MapperError::TooManyBanks(prg.len() / unit));
        }
        Ok(Self {
            prg,
            mmc4,
            ram: Box::new([0; 0x2000]),
            prg_bank: 0,
            chr: [0; 4],
            latch: [true; 2],
            horizontal: false,
        })
    }

    /// 4 KiB CHR bank currently selected for pattern table `half` (0 or 1).
    pub fn chr_bank(&self, half: usize) -> usize {
        usize::from(self.chr[half * 2 + usize::from(self.latch[half])])
    }
}

impl Cartridge for Mmc24 {
    fn save_ram(&self) -> Option<Vec<u8>> {
        self.mmc4.then(|| self.ram.to_vec())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        if !self.mmc4 {
            return Err(SaveRamError::NoSaveRam);
        }
        restore_ram(&mut self.ram[..], data)
    }
    fn state_kind(&self) -> u8 {
        if self.mmc4 {
            10
        } else {
            9
        }
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.prg, &self.ram[..]);
        w.u8(self.prg_bank);
        w.bytes(&self.chr);
        for f in [self.latch[0], self.latch[1], self.horizontal] {
            w.bool(f);
        }
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        self.prg_bank = r.small(0x0F, "MMC2 PRG")?;
        let mut chr = [0u8; 4];
        r.fill(&mut chr, "MMC2 CHR")?;
        self.chr = checked(chr, 0x1F, "MMC2 CHR")?;
        self.latch = [r.bool("MMC2 latch 0")?, r.bool("MMC2 latch 1")?];
        self.horizontal = r.bool("MMC2 mirroring")?;
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        let len = self.prg.len();
        match (addr, ram_index(addr)) {
            (_, Some(i)) if self.mmc4 => self.ram[i],
            (0x8000.., _) if self.mmc4 => {
                let bank = if addr < 0xC000 {
                    usize::from(self.prg_bank) % (len / BANK)
                } else {
                    len / BANK - 1
                };
                self.prg[bank * BANK + usize::from(addr & 0x3FFF)]
            }
            (0x8000.., _) => {
                let slot = usize::from((addr - 0x8000) >> 13);
                let banks = len / 0x2000;
                let bank = if slot == 0 {
                    usize::from(self.prg_bank) % banks
                } else {
                    banks - 4 + slot
                };
                self.prg[bank * 0x2000 + usize::from(addr & 0x1FFF)]
            }
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        if let Some(i) = ram_index(addr) {
            if self.mmc4 {
                self.ram[i] = value;
            }
            return;
        }
        match addr & 0xF000 {
            0xA000 => self.prg_bank = value & 0x0F,
            0xB000 => self.chr[0] = value & 0x1F,
            0xC000 => self.chr[1] = value & 0x1F,
            0xD000 => self.chr[2] = value & 0x1F,
            0xE000 => self.chr[3] = value & 0x1F,
            0xF000 => self.horizontal = value & 1 != 0,
            _ => {}
        }
    }
    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        self.chr_bank(a >> 12) * 0x1000 + (a & 0xFFF)
    }
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        Some(if self.horizontal {
            crate::ppu::Mirroring::Horizontal
        } else {
            crate::ppu::Mirroring::Vertical
        })
    }
    fn ppu_address(&mut self, addr: u16) {
        let a = addr & 0x3FFF;
        let half = usize::from(a >> 12);
        if half > 1 {
            return;
        }
        let exact = half == 0 && !self.mmc4;
        let tile = match (a & 0x0FF8, a & 7) {
            (0x0FD8, 0) => Some(false),
            (0x0FE8, 0) => Some(true),
            (0x0FD8, _) if !exact => Some(false),
            (0x0FE8, _) if !exact => Some(true),
            _ => None,
        };
        if let Some(fe) = tile {
            self.latch[half] = fe;
        }
    }
}

/// NINA-001 (mapper 34 with CHR ROM, NES 2.0 submapper 1): 8 KiB PRG RAM at
/// `$6000`–`$7FFF`; writes to `$7FFD`–`$7FFF` land in that RAM and also set
/// `$7FFD` bit 0 the 32 KiB PRG bank, `$7FFE` bits 3–0 the 4 KiB CHR bank at
/// `$0000`, `$7FFF` bits 3–0 the 4 KiB CHR bank at `$1000`. No bus conflicts.
pub struct Nina001 {
    prg: Vec<u8>,
    ram: Box<[u8; 0x2000]>,
    pub prg_bank: u8,
    pub chr: [u8; 2],
}

impl Nina001 {
    pub fn new(prg: Vec<u8>) -> Result<Self, MapperError> {
        if prg.is_empty() || prg.len() % (2 * BANK) != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        if prg.len() > 4 * BANK {
            return Err(MapperError::TooManyBanks(prg.len() / (2 * BANK)));
        }
        Ok(Self {
            prg,
            ram: Box::new([0; 0x2000]),
            prg_bank: 0,
            chr: [0, 1],
        })
    }
}

impl Cartridge for Nina001 {
    fn save_ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.to_vec())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        restore_ram(&mut self.ram[..], data)
    }
    fn state_kind(&self) -> u8 {
        34
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.prg, &self.ram[..]);
        w.u8(self.prg_bank);
        w.bytes(&self.chr);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        self.prg_bank = r.small(1, "NINA-001 PRG")?;
        let mut chr = [0u8; 2];
        r.fill(&mut chr, "NINA-001 CHR")?;
        self.chr = checked(chr, 0x0F, "NINA-001 CHR")?;
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        match (addr, ram_index(addr)) {
            (_, Some(i)) => self.ram[i],
            (0x8000.., _) => {
                let banks = self.prg.len() / (2 * BANK);
                let bank = usize::from(self.prg_bank) % banks;
                self.prg[bank * 2 * BANK + usize::from(addr - 0x8000)]
            }
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        if let Some(i) = ram_index(addr) {
            self.ram[i] = value;
        }
        match addr {
            0x7FFD => self.prg_bank = value & 1,
            0x7FFE => self.chr[0] = value & 0x0F,
            0x7FFF => self.chr[1] = value & 0x0F,
            _ => {}
        }
    }
    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        usize::from(self.chr[a >> 12]) * 0x1000 + (a & 0xFFF)
    }
}

/// Camerica/Codemasters (mapper 71, BF909x): a write to `$C000`–`$FFFF` selects the
/// 16 KiB bank at `$8000` (bits 3–0); `$C000` is fixed to the last bank. CHR is
/// 8 KiB RAM; no bus conflicts. On the BF9097 board (Fire Hawk, NES 2.0 submapper 1)
/// a write to `$8000`–`$9FFF` sets single-screen mirroring from bit 4; other boards
/// keep the header's mirroring and ignore `$8000`–`$BFFF`.
pub struct Camerica {
    prg: Vec<u8>,
    banks: usize,
    pub bank: usize,
    pub fire_hawk: bool,
    pub page: u8,
}

impl Camerica {
    pub fn new(prg: Vec<u8>, fire_hawk: bool) -> Result<Self, MapperError> {
        let banks = check_banks(&prg, 16)?;
        Ok(Self {
            prg,
            banks,
            bank: 0,
            fire_hawk,
            page: 0,
        })
    }
}

impl Cartridge for Camerica {
    fn state_kind(&self) -> u8 {
        71
    }
    fn save_state(&self, w: &mut StateWriter) {
        w.u64(crate::nes_state::fnv64(&self.prg));
        w.u32(self.bank as u32);
        w.u8(self.page);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        self.bank = bank_index(r, self.banks, "Camerica bank")?;
        self.page = r.small(1, "Camerica page")?;
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        let offset = usize::from(addr & 0x3FFF);
        match addr {
            0x8000..=0xBFFF => self.prg[self.bank * BANK + offset],
            0xC000.. => self.prg[(self.banks - 1) * BANK + offset],
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x8000..=0x9FFF if self.fire_hawk => self.page = (value >> 4) & 1,
            0xC000.. => self.bank = usize::from(value & 0x0F) % self.banks,
            _ => {}
        }
    }
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        use crate::ppu::Mirroring::*;
        self.fire_hawk.then_some(if self.page == 0 {
            SingleLow
        } else {
            SingleHigh
        })
    }
}

/// Konami VRC1 (mapper 75): 8 KiB PRG banks at `$8000` (`$8000`), `$A000` (`$A000`),
/// `$C000` (`$C000`), last bank fixed at `$E000`, bits 3–0 each. `$9000`: bit 0
/// mirroring (0 vertical, 1 horizontal; ignored with four-screen VRAM), bit 1 and
/// bit 2 the high bit (bit 4) of the 4 KiB CHR banks at `$0000` and `$1000`, whose
/// low four bits come from `$E000` and `$F000`. `$B000` and `$D000` do nothing.
pub struct Vrc1 {
    prg: Vec<u8>,
    banks8: usize,
    pub prg_banks: [u8; 3],
    pub chr: [u8; 2],
    pub horizontal: bool,
    pub four_screen: bool,
}

impl Vrc1 {
    pub fn new(prg: Vec<u8>, four_screen: bool) -> Result<Self, MapperError> {
        if prg.len() < 0x4000 || prg.len() % 0x2000 != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks8 = prg.len() / 0x2000;
        if banks8 > 16 {
            return Err(MapperError::TooManyBanks(banks8));
        }
        Ok(Self {
            prg,
            banks8,
            prg_banks: [0; 3],
            chr: [0; 2],
            horizontal: false,
            four_screen,
        })
    }
}

impl Cartridge for Vrc1 {
    fn state_kind(&self) -> u8 {
        75
    }
    fn save_state(&self, w: &mut StateWriter) {
        w.u64(crate::nes_state::fnv64(&self.prg));
        w.bytes(&self.prg_banks);
        w.bytes(&self.chr);
        w.bool(self.horizontal);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        let (mut prg, mut chr) = ([0u8; 3], [0u8; 2]);
        r.fill(&mut prg, "VRC1 PRG")?;
        r.fill(&mut chr, "VRC1 CHR")?;
        self.prg_banks = checked(prg, 0x0F, "VRC1 PRG")?;
        self.chr = checked(chr, 0x1F, "VRC1 CHR")?;
        self.horizontal = r.bool("VRC1 mirroring")?;
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        if addr < 0x8000 {
            return 0;
        }
        let slot = usize::from((addr - 0x8000) >> 13);
        let bank = if slot == 3 {
            self.banks8 - 1
        } else {
            usize::from(self.prg_banks[slot]) % self.banks8
        };
        self.prg[bank * 0x2000 + usize::from(addr & 0x1FFF)]
    }
    fn write(&mut self, addr: u16, value: u8) {
        match addr & 0xF000 {
            0x8000 => self.prg_banks[0] = value & 0x0F,
            0xA000 => self.prg_banks[1] = value & 0x0F,
            0xC000 => self.prg_banks[2] = value & 0x0F,
            0x9000 => {
                self.horizontal = value & 1 != 0;
                self.chr[0] = (self.chr[0] & 0x0F) | ((value << 3) & 0x10);
                self.chr[1] = (self.chr[1] & 0x0F) | ((value << 2) & 0x10);
            }
            0xE000 => self.chr[0] = (self.chr[0] & 0x10) | (value & 0x0F),
            0xF000 => self.chr[1] = (self.chr[1] & 0x10) | (value & 0x0F),
            _ => {}
        }
    }
    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        usize::from(self.chr[a >> 12]) * 0x1000 + (a & 0xFFF)
    }
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        use crate::ppu::Mirroring::*;
        (!self.four_screen).then_some(if self.horizontal {
            Horizontal
        } else {
            Vertical
        })
    }
}

/// Konami VRC2/VRC4 board variant: the two CPU address lines wired to the chip's
/// register-select inputs, and the chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VrcWiring {
    /// Address masks driving register bit 0 and bit 1; each may be the OR of two
    /// lines when the submapper is unknown (both candidate wirings at once).
    pub a0: u16,
    pub a1: u16,
    pub vrc4: bool,
    /// VRC2a (mapper 22): CHR bank numbers are shifted right one bit.
    pub chr_shift: bool,
}

impl VrcWiring {
    /// Board for iNES mapper `mapper` (21, 22, 23, 25) and NES 2.0 `submapper`,
    /// following the NESdev VRC2/VRC4 submapper table:
    /// * 21: 1 VRC4a (A1, A2), 2 VRC4c (A6, A7); 0 both.
    /// * 22: VRC2a (A1, A0; CHR >> 1), submapper 0 only.
    /// * 23: 1 VRC4f (A0, A1), 2 VRC4e (A2, A3), 3 VRC2b (A0, A1); 0 VRC4 with both.
    /// * 25: 1 VRC4b (A1, A0), 2 VRC4d (A3, A2), 3 VRC2c (A1, A0); 0 VRC4 with both.
    pub fn for_mapper(mapper: u16, submapper: u8) -> Option<Self> {
        let w = |a0, a1, vrc4| {
            Some(Self {
                a0,
                a1,
                vrc4,
                chr_shift: false,
            })
        };
        match (mapper, submapper) {
            (21, 1) => w(0x02, 0x04, true),
            (21, 2) => w(0x40, 0x80, true),
            (21, 0) => w(0x42, 0x84, true),
            (22, 0) => Some(Self {
                a0: 0x02,
                a1: 0x01,
                vrc4: false,
                chr_shift: true,
            }),
            (23, 1) => w(0x01, 0x02, true),
            (23, 2) => w(0x04, 0x08, true),
            (23, 3) => w(0x01, 0x02, false),
            (23, 0) => w(0x05, 0x0A, true),
            (25, 1) => w(0x02, 0x01, true),
            (25, 2) => w(0x08, 0x04, true),
            (25, 3) => w(0x02, 0x01, false),
            (25, 0) => w(0x0A, 0x05, true),
            _ => None,
        }
    }
}

/// VRC IRQ counter shared by VRC4 (and VRC6/7): an 8-bit up-counter clocked either
/// every CPU cycle (mode bit set) or once per scanline by a prescaler that subtracts
/// 3 per CPU cycle from 341 and clocks when it reaches zero or below. Clocking at
/// `$FF` reloads the latch and asserts IRQ when enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VrcIrq {
    pub latch: u8,
    pub counter: u8,
    pub prescaler: i16,
    pub enabled: bool,
    pub enable_after_ack: bool,
    pub cycle_mode: bool,
    pub pending: bool,
}

impl VrcIrq {
    /// Control write (`$F002` on VRC4): A, E, M bits; acknowledges; E reloads.
    pub fn control(&mut self, v: u8) {
        self.enable_after_ack = v & 1 != 0;
        self.enabled = v & 2 != 0;
        self.cycle_mode = v & 4 != 0;
        self.pending = false;
        if self.enabled {
            self.counter = self.latch;
            self.prescaler = 341;
        }
    }
    /// Acknowledge (`$F003`): clears the IRQ and copies A into E.
    pub fn ack(&mut self) {
        self.pending = false;
        self.enabled = self.enable_after_ack;
    }
    pub fn clock(&mut self) {
        if !self.enabled {
            return;
        }
        if self.cycle_mode {
            self.step();
        } else {
            self.prescaler -= 3;
            if self.prescaler <= 0 {
                self.prescaler += 341;
                self.step();
            }
        }
    }
    fn step(&mut self) {
        if self.counter == 0xFF {
            self.counter = self.latch;
            self.pending = true;
        } else {
            self.counter += 1;
        }
    }
    fn save(&self, w: &mut StateWriter) {
        w.u8(self.latch);
        w.u8(self.counter);
        w.u16(self.prescaler as u16);
        for f in [
            self.enabled,
            self.enable_after_ack,
            self.cycle_mode,
            self.pending,
        ] {
            w.bool(f);
        }
    }
    fn load(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        self.latch = r.u8("VRC IRQ latch")?;
        self.counter = r.u8("VRC IRQ counter")?;
        let p = r.u16("VRC prescaler")? as i16;
        if !(-2..=341).contains(&p) {
            return Err(NesStateError::BadValue {
                what: "VRC prescaler",
                value: u64::from(p as u16),
            });
        }
        self.prescaler = p;
        self.enabled = r.bool("VRC IRQ enable")?;
        self.enable_after_ack = r.bool("VRC IRQ A")?;
        self.cycle_mode = r.bool("VRC IRQ mode")?;
        self.pending = r.bool("VRC IRQ")?;
        Ok(())
    }
}

/// Konami VRC2 (mappers 22, 23, 25) and VRC4 (21, 23, 25).
///
/// After [`VrcWiring`] turns the CPU address into `$x000 + reg` (reg 0–3):
/// * `$8000`: PRG bank 0 (5 bits); `$A000`: PRG bank 1. VRC2: `$8000` R0,
///   `$A000` R1, `$C000` second-last, `$E000` last. VRC4 `$9002` bit 1 swaps:
///   `$8000` second-last, `$C000` R0.
/// * `$9000`: VRC2 bit 0 mirroring (vertical, horizontal); VRC4 `$9000`/`$9001` bits
///   1–0 vertical, horizontal, one-screen low, one-screen high; VRC4 `$9002`/`$9003`
///   bit 0 enables PRG RAM, bit 1 is the PRG swap mode.
/// * `$B000`–`$E003`: eight 1 KiB CHR banks, low nibble at reg 0/2, high bits at
///   reg 1/3 (VRC2: 4 bits, VRC4: 5 bits). VRC2a shifts the bank right one bit.
/// * VRC4 `$F000`/`$F001` IRQ latch low/high nibble, `$F002` control, `$F003` ack
///   ([`VrcIrq`]).
/// * VRC2 without PRG RAM: `$6000`–`$6FFF` is a one-bit latch (bit 0 reads back,
///   the rest is open bus, modelled as `$60`).
pub struct Vrc24 {
    prg: Vec<u8>,
    banks8: usize,
    pub wiring: VrcWiring,
    ram: Box<[u8; 0x2000]>,
    pub prg_regs: [u8; 2],
    pub chr: [u16; 8],
    pub mirroring_bits: u8,
    pub swap: bool,
    pub ram_enabled: bool,
    pub latch_bit: u8,
    pub irq: VrcIrq,
}

impl Vrc24 {
    pub fn new(prg: Vec<u8>, wiring: VrcWiring) -> Result<Self, MapperError> {
        if prg.len() < 0x4000 || prg.len() % 0x2000 != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks8 = prg.len() / 0x2000;
        if banks8 > 32 {
            return Err(MapperError::TooManyBanks(banks8));
        }
        Ok(Self {
            prg,
            banks8,
            wiring,
            ram: Box::new([0; 0x2000]),
            prg_regs: [0; 2],
            chr: [0; 8],
            mirroring_bits: 0,
            swap: false,
            ram_enabled: false,
            latch_bit: 0,
            irq: VrcIrq::default(),
        })
    }

    /// Register number 0–3 selected by the wired address lines.
    pub fn reg(&self, addr: u16) -> u8 {
        u8::from(addr & self.wiring.a0 != 0) | (u8::from(addr & self.wiring.a1 != 0) << 1)
    }

    pub fn prg_bank(&self, slot: usize) -> usize {
        let last = self.banks8 - 1;
        let r0 = usize::from(self.prg_regs[0] & 0x1F) % self.banks8;
        let r1 = usize::from(self.prg_regs[1] & 0x1F) % self.banks8;
        match (slot, self.wiring.vrc4 && self.swap) {
            (0, false) | (2, true) => r0,
            (1, _) => r1,
            (0, true) | (2, false) => last - 1,
            _ => last,
        }
    }

    pub fn chr_bank(&self, slot: usize) -> usize {
        let b = usize::from(self.chr[slot]);
        if self.wiring.chr_shift {
            b >> 1
        } else {
            b
        }
    }
}

impl Cartridge for Vrc24 {
    fn save_ram(&self) -> Option<Vec<u8>> {
        self.wiring.vrc4.then(|| self.ram.to_vec())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        if !self.wiring.vrc4 {
            return Err(SaveRamError::NoSaveRam);
        }
        restore_ram(&mut self.ram[..], data)
    }
    fn state_kind(&self) -> u8 {
        if self.wiring.vrc4 {
            21
        } else {
            22
        }
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.prg, &self.ram[..]);
        w.bytes(&self.prg_regs);
        for c in self.chr {
            w.u16(c);
        }
        w.u8(self.mirroring_bits);
        w.bool(self.swap);
        w.bool(self.ram_enabled);
        w.u8(self.latch_bit);
        self.irq.save(w);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        let mut prg = [0u8; 2];
        r.fill(&mut prg, "VRC PRG")?;
        self.prg_regs = checked(prg, 0x1F, "VRC PRG")?;
        for c in &mut self.chr {
            let v = r.u16("VRC CHR")?;
            if v > 0x1FF {
                return Err(NesStateError::BadValue {
                    what: "VRC CHR",
                    value: u64::from(v),
                });
            }
            *c = v;
        }
        self.mirroring_bits = r.small(3, "VRC mirroring")?;
        self.swap = r.bool("VRC swap")?;
        self.ram_enabled = r.bool("VRC RAM")?;
        self.latch_bit = r.small(1, "VRC2 latch")?;
        self.irq.load(r)
    }
    fn peek(&self, addr: u16) -> u8 {
        match (addr, ram_index(addr)) {
            (_, Some(i)) if self.wiring.vrc4 => {
                if self.ram_enabled {
                    self.ram[i]
                } else {
                    0
                }
            }
            (0x6000..=0x6FFF, _) => 0x60 | self.latch_bit,
            (0x8000.., _) => {
                let slot = usize::from((addr - 0x8000) >> 13);
                self.prg[self.prg_bank(slot) * 0x2000 + usize::from(addr & 0x1FFF)]
            }
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        if let Some(i) = ram_index(addr) {
            if self.wiring.vrc4 {
                if self.ram_enabled {
                    self.ram[i] = value;
                }
            } else if addr < 0x7000 {
                self.latch_bit = value & 1;
            }
            return;
        }
        if addr < 0x8000 {
            return;
        }
        let reg = self.reg(addr);
        let vrc4 = self.wiring.vrc4;
        match addr & 0xF000 {
            0x8000 => self.prg_regs[0] = value & 0x1F,
            0xA000 => self.prg_regs[1] = value & 0x1F,
            0x9000 if !vrc4 => self.mirroring_bits = value & 1,
            0x9000 if reg < 2 => self.mirroring_bits = value & 3,
            0x9000 => {
                self.ram_enabled = value & 1 != 0;
                self.swap = value & 2 != 0;
            }
            a @ 0xB000..=0xE000 => {
                let slot = usize::from((a - 0xB000) >> 12) * 2 + usize::from(reg >> 1);
                let c = &mut self.chr[slot];
                if reg & 1 == 0 {
                    *c = (*c & !0x0F) | u16::from(value & 0x0F);
                } else {
                    let hi = if vrc4 { value & 0x1F } else { value & 0x0F };
                    *c = (*c & 0x0F) | (u16::from(hi) << 4);
                }
            }
            0xF000 if vrc4 => match reg {
                0 => self.irq.latch = (self.irq.latch & 0xF0) | (value & 0x0F),
                1 => self.irq.latch = (self.irq.latch & 0x0F) | (value << 4),
                2 => self.irq.control(value),
                _ => self.irq.ack(),
            },
            _ => {}
        }
    }
    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        self.chr_bank(a >> 10) * 0x400 + (a & 0x3FF)
    }
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        use crate::ppu::Mirroring::*;
        Some([Vertical, Horizontal, SingleLow, SingleHigh][usize::from(self.mirroring_bits)])
    }
    fn clock(&mut self) {
        if self.wiring.vrc4 {
            self.irq.clock();
        }
    }
    fn irq(&self) -> bool {
        self.irq.pending
    }
}

/// Sunsoft FME-7 / 5B (mapper 69); `$C000` selects and `$E000` writes a
/// [`Sunsoft5B`] audio register.
///
/// `$8000`–`$9FFF` selects command 0–15, `$A000`–`$BFFF` writes its parameter:
/// * 0–7: 1 KiB CHR bank at `$0000 + n * $400`.
/// * 8: `$6000` bank: bits 5–0 bank, bit 6 RAM (1) / ROM (0), bit 7 RAM enable.
///   With RAM selected but disabled, reads are open bus (modelled as 0).
/// * 9, A, B: 8 KiB PRG banks at `$8000`, `$A000`, `$C000` (bits 5–0); `$E000` is
///   fixed to the last bank.
/// * C: mirroring 0 vertical, 1 horizontal, 2 one-screen low, 3 one-screen high.
/// * D: IRQ control, bit 0 IRQ enable, bit 7 counter enable; any write acknowledges.
/// * E, F: counter low and high byte.
///
/// The 16-bit counter decrements every CPU cycle while enabled; wrapping from
/// `$0000` to `$FFFF` asserts IRQ when IRQs are enabled.
pub struct Fme7 {
    prg: Vec<u8>,
    banks8: usize,
    ram: Box<[u8; 0x2000]>,
    pub command: u8,
    pub chr: [u8; 8],
    pub prg_regs: [u8; 4],
    pub mirroring_bits: u8,
    pub irq_enabled: bool,
    pub counter_enabled: bool,
    pub counter: u16,
    pub irq_pending: bool,
    /// The 5B audio inside the package (absent on the plain FME-7, harmless there:
    /// nothing writes `$C000`/`$E000`).
    pub audio: Sunsoft5B,
}

impl Fme7 {
    pub fn new(prg: Vec<u8>) -> Result<Self, MapperError> {
        if prg.len() < 0x4000 || prg.len() % 0x2000 != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks8 = prg.len() / 0x2000;
        if banks8 > 64 {
            return Err(MapperError::TooManyBanks(banks8));
        }
        Ok(Self {
            prg,
            banks8,
            ram: Box::new([0; 0x2000]),
            command: 0,
            chr: [0; 8],
            prg_regs: [0; 4],
            mirroring_bits: 0,
            irq_enabled: false,
            counter_enabled: false,
            counter: 0,
            irq_pending: false,
            audio: Sunsoft5B::default(),
        })
    }

    fn rom(&self, bank: u8, addr: u16) -> u8 {
        let bank = usize::from(bank & 0x3F) % self.banks8;
        self.prg[bank * 0x2000 + usize::from(addr & 0x1FFF)]
    }
}

impl Cartridge for Fme7 {
    fn save_ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.to_vec())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        restore_ram(&mut self.ram[..], data)
    }
    fn state_kind(&self) -> u8 {
        69
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.prg, &self.ram[..]);
        w.u8(self.command);
        w.bytes(&self.chr);
        w.bytes(&self.prg_regs);
        w.u8(self.mirroring_bits);
        w.bool(self.irq_enabled);
        w.bool(self.counter_enabled);
        w.u16(self.counter);
        w.bool(self.irq_pending);
        if w.version >= 6 {
            self.audio.save(w);
        }
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        self.command = r.small(0x0F, "FME-7 command")?;
        r.fill(&mut self.chr, "FME-7 CHR")?;
        let mut prg = [0u8; 4];
        r.fill(&mut prg, "FME-7 PRG")?;
        let max = [0xFF, 0x3F, 0x3F, 0x3F];
        for (i, &v) in prg.iter().enumerate() {
            if v > max[i] {
                return Err(NesStateError::BadValue {
                    what: "FME-7 PRG",
                    value: u64::from(v),
                });
            }
        }
        self.prg_regs = prg;
        self.mirroring_bits = r.small(3, "FME-7 mirroring")?;
        self.irq_enabled = r.bool("FME-7 IRQ enable")?;
        self.counter_enabled = r.bool("FME-7 counter enable")?;
        self.counter = r.u16("FME-7 counter")?;
        self.irq_pending = r.bool("FME-7 IRQ")?;
        if r.version >= 6 {
            self.audio.load(r)?;
        } else {
            let gain = self.audio.gain;
            self.audio = Sunsoft5B {
                gain,
                ..Sunsoft5B::default()
            };
        }
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x6000..=0x7FFF => {
                let r8 = self.prg_regs[0];
                match (r8 & 0x40 != 0, r8 & 0x80 != 0) {
                    (false, _) => self.rom(r8, addr),
                    (true, true) => self.ram[usize::from(addr - 0x6000)],
                    (true, false) => 0,
                }
            }
            0x8000..=0xDFFF => {
                let slot = usize::from((addr - 0x8000) >> 13);
                self.rom(self.prg_regs[slot + 1], addr)
            }
            0xE000.. => self.rom((self.banks8 - 1) as u8, addr),
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x6000..=0x7FFF => {
                if self.prg_regs[0] & 0xC0 == 0xC0 {
                    self.ram[usize::from(addr - 0x6000)] = value;
                }
            }
            0x8000..=0x9FFF => self.command = value & 0x0F,
            0xC000..=0xDFFF => self.audio.select = value & 0x0F,
            0xE000.. => self.audio.write_register(value),
            0xA000..=0xBFFF => match self.command {
                c @ 0..=7 => self.chr[usize::from(c)] = value,
                8 => self.prg_regs[0] = value,
                c @ 9..=11 => self.prg_regs[usize::from(c - 8)] = value & 0x3F,
                12 => self.mirroring_bits = value & 3,
                13 => {
                    self.irq_enabled = value & 1 != 0;
                    self.counter_enabled = value & 0x80 != 0;
                    self.irq_pending = false;
                }
                14 => self.counter = (self.counter & 0xFF00) | u16::from(value),
                _ => self.counter = (self.counter & 0x00FF) | (u16::from(value) << 8),
            },
            _ => {}
        }
    }
    fn audio(&self) -> f32 {
        self.audio.output()
    }
    fn clock(&mut self) {
        self.audio.clock();
        if self.counter_enabled {
            let (next, wrapped) = self.counter.overflowing_sub(1);
            self.counter = next;
            if wrapped && self.irq_enabled {
                self.irq_pending = true;
            }
        }
    }
    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        usize::from(self.chr[a >> 10]) * 0x400 + (a & 0x3FF)
    }
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        use crate::ppu::Mirroring::*;
        Some([Vertical, Horizontal, SingleLow, SingleHigh][usize::from(self.mirroring_bits)])
    }
    fn irq(&self) -> bool {
        self.irq_pending
    }
}

/// Namco 163 (mapper 19) with its [`N163Audio`] wavetable sound over the 128-byte
/// internal RAM (`$E000` bit 6 disables it).
///
/// * `$8000`–`$BFFF` (eight `$800` steps): 1 KiB CHR banks. In each pattern-table
///   half whose "CHR RAM disable" bit is clear (`$E800` bit 6 for `$0000`, bit 7
///   for `$1000`), values `$E0`–`$FF` map nametable RAM page `value & 1` instead.
/// * `$C000`–`$DFFF` (four `$800` steps): nametables `$2000`/`$2400`/`$2800`/
///   `$2C00`: `$E0`–`$FF` nametable RAM page `value & 1`, lower values CHR ROM.
/// * `$E000`: bits 5–0 PRG 8 KiB bank at `$8000` (bit 6 sound disable); `$E800`:
///   bits 5–0 bank at `$A000`, bits 7–6 above; `$F000`: bank at `$C000`; `$E000`–
///   `$FFFF` reads the last bank.
/// * `$F800`: sound address port (bits 6–0, bit 7 auto-increment) and PRG RAM write
///   protect: writes allowed when bits 7–4 are `0100` and the 2 KiB section's bit
///   (3–0) is clear. `$4800`: sound RAM data.
/// * `$5000`/`$5800`: IRQ counter low 8 bits / bits 14–8 with bit 7 IRQ enable; a
///   write acknowledges, a read returns the counter. The 15-bit counter counts up
///   each CPU cycle until `$7FFF`, where it stops and asserts IRQ if enabled.
pub struct Namco163 {
    prg: Vec<u8>,
    banks8: usize,
    ram: Box<[u8; 0x2000]>,
    pub sound_ram: [u8; 128],
    pub chr: [u8; 12],
    pub prg_regs: [u8; 3],
    pub port: u8,
    pub counter: u16,
    pub irq_enabled: bool,
    pub irq_pending: bool,
    pub audio: N163Audio,
}

impl Namco163 {
    pub fn new(prg: Vec<u8>) -> Result<Self, MapperError> {
        if prg.len() < 0x4000 || prg.len() % 0x2000 != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks8 = prg.len() / 0x2000;
        if banks8 > 64 {
            return Err(MapperError::TooManyBanks(banks8));
        }
        Ok(Self {
            prg,
            banks8,
            ram: Box::new([0; 0x2000]),
            sound_ram: [0; 128],
            chr: [0; 12],
            prg_regs: [0; 3],
            port: 0,
            counter: 0,
            irq_enabled: false,
            irq_pending: false,
            audio: N163Audio::default(),
        })
    }

    fn ram_writable(&self, addr: u16) -> bool {
        let section = (addr - 0x6000) >> 11;
        self.port & 0xF0 == 0x40 && self.port & (1 << section) == 0
    }

    fn sound_data(&mut self) -> &mut u8 {
        let i = usize::from(self.port & 0x7F);
        if self.port & 0x80 != 0 {
            self.port = 0x80 | (self.port.wrapping_add(1) & 0x7F);
        }
        &mut self.sound_ram[i]
    }
}

impl Cartridge for Namco163 {
    /// 8 KiB PRG RAM followed by the 128-byte internal RAM (also battery-backed).
    fn save_ram(&self) -> Option<Vec<u8>> {
        let mut v = self.ram.to_vec();
        v.extend_from_slice(&self.sound_ram);
        Some(v)
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        if data.len() != 0x2000 + 128 {
            return Err(SaveRamError::WrongSize {
                expected: 0x2000 + 128,
                found: data.len(),
            });
        }
        self.ram.copy_from_slice(&data[..0x2000]);
        self.sound_ram.copy_from_slice(&data[0x2000..]);
        Ok(())
    }
    fn state_kind(&self) -> u8 {
        19
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.prg, &self.ram[..]);
        w.bytes(&self.sound_ram);
        w.bytes(&self.chr);
        w.bytes(&self.prg_regs);
        w.u8(self.port);
        w.u16(self.counter);
        w.bool(self.irq_enabled);
        w.bool(self.irq_pending);
        if w.version >= 6 {
            self.audio.save(w);
        }
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        r.fill(&mut self.sound_ram, "N163 sound RAM")?;
        r.fill(&mut self.chr, "N163 CHR")?;
        r.fill(&mut self.prg_regs, "N163 PRG")?;
        self.port = r.u8("N163 port")?;
        self.counter = r.u16("N163 counter")?;
        if self.counter > 0x7FFF {
            return Err(NesStateError::BadValue {
                what: "N163 counter",
                value: u64::from(self.counter),
            });
        }
        self.irq_enabled = r.bool("N163 IRQ enable")?;
        self.irq_pending = r.bool("N163 IRQ")?;
        if r.version >= 6 {
            self.audio.load(r)?;
        } else {
            self.audio.divider = 0;
            self.audio.channel = 7;
            self.audio.current = 0;
        }
        Ok(())
    }
    fn read(&mut self, addr: u16) -> u8 {
        if (0x4800..=0x4FFF).contains(&addr) {
            return *self.sound_data();
        }
        self.peek(addr)
    }
    fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x4800..=0x4FFF => self.sound_ram[usize::from(self.port & 0x7F)],
            0x5000..=0x57FF => self.counter as u8,
            0x5800..=0x5FFF => ((self.counter >> 8) as u8) | (u8::from(self.irq_enabled) << 7),
            0x6000..=0x7FFF => self.ram[usize::from(addr - 0x6000)],
            0x8000.. => {
                let slot = usize::from((addr - 0x8000) >> 13);
                let bank = if slot == 3 {
                    self.banks8 - 1
                } else {
                    usize::from(self.prg_regs[slot] & 0x3F) % self.banks8
                };
                self.prg[bank * 0x2000 + usize::from(addr & 0x1FFF)]
            }
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x4800..=0x4FFF => *self.sound_data() = value,
            0x5000..=0x57FF => {
                self.counter = (self.counter & 0x7F00) | u16::from(value);
                self.irq_pending = false;
            }
            0x5800..=0x5FFF => {
                self.counter = (self.counter & 0x00FF) | (u16::from(value & 0x7F) << 8);
                self.irq_enabled = value & 0x80 != 0;
                self.irq_pending = false;
            }
            0x6000..=0x7FFF => {
                if self.ram_writable(addr) {
                    self.ram[usize::from(addr - 0x6000)] = value;
                }
            }
            0x8000..=0xDFFF => self.chr[usize::from((addr - 0x8000) >> 11)] = value,
            0xE000..=0xE7FF => self.prg_regs[0] = value,
            0xE800..=0xEFFF => self.prg_regs[1] = value,
            0xF000..=0xF7FF => self.prg_regs[2] = value,
            0xF800.. => self.port = value,
            _ => {}
        }
    }
    fn audio(&self) -> f32 {
        self.audio.output()
    }
    fn clock(&mut self) {
        let disabled = self.prg_regs[0] & 0x40 != 0;
        self.audio.clock(&mut self.sound_ram, disabled);
        if self.counter < 0x7FFF {
            self.counter += 1;
            if self.counter == 0x7FFF && self.irq_enabled {
                self.irq_pending = true;
            }
        }
    }
    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        usize::from(self.chr[a >> 10]) * 0x400 + (a & 0x3FF)
    }
    fn ppu_target(&self, addr: u16) -> PpuTarget {
        let a = usize::from(addr & 0x2FFF);
        let (value, ciram_allowed) = if a < 0x2000 {
            let disabled = self.prg_regs[1] & (0x40 << (a >> 12)) != 0;
            (self.chr[a >> 10], !disabled)
        } else {
            (self.chr[8 + ((a >> 10) & 3)], true)
        };
        if value >= 0xE0 && ciram_allowed {
            PpuTarget::Ciram(usize::from(value & 1) * 0x400 + (a & 0x3FF))
        } else {
            PpuTarget::Chr(usize::from(value) * 0x400 + (a & 0x3FF))
        }
    }
    fn irq(&self) -> bool {
        self.irq_pending
    }
}

/// Linear expansion-audio unit: one VRC6 volume step is 1/15 of a full-volume APU
/// pulse in the APU mixer, the relative level NESdev gives for VRC6 boards.
pub const APU_PULSE_FULL: f32 = 95.88 / (8128.0 / 15.0 + 100.0);

/// Sunsoft 5B audio (YM2149F core inside the FME-7 package): three square tone
/// channels, one noise generator, one envelope generator, logarithmic DAC.
///
/// * Registers 0–5: 12-bit tone periods (low byte, high nibble) for A, B, C. A tone
///   flips every `16 * P` CPU cycles, so it sounds at `CPU / (32 * P)` (P = 0 acts
///   as 1).
/// * 6: 5-bit noise period; the 17-bit LFSR (taps 0 and 3) shifts every `32 * N`
///   CPU cycles.
/// * 7: bits 0–2 disable tone A–C, bits 3–5 disable noise on A–C (1 = off). A
///   channel sounds while (tone or tone disabled) and (noise or noise disabled).
/// * 8–A: bit 4 envelope mode, bits 3–0 volume (DAC level `2v + 1`, 0 silent).
/// * B–C: 16-bit envelope period, one of its 32 steps every `16 * E` CPU cycles.
/// * D: shape (CONT, ATT, ALT, HOLD); writing restarts the envelope.
///
/// The DAC is 1.5 dB per level (32 levels, 0 silent). `gain` scales a channel at
/// level 31 relative to [`APU_PULSE_FULL`]; NESdev documents the 5B as
/// considerably louder than the APU without a single board-independent figure, so
/// the default 1.0 makes one full channel as loud as one full APU pulse.
#[derive(Debug, Clone, PartialEq)]
pub struct Sunsoft5B {
    pub regs: [u8; 16],
    pub select: u8,
    prescaler: u8,
    tone_count: [u16; 3],
    pub tone_out: [bool; 3],
    noise_count: u16,
    pub lfsr: u32,
    env_count: u32,
    pub env_step: u8,
    pub env_attack: bool,
    pub env_holding: bool,
    pub gain: f32,
}

impl Default for Sunsoft5B {
    fn default() -> Self {
        Self {
            regs: [0; 16],
            select: 0,
            prescaler: 0,
            tone_count: [0; 3],
            tone_out: [false; 3],
            noise_count: 0,
            lfsr: 1,
            env_count: 0,
            env_step: 0,
            env_attack: false,
            env_holding: false,
            gain: 1.0,
        }
    }
}

impl Sunsoft5B {
    pub fn write_register(&mut self, value: u8) {
        let r = usize::from(self.select & 0x0F);
        self.regs[r] = value;
        if r == 13 {
            self.env_step = 0;
            self.env_count = 0;
            self.env_holding = false;
            self.env_attack = value & 4 != 0;
        }
    }

    pub fn tone_period(&self, ch: usize) -> u16 {
        (u16::from(self.regs[ch * 2]) | (u16::from(self.regs[ch * 2 + 1] & 0x0F) << 8)).max(1)
    }

    /// Envelope level 0–31.
    pub fn envelope_level(&self) -> u8 {
        if self.env_attack {
            self.env_step
        } else {
            31 - self.env_step
        }
    }

    /// One CPU cycle.
    pub fn clock(&mut self) {
        self.prescaler = (self.prescaler + 1) & 15;
        if self.prescaler != 0 {
            return;
        }
        for ch in 0..3 {
            self.tone_count[ch] += 1;
            if self.tone_count[ch] >= self.tone_period(ch) {
                self.tone_count[ch] = 0;
                self.tone_out[ch] = !self.tone_out[ch];
            }
        }
        self.noise_count += 1;
        if self.noise_count >= 2 * u16::from((self.regs[6] & 0x1F).max(1)) {
            self.noise_count = 0;
            let bit = (self.lfsr ^ (self.lfsr >> 3)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 16);
        }
        let period = (u32::from(self.regs[11]) | (u32::from(self.regs[12]) << 8)).max(1);
        self.env_count += 1;
        if self.env_count >= period {
            self.env_count = 0;
            self.step_envelope();
        }
    }

    fn step_envelope(&mut self) {
        if self.env_holding {
            return;
        }
        if self.env_step < 31 {
            self.env_step += 1;
            return;
        }
        let shape = self.regs[13];
        let (cont, alt, hold) = (shape & 8 != 0, shape & 2 != 0, shape & 1 != 0);
        if !cont {
            self.env_holding = true;
            self.env_attack = false;
            self.env_step = 31; // level 0
        } else if hold {
            self.env_holding = true;
            if alt {
                self.env_attack = !self.env_attack;
            }
            // Hold the segment's last level, or with ALT its opposite (the attack
            // flip turns step 31 into that level).
            self.env_step = 31;
        } else {
            if alt {
                self.env_attack = !self.env_attack;
            }
            self.env_step = 0;
        }
    }

    /// DAC level 0–31 of channel `ch` now.
    pub fn channel_level(&self, ch: usize) -> u8 {
        let mixer = self.regs[7];
        let tone = self.tone_out[ch] || mixer & (1 << ch) != 0;
        let noise = self.lfsr & 1 != 0 || mixer & (8 << ch) != 0;
        if !(tone && noise) {
            return 0;
        }
        let v = self.regs[8 + ch];
        if v & 0x10 != 0 {
            self.envelope_level()
        } else if v & 0x0F == 0 {
            0
        } else {
            (v & 0x0F) * 2 + 1
        }
    }

    /// Amplitude of a DAC level: 1.5 dB steps below level 31, level 0 silent.
    pub fn dac(level: u8) -> f32 {
        if level == 0 {
            0.0
        } else {
            10f32.powf((f32::from(level) - 31.0) * 1.5 / 20.0)
        }
    }

    pub fn output(&self) -> f32 {
        let sum: f32 = (0..3).map(|c| Self::dac(self.channel_level(c))).sum();
        sum * APU_PULSE_FULL * self.gain
    }

    fn save(&self, w: &mut StateWriter) {
        w.bytes(&self.regs);
        w.u8(self.select);
        w.u8(self.prescaler);
        for c in self.tone_count {
            w.u16(c);
        }
        for f in self.tone_out {
            w.bool(f);
        }
        w.u16(self.noise_count);
        w.u32(self.lfsr);
        w.u32(self.env_count);
        w.u8(self.env_step);
        w.bool(self.env_attack);
        w.bool(self.env_holding);
    }

    fn load(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fill(&mut self.regs, "5B registers")?;
        self.select = r.u8("5B select")?;
        self.prescaler = r.small(15, "5B prescaler")?;
        for c in &mut self.tone_count {
            *c = r.u16("5B tone counter")?;
        }
        for f in &mut self.tone_out {
            *f = r.bool("5B tone")?;
        }
        self.noise_count = r.u16("5B noise counter")?;
        self.lfsr = r.u32("5B LFSR")?;
        if self.lfsr == 0 || self.lfsr >= 1 << 17 {
            return Err(NesStateError::BadValue {
                what: "5B LFSR",
                value: u64::from(self.lfsr),
            });
        }
        self.env_count = r.u32("5B envelope counter")?;
        self.env_step = r.small(31, "5B envelope step")?;
        self.env_attack = r.bool("5B attack")?;
        self.env_holding = r.bool("5B hold")?;
        Ok(())
    }
}

/// Namco 163 wavetable audio over the 128-byte internal RAM.
///
/// Channel `c` (0–7) uses `$40 + 8c`: frequency bits 7–0, 15–8 at +0/+2 and 17–16
/// in +4 bits 1–0; phase bits 7–0/15–8/23–16 at +1/+3/+5; wave length
/// `256 - (+4 & $FC)` samples; wave address +6; volume +7 bits 3–0. `$7F` bits 6–4
/// hold the number of enabled channels minus one; enabled channels are 7 down to
/// `8 - N`. One channel is updated every 15 CPU cycles, in turn: phase += frequency
/// modulo `length << 16`, then the 4-bit sample at nibble `(phase >> 16) + address`
/// (low nibble first) is output as `(sample - 8) * volume`. The chip outputs one
/// channel at a time (time-multiplexed), which is what [`N163Audio::output`] gives.
///
/// Level: NES 2.0 submappers 3/4/5 say the board is 11–13 / 16–17 / 18–19.5 dB
/// louder than the APU; the model uses the midpoints, referenced to a full-volume
/// APU pulse against a full-scale channel (`8 * 15`). Submapper 2 has no sound.
#[derive(Debug, Clone, PartialEq)]
pub struct N163Audio {
    pub divider: u8,
    pub channel: u8,
    pub current: i16,
    pub db: Option<f32>,
}

impl Default for N163Audio {
    fn default() -> Self {
        Self {
            divider: 0,
            channel: 7,
            current: 0,
            db: Some(12.0),
        }
    }
}

impl N163Audio {
    /// Gain in dB for an NES 2.0 mapper 19 submapper (0 and 1: the 11–13 dB class).
    pub fn db_for_submapper(sub: u8) -> Option<f32> {
        match sub {
            2 => None,
            4 => Some(16.5),
            5 => Some(18.75),
            _ => Some(12.0),
        }
    }

    pub fn enabled_channels(ram: &[u8; 128]) -> u8 {
        ((ram[0x7F] >> 4) & 7) + 1
    }

    /// One CPU cycle.
    pub fn clock(&mut self, ram: &mut [u8; 128], disabled: bool) {
        if disabled {
            self.current = 0;
            return;
        }
        self.divider += 1;
        if self.divider < 15 {
            return;
        }
        self.divider = 0;
        let n = Self::enabled_channels(ram);
        let c = self.channel;
        self.current = Self::update(ram, usize::from(c));
        self.channel = if c <= 8 - n { 7 } else { c - 1 };
    }

    /// Advance channel `c` once and return its output.
    pub fn update(ram: &mut [u8; 128], c: usize) -> i16 {
        let b = 0x40 + 8 * c;
        let freq =
            u32::from(ram[b]) | (u32::from(ram[b + 2]) << 8) | (u32::from(ram[b + 4] & 3) << 16);
        let phase =
            u32::from(ram[b + 1]) | (u32::from(ram[b + 3]) << 8) | (u32::from(ram[b + 5]) << 16);
        let length = 256 - u32::from(ram[b + 4] & 0xFC);
        let phase = (phase + freq) % (length << 16);
        ram[b + 1] = phase as u8;
        ram[b + 3] = (phase >> 8) as u8;
        ram[b + 5] = (phase >> 16) as u8;
        let addr = ((phase >> 16) + u32::from(ram[b + 6])) & 0xFF;
        let byte = ram[(addr >> 1) as usize];
        let sample = if addr & 1 == 0 {
            byte & 0x0F
        } else {
            byte >> 4
        };
        (i16::from(sample) - 8) * i16::from(ram[b + 7] & 0x0F)
    }

    pub fn output(&self) -> f32 {
        match self.db {
            None => 0.0,
            Some(db) => f32::from(self.current) * APU_PULSE_FULL * 10f32.powf(db / 20.0) / 120.0,
        }
    }

    fn save(&self, w: &mut StateWriter) {
        w.u8(self.divider);
        w.u8(self.channel);
        w.u16(self.current as u16);
    }

    fn load(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        self.divider = r.small(14, "N163 divider")?;
        self.channel = r.small(7, "N163 channel")?;
        let v = r.u16("N163 output")? as i16;
        if !(-120..=105).contains(&v) {
            return Err(NesStateError::BadValue {
                what: "N163 output",
                value: u64::from(v as u16),
            });
        }
        self.current = v;
        Ok(())
    }
}

/// One VRC6 pulse channel: `$x000` bit 7 mode (constant), bits 6–4 duty, 3–0
/// volume; `$x001`/`$x002` 12-bit period, `$x002` bit 7 enable. The divider counts
/// down every CPU cycle and, at zero, reloads and steps the 16-step duty counter;
/// the channel outputs its volume while the step is at or below the duty (duty
/// `d` is `(d + 1) / 16`), always in mode 1, and 0 when disabled. Frequency is
/// `CPU / (16 * (P + 1))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vrc6Pulse {
    pub regs: [u8; 3],
    pub divider: u16,
    pub step: u8,
}

impl Vrc6Pulse {
    pub fn period(&self) -> u16 {
        u16::from(self.regs[1]) | (u16::from(self.regs[2] & 0x0F) << 8)
    }
    pub fn enabled(&self) -> bool {
        self.regs[2] & 0x80 != 0
    }
    fn write(&mut self, reg: usize, v: u8) {
        self.regs[reg] = v;
        if reg == 2 && v & 0x80 == 0 {
            self.step = 15;
        }
    }
    fn clock(&mut self, shift: u8) {
        if !self.enabled() {
            return;
        }
        if self.divider == 0 {
            self.divider = self.period() >> shift;
            self.step = self.step.wrapping_sub(1) & 15;
        } else {
            self.divider -= 1;
        }
    }
    pub fn output(&self) -> u8 {
        let r = self.regs[0];
        let on = r & 0x80 != 0 || self.step <= (r >> 4) & 7;
        if self.enabled() && on {
            r & 0x0F
        } else {
            0
        }
    }
}

/// VRC6 sawtooth: `$B000` bits 5–0 accumulator rate, `$B001`/`$B002` 12-bit period,
/// `$B002` bit 7 enable. Each divider expiry is one of 14 steps: on every second
/// step the rate is added to the 8-bit accumulator, and after the 14th it resets
/// to 0; the output is the accumulator's top 5 bits. Frequency `CPU / (14 * (P+1))`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vrc6Saw {
    pub regs: [u8; 3],
    pub divider: u16,
    pub step: u8,
    pub acc: u8,
}

impl Vrc6Saw {
    pub fn period(&self) -> u16 {
        u16::from(self.regs[1]) | (u16::from(self.regs[2] & 0x0F) << 8)
    }
    fn clock(&mut self, shift: u8) {
        if self.regs[2] & 0x80 == 0 {
            self.acc = 0;
            self.step = 0;
            return;
        }
        if self.divider > 0 {
            self.divider -= 1;
            return;
        }
        self.divider = self.period() >> shift;
        self.step += 1;
        if self.step == 14 {
            self.step = 0;
            self.acc = 0;
        } else if self.step % 2 == 0 {
            self.acc = self.acc.wrapping_add(self.regs[0] & 0x3F);
        }
    }
    pub fn output(&self) -> u8 {
        self.acc >> 3
    }
}

/// Konami VRC6 (mapper 24 VRC6a; mapper 26 VRC6b swaps A0 and A1).
///
/// * `$8000`: 16 KiB PRG at `$8000` (bits 3–0); `$C000`: 8 KiB at `$C000` (bits
///   4–0); `$E000` fixed to the last 8 KiB. `$6000`: 8 KiB PRG RAM when `$B003` bit 7.
/// * `$B003`: bits 1–0 PPU banking mode, bits 3–2 mirroring, bit 4 nametables from
///   CHR ROM, bit 5 A10 rule (see [`Vrc6::nametable_page`]), bit 7 PRG RAM enable.
/// * `$D000`–`$D003`, `$E000`–`$E003`: CHR registers R0–R7. Mode 0: eight 1 KiB
///   banks. Mode 1: R0–R3 as 2 KiB banks. Modes 2 and 3: R0–R3 1 KiB at `$0000`, R4
///   and R5 2 KiB at `$1000`/`$1800` ([`Vrc6::chr_page`]).
/// * `$F000` IRQ latch, `$F001` control, `$F002` ack ([`VrcIrq`]).
/// * Audio: `$9000`–`$9002` pulse 1, `$A000`–`$A002` pulse 2, `$B000`–`$B002`
///   sawtooth; `$9003` bit 0 halts all three, bit 1 shifts periods right by 4, bit
///   2 by 8 (bit 2 wins). Mixed linearly, one step = `APU_PULSE_FULL / 15`.
pub struct Vrc6 {
    prg: Vec<u8>,
    banks8: usize,
    pub swapped: bool,
    ram: Box<[u8; 0x2000]>,
    pub prg16: u8,
    pub prg8: u8,
    pub chr: [u8; 8],
    pub control: u8,
    pub irq: VrcIrq,
    pub pulse: [Vrc6Pulse; 2],
    pub saw: Vrc6Saw,
    pub freq_ctrl: u8,
}

impl Vrc6 {
    pub fn new(prg: Vec<u8>, swapped: bool) -> Result<Self, MapperError> {
        check_banks(&prg, 16)?;
        let banks8 = prg.len() / 0x2000;
        Ok(Self {
            prg,
            banks8,
            swapped,
            ram: Box::new([0; 0x2000]),
            prg16: 0,
            prg8: 0,
            chr: [0; 8],
            control: 0,
            irq: VrcIrq::default(),
            pulse: [Vrc6Pulse::default(); 2],
            saw: Vrc6Saw::default(),
            freq_ctrl: 0,
        })
    }

    /// `$x000 + reg` after the board's A0/A1 wiring.
    pub fn decode(&self, addr: u16) -> u16 {
        let (a0, a1) = (addr & 1, (addr >> 1) & 1);
        let reg = if self.swapped {
            (a0 << 1) | a1
        } else {
            (a1 << 1) | a0
        };
        (addr & 0xF000) | reg
    }

    fn shift(&self) -> u8 {
        if self.freq_ctrl & 4 != 0 {
            8
        } else if self.freq_ctrl & 2 != 0 {
            4
        } else {
            0
        }
    }

    /// 1 KiB CHR page for pattern address `addr` (`$B003` bits 1–0 and 5). With bit
    /// 5 set, 2 KiB banks take A10 from the PPU; clear, from the register's LSB.
    pub fn chr_page(&self, addr: u16) -> usize {
        let slot = usize::from(addr & 0x1FFF) >> 10;
        let a10 = slot & 1;
        let r = |i: usize| usize::from(self.chr[i]);
        let two_k = |reg: usize| {
            if self.control & 0x20 != 0 {
                (r(reg) & !1) | a10
            } else {
                r(reg)
            }
        };
        match self.control & 3 {
            0 => r(slot),
            1 => two_k(slot >> 1),
            _ if slot < 4 => r(slot),
            _ => two_k(4 + ((slot - 4) >> 1)),
        }
    }

    /// 1 KiB page for nametable `addr`: the register comes from `$B003 & 7`
    /// (0/6/7: R6 R6 R7 R7; 1/5: R4 R5 R6 R7; 2/3/4: R6 R7 R6 R7). With `$B003` bit
    /// 5 set its LSB is replaced per mode: mode 0 by PPU A10, A11, 0 or 1 (vertical,
    /// horizontal, one-screen A/B for bits 3–2 = 0..3); mode 3 by A11, A10, 1 or 0;
    /// modes 1 and 2 keep the register LSB. The page goes to CHR ROM when bit 4 is
    /// set, otherwise its LSB is CIRAM A10.
    pub fn nametable_page(&self, addr: u16) -> usize {
        let q = usize::from((addr >> 10) & 3);
        let regs: [usize; 4] = match self.control & 7 {
            0 | 6 | 7 => [6, 6, 7, 7],
            1 | 5 => [4, 5, 6, 7],
            _ => [6, 7, 6, 7],
        };
        let reg = usize::from(self.chr[regs[q]]);
        if self.control & 0x20 == 0 {
            return reg;
        }
        let (a10, a11) = (q & 1, q >> 1);
        let mirror = (self.control >> 2) & 3;
        let lsb = match (self.control & 3, mirror) {
            (0, 0) => a10,
            (0, 1) => a11,
            (0, 2) => 0,
            (0, _) => 1,
            (3, 0) => a11,
            (3, 1) => a10,
            (3, 2) => 1,
            (3, _) => 0,
            _ => reg & 1,
        };
        (reg & !1) | lsb
    }

    /// Mixed level 0–61: two pulses (0–15) and the sawtooth (0–31).
    pub fn level(&self) -> u8 {
        self.pulse[0].output() + self.pulse[1].output() + self.saw.output()
    }
}

impl Cartridge for Vrc6 {
    fn save_ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.to_vec())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        restore_ram(&mut self.ram[..], data)
    }
    fn state_kind(&self) -> u8 {
        if self.swapped {
            26
        } else {
            24
        }
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.prg, &self.ram[..]);
        w.u8(self.prg16);
        w.u8(self.prg8);
        w.bytes(&self.chr);
        w.u8(self.control);
        self.irq.save(w);
        for p in &self.pulse {
            w.bytes(&p.regs);
            w.u16(p.divider);
            w.u8(p.step);
        }
        w.bytes(&self.saw.regs);
        w.u16(self.saw.divider);
        w.u8(self.saw.step);
        w.u8(self.saw.acc);
        w.u8(self.freq_ctrl);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        self.prg16 = r.small(0x0F, "VRC6 PRG16")?;
        self.prg8 = r.small(0x1F, "VRC6 PRG8")?;
        r.fill(&mut self.chr, "VRC6 CHR")?;
        self.control = r.u8("VRC6 control")?;
        self.irq.load(r)?;
        for p in &mut self.pulse {
            r.fill(&mut p.regs, "VRC6 pulse")?;
            p.divider = r.u16("VRC6 pulse divider")?;
            p.step = r.small(15, "VRC6 duty step")?;
        }
        r.fill(&mut self.saw.regs, "VRC6 saw")?;
        self.saw.divider = r.u16("VRC6 saw divider")?;
        self.saw.step = r.small(13, "VRC6 saw step")?;
        self.saw.acc = r.u8("VRC6 accumulator")?;
        self.freq_ctrl = r.small(7, "VRC6 frequency control")?;
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        let last = self.banks8 - 1;
        match addr {
            0x6000..=0x7FFF if self.control & 0x80 != 0 => self.ram[usize::from(addr - 0x6000)],
            0x8000..=0xBFFF => {
                let bank = usize::from(self.prg16 & 0x0F) * 2 % self.banks8;
                self.prg[bank * 0x2000 + usize::from(addr & 0x3FFF)]
            }
            0xC000..=0xDFFF => {
                let bank = usize::from(self.prg8 & 0x1F) % self.banks8;
                self.prg[bank * 0x2000 + usize::from(addr & 0x1FFF)]
            }
            0xE000.. => self.prg[last * 0x2000 + usize::from(addr & 0x1FFF)],
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        if (0x6000..0x8000).contains(&addr) {
            if self.control & 0x80 != 0 {
                self.ram[usize::from(addr - 0x6000)] = value;
            }
            return;
        }
        if addr < 0x8000 {
            return;
        }
        let a = self.decode(addr);
        match a {
            0x8000..=0x8003 => self.prg16 = value & 0x0F,
            0x9000..=0x9002 => self.pulse[0].write(usize::from(a & 3), value),
            0x9003 => self.freq_ctrl = value & 7,
            0xA000..=0xA002 => self.pulse[1].write(usize::from(a & 3), value),
            0xB000..=0xB002 => self.saw.regs[usize::from(a & 3)] = value,
            0xB003 => self.control = value,
            0xC000..=0xC003 => self.prg8 = value & 0x1F,
            0xD000..=0xD003 => self.chr[usize::from(a & 3)] = value,
            0xE000..=0xE003 => self.chr[4 + usize::from(a & 3)] = value,
            0xF000 => self.irq.latch = value,
            0xF001 => self.irq.control(value),
            0xF002 => self.irq.ack(),
            _ => {}
        }
    }
    fn chr_offset(&self, addr: u16) -> usize {
        self.chr_page(addr) * 0x400 + usize::from(addr & 0x3FF)
    }
    fn ppu_target(&self, addr: u16) -> PpuTarget {
        if addr < 0x2000 {
            return PpuTarget::Default;
        }
        let page = self.nametable_page(addr);
        let off = usize::from(addr & 0x3FF);
        if self.control & 0x10 != 0 {
            PpuTarget::Chr(page * 0x400 + off)
        } else {
            PpuTarget::Ciram((page & 1) * 0x400 + off)
        }
    }
    fn clock(&mut self) {
        self.irq.clock();
        if self.freq_ctrl & 1 == 0 {
            let s = self.shift();
            self.pulse[0].clock(s);
            self.pulse[1].clock(s);
            self.saw.clock(s);
        }
    }
    fn irq(&self) -> bool {
        self.irq.pending
    }
    fn audio(&self) -> f32 {
        f32::from(self.level()) * APU_PULSE_FULL / 15.0
    }
}

/// Nintendo MMC5 (mapper 5) with its pulse/PCM audio ([`Mmc5Pulse`], `$5010`/`$5011`,
/// `$5015`) and vertical split ([`Mmc5::split_fetch`], `$5200`–`$5202`).
///
/// * `$5100` PRG mode: 0 = 32 KiB (`$5117`), 1 = 16+16 (`$5115`, `$5117`), 2 =
///   16+8+8 (`$5115`, `$5116`, `$5117`), 3 = four 8 KiB (`$5114`–`$5117`). Bit 7 of
///   `$5114`–`$5116` selects ROM (1) or RAM (0); `$5117` is always ROM; `$5113`
///   banks RAM at `$6000`. 64 KiB of PRG RAM, written only while `$5102 = 2` and
///   `$5103 = 1`.
/// * `$5101` CHR mode: 8/4/2/1 KiB pages from `$5120`–`$5127` (set A) and
///   `$5128`–`$512B` (set B), upper bits from `$5130`. With 8×16 sprites, sprite
///   fetches use A and background fetches B; with 8×8 sprites the last written set
///   is used for everything.
/// * `$5104` ExRAM mode: 0 nametable, 1 extended attributes, 2 CPU RAM, 3 CPU
///   read-only. In modes 0/1, CPU writes outside rendering store `$00`.
/// * `$5105` nametable map (two bits per quadrant: CIRAM page 0/1, ExRAM, fill),
///   `$5106` fill tile, `$5107` fill attribute.
/// * Scanline detection: three consecutive PPU reads of the same nametable address
///   start a scanline. The first starts the frame (in-frame flag, counter 0); each
///   later one increments the counter and, if it then equals `$5203` (non-zero),
///   sets the IRQ pending flag. Three CPU cycles without PPU reads leave the frame.
///   `$5204` read: bit 7 pending (read clears it), bit 6 in-frame; write bit 7
///   enables the IRQ output.
/// * `$5205`/`$5206`: unsigned 8×8 multiplier; read back low/high product.
pub struct Mmc5 {
    prg: Vec<u8>,
    banks8: usize,
    pub ram: Vec<u8>,
    pub exram: [u8; 1024],
    pub prg_mode: u8,
    pub chr_mode: u8,
    pub protect: [u8; 2],
    pub exram_mode: u8,
    pub nt_map: u8,
    pub fill_tile: u8,
    pub fill_attr: u8,
    pub prg_regs: [u8; 5],
    pub chr_a: [u16; 8],
    pub chr_b: [u16; 4],
    pub chr_upper: u8,
    pub last_set_b: bool,
    pub irq_compare: u8,
    pub irq_enabled: bool,
    pub irq_pending: bool,
    pub in_frame: bool,
    pub scanline: u8,
    pub multiplicand: u8,
    pub multiplier: u8,
    pub sprites_8x16: bool,
    pub rendering: bool,
    last_nt: u16,
    matches: u8,
    idle: u8,
    read_seen: bool,
    pub reads_in_line: u16,
    ext_tile: u8,
    pub pulse: [Mmc5Pulse; 2],
    /// `$5010` bit 0: PCM read mode (reads of `$8000`–`$BFFF` feed the DAC).
    pub pcm_read_mode: bool,
    pub pcm_irq_enabled: bool,
    pub pcm_irq: bool,
    pub pcm: u8,
    /// CPU cycles toward the next 240 Hz envelope/length clock.
    pub frame_divider: u16,
    cpu_parity: bool,
    pub split_ctrl: u8,
    pub split_scroll: u8,
    pub split_bank: u8,
}

/// Length-counter table of the MMC5 pulses: the APU's, except index `$1C` loads
/// `$10` (the MMC5 table lacks the APU's bit 0, which only `$1C` would differ in).
pub const MMC5_LENGTHS: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

/// CPU cycles per MMC5 envelope/length clock (fixed ≈240 Hz, no frame sequencer).
pub const MMC5_FRAME_CYCLES: u16 = 7457;

const PULSE_DUTY: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

/// An MMC5 pulse: the APU pulse without a sweep unit (`$5001`/`$5005` do
/// nothing), periods below 8 not muted, envelope and length both clocked at 240 Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mmc5Pulse {
    pub enabled: bool,
    pub duty: u8,
    pub halt: bool,
    pub constant: bool,
    pub volume: u8,
    pub period: u16,
    pub timer: u16,
    pub step: u8,
    pub length: u8,
    pub env_start: bool,
    pub env_divider: u8,
    pub env_decay: u8,
}

impl Mmc5Pulse {
    fn write(&mut self, reg: u16, v: u8) {
        match reg & 3 {
            0 => {
                self.duty = v >> 6;
                self.halt = v & 0x20 != 0;
                self.constant = v & 0x10 != 0;
                self.volume = v & 0x0F;
            }
            2 => self.period = (self.period & 0x700) | u16::from(v),
            3 => {
                self.period = (self.period & 0xFF) | (u16::from(v & 7) << 8);
                if self.enabled {
                    self.length = MMC5_LENGTHS[usize::from(v >> 3)];
                }
                self.step = 0;
                self.env_start = true;
            }
            _ => {}
        }
    }
    /// One APU cycle (every second CPU cycle).
    fn clock_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.period;
            self.step = self.step.wrapping_sub(1) & 7;
        } else {
            self.timer -= 1;
        }
    }
    /// The 240 Hz clock: envelope and length counter.
    fn clock_frame(&mut self) {
        if self.env_start {
            self.env_start = false;
            self.env_decay = 15;
            self.env_divider = self.volume;
        } else if self.env_divider == 0 {
            self.env_divider = self.volume;
            if self.env_decay > 0 {
                self.env_decay -= 1;
            } else if self.halt {
                self.env_decay = 15;
            }
        } else {
            self.env_divider -= 1;
        }
        if !self.halt && self.length > 0 {
            self.length -= 1;
        }
    }
    pub fn output(&self) -> u8 {
        if !self.enabled
            || self.length == 0
            || PULSE_DUTY[usize::from(self.duty)][usize::from(self.step)] == 0
        {
            0
        } else if self.constant {
            self.volume
        } else {
            self.env_decay
        }
    }
}

impl Mmc5 {
    pub fn new(prg: Vec<u8>) -> Result<Self, MapperError> {
        if prg.len() < 0x2000 || prg.len() % 0x2000 != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks8 = prg.len() / 0x2000;
        Ok(Self {
            prg,
            banks8,
            ram: vec![0; 0x10000],
            exram: [0; 1024],
            prg_mode: 3,
            chr_mode: 3,
            protect: [0; 2],
            exram_mode: 0,
            nt_map: 0,
            fill_tile: 0,
            fill_attr: 0,
            prg_regs: [0, 0, 0, 0, 0xFF],
            chr_a: [0; 8],
            chr_b: [0; 4],
            chr_upper: 0,
            last_set_b: false,
            irq_compare: 0,
            irq_enabled: false,
            irq_pending: false,
            in_frame: false,
            scanline: 0,
            multiplicand: 0xFF,
            multiplier: 0xFF,
            sprites_8x16: false,
            rendering: false,
            last_nt: 0xFFFF,
            matches: 0,
            idle: 0,
            read_seen: false,
            reads_in_line: 0,
            ext_tile: 0,
            pulse: [Mmc5Pulse::default(); 2],
            pcm_read_mode: false,
            pcm_irq_enabled: false,
            pcm_irq: false,
            pcm: 0,
            frame_divider: 0,
            cpu_parity: false,
            split_ctrl: 0,
            split_scroll: 0,
            split_bank: 0,
        })
    }

    /// `(register, 8 KiB slot within it)` mapping CPU `$8000`–`$FFFF` in the PRG mode.
    fn prg_slot(&self, addr: u16) -> (usize, usize) {
        let q = usize::from((addr - 0x8000) >> 13);
        match self.prg_mode & 3 {
            0 => (4, q),
            1 => (if q < 2 { 2 } else { 4 }, q & 1),
            2 if q < 2 => (2, q),
            2 => (q + 1, 0),
            _ => (q + 1, 0),
        }
    }

    /// `(is_ram, byte offset)` for `$6000`–`$FFFF`.
    pub fn prg_target(&self, addr: u16) -> (bool, usize) {
        if addr < 0x8000 {
            let bank = usize::from(self.prg_regs[0] & 7);
            return (true, bank * 0x2000 + usize::from(addr & 0x1FFF));
        }
        let (reg, sub) = self.prg_slot(addr);
        let v = self.prg_regs[reg];
        let size = match self.prg_mode & 3 {
            0 => 4,
            1 => 2,
            2 if reg == 2 => 2,
            _ => 1,
        };
        let bank = (usize::from(v & 0x7F) & !(size - 1)) + sub;
        let ram = reg != 4 && v & 0x80 == 0;
        let off = usize::from(addr & 0x1FFF);
        if ram {
            (true, (bank & 7) * 0x2000 + off)
        } else {
            (false, (bank % self.banks8) * 0x2000 + off)
        }
    }

    fn ram_writable(&self) -> bool {
        self.protect == [2, 1]
    }

    pub fn product(&self) -> u16 {
        u16::from(self.multiplicand) * u16::from(self.multiplier)
    }

    /// Whether the PPU is in its sprite-pattern fetches (reads 128–159 of a line).
    pub fn sprite_phase(&self) -> bool {
        self.in_frame && (128..160).contains(&self.reads_in_line)
    }

    fn use_set_b(&self) -> bool {
        if self.sprites_8x16 && self.rendering && self.in_frame {
            !self.sprite_phase()
        } else {
            self.last_set_b
        }
    }

    /// CHR byte offset for pattern address `addr`.
    pub fn chr_page(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        if self.exram_mode == 1 && self.in_frame && !self.sprite_phase() {
            let bank = usize::from(self.ext_tile & 0x3F) | (usize::from(self.chr_upper & 3) << 6);
            return bank * 0x1000 + (a & 0xFFF);
        }
        let (size, idx) = match self.chr_mode & 3 {
            0 => (0x2000, 7),
            1 => (0x1000, (a >> 12) * 4 + 3),
            2 => (0x800, (a >> 11) * 2 + 1),
            _ => (0x400, a >> 10),
        };
        let reg = if self.use_set_b() {
            self.chr_b[idx & 3]
        } else {
            self.chr_a[idx]
        };
        usize::from(reg) * size + (a & (size - 1))
    }

    fn quadrant(&self, addr: u16) -> u8 {
        (self.nt_map >> (((addr >> 10) & 3) * 2)) & 3
    }

    fn pcm_write(&mut self, v: u8) {
        if v == 0 {
            self.pcm_irq = true;
        } else {
            self.pcm_irq = false;
            self.pcm = v;
        }
    }

    /// Expansion audio level: both pulses at APU-pulse volume and the 8-bit PCM at
    /// the APU DMC's scale, all with inverted polarity (NESdev MMC5 audio).
    pub fn audio_level(&self) -> f32 {
        let pulses =
            f32::from(self.pulse[0].output() + self.pulse[1].output()) * APU_PULSE_FULL / 15.0;
        let dmc_step = crate::apu::mix([0, 0, 0, 0, 127]) / 127.0;
        -(pulses + f32::from(self.pcm) * dmc_step)
    }

    /// For the background fetch about to happen (from the read count of this
    /// line): the split-region `(column, split y)` if it falls in the split, as
    /// enabled by `$5200` (left: columns below T; right: T and up) with ExRAM in
    /// mode 0/1. Columns 2–33 are fetched during dots 1–256 of a line, columns 0–1
    /// of the next line at 321–336; the split y is `$5201` plus the scanline,
    /// wrapping at 240 (at 256 for scroll values 240–255).
    pub fn split_fetch(&self) -> Option<(usize, usize)> {
        if self.split_ctrl & 0x80 == 0 || self.exram_mode >= 2 || !self.in_frame || !self.rendering
        {
            return None;
        }
        let u = usize::from(self.reads_in_line) + 1;
        let line = usize::from(self.scanline);
        let (col, line) = match u {
            0..=127 => (u / 4 + 2, line),
            160..=167 => ((u - 160) / 4, line + 1),
            170 => (2, line + 1),
            _ => return None,
        };
        let t = usize::from(self.split_ctrl & 0x1F);
        let right = self.split_ctrl & 0x40 != 0;
        if (col < t) == right {
            return None;
        }
        let scroll = usize::from(self.split_scroll);
        let y = if scroll < 240 {
            (scroll + line) % 240
        } else {
            (scroll + line) % 256
        };
        Some((col, y))
    }
}

impl Cartridge for Mmc5 {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x5204 => {
                let v = (u8::from(self.irq_pending) << 7) | (u8::from(self.in_frame) << 6);
                self.irq_pending = false;
                v
            }
            0x5010 => {
                let v = (u8::from(self.pcm_irq && self.pcm_irq_enabled) << 7) | 1;
                self.pcm_irq = false;
                v
            }
            0x8000..=0xBFFF if self.pcm_read_mode => {
                let v = self.peek(addr);
                self.pcm_write(v);
                v
            }
            _ => self.peek(addr),
        }
    }
    fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x5204 => (u8::from(self.irq_pending) << 7) | (u8::from(self.in_frame) << 6),
            0x5010 => (u8::from(self.pcm_irq && self.pcm_irq_enabled) << 7) | 1,
            0x5015 => {
                u8::from(self.pulse[0].length > 0) | (u8::from(self.pulse[1].length > 0) << 1)
            }
            0x5205 => self.product() as u8,
            0x5206 => (self.product() >> 8) as u8,
            0x5C00..=0x5FFF if self.exram_mode >= 2 => self.exram[usize::from(addr - 0x5C00)],
            0x6000.. => {
                let (ram, off) = self.prg_target(addr);
                if ram {
                    self.ram[off]
                } else {
                    self.prg[off]
                }
            }
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x5000..=0x5003 => self.pulse[0].write(addr, value),
            0x5004..=0x5007 => self.pulse[1].write(addr, value),
            0x5010 => {
                self.pcm_read_mode = value & 1 != 0;
                self.pcm_irq_enabled = value & 0x80 != 0;
            }
            0x5011 if !self.pcm_read_mode => self.pcm_write(value),
            0x5015 => {
                for (i, p) in self.pulse.iter_mut().enumerate() {
                    p.enabled = value & (1 << i) != 0;
                    if !p.enabled {
                        p.length = 0;
                    }
                }
            }
            0x5200 => self.split_ctrl = value,
            0x5201 => self.split_scroll = value,
            0x5202 => self.split_bank = value,
            0x5100 => self.prg_mode = value & 3,
            0x5101 => self.chr_mode = value & 3,
            0x5102 => self.protect[0] = value & 3,
            0x5103 => self.protect[1] = value & 3,
            0x5104 => self.exram_mode = value & 3,
            0x5105 => self.nt_map = value,
            0x5106 => self.fill_tile = value,
            0x5107 => self.fill_attr = value & 3,
            0x5113..=0x5117 => self.prg_regs[usize::from(addr - 0x5113)] = value,
            0x5120..=0x5127 => {
                self.chr_a[usize::from(addr - 0x5120)] =
                    u16::from(value) | (u16::from(self.chr_upper & 3) << 8);
                self.last_set_b = false;
            }
            0x5128..=0x512B => {
                self.chr_b[usize::from(addr - 0x5128)] =
                    u16::from(value) | (u16::from(self.chr_upper & 3) << 8);
                self.last_set_b = true;
            }
            0x5130 => self.chr_upper = value & 3,
            0x5203 => self.irq_compare = value,
            0x5204 => self.irq_enabled = value & 0x80 != 0,
            0x5205 => self.multiplicand = value,
            0x5206 => self.multiplier = value,
            0x5C00..=0x5FFF => match self.exram_mode {
                2 => self.exram[usize::from(addr - 0x5C00)] = value,
                3 => {}
                _ => self.exram[usize::from(addr - 0x5C00)] = if self.in_frame { value } else { 0 },
            },
            0x6000.. => {
                let (ram, off) = self.prg_target(addr);
                if ram && self.ram_writable() {
                    self.ram[off] = value;
                }
            }
            _ => {}
        }
    }
    fn snoop_ppu_register(&mut self, addr: u16, value: u8) {
        match addr & 7 {
            0 => self.sprites_8x16 = value & 0x20 != 0,
            1 => {
                self.rendering = value & 0x18 != 0;
                if !self.rendering {
                    self.in_frame = false;
                }
            }
            _ => {}
        }
    }
    fn ppu_read(&mut self, addr: u16) {
        self.read_seen = true;
        self.reads_in_line = self.reads_in_line.saturating_add(1);
        if (0x2000..0x3000).contains(&addr) && addr & 0x3FF < 0x3C0 && !self.sprite_phase() {
            self.ext_tile = self.exram[usize::from(addr & 0x3FF)];
        }
        if (0x2000..0x3000).contains(&addr) && addr == self.last_nt {
            self.matches += 1;
            if self.matches == 2 {
                self.reads_in_line = 0;
                if self.in_frame {
                    self.scanline = self.scanline.wrapping_add(1);
                    if self.scanline == self.irq_compare && self.irq_compare != 0 {
                        self.irq_pending = true;
                    }
                } else {
                    self.in_frame = true;
                    self.scanline = 0;
                }
            }
        } else {
            self.matches = 0;
        }
        self.last_nt = addr;
    }
    fn audio(&self) -> f32 {
        self.audio_level()
    }
    fn clock(&mut self) {
        self.cpu_parity = !self.cpu_parity;
        if self.cpu_parity {
            for p in &mut self.pulse {
                p.clock_timer();
            }
        }
        self.frame_divider += 1;
        if self.frame_divider >= MMC5_FRAME_CYCLES {
            self.frame_divider = 0;
            for p in &mut self.pulse {
                p.clock_frame();
            }
        }
        if self.read_seen {
            self.idle = 0;
        } else if self.idle < 3 {
            self.idle += 1;
            if self.idle == 3 {
                self.in_frame = false;
                self.last_nt = 0xFFFF;
                self.matches = 0;
            }
        }
        self.read_seen = false;
    }
    fn ppu_target(&self, addr: u16) -> PpuTarget {
        if let (Some((col, y)), false) = (self.split_fetch(), self.sprite_phase()) {
            let c = col % 32;
            return if addr < 0x2000 {
                PpuTarget::Chr(
                    usize::from(self.split_bank) * 0x1000 + (usize::from(addr) & 0xFF8) + (y & 7),
                )
            } else if addr & 0x3FF >= 0x3C0 {
                let a = self.exram[0x3C0 + (y / 32) * 8 + c / 4];
                let shift = ((y / 16) & 1) * 4 + ((c / 2) & 1) * 2;
                PpuTarget::Mapper(((a >> shift) & 3) * 0x55)
            } else {
                PpuTarget::Mapper(self.exram[(y / 8) * 32 + c])
            };
        }
        if addr < 0x2000 {
            return PpuTarget::Chr(self.chr_page(addr));
        }
        let off = usize::from(addr & 0x3FF);
        let attr = off >= 0x3C0;
        if attr && self.exram_mode == 1 && self.in_frame && !self.sprite_phase() {
            return PpuTarget::Mapper((self.ext_tile >> 6) * 0x55);
        }
        match self.quadrant(addr) {
            0 => PpuTarget::Ciram(off),
            1 => PpuTarget::Ciram(0x400 + off),
            2 if self.exram_mode <= 1 => PpuTarget::Mapper(self.exram[off]),
            2 => PpuTarget::Mapper(0),
            _ if attr => PpuTarget::Mapper(self.fill_attr * 0x55),
            _ => PpuTarget::Mapper(self.fill_tile),
        }
    }
    fn ppu_write(&mut self, addr: u16, value: u8) {
        if addr >= 0x2000 && self.quadrant(addr) == 2 && self.exram_mode <= 1 {
            self.exram[usize::from(addr & 0x3FF)] = value;
        }
    }
    fn irq(&self) -> bool {
        (self.irq_pending && self.irq_enabled) || (self.pcm_irq && self.pcm_irq_enabled)
    }
    fn save_ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.clone())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        restore_ram(&mut self.ram, data)
    }
    fn state_kind(&self) -> u8 {
        5
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.prg, &self.ram);
        w.bytes(&self.exram);
        for v in [
            self.prg_mode,
            self.chr_mode,
            self.protect[0],
            self.protect[1],
            self.exram_mode,
            self.nt_map,
            self.fill_tile,
            self.fill_attr,
        ] {
            w.u8(v);
        }
        w.bytes(&self.prg_regs);
        for c in self.chr_a.iter().chain(&self.chr_b) {
            w.u16(*c);
        }
        w.u8(self.chr_upper);
        for b in [
            self.last_set_b,
            self.irq_enabled,
            self.irq_pending,
            self.in_frame,
            self.sprites_8x16,
            self.rendering,
            self.read_seen,
        ] {
            w.bool(b);
        }
        w.u8(self.irq_compare);
        w.u8(self.scanline);
        w.u8(self.multiplicand);
        w.u8(self.multiplier);
        w.u16(self.last_nt);
        w.u8(self.matches);
        w.u8(self.idle);
        w.u16(self.reads_in_line);
        w.u8(self.ext_tile);
        if w.version >= 7 {
            for p in &self.pulse {
                for b in [p.enabled, p.halt, p.constant, p.env_start] {
                    w.bool(b);
                }
                for v in [
                    p.duty,
                    p.volume,
                    p.step,
                    p.length,
                    p.env_divider,
                    p.env_decay,
                ] {
                    w.u8(v);
                }
                w.u16(p.period);
                w.u16(p.timer);
            }
            for b in [
                self.pcm_read_mode,
                self.pcm_irq_enabled,
                self.pcm_irq,
                self.cpu_parity,
            ] {
                w.bool(b);
            }
            w.u8(self.pcm);
            w.u16(self.frame_divider);
            w.u8(self.split_ctrl);
            w.u8(self.split_scroll);
            w.u8(self.split_bank);
        }
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram, "MMC5 PRG RAM")?;
        r.fill(&mut self.exram, "MMC5 ExRAM")?;
        self.prg_mode = r.small(3, "MMC5 PRG mode")?;
        self.chr_mode = r.small(3, "MMC5 CHR mode")?;
        self.protect = [r.small(3, "MMC5 protect")?, r.small(3, "MMC5 protect")?];
        self.exram_mode = r.small(3, "MMC5 ExRAM mode")?;
        self.nt_map = r.u8("MMC5 nametables")?;
        self.fill_tile = r.u8("MMC5 fill tile")?;
        self.fill_attr = r.small(3, "MMC5 fill attribute")?;
        r.fill(&mut self.prg_regs, "MMC5 PRG")?;
        for c in self.chr_a.iter_mut().chain(self.chr_b.iter_mut()) {
            *c = r.u16("MMC5 CHR")?;
            if *c > 0x3FF {
                return Err(NesStateError::BadValue {
                    what: "MMC5 CHR",
                    value: u64::from(*c),
                });
            }
        }
        self.chr_upper = r.small(3, "MMC5 CHR upper")?;
        self.last_set_b = r.bool("MMC5 set")?;
        self.irq_enabled = r.bool("MMC5 IRQ enable")?;
        self.irq_pending = r.bool("MMC5 IRQ")?;
        self.in_frame = r.bool("MMC5 in frame")?;
        self.sprites_8x16 = r.bool("MMC5 8x16")?;
        self.rendering = r.bool("MMC5 rendering")?;
        self.read_seen = r.bool("MMC5 read")?;
        self.irq_compare = r.u8("MMC5 compare")?;
        self.scanline = r.u8("MMC5 scanline")?;
        self.multiplicand = r.u8("MMC5 multiplicand")?;
        self.multiplier = r.u8("MMC5 multiplier")?;
        self.last_nt = r.u16("MMC5 last NT")?;
        self.matches = r.small(2, "MMC5 matches")?;
        self.idle = r.small(3, "MMC5 idle")?;
        self.reads_in_line = r.u16("MMC5 reads")?;
        self.ext_tile = r.u8("MMC5 ext tile")?;
        if r.version >= 7 {
            for p in &mut self.pulse {
                p.enabled = r.bool("MMC5 pulse")?;
                p.halt = r.bool("MMC5 pulse")?;
                p.constant = r.bool("MMC5 pulse")?;
                p.env_start = r.bool("MMC5 pulse")?;
                p.duty = r.small(3, "MMC5 duty")?;
                p.volume = r.small(15, "MMC5 volume")?;
                p.step = r.small(7, "MMC5 step")?;
                p.length = r.u8("MMC5 length")?;
                p.env_divider = r.small(15, "MMC5 envelope")?;
                p.env_decay = r.small(15, "MMC5 decay")?;
                p.period = r.u16("MMC5 period")?;
                p.timer = r.u16("MMC5 timer")?;
                if p.period > 0x7FF || p.timer > 0x7FF {
                    return Err(NesStateError::BadValue {
                        what: "MMC5 period",
                        value: u64::from(p.period.max(p.timer)),
                    });
                }
            }
            self.pcm_read_mode = r.bool("MMC5 PCM mode")?;
            self.pcm_irq_enabled = r.bool("MMC5 PCM IRQ enable")?;
            self.pcm_irq = r.bool("MMC5 PCM IRQ")?;
            self.cpu_parity = r.bool("MMC5 parity")?;
            self.pcm = r.u8("MMC5 PCM")?;
            self.frame_divider = r.u16("MMC5 frame divider")?;
            if self.frame_divider >= MMC5_FRAME_CYCLES {
                return Err(NesStateError::BadValue {
                    what: "MMC5 frame divider",
                    value: u64::from(self.frame_divider),
                });
            }
            self.split_ctrl = r.u8("MMC5 split")?;
            self.split_scroll = r.u8("MMC5 split scroll")?;
            self.split_bank = r.u8("MMC5 split bank")?;
        }
        Ok(())
    }
}

/// Konami VRC7 (mapper 85). Submapper 1 is VRC7b (register select on A3,
/// `$x008`), submapper 2 VRC7a (A4, `$x010`); submapper 0 accepts both.
///
/// * `$8000`, `$8008/$8010`, `$9000`: 8 KiB PRG at `$8000`, `$A000`, `$C000` (6 bits);
///   `$E000` fixed to the last bank.
/// * `$A000`–`$D008/$D010`: eight 1 KiB CHR banks (CHR RAM is banked the same way).
/// * `$E000`: bits 1–0 mirroring (vertical, horizontal, one-screen low/high), bit
///   6 silences expansion sound, bit 7 enables the 8 KiB PRG RAM at `$6000`.
/// * `$E008/$E010` IRQ latch, `$F000` control, `$F008/$F010` ack ([`VrcIrq`]).
/// * `$9010` / `$9030`: the YM2413-derived (OPLL) sound register select and data.
///   The 64 registers are stored, but FM synthesis (and the VRC7's built-in
///   instrument ROM, which is only known from measurements) is not generated, so
///   [`Cartridge::audio`] stays 0.
pub struct Vrc7 {
    prg: Vec<u8>,
    banks8: usize,
    pub submapper: u8,
    ram: Box<[u8; 0x2000]>,
    pub prg_regs: [u8; 3],
    pub chr: [u8; 8],
    pub control: u8,
    pub irq: VrcIrq,
    pub audio_select: u8,
    pub audio_regs: [u8; 64],
}

impl Vrc7 {
    pub fn new(prg: Vec<u8>, submapper: u8) -> Result<Self, MapperError> {
        if prg.len() < 0x2000 || prg.len() % 0x2000 != 0 {
            return Err(MapperError::BadPrgSize(prg.len()));
        }
        let banks8 = prg.len() / 0x2000;
        Ok(Self {
            prg,
            banks8,
            submapper,
            ram: Box::new([0; 0x2000]),
            prg_regs: [0; 3],
            chr: [0; 8],
            control: 0,
            irq: VrcIrq::default(),
            audio_select: 0,
            audio_regs: [0; 64],
        })
    }

    /// `(base, second register)` for a write, honouring the board's select line.
    pub fn decode(&self, addr: u16) -> (u16, bool) {
        let second = match self.submapper {
            1 => addr & 0x08 != 0,
            2 => addr & 0x10 != 0,
            _ => addr & 0x18 != 0,
        };
        (addr & 0xF000, second)
    }
}

impl Cartridge for Vrc7 {
    fn save_ram(&self) -> Option<Vec<u8>> {
        Some(self.ram.to_vec())
    }
    fn load_save_ram(&mut self, data: &[u8]) -> Result<(), SaveRamError> {
        restore_ram(&mut self.ram[..], data)
    }
    fn state_kind(&self) -> u8 {
        85
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.prg, &self.ram[..]);
        w.bytes(&self.prg_regs);
        w.bytes(&self.chr);
        w.u8(self.control);
        self.irq.save(w);
        w.u8(self.audio_select);
        w.bytes(&self.audio_regs);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.prg, "PRG ROM")?;
        r.fill(&mut self.ram[..], "PRG RAM")?;
        r.fill(&mut self.prg_regs, "VRC7 PRG")?;
        if let Some(&v) = self.prg_regs.iter().find(|&&v| v > 0x3F) {
            return Err(NesStateError::BadValue {
                what: "VRC7 PRG",
                value: u64::from(v),
            });
        }
        r.fill(&mut self.chr, "VRC7 CHR")?;
        self.control = r.u8("VRC7 control")?;
        self.irq.load(r)?;
        self.audio_select = r.small(0x3F, "VRC7 audio select")?;
        r.fill(&mut self.audio_regs, "VRC7 audio")?;
        Ok(())
    }
    fn peek(&self, addr: u16) -> u8 {
        let bank = |b: usize| b % self.banks8;
        match addr {
            0x6000..=0x7FFF if self.control & 0x80 != 0 => self.ram[usize::from(addr - 0x6000)],
            0x8000..=0xDFFF => {
                let slot = usize::from((addr - 0x8000) >> 13);
                self.prg
                    [bank(usize::from(self.prg_regs[slot])) * 0x2000 + usize::from(addr & 0x1FFF)]
            }
            0xE000.. => self.prg[(self.banks8 - 1) * 0x2000 + usize::from(addr & 0x1FFF)],
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        if (0x6000..0x8000).contains(&addr) {
            if self.control & 0x80 != 0 {
                self.ram[usize::from(addr - 0x6000)] = value;
            }
            return;
        }
        if addr < 0x8000 {
            return;
        }
        // The sound port uses A5 together with A4 ($9010 select, $9030 data).
        if addr & 0xF030 == 0x9010 {
            self.audio_select = value & 0x3F;
            return;
        }
        if addr & 0xF030 == 0x9030 {
            self.audio_regs[usize::from(self.audio_select)] = value;
            return;
        }
        let (base, second) = self.decode(addr);
        match (base, second) {
            (0x8000, false) => self.prg_regs[0] = value & 0x3F,
            (0x8000, true) => self.prg_regs[1] = value & 0x3F,
            (0x9000, false) => self.prg_regs[2] = value & 0x3F,
            (0xA000..=0xD000, _) => {
                let i = usize::from((base - 0xA000) >> 12) * 2 + usize::from(second);
                self.chr[i] = value;
            }
            (0xE000, false) => self.control = value,
            (0xE000, true) => self.irq.latch = value,
            (0xF000, false) => self.irq.control(value),
            (0xF000, true) => self.irq.ack(),
            _ => {}
        }
    }
    fn chr_offset(&self, addr: u16) -> usize {
        let a = usize::from(addr & 0x1FFF);
        usize::from(self.chr[a >> 10]) * 0x400 + (a & 0x3FF)
    }
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        use crate::ppu::Mirroring::*;
        Some([Vertical, Horizontal, SingleLow, SingleHigh][usize::from(self.control & 3)])
    }
    fn clock(&mut self) {
        self.irq.clock();
    }
    fn irq(&self) -> bool {
        self.irq.pending
    }
}

/// CRC-16/KERMIT (reflected polynomial `$8408`, initial 0) as computed by the RP2C33
/// over each disk block, including the `$80` that ends the gap.
pub fn fds_crc(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc = fds_crc_step(crc, b);
    }
    crc
}

fn fds_crc_step(mut crc: u16, b: u8) -> u16 {
    crc ^= u16::from(b);
    for _ in 0..8 {
        crc = if crc & 1 != 0 {
            (crc >> 1) ^ 0x8408
        } else {
            crc >> 1
        };
    }
    crc
}

/// Gap before the first block: 28300 bits (typical), in bytes.
pub const FDS_LEAD_GAP: usize = 28300 / 8;
/// Gap between blocks: 976 bits (typical), in bytes.
pub const FDS_BLOCK_GAP: usize = 976 / 8;
/// CPU cycles per disk byte (96.4 kbit/s serial stream ≈ 149 cycles per byte).
pub const FDS_BYTE_CYCLES: u16 = 149;

/// Rebuild the bit stream of one `.fds` side (65500 bytes, no gaps or CRCs): lead
/// gap, then for each block a `$80` gap terminator, the block, its CRC (low byte
/// first) and an inter-block gap. Blocks follow the 1, 2, (3, 4)… structure; the
/// file size comes from each block 3. The stream stops at the first byte that is
/// not a valid next block code (the zero padding of a dump).
pub fn fds_raw_side(side: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; FDS_LEAD_GAP];
    let mut i = 0usize;
    let mut file_size = 0usize;
    let mut expect: &[u8] = &[1];
    while i < side.len() && expect.contains(&side[i]) {
        let len = match side[i] {
            1 => 56,
            2 => 2,
            3 => 16,
            _ => 1 + file_size,
        };
        if i + len > side.len() {
            break;
        }
        let block = &side[i..i + len];
        if block[0] == 3 {
            file_size = usize::from(u16::from_le_bytes([block[13], block[14]]));
        }
        let mut with_mark = vec![0x80];
        with_mark.extend_from_slice(block);
        let crc = fds_crc(&with_mark);
        out.extend_from_slice(&with_mark);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend(std::iter::repeat_n(0u8, FDS_BLOCK_GAP));
        expect = match block[0] {
            1 => &[2],
            2 | 4 => &[3],
            _ => &[4],
        };
        i += len;
    }
    out
}

/// FDS wavetable channel (2C33). See NESdev "FDS audio": 64 six-bit samples,
/// volume and mod envelopes ticking every `8 * (e + 1) * (m + 1)` CPU cycles, a
/// wave unit and a modulation unit each ticked every 16 cycles, and the documented
/// pitch formula combining `$4082/3`, the mod counter and the mod gain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdsAudio {
    pub wave: [u8; 64],
    pub mod_table: [u8; 32],
    pub wave_write: bool,
    pub master: u8,
    pub vol_ctrl: u8,
    pub vol_gain: u8,
    pub vol_out: u8,
    vol_timer: u32,
    pub mod_ctrl: u8,
    pub mod_gain: u8,
    mod_timer: u32,
    pub freq: u16,
    pub wave_halt: bool,
    pub env_halt: bool,
    pub mod_freq: u16,
    pub mod_halt: bool,
    pub mod_force: bool,
    /// Signed 7-bit mod counter (`$4085`).
    pub mod_counter: i8,
    pub env_speed: u8,
    /// 24-bit wave accumulator; the position is bits 23–18.
    pub wave_acc: u32,
    /// 18-bit mod accumulator; the table address is bits 17–13.
    pub mod_acc: u32,
    divider: u8,
    held: u8,
}

impl Default for FdsAudio {
    fn default() -> Self {
        Self {
            wave: [0; 64],
            mod_table: [0; 32],
            wave_write: false,
            master: 0,
            vol_ctrl: 0x80,
            vol_gain: 0,
            vol_out: 0,
            vol_timer: 0,
            mod_ctrl: 0x80,
            mod_gain: 0,
            mod_timer: 0,
            freq: 0,
            wave_halt: true,
            env_halt: false,
            mod_freq: 0,
            mod_halt: true,
            mod_force: false,
            mod_counter: 0,
            env_speed: 0xE8,
            wave_acc: 0,
            mod_acc: 0,
            divider: 0,
            held: 0,
        }
    }
}

/// Mod table entry → counter change (`None`: reset to 0).
pub const FDS_MOD_STEPS: [Option<i8>; 8] = [
    Some(0),
    Some(1),
    Some(2),
    Some(4),
    None,
    Some(-4),
    Some(-2),
    Some(-1),
];

impl FdsAudio {
    pub fn position(&self) -> usize {
        (self.wave_acc >> 18) as usize & 63
    }

    /// Envelope period in CPU cycles: `8 * (e + 1) * (m + 1)`.
    pub fn envelope_period(ctrl: u8, master: u8) -> u32 {
        8 * (u32::from(ctrl & 0x3F) + 1) * (u32::from(master) + 1)
    }

    /// The documented pitch formula: wave accumulator increment per tick.
    pub fn wave_pitch(freq: u16, counter: i8, gain: u8) -> u32 {
        let mut temp = (i32::from(counter) * i32::from(gain)) & 0xFFF;
        if temp & 0x0F != 0 && temp & 0x800 == 0 {
            temp += 0x20;
        }
        temp += 0x400;
        let temp = ((temp >> 4) & 0xFF) as u32;
        (u32::from(freq) * temp) & 0xFFFFF
    }

    pub fn read(&self, addr: u16) -> Option<u8> {
        Some(match addr {
            0x4040..=0x407F => {
                0x40 | if self.wave_write {
                    self.wave[usize::from(addr - 0x4040)]
                } else {
                    self.wave[self.position()]
                }
            }
            0x4090 => 0x40 | self.vol_gain,
            0x4091 => (self.wave_acc >> 12) as u8,
            0x4092 => 0x40 | self.mod_gain,
            0x4093 => ((self.mod_acc >> 5) & 0x7F) as u8,
            0x4097 => (self.mod_counter as u8) & 0x7F,
            _ => return None,
        })
    }

    pub fn write(&mut self, addr: u16, v: u8) {
        match addr {
            0x4040..=0x407F if self.wave_write => self.wave[usize::from(addr - 0x4040)] = v & 0x3F,
            0x4080 => {
                self.vol_ctrl = v;
                if v & 0x80 != 0 {
                    self.vol_gain = v & 0x3F;
                    if self.vol_gain == 0 {
                        self.vol_out = 0; // muting takes effect at once
                    }
                }
                self.vol_timer = 0;
            }
            0x4082 => self.freq = (self.freq & 0xF00) | u16::from(v),
            0x4083 => {
                self.freq = (self.freq & 0xFF) | (u16::from(v & 0x0F) << 8);
                self.wave_halt = v & 0x80 != 0;
                self.env_halt = v & 0x40 != 0;
                if self.wave_halt {
                    self.wave_acc = 0;
                }
                if self.env_halt {
                    self.vol_timer = 0;
                    self.mod_timer = 0;
                }
            }
            0x4084 => {
                self.mod_ctrl = v;
                if v & 0x80 != 0 {
                    self.mod_gain = v & 0x3F;
                }
                self.mod_timer = 0;
            }
            0x4085 => self.mod_counter = (((v & 0x7F) << 1) as i8) >> 1,
            0x4086 => self.mod_freq = (self.mod_freq & 0xF00) | u16::from(v),
            0x4087 => {
                self.mod_freq = (self.mod_freq & 0xFF) | (u16::from(v & 0x0F) << 8);
                self.mod_halt = v & 0x80 != 0;
                self.mod_force = v & 0x40 != 0;
                if self.mod_halt {
                    self.mod_acc &= !0x1FFF;
                }
            }
            0x4088 if self.mod_halt => {
                self.mod_table[((self.mod_acc >> 13) & 31) as usize] = v & 7;
                self.mod_acc = (self.mod_acc + 0x2000) & 0x3FFFF;
            }
            0x4089 => {
                self.wave_write = v & 0x80 != 0;
                self.master = v & 3;
            }
            0x408A => self.env_speed = v,
            _ => {}
        }
    }

    fn tick_envelope(ctrl: u8, gain: &mut u8) {
        if ctrl & 0x40 != 0 {
            if *gain < 32 {
                *gain += 1;
            }
        } else if *gain > 0 {
            *gain -= 1;
        }
    }

    /// One CPU cycle.
    pub fn clock(&mut self) {
        let envelopes = !self.wave_halt && !self.env_halt && self.env_speed != 0;
        if envelopes && self.vol_ctrl & 0x80 == 0 {
            self.vol_timer += 1;
            if self.vol_timer >= Self::envelope_period(self.vol_ctrl, self.env_speed) {
                self.vol_timer = 0;
                Self::tick_envelope(self.vol_ctrl, &mut self.vol_gain);
            }
        }
        if envelopes && self.mod_ctrl & 0x80 == 0 {
            self.mod_timer += 1;
            if self.mod_timer >= Self::envelope_period(self.mod_ctrl, self.env_speed) {
                self.mod_timer = 0;
                Self::tick_envelope(self.mod_ctrl, &mut self.mod_gain);
            }
        }
        self.divider = (self.divider + 1) & 15;
        if self.divider != 0 {
            return;
        }
        if !self.mod_halt {
            let step = if self.mod_force {
                0x1000
            } else {
                u32::from(self.mod_freq)
            };
            let old = self.mod_acc;
            if step != 0 {
                if (old & 0xFFF) + step > 0xFFF {
                    let entry = self.mod_table[((old >> 13) & 31) as usize];
                    self.mod_counter = match FDS_MOD_STEPS[usize::from(entry)] {
                        None => 0,
                        Some(d) => ((((self.mod_counter.wrapping_add(d)) as u8) << 1) as i8) >> 1,
                    };
                }
                self.mod_acc = (old + step) & 0x3FFFF;
            }
        }
        if !self.wave_halt {
            let pitch = Self::wave_pitch(self.freq, self.mod_counter, self.mod_gain);
            self.wave_acc = (self.wave_acc + pitch) & 0xFFFFFF;
        }
        if self.position() == 0 {
            self.vol_out = self.vol_gain.min(32);
        }
        if !self.wave_write {
            self.held = self.wave[self.position()];
        }
    }

    /// DAC level: held/current sample × volume (≤ 32) × master volume.
    pub fn level(&self) -> f32 {
        let master = [30.0, 20.0, 15.0, 12.0][usize::from(self.master)] / 30.0;
        f32::from(self.held) * f32::from(self.vol_out) * master
    }

    /// In APU-mixer units: full scale (63 × 32) is 2.4 × a full APU pulse.
    pub fn output(&self) -> f32 {
        self.level() * 2.4 * APU_PULSE_FULL / (63.0 * 32.0)
    }

    fn save(&self, w: &mut StateWriter) {
        w.bytes(&self.wave);
        w.bytes(&self.mod_table);
        for b in [
            self.wave_write,
            self.wave_halt,
            self.env_halt,
            self.mod_halt,
            self.mod_force,
        ] {
            w.bool(b);
        }
        for v in [
            self.master,
            self.vol_ctrl,
            self.vol_gain,
            self.vol_out,
            self.mod_ctrl,
            self.mod_gain,
            self.mod_counter as u8,
            self.env_speed,
            self.divider,
            self.held,
        ] {
            w.u8(v);
        }
        w.u32(self.vol_timer);
        w.u32(self.mod_timer);
        w.u16(self.freq);
        w.u16(self.mod_freq);
        w.u32(self.wave_acc);
        w.u32(self.mod_acc);
    }

    fn load(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fill(&mut self.wave, "FDS wave")?;
        r.fill(&mut self.mod_table, "FDS mod table")?;
        self.wave_write = r.bool("FDS wave write")?;
        self.wave_halt = r.bool("FDS wave halt")?;
        self.env_halt = r.bool("FDS envelope halt")?;
        self.mod_halt = r.bool("FDS mod halt")?;
        self.mod_force = r.bool("FDS mod force")?;
        self.master = r.small(3, "FDS master")?;
        self.vol_ctrl = r.u8("FDS volume")?;
        self.vol_gain = r.small(63, "FDS volume gain")?;
        self.vol_out = r.small(32, "FDS volume out")?;
        self.mod_ctrl = r.u8("FDS mod")?;
        self.mod_gain = r.small(63, "FDS mod gain")?;
        let c = r.u8("FDS mod counter")? as i8;
        if !(-64..=63).contains(&c) {
            return Err(NesStateError::BadValue {
                what: "FDS mod counter",
                value: u64::from(c as u8),
            });
        }
        self.mod_counter = c;
        self.env_speed = r.u8("FDS envelope speed")?;
        self.divider = r.small(15, "FDS divider")?;
        self.held = r.small(63, "FDS held sample")?;
        self.vol_timer = r.u32("FDS volume timer")?;
        self.mod_timer = r.u32("FDS mod timer")?;
        self.freq = r.u16("FDS frequency")?;
        self.mod_freq = r.u16("FDS mod frequency")?;
        self.wave_acc = r.u32("FDS wave accumulator")?;
        self.mod_acc = r.u32("FDS mod accumulator")?;
        if self.freq > 0xFFF
            || self.mod_freq > 0xFFF
            || self.wave_acc > 0xFFFFFF
            || self.mod_acc > 0x3FFFF
            || self.mod_table.iter().any(|&m| m > 7)
            || self.wave.iter().any(|&w| w > 63)
        {
            return Err(NesStateError::BadValue {
                what: "FDS audio",
                value: 0,
            });
        }
        Ok(())
    }
}

/// Famicom Disk System RAM adapter (RP2C33) with its drive.
///
/// * `$6000`–`$DFFF` 32 KiB PRG RAM, `$E000`–`$FFFF` the 8 KiB BIOS (supplied by
///   the user), 8 KiB CHR RAM on the PPU side.
/// * Timer IRQ (`$4020`/`$4021` reload, `$4022` bit 0 repeat, bit 1 enable): a
///   `$4022` write with bit 1 copies the reload into the counter; each CPU cycle
///   an enabled counter at 0 raises the IRQ, reloads, and (without repeat) stops;
///   otherwise it decrements. `$4022` with bit 1 clear, reading `$4030`, or
///   clearing `$4023` bit 0 stop/acknowledge it; `$4022` is ignored while `$4023`
///   bit 0 is clear.
/// * Drive: `$4025` bit 0 scan (0 returns the head to the start), bit 1 motor (0
///   runs), bit 2 read (1) / write (0), bit 3 mirroring (1 horizontal), bit 4 CRC
///   transfer, bit 6 CRC enable, bit 7 byte-transfer IRQ. A running scan moves one
///   byte every [`FDS_BYTE_CYCLES`]. Reading: with CRC enabled the adapter waits for
///   the `$80` that ends a gap, then each byte goes to `$4031`, raises the transfer
///   flag (`$4030` bit 7) and, if enabled, the disk IRQ. The CRC runs over the
///   `$80`, the block and the stored CRC, so `$4030` bit 4 (error) is set while
///   CRC transfer is on and the running CRC is not zero. Writing: each byte time
///   stores `$4024` (or, with CRC transfer, the CRC low then high byte; with CRC
///   disabled, gap zeros) unless the side is write-protected. At the end of the
///   side `$4030` bit 6 (end of head) and `$4032` bit 1 (not ready) are set until
///   the next scan reset.
/// * Disk IRQ acknowledged by `$4024` and `$4025` writes and `$4030`, `$4031`,
///   `$4032` reads. `$4032`: bit 0 no disk, bit 1 not ready, bit 2 protected or no
///   disk. `$4033` bit 7: drive power good, reported while the motor runs.
pub struct Fds {
    bios: Vec<u8>,
    pub ram: Box<[u8; 0x8000]>,
    /// Bit streams of every side (see [`fds_raw_side`]).
    pub sides: Vec<Vec<u8>>,
    pub inserted: Option<usize>,
    pub write_protected: bool,
    pub reload: u16,
    pub counter: u16,
    pub timer_ctrl: u8,
    pub timer_irq: bool,
    pub io_enable: u8,
    pub write_data: u8,
    pub read_data: u8,
    pub control: u8,
    pub ext_out: u8,
    pub transfer_flag: bool,
    pub disk_irq: bool,
    pub head: usize,
    pub end_of_head: bool,
    pub gap_ended: bool,
    pub crc: u16,
    crc_out: u8,
    byte_timer: u16,
    pub audio: FdsAudio,
}

impl Fds {
    /// `bios` is the 8 KiB RAM adapter ROM; `sides` the `.fds` sides (65500 bytes).
    pub fn new(bios: Vec<u8>, sides: &[Vec<u8>]) -> Result<Self, MapperError> {
        if bios.len() != 0x2000 {
            return Err(MapperError::BadPrgSize(bios.len()));
        }
        Ok(Self {
            bios,
            ram: Box::new([0; 0x8000]),
            sides: sides.iter().map(|s| fds_raw_side(s)).collect(),
            inserted: (!sides.is_empty()).then_some(0),
            write_protected: false,
            reload: 0,
            counter: 0,
            timer_ctrl: 0,
            timer_irq: false,
            io_enable: 0,
            write_data: 0,
            read_data: 0,
            control: 0x26,
            ext_out: 0xFF,
            transfer_flag: false,
            disk_irq: false,
            head: 0,
            end_of_head: false,
            gap_ended: false,
            crc: 0,
            crc_out: 0,
            byte_timer: 0,
            audio: FdsAudio::default(),
        })
    }

    /// Eject (`None`) or insert side `n`.
    pub fn insert(&mut self, side: Option<usize>) {
        self.inserted = side.filter(|&s| s < self.sides.len());
        self.head = 0;
        self.end_of_head = false;
        self.gap_ended = false;
    }

    fn disk_on(&self) -> bool {
        self.io_enable & 1 != 0
    }

    fn motor_running(&self) -> bool {
        self.inserted.is_some()
            && self.control & 2 == 0
            && self.control & 1 != 0
            && !self.end_of_head
    }

    pub fn drive_status(&self) -> u8 {
        let none = self.inserted.is_none();
        u8::from(none)
            | (u8::from(none || self.end_of_head || self.control & 1 == 0) << 1)
            | (u8::from(none || self.write_protected) << 2)
    }

    pub fn status(&self) -> u8 {
        let crc_err = self.control & 0x10 != 0 && self.crc != 0;
        u8::from(self.timer_irq)
            | (u8::from(self.control & 8 != 0) << 3)
            | (u8::from(crc_err) << 4)
            | (u8::from(self.end_of_head) << 6)
            | (u8::from(self.transfer_flag) << 7)
    }

    fn byte_time(&mut self) {
        let Some(side) = self.inserted else { return };
        let len = self.sides[side].len();
        if self.head >= len {
            self.end_of_head = true;
            return;
        }
        let reading = self.control & 4 != 0;
        let crc_on = self.control & 0x40 != 0;
        if reading {
            let b = self.sides[side][self.head];
            if !crc_on {
                self.gap_ended = false;
                self.crc = 0;
            } else if !self.gap_ended {
                if b == 0x80 {
                    self.gap_ended = true;
                    self.crc = fds_crc_step(0, 0x80);
                }
            } else {
                self.read_data = b;
                self.crc = fds_crc_step(self.crc, b);
                self.raise_transfer();
            }
        } else {
            let out = if !crc_on {
                self.crc = 0;
                self.crc_out = 0;
                0
            } else if self.control & 0x10 != 0 {
                let crc = self.crc.to_le_bytes();
                let b = crc[usize::from(self.crc_out.min(1))];
                self.crc_out = (self.crc_out + 1).min(2);
                b
            } else {
                let b = self.write_data;
                self.crc = fds_crc_step(self.crc, b);
                b
            };
            if !self.write_protected {
                self.sides[side][self.head] = out;
            }
            self.raise_transfer();
        }
        self.head += 1;
        if self.head >= len {
            self.end_of_head = true;
        }
    }

    fn raise_transfer(&mut self) {
        self.transfer_flag = true;
        if self.control & 0x80 != 0 {
            self.disk_irq = true;
        }
    }
}

impl Cartridge for Fds {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x4030 => {
                let v = self.status();
                self.timer_irq = false;
                self.disk_irq = false;
                self.transfer_flag = false;
                v
            }
            0x4031 => {
                self.disk_irq = false;
                self.transfer_flag = false;
                self.read_data
            }
            0x4032 => {
                self.disk_irq = false;
                self.drive_status()
            }
            _ => self.peek(addr),
        }
    }
    fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x4030 => self.status(),
            0x4031 => self.read_data,
            0x4032 => self.drive_status(),
            0x4033 => {
                (self.ext_out & 0x7F)
                    | (u8::from(self.inserted.is_some() && self.control & 2 == 0) << 7)
            }
            0x4040..=0x4097 if self.io_enable & 2 != 0 => self.audio.read(addr).unwrap_or(0),
            0x6000..=0xDFFF => self.ram[usize::from(addr - 0x6000)],
            0xE000.. => self.bios[usize::from(addr - 0xE000)],
            _ => 0,
        }
    }
    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x4020 => self.reload = (self.reload & 0xFF00) | u16::from(value),
            0x4021 => self.reload = (self.reload & 0x00FF) | (u16::from(value) << 8),
            0x4022 if self.disk_on() => {
                self.timer_ctrl = value & 3;
                if value & 2 != 0 {
                    self.counter = self.reload;
                } else {
                    self.timer_irq = false;
                }
            }
            0x4023 => {
                self.io_enable = value;
                if value & 1 == 0 {
                    self.timer_ctrl &= !2;
                    self.timer_irq = false;
                    self.disk_irq = false;
                }
            }
            0x4024 if self.disk_on() => {
                self.write_data = value;
                self.disk_irq = false;
                self.transfer_flag = false;
            }
            0x4025 if self.disk_on() => {
                self.disk_irq = false;
                if value & 1 == 0 {
                    self.head = 0;
                    self.end_of_head = false;
                    self.gap_ended = false;
                }
                if value & 0x40 == 0 {
                    self.gap_ended = false;
                    self.crc = 0;
                    self.crc_out = 0;
                }
                self.control = value;
            }
            0x4026 => self.ext_out = value,
            0x4040..=0x408A if self.io_enable & 2 != 0 => self.audio.write(addr, value),
            0x6000..=0xDFFF => self.ram[usize::from(addr - 0x6000)] = value,
            _ => {}
        }
    }
    fn mirroring(&self) -> Option<crate::ppu::Mirroring> {
        use crate::ppu::Mirroring::*;
        Some(if self.control & 8 != 0 {
            Horizontal
        } else {
            Vertical
        })
    }
    fn clock(&mut self) {
        if self.disk_on() && self.timer_ctrl & 2 != 0 {
            if self.counter == 0 {
                self.timer_irq = true;
                self.counter = self.reload;
                if self.timer_ctrl & 1 == 0 {
                    self.timer_ctrl &= !2;
                }
            } else {
                self.counter -= 1;
            }
        }
        if self.disk_on() && self.motor_running() {
            self.byte_timer += 1;
            if self.byte_timer >= FDS_BYTE_CYCLES {
                self.byte_timer = 0;
                self.byte_time();
            }
        } else {
            self.byte_timer = 0;
        }
        if self.io_enable & 2 != 0 {
            self.audio.clock();
        }
    }
    fn irq(&self) -> bool {
        self.timer_irq || self.disk_irq
    }
    fn audio(&self) -> f32 {
        if self.io_enable & 2 != 0 {
            self.audio.output()
        } else {
            0.0
        }
    }
    fn state_kind(&self) -> u8 {
        20
    }
    fn save_state(&self, w: &mut StateWriter) {
        prg_ram_state(w, &self.bios, &self.ram[..]);
        w.u8(self.sides.len() as u8);
        for s in &self.sides {
            w.u32(s.len() as u32);
            w.bytes(s);
        }
        w.u8(self.inserted.map_or(0xFF, |s| s as u8));
        for b in [
            self.write_protected,
            self.timer_irq,
            self.transfer_flag,
            self.disk_irq,
            self.end_of_head,
            self.gap_ended,
        ] {
            w.bool(b);
        }
        w.u16(self.reload);
        w.u16(self.counter);
        for v in [
            self.timer_ctrl,
            self.io_enable,
            self.write_data,
            self.read_data,
            self.control,
            self.ext_out,
            self.crc_out,
        ] {
            w.u8(v);
        }
        w.u32(self.head as u32);
        w.u16(self.crc);
        w.u16(self.byte_timer);
        self.audio.save(w);
    }
    fn load_state(&mut self, r: &mut StateReader) -> Result<(), NesStateError> {
        r.fingerprint(&self.bios, "FDS BIOS")?;
        r.fill(&mut self.ram[..], "FDS RAM")?;
        let n = usize::from(r.u8("FDS sides")?);
        if n != self.sides.len() {
            return Err(NesStateError::BadValue {
                what: "FDS sides",
                value: n as u64,
            });
        }
        for s in &mut self.sides {
            let len = r.u32("FDS side length")? as usize;
            if len != s.len() {
                return Err(NesStateError::BadValue {
                    what: "FDS side length",
                    value: len as u64,
                });
            }
            r.fill(s, "FDS side")?;
        }
        let ins = r.u8("FDS inserted")?;
        self.inserted = match ins {
            0xFF => None,
            s if usize::from(s) < n => Some(usize::from(s)),
            s => {
                return Err(NesStateError::BadValue {
                    what: "FDS inserted",
                    value: u64::from(s),
                })
            }
        };
        self.write_protected = r.bool("FDS protect")?;
        self.timer_irq = r.bool("FDS timer IRQ")?;
        self.transfer_flag = r.bool("FDS transfer")?;
        self.disk_irq = r.bool("FDS disk IRQ")?;
        self.end_of_head = r.bool("FDS end")?;
        self.gap_ended = r.bool("FDS gap")?;
        self.reload = r.u16("FDS reload")?;
        self.counter = r.u16("FDS counter")?;
        self.timer_ctrl = r.small(3, "FDS timer control")?;
        self.io_enable = r.u8("FDS I/O")?;
        self.write_data = r.u8("FDS write data")?;
        self.read_data = r.u8("FDS read data")?;
        self.control = r.u8("FDS control")?;
        self.ext_out = r.u8("FDS ext")?;
        self.crc_out = r.small(2, "FDS CRC out")?;
        self.head = r.u32("FDS head")? as usize;
        self.crc = r.u16("FDS CRC")?;
        self.byte_timer = r.u16("FDS byte timer")?;
        if self.byte_timer >= FDS_BYTE_CYCLES {
            return Err(NesStateError::BadValue {
                what: "FDS byte timer",
                value: u64::from(self.byte_timer),
            });
        }
        self.audio.load(r)
    }
}
