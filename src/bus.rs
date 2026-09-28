//! Abstract and concrete memory buses for 16-bit address space (0x0000..=0xFFFF).
//!
//! Provides the core [`MemoryBus`] trait, the default [`FlatMemory`] 64 KiB backend,
//! and [`SystemBus`] for memory-mapped I/O devices (PIA 6820, VIA 6522, ACIA 6551, ROMs, RAM).

/// Abstraction for 8-bit read/write access across a 64 KiB address space.
pub trait MemoryBus {
    /// Read a single byte at the given address (may trigger hardware side-effects).
    fn read(&mut self, addr: u16) -> u8;

    /// Write a single byte to the given address.
    fn write(&mut self, addr: u16, value: u8);

    /// Non-intrusive read for debugger/trace (must not cause hardware side-effects).
    fn peek(&self, addr: u16) -> u8 {
        let _ = addr;
        0
    }

    /// Half-cycle clock for devices on this bus, called by [`crate::cpu::Cpu::step_phase`]
    /// before each half (before the access in PHI2). Default: no clocked devices.
    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        let _ = phase;
    }

    /// Little-endian 16-bit read (low byte at addr, high byte at addr+1).
    fn read_u16(&mut self, addr: u16) -> u16 {
        let lo = self.read(addr) as u16;
        let hi = self.read(addr.wrapping_add(1)) as u16;
        (hi << 8) | lo
    }

    /// Little-endian 16-bit write.
    fn write_u16(&mut self, addr: u16, value: u16) {
        self.write(addr, (value & 0xFF) as u8);
        self.write(addr.wrapping_add(1), (value >> 8) as u8);
    }
}

/// Linear 64 KiB Flat RAM — standard pedagogical backend.
#[derive(Clone)]
pub struct FlatMemory {
    pub data: [u8; 65536],
}

impl FlatMemory {
    /// Create empty 64 KiB zeroed memory.
    pub fn new() -> Self {
        Self { data: [0; 65536] }
    }

    /// Load a slice of bytes into memory starting at `base` address (wrapping on 64 KiB boundary).
    pub fn load_at(&mut self, base: u16, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            let addr = base.wrapping_add(i as u16);
            self.data[addr as usize] = b;
        }
    }

    /// Like [`FlatMemory::load_at`] but refuses an image that would wrap past `$FFFF`.
    pub fn try_load_at(&mut self, base: u16, bytes: &[u8]) -> Result<(), MemoryMapError> {
        if usize::from(base) + bytes.len() > 0x10000 {
            return Err(MemoryMapError::RomOutOfRange {
                start: base,
                length: bytes.len(),
            });
        }
        self.load_at(base, bytes);
        Ok(())
    }

    /// Set Reset Vector (0xFFFC/0xFFFD) to jump to `entry`.
    pub fn set_reset_vector(&mut self, entry: u16) {
        self.data[0xFFFC] = (entry & 0xFF) as u8;
        self.data[0xFFFD] = (entry >> 8) as u8;
    }

    /// Set IRQ/BRK Vector (0xFFFE/0xFFFF) to jump to `entry`.
    pub fn set_irq_vector(&mut self, entry: u16) {
        self.data[0xFFFE] = (entry & 0xFF) as u8;
        self.data[0xFFFF] = (entry >> 8) as u8;
    }

    /// Set NMI Vector (0xFFFA/0xFFFB) to jump to `entry`.
    pub fn set_nmi_vector(&mut self, entry: u16) {
        self.data[0xFFFA] = (entry & 0xFF) as u8;
        self.data[0xFFFB] = (entry >> 8) as u8;
    }
}

impl Default for FlatMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBus for FlatMemory {
    #[inline]
    fn read(&mut self, addr: u16) -> u8 {
        self.data[addr as usize]
    }

    #[inline]
    fn write(&mut self, addr: u16, value: u8) {
        self.data[addr as usize] = value;
    }

    #[inline]
    fn peek(&self, addr: u16) -> u8 {
        self.data[addr as usize]
    }
}

