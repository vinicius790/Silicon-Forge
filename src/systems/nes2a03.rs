//! 2A03 machine without a cartridge: the Ricoh core (6502 without decimal mode),
//! 2 KiB of RAM mirrored through `$0000`–`$1FFF`, the APU at `$4000`–`$4017`, and
//! 32 KiB of program RAM at `$8000`–`$FFFF` filled from the project's assembler, or
//! a [`Cartridge`] mapper that then owns `$4020`–`$FFFF`.
//! The 2C02 [`Ppu`] sits at `$2000`–`$3FFF` (three dots per CPU cycle, NMI wired to
//! the CPU) and reads pattern data from [`NesBus::chr`] through the cartridge's CHR
//! banking; without a cartridge `chr` is 8 KiB of CHR RAM. `$4014` starts OAM DMA:
//! the CPU stops for 513 cycles (514 when the write lands on an odd cycle) and the
//! page is copied into OAM. `$4018`–`$401F` read 0 and ignore writes.
//!
//! [`Nes2A03::run_cycles`] advances the CPU one bus cycle at a time with
//! `Cpu::step_cycle`, ticks the APU once per cycle and drives the CPU IRQ line from
//! the frame interrupt. `$4016`/`$4017` are the controller ports of [`crate::input`]:
//! writing `$4016` sets the strobe (OUT0) for both ports, reads shift one bit out of
//! each pad with `$40` open bus above it; a [`Zapper`] can take port 2.

use crate::apu::{Apu, DMC_STALL};
use crate::asm::assemble_with_symbols;
use crate::bus::MemoryBus;
use crate::cpu::Cpu;
use crate::mapper::Cartridge;
use crate::opcodes::CpuVariant;
use crate::ppu::{Mirroring, Ppu, PpuMemory};

pub struct NesBus {
    pub ram: [u8; 0x800],
    pub prg: Box<[u8; 0x8000]>,
    pub apu: Apu,
    pub cart: Option<Box<dyn Cartridge>>,
    pub ppu: Ppu,
    pub chr: Vec<u8>,
    pub chr_writable: bool,
    pub mirroring: Mirroring,
    /// Page written to `$4014`, waiting for the DMA to run.
    pub dma_page: Option<u8>,
    /// CPU cycles spent in OAM DMA.
    pub oam_dma_cycles: u64,
    /// Standard controllers on `$4016` and `$4017`.
    pub pads: [Controller; 2],
    /// `$4016` bit 0 (OUT0): while high both shift registers reload continuously.
    pub strobe: bool,
    /// A Zapper in port 2 replaces controller 2 on `$4017` (see [`Zapper`]). It has
    /// no latched state, so it is not part of the machine snapshot.
    pub zapper: Option<Zapper>,
    /// Four Score on both ports (pads 3 and 4 in it).
    pub four_score: Option<crate::input::FourScore>,
    /// Arkanoid controller in port 2.
    pub paddle: Option<crate::input::ArkanoidPaddle>,
    /// Power Pad in port 2.
    pub power_pad: Option<crate::input::PowerPad>,
    /// Set when DMC DMA halted the CPU on a read cycle: the 2A03 keeps driving that
    /// read during the halt, so if it was `$4016`/`$4017` the pad is clocked once more
    /// (back-to-back halt reads keep /OE low and count once) before the real read
    /// after the DMA. One controller bit is lost, the documented DMC/controller
    /// conflict that games avoid by re-reading until two reads agree.
    pub dmc_halted_read: bool,
    /// Controller bits lost to that conflict (diagnostics).
    pub dmc_conflicts: u64,
    /// Read cycles the CPU has spent halted in the current DMC DMA window.
    pub dmc_stall_reads: u8,
    /// Extra PPU register accesses owed to the halted read: the 2C02's /CS is
    /// qualified by M2, so unlike the joypad each halted cycle except the DMA's
    /// own fetch repeats a `$2002`/`$2007` read (clearing vblank and the write
    /// toggle again, or advancing the VRAM address and skipping a byte).
    pub dmc_repeats: u8,
    /// PPU register reads repeated that way (diagnostics).
    pub dmc_ppu_repeats: u64,
}

