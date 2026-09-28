//! iNES / NES 2.0 cartridge file format: header parsing, validation and building.
//!
//! Layout: 16-byte header, optional 512-byte trainer (flags 6 bit 2), PRG ROM in
//! 16 KiB units, CHR ROM in 8 KiB units, then anything else (kept as `trailing`).
//!
//! Header bytes:
//! * 0–3 `NES\x1A`; 4 PRG size (16 KiB units, LSB); 5 CHR size (8 KiB units, LSB).
//! * 6: bit 0 vertical mirroring, 1 battery, 2 trainer, 3 four-screen, 7–4 mapper bits 3–0.
//! * 7: bit 0 Vs. System, 1 PlayChoice-10, 3–2 `10` marks NES 2.0, 7–4 mapper bits 7–4.
//! * NES 2.0: 8 low nibble mapper bits 11–8, high nibble submapper; 9 low/high nibble
//!   PRG/CHR size MSB (`$F` selects the exponent-multiplier form, `2^E * (2M+1)` bytes);
//!   10 low nibble PRG RAM shift (`64 << n` bytes, 0 = none), high nibble PRG NVRAM shift;
//!   12 bits 1–0 timing (0 NTSC, 1 PAL, 2 multi, 3 Dendy).
//! * iNES 1: 8 PRG RAM in 8 KiB units (0 means 8 KiB). If bytes 12–15 are not zero the
//!   header was written by an old tool ("DiskDude!") and byte 7 is ignored, as emulators do.
//!
//! [`Ines::cartridge`] builds the mapper ([`crate::mapper`]), honouring the documented
//! NES 2.0 submappers; the machine banks CHR through it.

use crate::mapper::{
    Axrom, Camerica, Cartridge, Fme7, Latch, MapperError, Mmc1, Mmc24, Mmc3, Namco163, Nina001,
    Nrom, Uxrom, Vrc1, Vrc24, VrcWiring,
};
use std::fmt;

pub const MAGIC: [u8; 4] = *b"NES\x1A";
pub const TRAINER: usize = 512;

pub use crate::ppu::Mirroring;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timing {
    Ntsc,
    Pal,
    Multi,
    Dendy,
}

impl Timing {
    /// Console timing to emulate: multi-region images run as NTSC, the first
    /// region the documentation lists for them.
    pub fn region(self) -> crate::ppu::Region {
        use crate::ppu::Region;
        match self {
            Self::Ntsc | Self::Multi => Region::Ntsc,
            Self::Pal => Region::Pal,
            Self::Dendy => Region::Dendy,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InesError {
    TooShort(usize),
    BadMagic([u8; 4]),
    /// The file ends before the trainer/PRG/CHR the header announces.
    Truncated {
        need: usize,
        have: usize,
    },
    NoPrg,
    /// Size does not fit in `usize` or exceeds 4 GiB.
    SizeOverflow,
    UnsupportedMapper(u16),
    /// Mapper, submapper: a NES 2.0 submapper this model does not implement.
    UnsupportedSubmapper(u16, u8),
    Mapper(MapperError),
}

impl fmt::Display for InesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort(n) => write!(f, "arquivo com {n} bytes; o cabeçalho tem 16"),
            Self::BadMagic(m) => write!(f, "assinatura inválida: {m:02X?}"),
            Self::Truncated { need, have } => {
                write!(f, "arquivo truncado: precisa de {need} bytes, tem {have}")
            }
            Self::NoPrg => f.write_str("PRG ROM vazia"),
            Self::SizeOverflow => f.write_str("tamanho de ROM fora da faixa"),
            Self::UnsupportedMapper(m) => write!(f, "mapper {m} não suportado"),
            Self::UnsupportedSubmapper(m, s) => {
                write!(f, "submapper {s} do mapper {m} não suportado")
            }
            Self::Mapper(e) => write!(f, "mapper: {e}"),
        }
    }
}

impl std::error::Error for InesError {}