/// Interface for memory-mapped I/O peripherals (such as 6820 PIA, 6522 VIA, ACIA).
pub trait IoDevice {
    /// Read register at offset within device address window.
    fn read(&mut self, offset: u16) -> u8;
    /// Write register at offset within device address window.
    fn write(&mut self, offset: u16, value: u8);
    /// Non-intrusive peek.
    fn peek(&self, offset: u16) -> u8 {
        let _ = offset;
        0
    }
    /// Half-cycle clock forwarded by [`SystemBus`] from [`MemoryBus::phase`]. Default: none.
    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        let _ = phase;
    }
}

/// Failure to register a ROM or memory-mapped I/O device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryMapError {
    InvalidRange {
        start: u16,
        end: u16,
    },
    RomOutOfRange {
        start: u16,
        length: usize,
    },
    Overlap {
        start: u16,
        end: u16,
        existing_start: u16,
        existing_end: u16,
    },
}

impl std::fmt::Display for MemoryMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRange { start, end } => {
                write!(f, "invalid device range ${start:04X}..=${end:04X}")
            }
            Self::RomOutOfRange { start, length } => write!(
                f,
                "ROM image of {length} bytes at ${start:04X} extends past the 16-bit address space"
            ),
            Self::Overlap { start, end, existing_start, existing_end } => write!(
                f,
                "memory range ${start:04X}..=${end:04X} overlaps existing mapping ${existing_start:04X}..=${existing_end:04X}"
            ),
        }
    }
}

impl std::error::Error for MemoryMapError {}

/// What answers at an address of a [`SystemBus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    Ram,
    Rom,
    Device,
}

/// Inclusive address range of one kind, from [`SystemBus::memory_map`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub start: u16,
    pub end: u16,
    pub kind: RegionKind,
}

/// Memory-mapped device registration in a [`SystemBus`].
struct MappedDevice {
    start: u16,
    end: u16,
    device: Box<dyn IoDevice>,
}

/// Modular System Bus supporting RAM, ROM, and memory-mapped peripheral devices.
pub struct SystemBus {
    ram: [u8; 65536],
    readonly: [bool; 65536],
    rom_ranges: Vec<(u16, u16)>,
    devices: Vec<MappedDevice>,
}

impl SystemBus {
    /// Create a new system bus with 64 KiB read-write RAM.
    pub fn new() -> Self {
        Self {
            ram: [0; 65536],
            readonly: [false; 65536],
            rom_ranges: Vec::new(),
            devices: Vec::new(),
        }
    }

    /// Map a ROM image into the given address range, marking it read-only.
    pub fn map_rom(&mut self, start: u16, bytes: &[u8]) {
        self.try_map_rom(start, bytes)
            .expect("ROM image must fit and not overlap another memory mapping");
    }

    /// Fallibly map a ROM image, rejecting address wrap and conflicting mappings.
    pub fn try_map_rom(&mut self, start: u16, bytes: &[u8]) -> Result<(), MemoryMapError> {
        let last_address = start as usize + bytes.len().saturating_sub(1);
        if !bytes.is_empty() && last_address > u16::MAX as usize {
            return Err(MemoryMapError::RomOutOfRange {
                start,
                length: bytes.len(),
            });
        }
        if !bytes.is_empty() {
            let end = last_address as u16;
            if let Some(existing) = self
                .devices
                .iter()
                .find(|device| start <= device.end && end >= device.start)
            {
                return Err(MemoryMapError::Overlap {
                    start,
                    end,
                    existing_start: existing.start,
                    existing_end: existing.end,
                });
            }
            if let Some(&(existing_start, existing_end)) =
                self.rom_ranges
                    .iter()
                    .find(|&&(existing_start, existing_end)| {
                        start <= existing_end && end >= existing_start
                    })
            {
                return Err(MemoryMapError::Overlap {
                    start,
                    end,
                    existing_start,
                    existing_end,
                });
            }
        }
        for (i, &b) in bytes.iter().enumerate() {
            let addr = start as usize + i;
            self.ram[addr] = b;
            self.readonly[addr] = true;
        }
        if !bytes.is_empty() {
            self.rom_ranges.push((start, last_address as u16));
        }
        Ok(())
    }