macro_rules! ppu_mem {
    ($bus:expr) => {
        PpuMemory {
            cart: match $bus.cart.as_mut() {
                Some(c) => Some(c.as_mut()),
                None => None,
            },
            chr: &mut $bus.chr,
            chr_writable: $bus.chr_writable,
            mirroring: $bus.mirroring,
        }
    };
}

impl NesBus {
    /// Device bits of one read of port `port` (0 = `$4016`), clocking it.
    fn port_read(&mut self, port: usize) -> u8 {
        let strobe = self.strobe;
        if port == 1 {
            if let Some(p) = self.paddle.as_mut() {
                return p.read(strobe);
            }
            if let Some(p) = self.power_pad.as_mut() {
                return p.read(strobe);
            }
        }
        match self.four_score.as_mut() {
            Some(f) => f.read(port, &mut self.pads, strobe),
            None => self.pads[port].read(strobe),
        }
    }

    fn port_peek(&self, port: usize) -> u8 {
        if port == 1 {
            if let Some(p) = &self.paddle {
                return p.peek();
            }
            if let Some(p) = &self.power_pad {
                return p.peek();
            }
        }
        match &self.four_score {
            Some(f) => f.peek(port, &self.pads, self.strobe),
            None => self.pads[port].peek(self.strobe),
        }
    }

    pub fn new() -> Self {
        Self {
            ram: [0; 0x800],
            prg: Box::new([0; 0x8000]),
            apu: Apu::new(),
            cart: None,
            ppu: Ppu::new(),
            chr: vec![0; 0x2000],
            chr_writable: true,
            mirroring: Mirroring::Horizontal,
            dma_page: None,
            oam_dma_cycles: 0,
            pads: [Controller::default(); 2],
            strobe: false,
            zapper: None,
            four_score: None,
            paddle: None,
            power_pad: None,
            dmc_halted_read: false,
            dmc_conflicts: 0,
            dmc_stall_reads: 0,
            dmc_repeats: 0,
            dmc_ppu_repeats: 0,
        }
    }

    /// Advance the PPU one dot.
    pub fn ppu_tick(&mut self) {
        let mut mem = ppu_mem!(self);
        self.ppu.tick(&mut mem);
    }
}

