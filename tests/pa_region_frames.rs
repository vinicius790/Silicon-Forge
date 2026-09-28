//! PAL (2C07/2A07) and Dendy timing, the 2C02G OAMADDR copy, repeated PPU reads
//! during DMC DMA, and the headless frame runner writing PPM files.

use mos6502::apu::{Apu, DMC_RATES, DMC_RATES_PAL, NOISE_PERIODS, NOISE_PERIODS_PAL};
use mos6502::bus::MemoryBus;
use mos6502::ines::MAGIC;
use mos6502::nes_state::NesStateError;
use mos6502::ppu::{Mirroring, Ppu, PpuMemory, Region, HEIGHT, WIDTH};
use mos6502::systems::nes2a03::Nes2A03;

fn tick(ppu: &mut Ppu, chr: &mut [u8]) {
    let mut m = PpuMemory {
        cart: None,
        chr,
        chr_writable: true,
        mirroring: Mirroring::Vertical,
    };
    ppu.tick(&mut m);
}

fn frame_dots(region: Region, mask: u8) -> Vec<u64> {
    let mut ppu = Ppu::new();
    ppu.set_region(region);
    ppu.mask = mask;
    let mut chr = vec![0u8; 0x2000];
    let mut out = Vec::new();
    for _ in 0..4 {
        let f = ppu.frame;
        let mut n = 0;
        while ppu.frame == f {
            tick(&mut ppu, &mut chr);
            n += 1;
        }
        out.push(n);
    }
    out
}

#[test]
fn frame_lengths_per_region() {
    assert_eq!(frame_dots(Region::Ntsc, 0), vec![89342; 4]);
    // Rendering on: odd NTSC frames skip one dot (frame 0 is even).
    assert_eq!(
        frame_dots(Region::Ntsc, 0x18),
        vec![89342, 89341, 89342, 89341]
    );
    for r in [Region::Pal, Region::Dendy] {
        for mask in [0, 0x18] {
            assert_eq!(frame_dots(r, mask), vec![312 * 341; 4], "{r:?}");
        }
    }
}

#[test]
fn vblank_flag_rises_and_falls_on_the_region_lines() {
    for (region, set, pre) in [
        (Region::Ntsc, 241u16, 261u16),
        (Region::Pal, 241, 311),
        (Region::Dendy, 291, 311),
    ] {
        let mut ppu = Ppu::new();
        ppu.set_region(region);
        let mut chr = vec![0u8; 0x2000];
        let mut edges = Vec::new();
        let mut last = false;
        while ppu.frame < 2 {
            let (l, d) = (ppu.line, ppu.dot);
            tick(&mut ppu, &mut chr);
            let now = ppu.status & 0x80 != 0;
            if now != last {
                edges.push((now, l, d));
            }
            last = now;
        }
        assert_eq!(
            edges,
            vec![
                (true, set, 1),
                (false, pre, 1),
                (true, set, 1),
                (false, pre, 1)
            ],
            "{region:?}"
        );
        assert_eq!(region.lines() - 1, pre);
    }
}

#[test]
fn cpu_ppu_clock_ratio_per_region() {
    for (region, dots, cycles) in [
        (Region::Ntsc, 3u64, 1u64),
        (Region::Pal, 16, 5),
        (Region::Dendy, 3, 1),
    ] {
        let mut nes = Nes2A03::from_source(".org $8000\nr: jmp r\n.org $FFFC\n.word r\n").unwrap();
        nes.set_region(region);
        let pos = |n: &Nes2A03| {
            let p = &n.cpu.bus.ppu;
            p.frame * u64::from(region.lines()) * 341 + u64::from(p.line) * 341 + u64::from(p.dot)
        };
        for k in 1..=40u64 {
            let before = pos(&nes);
            nes.run_cycles(cycles * k).unwrap();
            assert_eq!(pos(&nes) - before, dots * k, "{region:?} {k}");
        }
        // Partial PAL groups: dots are floor(16 n / 5) from a group boundary.
        if region == Region::Pal {
            for n in 1..5u64 {
                let before = pos(&nes);
                nes.run_cycles(n).unwrap();
                let after = pos(&nes);
                nes.run_cycles(5 - n).unwrap();
                assert_eq!(after - before, 16 * n / 5);
            }
        }
    }
    assert!((Region::Ntsc.cpu_hz() - 1_789_772.7).abs() < 1.0);
    assert!((Region::Pal.cpu_hz() - 1_662_607.0).abs() < 1.0);
    assert!((Region::Dendy.cpu_hz() - 1_773_447.5).abs() < 1.0);
}

#[test]
fn apu_frame_sequencer_tables() {
    for (pal, four_irq) in [(false, 29829u32), (true, 33253)] {
        let mut apu = Apu::new();
        apu.set_pal(pal);
        let mut irqs = Vec::new();
        for c in 1..=3 * (four_irq + 1) {
            apu.tick();
            if apu.read_status() & 0x40 != 0 {
                irqs.push(c);
            }
        }
        let p = four_irq + 1;
        assert_eq!(
            irqs,
            vec![four_irq, four_irq + p, four_irq + 2 * p],
            "pal {pal}"
        );
    }
}

