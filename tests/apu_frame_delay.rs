//! `$4017` restart delay: 3 cycles after a write on an even cycle, 4 on an odd one,
//! checked through the first quarter frame and the frame IRQ at every write offset.

use mos6502::apu::Apu;

fn after(offset: u32, value: u8) -> Apu {
    let mut apu = Apu::new();
    apu.write_register(0x17, 0x40); // quiet start
    for _ in 0..offset {
        apu.tick();
    }
    apu.write_register(0x17, value);
    apu
}

#[test]
fn delay_alternates_with_cycle_parity() {
    for offset in 0..64u32 {
        let apu = after(offset, 0x00);
        assert_eq!(
            apu.frame_reset_pending(),
            if offset % 2 == 0 { 3 } else { 4 },
            "{offset}"
        );
    }
}

#[test]
fn frame_irq_lands_29829_cycles_after_the_restart() {
    for offset in 0..16u32 {
        let mut apu = after(offset, 0x00);
        let delay = u32::from(apu.frame_reset_pending());
        let mut n = 0u32;
        while !apu.irq_asserted() {
            apu.tick();
            n += 1;
            assert!(n < 40_000);
        }
        assert_eq!(n, delay + 29829, "offset {offset}");
    }
}

#[test]
fn inhibit_and_mode_apply_at_once_and_a_rewrite_restarts_the_delay() {
    let mut apu = Apu::new();
    for _ in 0..29_830 {
        apu.tick();
    }
    assert!(apu.irq_asserted());
    apu.write_register(0x17, 0x40);
    assert!(!apu.irq_asserted(), "inhibit clears immediately");
    let mut apu = after(10, 0x00);
    apu.tick();
    apu.write_register(0x17, 0x00);
    assert_eq!(apu.frame_reset_pending(), 4, "rewritten on an odd cycle");
    // Length counter half-frame in 5-step comes only at the restart.
    let mut apu = Apu::new();
    apu.write_register(0x15, 0x01);
    apu.write_register(0x03, 0x18); // length 2
    apu.write_register(0x17, 0x80);
    for k in 0..3 {
        assert_eq!(apu.pulse[0].length.value, 2, "tick {k}");
        apu.tick();
    }
    assert_eq!(apu.pulse[0].length.value, 1);
}