impl Default for NesBus {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBus for NesBus {
    fn read(&mut self, addr: u16) -> u8 {
        let halted = std::mem::take(&mut self.dmc_halted_read);
        let repeats = std::mem::take(&mut self.dmc_repeats);
        if let (0x4020.., Some(cart)) = (addr, self.cart.as_mut()) {
            return cart.read(addr);
        }
        if (0x2000..0x4000).contains(&addr) {
            let mut mem = ppu_mem!(self);
            if matches!(addr & 7, 2 | 7) {
                for _ in 0..repeats {
                    self.ppu.read_register(&mut mem, addr);
                }
                self.dmc_ppu_repeats += u64::from(repeats);
            }
            return self.ppu.read_register(&mut mem, addr);
        }
        match addr {
            0x0000..=0x1FFF => self.ram[usize::from(addr & 0x7FF)],
            0x4015 => self.apu.read_status(),
            0x4017 if self.zapper.is_some() => {
                OPEN_BUS_4016 | self.zapper.as_ref().map_or(0, |z| z.report(&self.ppu))
            }
            0x4016 | 0x4017 => {
                let port = usize::from(addr - 0x4016);
                if halted && !self.strobe {
                    self.port_read(port);
                    self.dmc_conflicts += 1;
                }
                OPEN_BUS_4016 | self.port_read(port)
            }
            0x8000..=0xFFFF => self.prg[usize::from(addr - 0x8000)],
            _ => 0,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        if let (0x4020.., Some(cart)) = (addr, self.cart.as_mut()) {
            return cart.write(addr, value);
        }
        if (0x2000..0x4000).contains(&addr) {
            if let Some(c) = self.cart.as_mut() {
                c.snoop_ppu_register(addr, value);
            }
            let mut mem = ppu_mem!(self);
            return self.ppu.write_register(&mut mem, addr, value);
        }
        if addr == 0x4014 {
            self.dma_page = Some(value);
            return;
        }
        match addr {
            0x0000..=0x1FFF => self.ram[usize::from(addr & 0x7FF)] = value,
            0x4016 => {
                let was = self.strobe;
                self.strobe = value & 1 != 0;
                if self.strobe {
                    for pad in &mut self.pads {
                        pad.latch();
                    }
                    if let Some(f) = self.four_score.as_mut() {
                        f.latch();
                    }
                    if let Some(p) = self.power_pad.as_mut() {
                        p.latch();
                    }
                } else if was {
                    if let Some(p) = self.paddle.as_mut() {
                        p.latch();
                    }
                }
            }
            0x4000..=0x4017 => self.apu.write_register(addr - 0x4000, value),
            0x8000..=0xFFFF => self.prg[usize::from(addr - 0x8000)] = value,
            _ => {}
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        if let (0x4020.., Some(cart)) = (addr, self.cart.as_ref()) {
            return cart.peek(addr);
        }
        match addr {
            0x0000..=0x1FFF => self.ram[usize::from(addr & 0x7FF)],
            0x2000..=0x3FFF => self.ppu.peek_register(addr),
            0x4015 => self.apu.peek_status(),
            0x4017 if self.zapper.is_some() => {
                OPEN_BUS_4016 | self.zapper.as_ref().map_or(0, |z| z.report(&self.ppu))
            }
            0x4016 | 0x4017 => OPEN_BUS_4016 | self.port_peek(usize::from(addr - 0x4016)),
            0x8000..=0xFFFF => self.prg[usize::from(addr - 0x8000)],
            _ => 0,
        }
    }
}

pub struct Nes2A03 {
    pub cpu: Cpu<NesBus>,
    /// CPU cycles lost to DMC sample fetches.
    pub stalled: u64,
    /// Cycles left in an open DMC DMA window; kept across [`Nes2A03::run_cycles`] calls.
    pub dma_window: u32,
    /// OAM DMA cycles still owed when a `run_cycles` budget ended inside the stall.
    pub oam_stall: u64,
    /// PPU dots owed in units of 1/den of a CPU cycle (PAL: 16 dots per 5 cycles).
    pub dot_phase: u32,
    /// The header's battery flag: whether [`Nes2A03::save_ram`] reports PRG RAM.
    pub battery: bool,
}

impl Nes2A03 {
    /// Assemble `source` (it must place code in `$8000`–`$FFFF` and set the vectors),
    /// load it and reset.
    pub fn from_source(source: &str) -> Result<Self, String> {
        let image = assemble_with_symbols(source, 0x8000).map_err(|e| e.0)?;
        if image.origin < 0x8000 {
            return Err(format!(
                "programa em ${:04X}, abaixo de $8000",
                image.origin
            ));
        }
        if usize::from(image.origin - 0x8000) + image.bytes.len() > 0x8000 {
            return Err("programa passa de $FFFF".into());
        }
        let mut bus = NesBus::new();
        let start = usize::from(image.origin - 0x8000);
        bus.prg[start..start + image.bytes.len()].copy_from_slice(&image.bytes);
        let vector = u16::from_le_bytes([bus.prg[0x7FFC], bus.prg[0x7FFD]]);
        if vector < 0x8000 {
            return Err(format!("vetor de reset ${vector:04X} fora do PRG"));
        }
        let mut cpu = Cpu::with_variant(bus, CpuVariant::Ricoh2A03);
        cpu.reset();
        Ok(Self {
            cpu,
            stalled: 0,
            dma_window: 0,
            oam_stall: 0,
            dot_phase: 0,
            battery: false,
        })
    }

    fn clock_devices(&mut self) {
        let (dots, cycles) = self.cpu.bus.ppu.region.dots_per_cpu();
        self.dot_phase += dots;
        while self.dot_phase >= cycles {
            self.dot_phase -= cycles;
            self.cpu.bus.ppu_tick();
        }
        if self.cpu.bus.ppu.take_nmi() {
            self.cpu.pulse_nmi();
        }
        self.cpu.bus.apu.tick();
        if let Some(cart) = self.cpu.bus.cart.as_mut() {
            cart.clock();
        }
    }

    /// Boot from a cartridge (vectors come from its last bank).
    pub fn with_cartridge(cart: Box<dyn Cartridge>) -> Self {
        let mut bus = NesBus::new();
        bus.cart = Some(cart);
        let mut cpu = Cpu::with_variant(bus, CpuVariant::Ricoh2A03);
        cpu.reset();
        Self {
            cpu,
            stalled: 0,
            dma_window: 0,
            oam_stall: 0,
            dot_phase: 0,
            battery: false,
        }
    }

    /// Boot an iNES / NES 2.0 image through [`crate::ines::Ines::cartridge`].
    pub fn from_ines(data: &[u8]) -> Result<Self, crate::ines::InesError> {
        let image = crate::ines::Ines::parse(data)?;
        let cart = image.cartridge()?;
        let mut nes = Self::with_cartridge(cart);
        if !image.chr.is_empty() {
            nes.cpu.bus.chr = image.chr;
            nes.cpu.bus.chr_writable = false;
        }
        nes.cpu.bus.mirroring = image.mirroring;
        nes.battery = image.battery;
        nes.set_region(image.timing.region());
        Ok(nes)
    }

    /// Current audio sample: the APU's nonlinear mix plus the cartridge's expansion
    /// audio ([`Cartridge::audio`]), summed as on the cartridge's audio-in pin.
    pub fn audio_sample(&self) -> f32 {
        let ext = self.cpu.bus.cart.as_ref().map_or(0.0, |c| c.audio());
        crate::apu::mix_expansion(self.cpu.bus.apu.sample(), ext)
    }

    /// `count` mixed samples, running `cycles_per_sample` CPU cycles before each.
    pub fn render_audio(
        &mut self,
        cycles_per_sample: u64,
        count: usize,
    ) -> Result<Vec<f32>, String> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            self.run_cycles(cycles_per_sample)?;
            out.push(self.audio_sample());
        }
        Ok(out)
    }

