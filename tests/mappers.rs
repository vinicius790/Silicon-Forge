//! NROM, UxROM and MMC1 PRG banking; a UxROM program boots on the 2A03 machine.

use mos6502::assemble;
use mos6502::mapper::{Cartridge, MapperError, Mmc1, Nrom, Uxrom};
use mos6502::systems::nes2a03::Nes2A03;

/// `banks` × 16 KiB, every byte of bank `n` equal to `n | 0x40`.
fn marked(banks: usize) -> Vec<u8> {
    (0..banks * 0x4000)
        .map(|i| (i / 0x4000) as u8 | 0x40)
        .collect()
}

#[test]
fn sizes_are_validated() {
    assert_eq!(
        Nrom::new(vec![0; 0x2000], false).err(),
        Some(MapperError::BadPrgSize(0x2000))
    );
    assert!(Nrom::new(vec![0; 0xC000], false).is_err());
    assert_eq!(
        Uxrom::new(vec![0; 0x5000]).err(),
        Some(MapperError::BadPrgSize(0x5000))
    );
    assert_eq!(
        Mmc1::new(marked(17)).err(),
        Some(MapperError::TooManyBanks(17))
    );
    assert!(Uxrom::new(Vec::new()).is_err());
}

#[test]
fn nrom_mirrors_sixteen_kib_and_keeps_prg_ram() {
    let mut prg = vec![0u8; 0x4000];
    prg[0] = 0xAB;
    let mut cart = Nrom::new(prg, true).unwrap();
    assert_eq!((cart.peek(0x8000), cart.peek(0xC000)), (0xAB, 0xAB));
    cart.write(0x8000, 0x00);
    assert_eq!(cart.peek(0x8000), 0xAB, "ROM ignores writes");
    cart.write(0x6123, 0x5A);
    assert_eq!(cart.peek(0x6123), 0x5A);
    let mut bare = Nrom::new(marked(2), false).unwrap();
    bare.write(0x6000, 1);
    assert_eq!((bare.peek(0x6000), bare.peek(0xC000)), (0, 0x41));
}

#[test]
fn uxrom_switches_low_bank_with_bus_conflicts_and_fixes_the_last() {
    let mut prg = marked(4);
    prg[0x4000 * 3 + 0x10] = 0x02; // byte at $C010 in the fixed bank
    let mut cart = Uxrom::new(prg).unwrap();
    assert_eq!((cart.peek(0x8000), cart.peek(0xC000)), (0x40, 0x43));
    cart.write(0xC010, 0x03);
    assert_eq!(cart.bank, 2, "3 AND ROM 2 = 2");
    assert_eq!(cart.peek(0x8000), 0x42);
    cart.write(0x8000, 0x0F); // ROM byte 0x42: 0x0F & 0x42 = 2
    assert_eq!(cart.bank, 2);
    assert_eq!(cart.peek(0xFFFF), 0x43);
}

fn serial(cart: &mut Mmc1, addr: u16, value: u8) {
    for bit in 0..5 {
        cart.write(addr, (value >> bit) & 1);
    }
}

#[test]
fn mmc1_serial_port_modes_reset_and_prg_ram_enable() {
    let mut cart = Mmc1::new(marked(8)).unwrap();
    assert_eq!(
        (cart.bank_at(false), cart.bank_at(true)),
        (0, 7),
        "power-up mode 3"
    );
    serial(&mut cart, 0xE000, 0x05);
    assert_eq!((cart.peek(0x8000), cart.peek(0xC000)), (0x45, 0x47));
    serial(&mut cart, 0x8000, 0x08); // mode 2
    assert_eq!((cart.bank_at(false), cart.bank_at(true)), (0, 5));
    serial(&mut cart, 0x8000, 0x00); // 32 KiB
    assert_eq!((cart.bank_at(false), cart.bank_at(true)), (4, 5));
    cart.write(0x8000, 1);
    cart.write(0x8000, 1);
    cart.write(0x8000, 0x80); // reset mid-sequence
    assert_eq!(cart.control & 0x0C, 0x0C);
    serial(&mut cart, 0xA000, 0x1F);
    assert_eq!(cart.chr[0], 0x1F, "partial write discarded, next five land");
    cart.write(0x6000, 0x99);
    assert_eq!(cart.peek(0x6000), 0x99);
    serial(&mut cart, 0xE000, 0x15); // bit 4: RAM off
    cart.write(0x6000, 0x11);
    assert_eq!(cart.peek(0x6000), 0);
    serial(&mut cart, 0xE000, 0x05);
    assert_eq!(cart.peek(0x6000), 0x99);
}

#[test]
fn uxrom_program_on_the_2a03_reads_a_switched_bank() {
    let code = assemble(
        "
.org $C000
reset:
    lda #$02
    sta select+2
    lda $8000
    sta $00
    lda #$01
    sta select+1
    lda $8000
    sta $01
idle:
    jmp idle
select:
    .byte 0, 1, 2, 3
.org $FFFC
    .word reset
",
        0xC000,
    )
    .unwrap();
    let mut prg = marked(4);
    prg[0xC000..0xC000 + code.len()].copy_from_slice(&code);
    let mut nes = Nes2A03::with_cartridge(Box::new(Uxrom::new(prg).unwrap()));
    nes.run_cycles(200).unwrap();
    assert_eq!(nes.cpu.bus.ram[..2], [0x42, 0x41]);
}

#[test]
fn forward_label_with_offset_keeps_one_size_in_both_passes() {
    let code = assemble("sta t+1\nnop\nt: .byte 7, 8\n", 0xC000).unwrap();
    assert_eq!(code, [0x8D, 0x05, 0xC0, 0xEA, 7, 8]);
}

#[test]
fn mmc1_ignores_a_write_on_the_next_cycle() {
    let mut cart = Mmc1::new(marked(8)).unwrap();
    cart.write(0xE000, 1);
    cart.clock();
    cart.write(0xE000, 1); // next cycle: dropped
    for _ in 0..4 {
        cart.clock();
        cart.clock();
        cart.write(0xE000, 0);
    }
    assert_eq!(
        cart.prg_bank, 0x01,
        "five spaced writes, the adjacent one ignored"
    );
}

#[test]
fn rmw_double_write_counts_once_on_the_machine() {
    // INC $E000 reads $00 (padding of the assembled fixed bank) and writes $00 then $01
    // on consecutive cycles. If both landed the shift register would take 0,1,0,1,0
    // (bank 10 % 8 = 2); with the second ignored it takes five 0s: bank 0.
    let code = assemble(
        "
.org $C000
reset:
    lda #$80
    sta $8000
    ldx #5
loop:
    nop
    inc $E000
    dex
    bne loop
    lda $8000
    sta $00
idle:
    jmp idle
.org $FFFC
    .word reset
",
        0xC000,
    )
    .unwrap();
    let mut prg = marked(8);
    prg[0x4000 * 7..0x4000 * 7 + code.len()].copy_from_slice(&code);
    let mut nes = Nes2A03::with_cartridge(Box::new(Mmc1::new(prg).unwrap()));
    nes.run_cycles(300).unwrap();
    assert_eq!(nes.cpu.bus.ram[0], 0x40, "bank 0 at $8000, not bank 2");
}