    /// Map a memory-mapped I/O peripheral into the address range [start..=end].
    pub fn map_device<D: IoDevice + 'static>(&mut self, start: u16, end: u16, device: D) {
        self.try_map_device(start, end, device)
            .expect("device mapping must have a valid, non-overlapping range");
    }

    /// Fallibly map a device, rejecting invalid or overlapping address ranges.
    pub fn try_map_device<D: IoDevice + 'static>(
        &mut self,
        start: u16,
        end: u16,
        device: D,
    ) -> Result<(), MemoryMapError> {
        if start > end {
            return Err(MemoryMapError::InvalidRange { start, end });
        }
        if let Some(existing) = self
            .devices
            .iter()
            .find(|existing| start <= existing.end && end >= existing.start)
        {
            return Err(MemoryMapError::Overlap {
                start,
                end,
                existing_start: existing.start,
                existing_end: existing.end,
            });
        }
        if let Some(&(existing_start, existing_end)) = self
            .rom_ranges
            .iter()
            .find(|&&(rom_start, rom_end)| start <= rom_end && end >= rom_start)
        {
            return Err(MemoryMapError::Overlap {
                start,
                end,
                existing_start,
                existing_end,
            });
        }
        self.devices.push(MappedDevice {
            start,
            end,
            device: Box::new(device),
        });
        Ok(())
    }

    /// The whole 64 KiB as contiguous regions in address order: every address belongs to
    /// exactly one entry, gaps between ROMs and devices are RAM.
    pub fn memory_map(&self) -> Vec<Region> {
        let mut mapped: Vec<Region> = self
            .rom_ranges
            .iter()
            .map(|&(start, end)| Region {
                start,
                end,
                kind: RegionKind::Rom,
            })
            .chain(self.devices.iter().map(|d| Region {
                start: d.start,
                end: d.end,
                kind: RegionKind::Device,
            }))
            .collect();
        mapped.sort_by_key(|r| r.start);
        let mut out = Vec::with_capacity(mapped.len() * 2 + 1);
        let mut next = 0u32;
        for region in mapped {
            if u32::from(region.start) > next {
                out.push(Region {
                    start: next as u16,
                    end: region.start - 1,
                    kind: RegionKind::Ram,
                });
            }
            next = u32::from(region.end) + 1;
            out.push(region);
        }
        if next <= 0xFFFF {
            out.push(Region {
                start: next as u16,
                end: 0xFFFF,
                kind: RegionKind::Ram,
            });
        }
        debug_assert_eq!(
            out.iter()
                .map(|r| u32::from(r.end) - u32::from(r.start) + 1)
                .sum::<u32>(),
            0x10000
        );
        out
    }

    /// Helper to set standard CPU vectors.
    pub fn set_reset_vector(&mut self, entry: u16) {
        self.ram[0xFFFC] = (entry & 0xFF) as u8;
        self.ram[0xFFFD] = (entry >> 8) as u8;
    }

    pub fn set_irq_vector(&mut self, entry: u16) {
        self.ram[0xFFFE] = (entry & 0xFF) as u8;
        self.ram[0xFFFF] = (entry >> 8) as u8;
    }

    pub fn set_nmi_vector(&mut self, entry: u16) {
        self.ram[0xFFFA] = (entry & 0xFF) as u8;
        self.ram[0xFFFB] = (entry >> 8) as u8;
    }

    /// Direct access to underlying RAM for diagnostics.
    pub fn ram(&self) -> &[u8; 65536] {
        &self.ram
    }

    pub fn ram_mut(&mut self) -> &mut [u8; 65536] {
        &mut self.ram
    }
}

