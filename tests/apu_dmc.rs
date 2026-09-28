//! DMC: register decoding, output unit, sample fetch with stall, IRQ, loop, wrap.

use mos6502::apu::{Apu, DMC_RATES, DMC_STALL};
use mos6502::systems::nes2a03::Nes2A03;

fn ticks(apu: &mut Apu, n: u32) {
    for _ in 0..n {
        apu.tick();
    }
}

/// Rate 15 (54 cycles/bit), one byte at `$C000`, started.
fn playing(level: u8, flags: u8) -> Apu {
    let mut apu = Apu::new();
    apu.write_register(0x10, flags | 0x0F);
    apu.write_register(0x11, level);
    apu.write_register(0x12, 0x00);
    apu.write_register(0x13, 0x00);
    apu.write_register(0x15, 0x10);
    apu
}

#[test]
fn registers_decode_address_length_level_and_rate() {
    let mut apu = Apu::new();
    apu.write_register(0x11, 0xFF);
    apu.write_register(0x12, 0x01);
    apu.write_register(0x13, 0x01);
    apu.write_register(0x10, 0x4A);
    assert_eq!(
        (apu.dmc.level, apu.dmc.start, apu.dmc.length),
        (0x7F, 0xC040, 17)
    );
    assert_eq!(
        (apu.dmc.rate, apu.dmc.looping, apu.dmc.irq_enabled),
        (0x0A, true, false)
    );
    assert_eq!(DMC_RATES[0x0A], 142);
    assert_eq!(apu.dmc_fetch_address(), None, "not started");
    apu.write_register(0x15, 0x10);
    assert_eq!(
        (apu.dmc_fetch_address(), apu.peek_status() & 0x10),
        (Some(0xC040), 0x10)
    );
    apu.write_register(0x15, 0x00);
    assert_eq!(
        (apu.dmc_fetch_address(), apu.peek_status() & 0x10),
        (None, 0)
    );
}

#[test]
fn output_unit_moves_two_per_bit_and_clamps() {
    for (byte, start, end) in [
        (0xFF, 64, 80),
        (0x00, 64, 48),
        (0x0F, 64, 64),
        (0xFF, 120, 126),
    ] {
        let mut apu = playing(start, 0);
        apu.dmc_deliver(byte);
        ticks(&mut apu, 1);
        assert_eq!(apu.dmc.level, start, "the first expiry only loads the byte");
        ticks(&mut apu, 54 * 8);
        assert_eq!(apu.dmc.level, end, "{byte:02X} from {start}");
        assert_eq!(apu.channel_outputs()[4], end);
    }
    let mut low = playing(1, 0);
    low.dmc_deliver(0x00);
    ticks(&mut low, 1 + 54 * 8);
    assert_eq!(low.dmc.level, 1, "never below 0");
}

#[test]
fn empty_buffer_silences_the_next_byte() {
    let mut apu = playing(64, 0);
    ticks(&mut apu, 1 + 54 * 16);
    assert_eq!(apu.dmc.level, 64, "nothing delivered: silent, level holds");
}

#[test]
fn last_byte_raises_irq_unless_looping_and_4015_clears_it() {
    let mut apu = playing(0, 0x80);
    apu.dmc_deliver(0x12);
    assert!(apu.irq_asserted());
    assert_eq!(apu.peek_status() & 0x90, 0x80);
    apu.dmc_deliver(0x34);
    assert_eq!(apu.dmc.buffer, Some(0x12), "nothing due: ignored");
    apu.write_register(0x15, 0x00);
    assert!(!apu.irq_asserted());

    let mut off = playing(0, 0x80);
    off.dmc_deliver(1);
    off.write_register(0x10, 0x0F);
    assert!(!off.irq_asserted(), "clearing bit 7 clears the flag");

    let mut looping = playing(0, 0xC0);
    looping.dmc_deliver(1);
    assert!(!looping.irq_asserted());
    assert_eq!((looping.dmc.remaining, looping.dmc.address), (1, 0xC000));
}

#[test]
fn address_wraps_from_ffff_to_8000() {
    let mut apu = Apu::new();
    apu.write_register(0x12, 0xFF);
    apu.write_register(0x13, 0x04);
    apu.write_register(0x15, 0x10);
    for _ in 0..64 {
        apu.dmc_deliver(0);
        apu.dmc.buffer = None;
    }
    assert_eq!(
        (apu.dmc_fetch_address(), apu.dmc.remaining),
        (Some(0x8000), 1)
    );
}

#[test]
fn machine_fetches_through_the_bus_stalls_and_interrupts() {
    let mut nes = Nes2A03::from_source(
        "
.org $8000
    sei
    lda #$10
    sta $4012
    lda #$00
    sta $4013
    lda #$8F
    sta $4010
    lda #$10
    sta $4015
idle:
    jmp idle
.org $C400
    .byte $FF
.org $FFFC
    .word $8000
",
    )
    .unwrap();
    // Power-up rate 0 (428) runs until 429, then rate 15: silent byte to 753, data to 1185.
    nes.run_cycles(1300).unwrap();
    assert_eq!(nes.stalled, u64::from(DMC_STALL), "one byte, one stall");
    assert_eq!(
        nes.cpu.bus.apu.dmc.level, 16,
        "$FF from $C400: eight +2 steps"
    );
    assert!(nes.cpu.bus.apu.irq_asserted());
    assert_eq!(nes.cpu.bus.apu.peek_status() & 0x90, 0x80);
}

#[test]
fn a_program_without_a_reset_vector_is_rejected() {
    let err = Nes2A03::from_source(".org $8000\nnop\n").err().unwrap();
    assert!(err.contains("vetor de reset $0000"), "{err}");
}