    /// Battery-backed RAM for a `.sav` file; `None` without a battery or PRG RAM.
    pub fn save_ram(&self) -> Option<Vec<u8>> {
        if !self.battery {
            return None;
        }
        self.cpu.bus.cart.as_ref()?.save_ram()
    }

    /// Restore a `.sav` image (same size as [`Nes2A03::save_ram`]).
    pub fn load_save_ram(&mut self, data: &[u8]) -> Result<(), crate::mapper::SaveRamError> {
        match (self.battery, self.cpu.bus.cart.as_mut()) {
            (true, Some(c)) => c.load_save_ram(data),
            _ => Err(crate::mapper::SaveRamError::NoSaveRam),
        }
    }

    /// Write [`Nes2A03::save_ram`] to `path` (atomically: a temporary file is
    /// renamed over it). Returns `Ok(false)` when there is nothing to save.
    pub fn write_sav(&self, path: &std::path::Path) -> std::io::Result<bool> {
        let Some(data) = self.save_ram() else {
            return Ok(false);
        };
        let tmp = path.with_extension("sav.tmp");
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)?;
        Ok(true)
    }

    /// Load `path` into battery RAM. A missing file is `Ok(false)` (first boot).
    pub fn read_sav(&mut self, path: &std::path::Path) -> std::io::Result<bool> {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        self.load_save_ram(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(true)
    }

    /// Boot the Famicom Disk System: `bios` is the 8 KiB RAM adapter ROM, `disk` a
    /// `.fds` image (side A inserted). CHR is the adapter's 8 KiB CHR RAM.
    pub fn from_fds(bios: &[u8], disk: &[u8]) -> Result<Self, String> {
        let sides = crate::ines::parse_fds(disk).map_err(|e| e.to_string())?;
        let fds = crate::mapper::Fds::new(bios.to_vec(), &sides).map_err(|e| e.to_string())?;
        let mut nes = Self::with_cartridge(Box::new(fds));
        nes.cpu.bus.chr = vec![0; 0x2000];
        nes.cpu.bus.chr_writable = true;
        Ok(nes)
    }

    /// Select NTSC, PAL or Dendy timing: PPU line count and vblank line, the
    /// CPU/PPU clock ratio, and the APU tables (RP2A07 for PAL). Call before running.
    pub fn set_region(&mut self, region: crate::ppu::Region) {
        self.cpu.bus.ppu.set_region(region);
        self.cpu.bus.apu.set_pal(region == crate::ppu::Region::Pal);
        self.dot_phase = 0;
    }

    pub fn region(&self) -> crate::ppu::Region {
        self.cpu.bus.ppu.region
    }

    /// Run until `n` more PPU frames have completed (the PPU is then at line 0,
    /// dot 0 or within the CPU cycle that crossed it).
    pub fn run_frames(&mut self, n: u64) -> Result<(), String> {
        let target = self.cpu.bus.ppu.frame + n;
        while self.cpu.bus.ppu.frame < target {
            self.run_cycles(1)?;
        }
        Ok(())
    }

    /// The framebuffer as a binary PPM (P6, 256x240, 8-bit RGB).
    pub fn frame_ppm(&self) -> Vec<u8> {
        let mut out =
            format!("P6\n{} {}\n255\n", crate::ppu::WIDTH, crate::ppu::HEIGHT).into_bytes();
        out.extend(self.cpu.bus.ppu.rgb_frame());
        out
    }

    /// Headless runner: render `n` frames and write each to `dir/<stem>_NNNN.ppm`.
    pub fn render_frames_ppm(
        &mut self,
        n: u64,
        dir: &std::path::Path,
        stem: &str,
    ) -> Result<Vec<std::path::PathBuf>, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        for i in 0..n {
            self.run_frames(1)?;
            let path = dir.join(format!("{stem}_{i:04}.ppm"));
            std::fs::write(&path, self.frame_ppm()).map_err(|e| e.to_string())?;
            paths.push(path);
        }
        Ok(paths)
    }