impl Default for SystemBus {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBus for SystemBus {
    /// Every mapped device sees each half cycle, in mapping order.
    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        for dev in &mut self.devices {
            dev.device.phase(phase);
        }
    }

    fn read(&mut self, addr: u16) -> u8 {
        for dev in &mut self.devices {
            if addr >= dev.start && addr <= dev.end {
                return dev.device.read(addr - dev.start);
            }
        }
        self.ram[addr as usize]
    }

    fn write(&mut self, addr: u16, value: u8) {
        for dev in &mut self.devices {
            if addr >= dev.start && addr <= dev.end {
                dev.device.write(addr - dev.start, value);
                return;
            }
        }
        if !self.readonly[addr as usize] {
            self.ram[addr as usize] = value;
        }
    }

    fn peek(&self, addr: u16) -> u8 {
        for dev in &self.devices {
            if addr >= dev.start && addr <= dev.end {
                return dev.device.peek(addr - dev.start);
            }
        }
        self.ram[addr as usize]
    }
}

/// Phase window check. Setup and hold times are nanoseconds inside a phase and this
/// core has no sub-phase time, so they are not modelled. What it can check is the
/// window itself: data moves only in PHI2. After [`MemoryBus::phase`] reports PHI1,
/// a read or write is rejected: it never reaches `inner`, a read returns
/// `inner.peek` (no side effect), a write is dropped, and the access is logged in
/// `rejected` as `(address, write)`. Before any phase report (plain `step` or
/// `step_cycle`) every access is allowed. `Cpu::step_phase` never trips it.
pub struct PhaseCheckBus<B: MemoryBus> {
    pub inner: B,
    pub rejected: Vec<(u16, bool)>,
    phase: Option<crate::cycles::BusPhase>,
}

impl<B: MemoryBus> PhaseCheckBus<B> {
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            rejected: Vec::new(),
            phase: None,
        }
    }

    fn in_phi1(&self) -> bool {
        self.phase == Some(crate::cycles::BusPhase::Phi1)
    }
}

impl<B: MemoryBus> MemoryBus for PhaseCheckBus<B> {
    fn read(&mut self, addr: u16) -> u8 {
        if self.in_phi1() {
            self.rejected.push((addr, false));
            return self.inner.peek(addr);
        }
        self.inner.read(addr)
    }

    fn write(&mut self, addr: u16, value: u8) {
        if self.in_phi1() {
            self.rejected.push((addr, true));
            return;
        }
        self.inner.write(addr, value);
    }

    fn peek(&self, addr: u16) -> u8 {
        self.inner.peek(addr)
    }

    fn phase(&mut self, phase: crate::cycles::BusPhase) {
        self.phase = Some(phase);
        self.inner.phase(phase);
    }
}

/// Inclusive named address window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedRegion {
    pub name: String,
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamedMapError {
    InvalidRange {
        name: String,
        start: u16,
        end: u16,
    },
    Overlap {
        name: String,
        other: String,
    },
    UnknownRegion {
        name: String,
    },
    /// `length` bytes do not fit in the region's `room` bytes. Memory is unchanged.
    PastEnd {
        name: String,
        length: usize,
        room: usize,
    },
}

impl std::fmt::Display for NamedMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRange { name, start, end } => {
                write!(f, "região {name} inválida ${start:04X}..=${end:04X}")
            }
            Self::Overlap { name, other } => write!(f, "região {name} sobrepõe {other}"),
            Self::UnknownRegion { name } => write!(f, "região desconhecida: {name}"),
            Self::PastEnd { name, length, room } => {
                write!(
                    f,
                    "carga de {length} bytes passa do fim de {name} ({room} bytes)"
                )
            }
        }
    }
}

impl std::error::Error for NamedMapError {}

/// Named regions over a flat 64 KiB image. Overlaps are rejected. A load that
/// would write past the named region returns [`NamedMapError::PastEnd`].
#[derive(Clone)]
pub struct NamedMemory {
    regions: Vec<NamedRegion>,
    data: Box<[u8; 65536]>,
}

