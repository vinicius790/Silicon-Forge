//! Whole-NES save states: a state taken at any CPU cycle, loaded into a freshly
//! booted machine, continues to exactly the same final state (CPU, RAM, PPU with
//! framebuffer, APU with DMC, mapper with IRQ) for every supported mapper.

use mos6502::ines::MAGIC;
use mos6502::nes_state::{NesStateError, NES_STATE_VERSION};
use mos6502::systems::nes2a03::Nes2A03;

const SRC: &str = "
.org $E000
reset: sei
 cld
 ldx #$FF
 txs
 lda #$00
 sta $4017
 lda #$BF
 sta $4000
 lda #$89
 sta $4001
 lda #$40
 sta $4002
 lda #$02
 sta $4003
 lda #$81
 sta $4008
 lda #$20
 sta $400A
 lda #$01
 sta $400B
 lda #$3F
 sta $400C
 lda #$85
 sta $400E
 lda #$08
 sta $400F
 lda #$CF
 sta $4010
 lda #$40
 sta $4011
 lda #$00
 sta $4012
 lda #$04
 sta $4013
 lda #$1F
 sta $4015
 lda #$10
 sta $C000
 sta $C001
 sta $E001
 lda #$20
 sta $2006
 lda #$00
 sta $2006
 ldx #$40
fill: stx $2007
 dex
 bne fill
 lda #$00
 sta $2006
 sta $2006
 ldx #$20
chr: txa
 sta $2007
 dex
 bne chr
 lda #$80
 sta $2000
 lda #$1E
 sta $2001
 cli
main: inc $00
 lda $00
 sta $2005
 bit $2002
 jmp main
nmi: pha
 lda #$02
 sta $4014
 inc $01
 inc $0200
 pla
 rti
irq: pha
 lda $4015
 sta $02
 sta $E000
 lda #$1F
 sta $4015
 pla
 rti
.org $FFFA
.word nmi, reset, irq
";

fn rom(mapper: u8, chr_banks: u8) -> Vec<u8> {
    let img = mos6502::asm::assemble_with_symbols(SRC, 0x8000).unwrap();
    let mut prg: Vec<u8> = (0..0x8000).map(|i| (i * 7 + 3) as u8).collect();
    let at = usize::from(img.origin - 0x8000);
    prg[at..at + img.bytes.len()].copy_from_slice(&img.bytes);
    let mut f = MAGIC.to_vec();
    f.extend([
        2,
        chr_banks,
        (mapper << 4) | 1,
        mapper & 0xF0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ]);
    f.extend(prg);
    f.extend((0..usize::from(chr_banks) * 0x2000).map(|i| (i * 13) as u8));
    f
}

const MAPPERS: [(u8, u8); 7] = [(0, 0), (1, 0), (2, 0), (3, 4), (4, 4), (7, 0), (66, 4)];
const TOTAL: u64 = 75_000;

#[test]
fn every_mapper_resumes_identically_from_many_cut_points() {
    for (mapper, chr) in MAPPERS {
        let file = rom(mapper, chr);
        let mut reference = Nes2A03::from_ines(&file).unwrap();
        reference.run_cycles(TOTAL).unwrap();
        let want = reference.save_state();
        assert!(reference.cpu.bus.ppu.frame >= 2, "rendered");
        let mut cuts: Vec<u64> = (0..24).map(|k| k * 2900 + k * k * 7).collect();
        cuts.extend([1, 2, 3, 7, 29_780, 29_781, 29_782, TOTAL - 1, TOTAL]);
        for cut in cuts {
            let mut a = Nes2A03::from_ines(&file).unwrap();
            a.run_cycles(cut).unwrap();
            let state = a.save_state();
            let mut b = Nes2A03::from_ines(&file).unwrap();
            b.run_cycles(1234).unwrap(); // a different, non-reset starting point
            b.load_state(&state).unwrap();
            assert_eq!(
                b.save_state(),
                state,
                "mapper {mapper} cut {cut}: reload is exact"
            );
            b.run_cycles(TOTAL - cut).unwrap();
            assert_eq!(b.save_state(), want, "mapper {mapper} cut {cut}");
            assert_eq!(b.cpu.bus.ppu.framebuffer, reference.cpu.bus.ppu.framebuffer);
        }
    }
}

#[test]
fn version_1_loads_and_the_next_frame_rewrites_the_screen() {
    let file = rom(4, 4);
    let mut a = Nes2A03::from_ines(&file).unwrap();
    a.run_cycles(40_000).unwrap();
    let v1 = a.save_state_version(1);
    assert_eq!(u16::from_le_bytes([v1[4], v1[5]]), 1);
    assert!(v1.len() < a.save_state().len());
    let mut b = Nes2A03::from_ines(&file).unwrap();
    b.load_state(&v1).unwrap();
    assert!(b.cpu.bus.ppu.framebuffer.iter().all(|&p| p == 0));
    a.run_cycles(35_000).unwrap();
    b.run_cycles(35_000).unwrap();
    assert_eq!(
        b.save_state(),
        a.save_state(),
        "identical after a full frame"
    );
}

