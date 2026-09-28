//! A built AxROM image boots on the 2A03 and its code switches 32 KiB banks.

use mos6502::asm::assemble;
use mos6502::bus::MemoryBus;
use mos6502::ines::{InesError, MAGIC};
use mos6502::systems::nes2a03::Nes2A03;

fn bank(n: u8) -> Vec<u8> {
    // Every bank: reset at $8000 stores its own number in $00+n, then selects bank n+1.
    let src = format!(
        ".org $8000\nreset: lda #${n:02X}\n sta ${n:02X}\n lda #${:02X}\n sta $8000\n jmp reset\n.org $FFFC\n.word reset\n.word reset\n",
        (n + 1) % 4
    );
    let code = assemble(&src, 0x8000).unwrap();
    let mut b = vec![0; 0x8000];
    b[..code.len()].copy_from_slice(&code);
    b
}

#[test]
fn axrom_program_walks_all_four_banks() {
    let mut f = MAGIC.to_vec();
    f.extend([8, 0, 0x70, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for n in 0..4 {
        f.extend(bank(n));
    }
    let mut nes = Nes2A03::from_ines(&f).unwrap();
    assert_eq!(nes.cpu.pc, 0x8000);
    for n in 0..4u16 {
        nes.cpu.bus.write(n, 0xEE);
    }
    nes.run_cycles(200).unwrap();
    for n in 0..4u16 {
        assert_eq!(nes.cpu.bus.peek(n), n as u8, "bank {n} ran");
    }
    f[6] = 0x60; // mapper 6 (FFE): not supported (mapper 5 is MMC5 now)
    assert_eq!(
        Nes2A03::from_ines(&f).err(),
        Some(InesError::UnsupportedMapper(6))
    );
}