#[test]
fn noise_and_dmc_tables_follow_the_region() {
    for pal in [false, true] {
        let (noise, dmc) = if pal {
            (NOISE_PERIODS_PAL, DMC_RATES_PAL)
        } else {
            (NOISE_PERIODS, DMC_RATES)
        };
        let mut apu = Apu::new();
        apu.set_pal(pal);
        for i in 0..16u8 {
            apu.write_register(0x0E, i);
            assert_eq!(apu.noise.timer_period, noise[usize::from(i)]);
        }
        for rate in 0..16u8 {
            let mut apu = Apu::new();
            apu.set_pal(pal);
            apu.write_register(0x10, 0x40 | rate);
            apu.write_register(0x13, 0);
            apu.write_register(0x15, 0x10);
            let mut refills = Vec::new();
            let mut c = 0u32;
            while refills.len() < 4 {
                if apu.dmc_fetch_address().is_some() {
                    apu.dmc_deliver(0x55);
                    refills.push(c);
                }
                apu.tick();
                c += 1;
            }
            let want = 8 * u32::from(dmc[usize::from(rate)]);
            assert_eq!(refills[3] - refills[2], want, "pal {pal} rate {rate}");
            assert_eq!(refills[2] - refills[1], want);
        }
    }
}

#[test]
fn pal_emphasis_swaps_red_and_green() {
    for (region, want) in [(Region::Ntsc, [1u8, 2, 4]), (Region::Pal, [2, 1, 4])] {
        for (bit, &w) in want.iter().enumerate() {
            let mut ppu = Ppu::new();
            ppu.set_region(region);
            ppu.mask = 0x20 << bit;
            let mut chr = vec![0u8; 0x2000];
            while ppu.line < 2 {
                tick(&mut ppu, &mut chr);
            }
            assert_eq!(ppu.emphasis[WIDTH + 5], w, "{region:?} bit {bit}");
        }
    }
}

fn header(timing: u8) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([2, 1, 0, 0x08, 0, 0, 0, 0, timing, 0, 0, 0]);
    f.extend(vec![0xEA; 0x8000 - 4]);
    f.extend([0x00, 0x80, 0x00, 0x80]);
    f.extend(vec![0; 0x2000]);
    f
}

#[test]
fn nes2_timing_byte_selects_the_region_and_states_must_match() {
    for (t, r) in [
        (0u8, Region::Ntsc),
        (1, Region::Pal),
        (2, Region::Ntsc),
        (3, Region::Dendy),
    ] {
        let nes = Nes2A03::from_ines(&header(t)).unwrap();
        assert_eq!(nes.region(), r, "timing {t}");
        assert_eq!(nes.cpu.bus.apu.pal, r == Region::Pal);
    }
    let mut pal = Nes2A03::from_ines(&header(1)).unwrap();
    pal.run_cycles(10_001).unwrap();
    let s = pal.save_state();
    let mut ntsc = Nes2A03::from_ines(&header(0)).unwrap();
    let before = ntsc.save_state();
    assert_eq!(
        ntsc.load_state(&s).err(),
        Some(NesStateError::RomMismatch("region"))
    );
    assert_eq!(ntsc.save_state(), before);
    let mut b = Nes2A03::from_ines(&header(1)).unwrap();
    b.load_state(&s).unwrap();
    pal.run_cycles(50_003).unwrap();
    b.run_cycles(50_003).unwrap();
    assert_eq!(
        pal.save_state(),
        b.save_state(),
        "PAL resumes mid divider group"
    );
    // Version 4 had no region: NTSC only.
    let v4 = ntsc.save_state_version(4);
    assert!(Nes2A03::from_ines(&header(0))
        .unwrap()
        .load_state(&v4)
        .is_ok());
    assert_eq!(
        Nes2A03::from_ines(&header(1))
            .unwrap()
            .load_state(&v4)
            .err(),
        Some(NesStateError::RomMismatch("region"))
    );
}

#[test]
fn oamaddr_at_least_8_copies_its_row_over_bytes_0_to_7() {
    for region in [Region::Ntsc, Region::Pal, Region::Dendy] {
        for addr in 0..=255u8 {
            let mut ppu = Ppu::new();
            ppu.set_region(region);
            for (i, b) in ppu.oam.iter_mut().enumerate() {
                *b = i as u8;
            }
            let original = ppu.oam;
            let mut chr = vec![0u8; 0x2000];
            while ppu.line != region.prerender() {
                tick(&mut ppu, &mut chr);
            }
            ppu.mask = 0x18;
            ppu.oam_addr = addr;
            tick(&mut ppu, &mut chr);
            tick(&mut ppu, &mut chr);
            let mut want = original;
            if addr >= 8 && region == Region::Ntsc {
                let from = usize::from(addr & 0xF8);
                want.copy_within(from..from + 8, 0);
            }
            assert_eq!(ppu.oam, want, "{region:?} {addr:02X}");
        }
    }
}