impl From<MapperError> for InesError {
    fn from(e: MapperError) -> Self {
        Self::Mapper(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ines {
    pub nes2: bool,
    pub mapper: u16,
    pub submapper: u8,
    pub mirroring: Mirroring,
    pub battery: bool,
    pub vs: bool,
    pub playchoice: bool,
    /// PRG RAM bytes (volatile + battery-backed).
    pub prg_ram: usize,
    pub timing: Timing,
    pub trainer: Option<Vec<u8>>,
    pub prg: Vec<u8>,
    pub chr: Vec<u8>,
    pub trailing: Vec<u8>,
}

fn rom_size(lsb: u8, msb: u8, unit: usize) -> Result<usize, InesError> {
    if msb == 0xF {
        let exp = u32::from(lsb >> 2);
        let mult = u64::from(lsb & 3) * 2 + 1;
        let bytes = 1u64
            .checked_shl(exp)
            .and_then(|p| p.checked_mul(mult))
            .filter(|&b| b <= 1 << 32)
            .ok_or(InesError::SizeOverflow)?;
        return usize::try_from(bytes).map_err(|_| InesError::SizeOverflow);
    }
    Ok(((usize::from(msb) << 8) | usize::from(lsb)) * unit)
}

fn shift_size(n: u8) -> usize {
    if n == 0 {
        0
    } else {
        64 << n
    }
}

impl Ines {
    pub fn parse(data: &[u8]) -> Result<Self, InesError> {
        if data.len() < 16 {
            return Err(InesError::TooShort(data.len()));
        }
        let h = &data[..16];
        if h[..4] != MAGIC {
            return Err(InesError::BadMagic([h[0], h[1], h[2], h[3]]));
        }
        let nes2 = h[7] & 0x0C == 0x08;
        let archaic = !nes2 && h[12..16].iter().any(|&b| b != 0);
        let f7 = if archaic { 0 } else { h[7] };
        let mut mapper = u16::from(h[6] >> 4) | u16::from(f7 & 0xF0);
        let (prg_len, chr_len, submapper, prg_ram, timing);
        if nes2 {
            mapper |= u16::from(h[8] & 0x0F) << 8;
            submapper = h[8] >> 4;
            prg_len = rom_size(h[4], h[9] & 0x0F, 0x4000)?;
            chr_len = rom_size(h[5], h[9] >> 4, 0x2000)?;
            prg_ram = shift_size(h[10] & 0x0F) + shift_size(h[10] >> 4);
            timing =
                [Timing::Ntsc, Timing::Pal, Timing::Multi, Timing::Dendy][usize::from(h[12] & 3)];
        } else {
            submapper = 0;
            prg_len = usize::from(h[4]) * 0x4000;
            chr_len = usize::from(h[5]) * 0x2000;
            prg_ram = usize::from(h[8].max(1)) * 0x2000;
            timing = Timing::Ntsc;
        }
        if prg_len == 0 {
            return Err(InesError::NoPrg);
        }
        let mirroring = if h[6] & 8 != 0 {
            Mirroring::FourScreen
        } else if h[6] & 1 != 0 {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };
        let has_trainer = h[6] & 4 != 0;
        let trainer_len = if has_trainer { TRAINER } else { 0 };
        let need = 16usize
            .checked_add(trainer_len)
            .and_then(|n| n.checked_add(prg_len))
            .and_then(|n| n.checked_add(chr_len))
            .ok_or(InesError::SizeOverflow)?;
        if data.len() < need {
            return Err(InesError::Truncated {
                need,
                have: data.len(),
            });
        }
        let mut at = 16;
        let mut take = |n: usize| {
            let s = data[at..at + n].to_vec();
            at += n;
            s
        };
        let trainer = has_trainer.then(|| take(TRAINER));
        let prg = take(prg_len);
        let chr = take(chr_len);
        debug_assert_eq!(at, need);
        Ok(Self {
            nes2,
            mapper,
            submapper,
            mirroring,
            battery: h[6] & 2 != 0,
            vs: f7 & 1 != 0,
            playchoice: f7 & 2 != 0,
            prg_ram,
            timing,
            trainer,
            prg,
            chr,
            trailing: data[need..].to_vec(),
        })
    }

    /// Serialize as iNES 1 (or NES 2.0 when `nes2`). Sizes must be whole units below
    /// 4096 (NES 2.0) or 256 (iNES); PRG RAM is written in the format's own encoding.
    pub fn to_bytes(&self) -> Result<Vec<u8>, InesError> {
        let (pu, cu) = (self.prg.len() / 0x4000, self.chr.len() / 0x2000);
        let limit = if self.nes2 { 0x1000 } else { 0x100 };
        if self.prg.len() % 0x4000 != 0
            || self.chr.len() % 0x2000 != 0
            || pu >= limit
            || cu >= limit
        {
            return Err(InesError::SizeOverflow);
        }
        if self.mapper > if self.nes2 { 0xFFF } else { 0xFF } {
            return Err(InesError::UnsupportedMapper(self.mapper));
        }
        let mut h = [0u8; 16];
        h[..4].copy_from_slice(&MAGIC);
        h[4] = pu as u8;
        h[5] = cu as u8;
        h[6] = ((self.mapper as u8 & 0x0F) << 4)
            | u8::from(self.mirroring == Mirroring::Vertical)
            | (u8::from(self.battery) << 1)
            | (u8::from(self.trainer.is_some()) << 2)
            | (u8::from(self.mirroring == Mirroring::FourScreen) << 3);
        h[7] = (self.mapper as u8 & 0xF0) | u8::from(self.vs) | (u8::from(self.playchoice) << 1);
        if self.nes2 {
            h[7] |= 0x08;
            h[8] = (self.mapper >> 8) as u8 | (self.submapper << 4);
            h[9] = (pu >> 8) as u8 | (((cu >> 8) as u8) << 4);
            h[10] = (1..=15u8).find(|&n| 64 << n == self.prg_ram).unwrap_or(0);
            h[12] = self.timing as u8;
        } else {
            h[8] = (self.prg_ram / 0x2000) as u8;
        }
        let mut out = h.to_vec();
        if let Some(t) = &self.trainer {
            out.extend_from_slice(t);
        }
        out.extend_from_slice(&self.prg);
        out.extend_from_slice(&self.chr);
        out.extend_from_slice(&self.trailing);
        Ok(out)
    }

    /// CPU-side mapper for this cartridge: 0 NROM, 1 MMC1, 2 UxROM, 3 CNROM, 4 MMC3,
    /// 7 AxROM, 9 MMC2, 10 MMC4, 11 Color Dreams, 34 BNROM / NINA-001, 66 GxROM,
    /// 71 Camerica, 75 VRC1, 19 Namco 163 (no audio), 21/22/23/25 VRC2/VRC4,
    /// 69 Sunsoft FME-7 (no audio).
    ///
    /// NES 2.0 submappers honoured (others of these mappers are refused with
    /// [`InesError::UnsupportedSubmapper`]):
    /// * 2, 3, 7: 1 no bus conflicts, 2 bus conflicts; 0 keeps the board default
    ///   (conflicts on UxROM and CNROM, none on AxROM).
    /// * 4: 0 Sharp MMC3, 4 MMC3A/NEC IRQ ([`Mmc3::alt_irq`]).
    /// * 34: 1 NINA-001, 2 BNROM; 0 (and iNES 1) picks NINA-001 when CHR ROM is
    ///   larger than 8 KiB, as documented, BNROM otherwise.
    /// * 71: 1 Fire Hawk (BF9097 mirroring control); 0 none.
    /// * 21, 22, 23, 25: the VRC2/VRC4 wiring table of [`VrcWiring::for_mapper`].
    pub fn cartridge(&self) -> Result<Box<dyn Cartridge>, InesError> {
        let prg = self.prg.clone();
        let sub = if self.nes2 { self.submapper } else { 0 };
        let refuse = |ok: &[u8]| -> Result<(), InesError> {
            if ok.contains(&sub) {
                Ok(())
            } else {
                Err(InesError::UnsupportedSubmapper(self.mapper, sub))
            }
        };
        let conflicts = |default: bool| match sub {
            1 => false,
            2 => true,
            _ => default,
        };
        let chr8 = self.chr.len() / 0x2000;
        let four = self.mirroring == Mirroring::FourScreen;
        Ok(match self.mapper {
            0 => Box::new(Nrom::new(prg, self.battery || self.prg_ram > 0)?),
            1 => Box::new(Mmc1::new(prg)?),
            2 => {
                refuse(&[0, 1, 2])?;
                let mut m = Uxrom::new(prg)?;
                m.bus_conflicts = conflicts(true);
                Box::new(m)
            }
            3 => {
                refuse(&[0, 1, 2])?;
                let mut m = Latch::cnrom(prg, chr8)?;
                m.bus_conflicts = conflicts(true);
                Box::new(m)
            }
            4 => {
                refuse(&[0, 1, 4])?;
                let board = if sub == 1 {
                    crate::mapper::Mmc3Board::Mmc6
                } else {
                    crate::mapper::Mmc3Board::Mmc3
                };
                let mut m = Mmc3::with_board(prg, four, board)?;
                m.alt_irq = sub == 4;
                Box::new(m)
            }
            7 => {
                refuse(&[0, 1, 2])?;
                let mut m = Axrom::new(prg)?;
                m.bus_conflicts = conflicts(false);
                Box::new(m)
            }
            9 => Box::new(Mmc24::mmc2(prg)?),
            10 => Box::new(Mmc24::mmc4(prg)?),
            66 => Box::new(Latch::gxrom(prg, chr8)?),
            11 => Box::new(Latch::color_dreams(prg, chr8)?),
            34 => {
                refuse(&[0, 1, 2])?;
                let nina = match sub {
                    1 => true,
                    2 => false,
                    _ => self.chr.len() > 0x2000,
                };
                if nina {
                    Box::new(Nina001::new(prg)?)
                } else {
                    Box::new(Latch::bnrom(prg)?)
                }
            }
            71 => {
                refuse(&[0, 1])?;
                Box::new(Camerica::new(prg, sub == 1)?)
            }
            75 => Box::new(Vrc1::new(prg, four)?),
            19 => {
                let mut n = Namco163::new(prg)?;
                n.audio.db = crate::mapper::N163Audio::db_for_submapper(sub);
                Box::new(n)
            }
            24 | 26 => Box::new(crate::mapper::Vrc6::new(prg, self.mapper == 26)?),
            5 => Box::new(crate::mapper::Mmc5::new(prg)?),
            85 => {
                refuse(&[0, 1, 2])?;
                Box::new(crate::mapper::Vrc7::new(prg, sub)?)
            }
            118 => Box::new(Mmc3::with_board(
                prg,
                false,
                crate::mapper::Mmc3Board::TxSrom,
            )?),
            119 => Box::new(Mmc3::with_board(
                prg,
                false,
                crate::mapper::Mmc3Board::Tqrom,
            )?),
            69 => Box::new(Fme7::new(prg)?),
            21 | 22 | 23 | 25 => {
                let wiring = VrcWiring::for_mapper(self.mapper, sub)
                    .ok_or(InesError::UnsupportedSubmapper(self.mapper, sub))?;
                Box::new(Vrc24::new(prg, wiring)?)
            }
            m => return Err(InesError::UnsupportedMapper(m)),
        })
    }
}

/// Bytes per side in a `.fds` dump (no gaps, no CRCs).
pub const FDS_SIDE: usize = 65500;
/// `.fds` header magic (the 16-byte fwNES header is optional).
pub const FDS_MAGIC: [u8; 4] = *b"FDS\x1A";

/// Errors parsing a `.fds` image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FdsError {
    /// Size (after the optional header) is not a non-zero multiple of 65500.
    BadSize(usize),
    /// The header's side count disagrees with the data.
    SideCount { header: u8, data: usize },
    /// A side does not start with a disk info block carrying `*NINTENDO-HVC*`.
    NotADisk(usize),
}

impl fmt::Display for FdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSize(n) => write!(f, "imagem FDS de {n} bytes não é múltiplo de 65500"),
            Self::SideCount { header, data } => {
                write!(f, "cabeçalho diz {header} lados, dados têm {data}")
            }
            Self::NotADisk(s) => write!(f, "lado {s} sem bloco *NINTENDO-HVC*"),
        }
    }
}

impl std::error::Error for FdsError {}

/// Split a `.fds` file into its sides.
pub fn parse_fds(data: &[u8]) -> Result<Vec<Vec<u8>>, FdsError> {
    let (count, body) = if data.len() >= 16 && data[..4] == FDS_MAGIC {
        (Some(data[4]), &data[16..])
    } else {
        (None, data)
    };
    if body.is_empty() || body.len() % FDS_SIDE != 0 {
        return Err(FdsError::BadSize(body.len()));
    }
    let sides: Vec<Vec<u8>> = body.chunks(FDS_SIDE).map(<[u8]>::to_vec).collect();
    if let Some(h) = count {
        if usize::from(h) != sides.len() {
            return Err(FdsError::SideCount {
                header: h,
                data: sides.len(),
            });
        }
    }
    for (i, s) in sides.iter().enumerate() {
        if s[0] != 1 || &s[1..15] != b"*NINTENDO-HVC*" {
            return Err(FdsError::NotADisk(i));
        }
    }
    Ok(sides)
}