    /// `n` CPU cycles; the APU ticks once per cycle.
    pub fn run_cycles(&mut self, n: u64) -> Result<(), String> {
        let mut left = n;
        let carry = self.oam_stall.min(left);
        for _ in 0..carry {
            self.clock_devices();
        }
        self.oam_stall -= carry;
        left -= carry;
        while left > 0 {
            if self.dma_window == 0 && self.cpu.bus.apu.dmc_fetch_address().is_some() {
                self.dma_window = DMC_STALL;
            }
            if self.dma_window > 0 {
                let address = self
                    .cpu
                    .bus
                    .apu
                    .dmc_fetch_address()
                    .ok_or("DMC request vanished inside the DMA window")?;
                let mut window = self.dma_window;
                while window > 0 && left > 0 {
                    if self.cpu.next_cycle_writes() {
                        self.cpu.step_cycle()?;
                    } else {
                        self.stalled += 1;
                        let b = &mut self.cpu.bus;
                        b.dmc_stall_reads = b.dmc_stall_reads.saturating_add(1);
                    }
                    self.clock_devices();
                    window -= 1;
                    left -= 1;
                }
                self.dma_window = window;
                if window == 0 {
                    let byte = self.cpu.bus.read(address);
                    self.cpu.bus.apu.dmc_deliver(byte);
                    let b = &mut self.cpu.bus;
                    let reads = std::mem::take(&mut b.dmc_stall_reads);
                    b.dmc_halted_read = reads > 0;
                    // The last halted cycle is the DMA's own fetch.
                    b.dmc_repeats = reads.saturating_sub(1);
                }
                continue;
            }
            let cart_irq = self.cpu.bus.cart.as_ref().is_some_and(|c| c.irq());
            self.cpu
                .set_irq_line(self.cpu.bus.apu.irq_asserted() || cart_irq);
            self.cpu.step_cycle()?;
            self.clock_devices();
            left -= 1;
            if let Some(page) = self.cpu.bus.dma_page.take() {
                let odd = self.cpu.cycles.wrapping_sub(1) & 1 == 1;
                let mut data = [0u8; 256];
                for (i, b) in data.iter_mut().enumerate() {
                    *b = self.cpu.bus.read((u16::from(page) << 8) | i as u16);
                }
                self.cpu.bus.ppu.oam_dma(&data);
                let stall = 513 + u64::from(odd);
                for _ in 0..stall.min(left) {
                    self.clock_devices();
                }
                // A stall longer than the remaining budget finishes on the next call.
                self.oam_stall = stall.saturating_sub(left);
                self.cpu.bus.oam_dma_cycles += stall;
                left = left.saturating_sub(stall);
            }
        }
        Ok(())
    }
}

pub use crate::input::{button, Controller, Zapper, OPEN_BUS_4016};