fn dmc_reader(delay: u8, reg: u16) -> String {
    format!(
        "
.org $8000
r: sei
 lda #$C0
 sta $4012
 lda #$FF
 sta $4013
 lda #$4F
 sta $4010
 lda #$10
 sta $4015
 ldy #${delay:02X}
w: dey
 bne w
 lda #$20
 sta $2006
 lda #$00
 sta $2006
 ldx #0
l: lda ${reg:04X}
 inx
 bne l
d: jmp d
.org $FFFC
.word r
"
    )
}

#[test]
fn dmc_halt_on_ppudata_repeats_the_read_and_skips_bytes() {
    let mut hit = 0;
    for delay in (1..=255u8).step_by(3) {
        let mut nes = Nes2A03::from_source(&dmc_reader(delay, 0x2007)).unwrap();
        nes.run_cycles(4000).unwrap();
        let rep = nes.cpu.bus.dmc_ppu_repeats;
        assert_eq!(
            u64::from(nes.cpu.bus.ppu.v),
            0x2000 + 256 + rep,
            "delay {delay}"
        );
        // Four halted read cycles, the last being the DMA's own fetch.
        assert_eq!(rep % 3, 0);
        hit += u32::from(rep > 0);
    }
    assert!(hit > 0);
}

#[test]
fn repeated_status_reads_clear_vblank_and_the_toggle() {
    let mut nes = Nes2A03::from_source(&dmc_reader(1, 0x2002)).unwrap();
    let bus = &mut nes.cpu.bus;
    bus.ppu.status = 0x80;
    bus.ppu.w = true;
    bus.dmc_repeats = 3;
    let v = bus.read(0x2002);
    assert_eq!(v & 0x80, 0, "the repeated read already took the flag");
    assert!(!bus.ppu.w);
    assert_eq!(bus.dmc_ppu_repeats, 3);
    bus.ppu.status = 0x80;
    bus.dmc_repeats = 3;
    let _ = bus.read(0x0000);
    assert_eq!(bus.read(0x2002) & 0x80, 0x80, "consumed by another read");
    bus.dmc_repeats = 2;
    let _ = bus.read(0x2004);
    assert_eq!(bus.dmc_ppu_repeats, 3, "only $2002 and $2007 repeat");
}

const SCREEN: &str = "
.org $8000
r: sei
v1: bit $2002
 bpl v1
 lda #$3F
 sta $2006
 lda #$00
 sta $2006
 lda #$21
 sta $2007
 lda #$00
 sta $2006
 sta $2006
 lda #$0A
 sta $2001
d: jmp d
.org $FFFC
.word r
";

#[test]
fn headless_runner_writes_ppm_frames() {
    let dir = std::env::temp_dir().join(format!("pa_frames_{}", std::process::id()));
    let mut nes = Nes2A03::from_source(SCREEN).unwrap();
    let paths = nes.render_frames_ppm(3, &dir, "shot").unwrap();
    assert_eq!(paths.len(), 3);
    assert_eq!(nes.cpu.bus.ppu.frame, 3);
    let head = format!("P6\n{WIDTH} {HEIGHT}\n255\n");
    for (i, p) in paths.iter().enumerate() {
        assert!(p.ends_with(format!("shot_{i:04}.ppm")));
        let data = std::fs::read(p).unwrap();
        assert_eq!(data.len(), head.len() + WIDTH * HEIGHT * 3);
        assert_eq!(&data[..head.len()], head.as_bytes());
    }
    let last = std::fs::read(&paths[2]).unwrap();
    let px = &last[head.len()..];
    assert!(px.chunks(3).all(|c| c == &px[..3]), "uniform backdrop");
    assert_eq!(&px[..3], &mos6502::ppu::ntsc_rgb(0x21, 0)[..]);
    assert_eq!(last, nes.frame_ppm());
    let mut again = Nes2A03::from_source(SCREEN).unwrap();
    again.run_frames(3).unwrap();
    assert_eq!(again.frame_ppm(), last, "deterministic");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn run_frames_counts_region_frames() {
    for (region, lo, hi) in [
        (Region::Ntsc, 29_780u64, 29_782u64),
        (Region::Pal, 33_247, 33_248),
        (Region::Dendy, 35_464, 35_464),
    ] {
        let mut nes = Nes2A03::from_source(".org $8000\nr: jmp r\n.org $FFFC\n.word r\n").unwrap();
        nes.set_region(region);
        nes.run_frames(1).unwrap();
        let c0 = nes.cpu.cycles;
        nes.run_frames(4).unwrap();
        let per = (nes.cpu.cycles - c0) / 4;
        assert!((lo..=hi).contains(&per), "{region:?} {per}");
    }
}
