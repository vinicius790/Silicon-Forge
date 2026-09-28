//! Whole-machine behaviour of the PPU/input work: the DMC DMA / controller read
//! conflict on real 6502 code, and save states (version 4) taken anywhere in a
//! rendered frame, with version 3 still loading.

use mos6502::bus::MemoryBus;
use mos6502::nes_state::NES_STATE_VERSION;
use mos6502::systems::nes2a03::{Nes2A03, OPEN_BUS_4016};

fn reader(delay: u8) -> String {
    format!(
        "
.org $8000
reset: sei
 lda #$C0
 sta $4012
 lda #$FF
 sta $4013
 lda #$4F
 sta $4010
 lda #$10
 sta $4015
 ldy #${delay:02X}
wait: dey
 bne wait
 lda #1
 sta $4016
 lda #0
 sta $4016
 ldx #0
read: lda $4016
 sta $0300,x
 inx
 cpx #8
 bne read
done: jmp done
.org $FFFC
.word reset
"
    )
}

#[test]
fn dmc_dma_during_a_controller_read_deletes_one_bit() {
    let buttons = 0b1011_0010u8;
    let mut hit = 0;
    for delay in 1..=255u8 {
        let mut nes = Nes2A03::from_source(&reader(delay)).unwrap();
        nes.cpu.bus.pads[0].buttons = buttons;
        nes.run_cycles(2200).unwrap();
        let lost = nes.cpu.bus.dmc_conflicts;
        assert!(lost <= 2, "delay {delay}: {lost}");
        let got: Vec<u8> = (0..8)
            .map(|i| nes.cpu.bus.peek(0x0300 + i) ^ OPEN_BUS_4016)
            .collect();
        // Each stored bit is button bit i + (bits deleted so far): a subsequence of
        // the report with exactly `lost` deletions, the tail filled with 1s.
        let report: Vec<u8> = (0..16)
            .map(|i| if i < 8 { (buttons >> i) & 1 } else { 1 })
            .collect();
        let mut skipped = 0usize;
        for (i, &g) in got.iter().enumerate() {
            while report[i + skipped] != g {
                skipped += 1;
                assert!(skipped as u64 <= lost, "delay {delay}: {got:?}");
            }
        }
        if lost > 0 {
            hit += 1;
            // The pad was clocked 8 + lost times.
            let clocks = 8 + lost as usize;
            let shift = nes.cpu.bus.pads[0].shift;
            let want = (0..8)
                .map(|b| report.get(b + clocks).copied().unwrap_or(1) << b)
                .fold(0u8, |a, b| a | b);
            assert_eq!(shift, want, "delay {delay}");
        }
    }
    assert!(hit > 0, "the sweep crosses DMC fetches");
}

#[test]
fn halted_read_flag_semantics() {
    let mut nes = Nes2A03::from_source(&reader(1)).unwrap();
    let bus = &mut nes.cpu.bus;
    bus.pads[0].buttons = 0b10;
    bus.pads[0].latch();
    bus.dmc_halted_read = true;
    assert_eq!(bus.read(0x4016) & 1, 1, "bit 0 deleted, bit 1 returned");
    assert_eq!(bus.dmc_conflicts, 1);
    bus.pads[0].latch();
    bus.dmc_halted_read = true;
    let _ = bus.read(0x0000);
    assert_eq!(bus.read(0x4016) & 1, 0, "another read consumed the halt");
    bus.strobe = true;
    bus.dmc_halted_read = true;
    assert_eq!(
        bus.read(0x4016) & 1,
        0,
        "strobe high: reloads, nothing lost"
    );
    assert_eq!(bus.dmc_conflicts, 1);
}

const RENDER: &str = "
.org $8000
reset: sei
 ldx #0
fill: txa
 sta $0200,x
 inx
 bne fill
vb: bit $2002
 bpl vb
 lda #$00
 sta $2003
 lda #$02
 sta $4014
 lda #$08
 sta $2000
 lda #$1E
 sta $2001
loop: inc $10
 lda $10
 sta $2004
 and #$07
 bne loop
 lda #$18
 sta $2001
 lda #$1E
 sta $2001
 jmp loop
.org $FFFC
.word reset
";

#[test]
fn version_4_states_resume_identically_mid_evaluation() {
    let mut base = Nes2A03::from_source(RENDER).unwrap();
    base.run_cycles(40_000).unwrap();
    for step in 0..40u64 {
        base.run_cycles(97 + step * 13).unwrap();
        let s = base.save_state();
        assert_eq!(u16::from_le_bytes([s[4], s[5]]), NES_STATE_VERSION);
        let mut b = Nes2A03::from_source(RENDER).unwrap();
        b.load_state(&s).unwrap();
        let mut a = Nes2A03::from_source(RENDER).unwrap();
        a.load_state(&s).unwrap();
        a.run_cycles(3000).unwrap();
        b.run_cycles(1000).unwrap();
        b.run_cycles(2000).unwrap();
        assert_eq!(a.save_state(), b.save_state(), "step {step}");
        assert_eq!(a.cpu.bus.ppu.framebuffer, b.cpu.bus.ppu.framebuffer);
    }
}

#[test]
fn version_3_states_still_load_with_evaluation_idle() {
    let mut a = Nes2A03::from_source(RENDER).unwrap();
    a.run_cycles(60_000).unwrap();
    let v3 = a.save_state_version(3);
    let mut b = Nes2A03::from_source(RENDER).unwrap();
    b.load_state(&v3).unwrap();
    assert_eq!(b.cpu.bus.ppu.corrupt_rows, 0);
    assert!(!b.cpu.bus.dmc_halted_read);
    assert_eq!(v3.len() + 10, a.save_state_version(4).len());
    b.run_cycles(30_000).unwrap();
}

#[test]
fn bad_evaluation_fields_are_refused_atomically() {
    let mut a = Nes2A03::from_source(RENDER).unwrap();
    a.run_cycles(50_000).unwrap();
    let good = a.save_state();
    let mut tried = 0;
    for i in (6..good.len()).rev().take(400) {
        let mut bad = good.clone();
        bad[i] ^= 0xFF;
        if a.load_state(&bad).is_err() {
            assert_eq!(a.save_state(), good, "byte {i}");
            tried += 1;
        } else {
            a.load_state(&good).unwrap();
        }
    }
    assert!(tried > 0);
}