#[test]
fn typed_errors() {
    let file = rom(4, 4);
    let mut a = Nes2A03::from_ines(&file).unwrap();
    a.run_cycles(5_000).unwrap();
    let good = a.save_state();
    let mut b = Nes2A03::from_ines(&file).unwrap();
    assert_eq!(b.load_state(b"NESX\x02\x00"), Err(NesStateError::BadMagic));
    assert_eq!(b.load_state(b"NES"), Err(NesStateError::BadMagic));
    for v in [0u16, NES_STATE_VERSION + 1, 0xFFFF] {
        let mut s = good.clone();
        s[4..6].copy_from_slice(&v.to_le_bytes());
        assert_eq!(b.load_state(&s), Err(NesStateError::UnsupportedVersion(v)));
    }
    for cut in [7, 20, 100, 3000, good.len() - 1] {
        assert!(
            matches!(
                b.load_state(&good[..cut]),
                Err(NesStateError::Truncated { .. })
            ),
            "{cut}"
        );
    }
    let mut long = good.clone();
    long.extend([0, 0]);
    assert_eq!(b.load_state(&long), Err(NesStateError::Trailing(2)));
    let mut bad_bool = good.clone();
    bad_bool[6 + 5 + 2 + 8 + 1] = 2; // halt_on_brk
    assert!(matches!(
        b.load_state(&bad_bool),
        Err(NesStateError::BadValue {
            what: "halt_on_brk",
            value: 2
        })
    ));
    let mut variant = good.clone();
    variant[6 + 5 + 2 + 8] = 0;
    assert!(matches!(
        b.load_state(&variant),
        Err(NesStateError::BadValue {
            what: "variant",
            ..
        })
    ));
    // Other ROM, other mapper, CHR RAM vs ROM.
    let mut other = rom(4, 4);
    other[16 + 0x10] ^= 1;
    let mut c = Nes2A03::from_ines(&other).unwrap();
    assert_eq!(
        c.load_state(&good),
        Err(NesStateError::RomMismatch("PRG ROM"))
    );
    let mut c = Nes2A03::from_ines(&rom(3, 4)).unwrap();
    assert_eq!(
        c.load_state(&good),
        Err(NesStateError::RomMismatch("mapper"))
    );
    let mut c = Nes2A03::from_ines(&rom(0, 0)).unwrap();
    assert_eq!(
        c.load_state(&good),
        Err(NesStateError::RomMismatch("CHR kind"))
    );
    let mut chr = rom(4, 4);
    let n = chr.len();
    chr[n - 1] ^= 1;
    let mut c = Nes2A03::from_ines(&chr).unwrap();
    assert_eq!(
        c.load_state(&good),
        Err(NesStateError::RomMismatch("CHR ROM"))
    );
    for e in [
        NesStateError::BadMagic,
        NesStateError::Trailing(3),
        NesStateError::RomMismatch("x"),
    ] {
        assert!(!e.to_string().is_empty());
    }
}

#[test]
fn machine_without_cartridge_round_trips() {
    let src = SRC;
    let mut a = Nes2A03::from_source(src).unwrap();
    let mut r = Nes2A03::from_source(src).unwrap();
    r.run_cycles(50_000).unwrap();
    for cut in [0u64, 1, 999, 12_345, 33_333, 49_999] {
        a = Nes2A03::from_source(src).unwrap();
        a.run_cycles(cut).unwrap();
        let s = a.save_state();
        let mut b = Nes2A03::from_source(src).unwrap();
        b.load_state(&s).unwrap();
        b.run_cycles(50_000 - cut).unwrap();
        assert_eq!(b.save_state(), r.save_state(), "{cut}");
    }
    let mut cart = Nes2A03::from_ines(&rom(0, 0)).unwrap();
    assert_eq!(
        cart.load_state(&a.save_state()),
        Err(NesStateError::RomMismatch("cartridge"))
    );
}

#[test]
fn failed_loads_leave_the_machine_unchanged() {
    let file = rom(4, 4);
    let mut src = Nes2A03::from_ines(&file).unwrap();
    src.run_cycles(31_000).unwrap();
    let good = src.save_state();
    let mut m = Nes2A03::from_ines(&file).unwrap();
    m.run_cycles(12_345).unwrap();
    let before = m.save_state();
    let fb = m.cpu.bus.ppu.framebuffer.clone();
    // Corrupt every region: truncations at many offsets, a bad trailing byte, and a
    // flipped byte late in the stream (after RAM, PPU and APU were written).
    let mut bad: Vec<Vec<u8>> = (6..good.len())
        .step_by(997)
        .map(|n| good[..n].to_vec())
        .collect();
    let mut t = good.clone();
    t.push(0);
    bad.push(t);
    let mut wrong_mapper = good.clone();
    let n = wrong_mapper.len();
    // The MMC3 IRQ-pending flag sits near the end; 7 is not a bool.
    wrong_mapper[n - 14] = 7;
    bad.push(wrong_mapper);
    for (i, b) in bad.iter().enumerate() {
        assert!(m.load_state(b).is_err(), "{i}");
        assert_eq!(m.save_state(), before, "case {i}");
        assert_eq!(m.cpu.bus.ppu.framebuffer, fb);
    }
    // And it still runs exactly like an untouched twin.
    let mut twin = Nes2A03::from_ines(&file).unwrap();
    twin.run_cycles(12_345).unwrap();
    m.run_cycles(20_000).unwrap();
    twin.run_cycles(20_000).unwrap();
    assert_eq!(m.save_state(), twin.save_state());
}
