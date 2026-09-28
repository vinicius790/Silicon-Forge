//! SystemBus memory map and FlatMemory bounds.

use mos6502::{FlatMemory, IoDevice, MemoryBus, MemoryMapError, Region, RegionKind, SystemBus};

struct Echo;
impl IoDevice for Echo {
    fn read(&mut self, offset: u16) -> u8 {
        offset as u8
    }
    fn write(&mut self, _: u16, _: u8) {}
}

#[test]
fn map_partitions_the_address_space_and_matches_reads() {
    let mut bus = SystemBus::new();
    assert_eq!(
        bus.memory_map(),
        [Region {
            start: 0,
            end: 0xFFFF,
            kind: RegionKind::Ram
        }]
    );
    bus.map_rom(0xF000, &[0xAA; 0x1000]);
    bus.map_device(0x6000, 0x600F, Echo);
    bus.map_rom(0x0000, &[0x11]);
    let map = bus.memory_map();
    let kinds: Vec<_> = map.iter().map(|r| (r.start, r.end, r.kind)).collect();
    assert_eq!(
        kinds,
        [
            (0x0000, 0x0000, RegionKind::Rom),
            (0x0001, 0x5FFF, RegionKind::Ram),
            (0x6000, 0x600F, RegionKind::Device),
            (0x6010, 0xEFFF, RegionKind::Ram),
            (0xF000, 0xFFFF, RegionKind::Rom),
        ]
    );
    // Every address: ROM ignores writes, RAM keeps them, the device answers its offset.
    for addr in 0..=0xFFFFu16 {
        let kind = map
            .iter()
            .find(|r| (r.start..=r.end).contains(&addr))
            .unwrap()
            .kind;
        let before = bus.peek(addr);
        bus.write(addr, 0x5C);
        let after = bus.read(addr);
        match kind {
            RegionKind::Rom => assert_eq!(after, before, "{addr:04X}"),
            RegionKind::Ram => assert_eq!(after, 0x5C, "{addr:04X}"),
            RegionKind::Device => assert_eq!(after, (addr - 0x6000) as u8),
        }
    }
}

#[test]
fn flat_memory_try_load_rejects_wrap_and_accepts_the_last_byte() {
    let mut mem = FlatMemory::new();
    assert!(mem.try_load_at(0xFFFF, &[1]).is_ok());
    assert_eq!(
        mem.try_load_at(0xFFFF, &[1, 2]),
        Err(MemoryMapError::RomOutOfRange {
            start: 0xFFFF,
            length: 2
        })
    );
    assert_eq!(mem.data[0], 0, "nothing written on error");
    mem.load_at(0xFFFF, &[3, 4]);
    assert_eq!(
        (mem.data[0xFFFF], mem.data[0]),
        (3, 4),
        "load_at wraps by design"
    );
}