impl Default for NamedMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl NamedMemory {
    pub fn new() -> Self {
        Self {
            regions: Vec::new(),
            data: Box::new([0; 65536]),
        }
    }

    pub fn region(&mut self, name: &str, start: u16, end: u16) -> Result<(), NamedMapError> {
        if start > end {
            return Err(NamedMapError::InvalidRange {
                name: name.to_string(),
                start,
                end,
            });
        }
        if let Some(other) = self
            .regions
            .iter()
            .find(|region| start <= region.end && region.start <= end)
        {
            return Err(NamedMapError::Overlap {
                name: name.to_string(),
                other: other.name.clone(),
            });
        }
        self.regions.push(NamedRegion {
            name: name.to_string(),
            start,
            end,
        });
        Ok(())
    }

    pub fn load(&mut self, name: &str, bytes: &[u8]) -> Result<(), NamedMapError> {
        let region = self
            .regions
            .iter()
            .find(|region| region.name == name)
            .ok_or_else(|| NamedMapError::UnknownRegion {
                name: name.to_string(),
            })?;
        let room = usize::from(region.end - region.start) + 1;
        if bytes.len() > room {
            return Err(NamedMapError::PastEnd {
                name: name.to_string(),
                length: bytes.len(),
                room,
            });
        }
        let start = usize::from(region.start);
        self.data[start..start + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    pub fn get(&self, addr: u16) -> u8 {
        self.data[usize::from(addr)]
    }
}

#[cfg(test)]
mod tests {
    use super::{IoDevice, MemoryBus, MemoryMapError, SystemBus};

    struct ConstantDevice(u8);

    impl IoDevice for ConstantDevice {
        fn read(&mut self, _offset: u16) -> u8 {
            self.0
        }

        fn write(&mut self, _offset: u16, value: u8) {
            self.0 = value;
        }
    }

    #[test]
    fn device_mapping_rejects_invalid_and_overlapping_ranges() {
        let mut bus = SystemBus::new();
        bus.try_map_device(0x1000, 0x10FF, ConstantDevice(0x42))
            .unwrap();

        assert_eq!(
            bus.try_map_device(0x1100, 0x10FF, ConstantDevice(0)),
            Err(MemoryMapError::InvalidRange {
                start: 0x1100,
                end: 0x10FF
            })
        );
        assert_eq!(
            bus.try_map_device(0x10FF, 0x1100, ConstantDevice(0)),
            Err(MemoryMapError::Overlap {
                start: 0x10FF,
                end: 0x1100,
                existing_start: 0x1000,
                existing_end: 0x10FF
            })
        );
        assert_eq!(bus.read(0x10FF), 0x42);
    }

    #[test]
    fn rom_mapping_rejects_address_space_wraparound() {
        let mut bus = SystemBus::new();
        bus.ram_mut()[0] = 0xA5;

        assert_eq!(
            bus.try_map_rom(0xFFFF, &[0x11, 0x22]),
            Err(MemoryMapError::RomOutOfRange {
                start: 0xFFFF,
                length: 2
            })
        );
        assert_eq!(bus.ram()[0], 0xA5);
        bus.try_map_rom(0xFFFF, &[0x11]).unwrap();
        assert_eq!(bus.ram()[0xFFFF], 0x11);
    }

    #[test]
    fn rom_and_device_mappings_reject_overlaps_in_either_order() {
        let mut bus = SystemBus::new();
        bus.try_map_device(0x2000, 0x200F, ConstantDevice(0))
            .unwrap();
        assert!(matches!(
            bus.try_map_rom(0x2008, &[0xAA; 4]),
            Err(MemoryMapError::Overlap { .. })
        ));

        let mut bus = SystemBus::new();
        bus.try_map_rom(0x3000, &[0xAA; 16]).unwrap();
        assert!(matches!(
            bus.try_map_device(0x300F, 0x3010, ConstantDevice(0)),
            Err(MemoryMapError::Overlap { .. })
        ));
    }
}
