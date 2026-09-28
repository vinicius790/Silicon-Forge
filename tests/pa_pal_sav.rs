//! 2C07 forced OAM refresh, the PAL palette decoder, and battery RAM (`.sav`)
//! persistence across every board that has PRG RAM.

use mos6502::ines::MAGIC;
use mos6502::mapper::SaveRamError;
use mos6502::ppu::{ntsc_rgb, pal_rgb, Mirroring, Ppu, PpuMemory, Region, PAL_HUE_SHIFT_DEG};
use mos6502::systems::nes2a03::Nes2A03;

fn with_mem<R>(f: impl FnOnce(&mut PpuMemory) -> R) -> R {
    let mut chr = vec![0u8; 0x2000];
    let mut m = PpuMemory {
        cart: None,
        chr: &mut chr,
        chr_writable: true,
        mirroring: Mirroring::Vertical,
    };
    f(&mut m)
}

#[test]
fn pal_oam_refresh_window() {
    for region in [Region::Ntsc, Region::Pal, Region::Dendy] {
        for rendering in [false, true] {
            for line in [240u16, 241, 250, 264, 265, 266, 290, 309, 310] {
                if line >= region.lines() {
                    continue;
                }
                let mut p = Ppu::new();
                p.set_region(region);
                p.mask = if rendering { 0x18 } else { 0 };
                p.line = line;
                p.dot = 100;
                let refresh =
                    region == Region::Pal && rendering && line >= 265 && line < region.prerender();
                assert_eq!(p.oam_refresh(), refresh, "{region:?} {rendering} {line}");
                p.oam_addr = 0x11;
                with_mem(|m| p.write_register(m, 0x2004, 0x5A));
                if refresh {
                    assert_eq!((p.oam[0x11], p.oam_addr), (0, 0x15), "dropped, +4");
                } else if line < 240 || line == region.prerender() {
                    unreachable!();
                } else {
                    assert_eq!((p.oam[0x11], p.oam_addr), (0x5A, 0x12), "{region:?} {line}");
                }
                let mut page = [0u8; 256];
                for (i, b) in page.iter_mut().enumerate() {
                    *b = i as u8 ^ 0xA5;
                }
                p.oam = [0; 256];
                p.oam_addr = 7;
                p.oam_dma(&page);
                assert_eq!(p.oam_addr, 7);
                if refresh {
                    assert_eq!(p.oam, [0; 256], "DMA lost to the refresh");
                } else {
                    assert_eq!(p.oam[7], page[0]);
                    assert_eq!(p.oam[6], page[255]);
                }
            }
        }
    }
}

/// Inverse PAL matrix: RGB (0–1) to Y/U/V.
fn yuv(c: [u8; 3]) -> (f64, f64, f64) {
    let [r, g, b] = c.map(|v| f64::from(v) / 255.0);
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    (y, 0.492 * (b - y), 0.877 * (r - y))
}

#[test]
fn pal_palette_greys_extremes_and_hue_rotation() {
    for idx in [
        0x00u8, 0x10, 0x20, 0x30, 0x0D, 0x1D, 0x2D, 0x3D, 0x0E, 0x0F, 0x1F,
    ] {
        let [r, g, b] = pal_rgb(idx, 0);
        assert!(
            r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1,
            "{idx:02X} is grey: {r} {g} {b}"
        );
        let n = ntsc_rgb(idx, 0);
        assert!(
            n.iter().zip([r, g, b]).all(|(a, b)| a.abs_diff(b) <= 2),
            "same luma {idx:02X}"
        );
    }
    assert_eq!(pal_rgb(0x0F, 0), [0, 0, 0]);
    let mut checked = 0;
    for idx in (0x01..0x0D)
        .chain(0x11..0x1D)
        .chain(0x21..0x2D)
        .chain(0x31..0x3D)
    {
        let p = pal_rgb(idx, 0);
        let n = ntsc_rgb(idx, 0);
        if p.iter().chain(&n).any(|&v| v == 0 || v == 255) {
            continue; // clipped: the angle is no longer exact
        }
        let (_, pu, pv) = yuv(p);
        let (_, nu, nv) = yuv(n);
        let d = (pv.atan2(pu) - nv.atan2(nu)).to_degrees();
        let d = (d + 540.0).rem_euclid(360.0) - 180.0;
        assert!((d - PAL_HUE_SHIFT_DEG).abs() < 3.0, "{idx:02X}: {d}");
        checked += 1;
    }
    assert!(checked >= 4, "{checked}");
    // Emphasis darkens on PAL too (all three bits: every chroma phase attenuated).
    for idx in [0x16u8, 0x2A, 0x30] {
        let plain: u32 = pal_rgb(idx, 0).iter().map(|&v| u32::from(v)).sum();
        let dark: u32 = pal_rgb(idx, 7).iter().map(|&v| u32::from(v)).sum();
        assert!(dark < plain, "{idx:02X}");
    }
}

#[test]
fn rgb_frame_uses_the_region_decoder() {
    for region in [Region::Ntsc, Region::Pal, Region::Dendy] {
        let mut p = Ppu::new();
        p.set_region(region);
        p.framebuffer[0] = 0x16;
        p.emphasis[0] = 2;
        p.framebuffer[1] = 0x2B;
        let rgb = p.rgb_frame();
        let f = if region == Region::Ntsc {
            ntsc_rgb
        } else {
            pal_rgb
        };
        assert_eq!(&rgb[..3], &f(0x16, 2));
        assert_eq!(&rgb[3..6], &f(0x2B, p.emphasis[1]));
    }
}

fn image(mapper: u8, sub: u8, battery: bool, prg16: u8, chr8: u8) -> Vec<u8> {
    let mut f = MAGIC.to_vec();
    f.extend([
        prg16,
        chr8,
        ((mapper & 15) << 4) | (u8::from(battery) << 1),
        (mapper & 0xF0) | 8,
        sub << 4,
        0,
        0x07, // 8 KiB PRG NVRAM (NES 2.0 shift count 7)
        0,
        0,
        0,
        0,
        0,
    ]);
    let n = usize::from(prg16) * 0x4000;
    let mut p = vec![0xEAu8; n];
    // Every 8 KiB bank: JMP to itself at its start; vectors point at $E000.
    for b in (0..n).step_by(0x2000) {
        let base = 0x8000u16 + (b % 0x8000) as u16;
        p[b..b + 3].copy_from_slice(&[0x4C, base as u8, (base >> 8) as u8]);
    }
    p[n - 0x2000..n - 0x1FFD].copy_from_slice(&[0x4C, 0x00, 0xE0]);
    p[n - 4..].copy_from_slice(&[0x00, 0xE0, 0x00, 0xE0]);
    f.extend(p);
    f.extend(vec![0; usize::from(chr8) * 0x2000]);
    f
}

/// (mapper, submapper, PRG 16 KiB units, CHR 8 KiB units, expected .sav size)
const BOARDS: [(u8, u8, u8, u8, Option<usize>); 14] = [
    (0, 0, 2, 1, Some(0x2000)),
    (1, 0, 8, 2, Some(0x2000)),
    (2, 0, 8, 0, None),
    (4, 0, 8, 4, Some(0x2000)),
    (5, 0, 8, 4, Some(0x10000)),
    (9, 0, 8, 4, None),
    (10, 0, 8, 4, Some(0x2000)),
    (19, 0, 8, 4, Some(0x2000 + 128)),
    (21, 1, 8, 4, Some(0x2000)),
    (22, 0, 8, 4, None),
    (24, 0, 8, 4, Some(0x2000)),
    (26, 0, 8, 4, Some(0x2000)),
    (34, 1, 4, 4, Some(0x2000)),
    (69, 0, 8, 4, Some(0x2000)),
];

/// Store `tag` in the board's PRG RAM at `$6000` (enabling it first).
fn poke_ram(nes: &mut Nes2A03, mapper: u8, tag: u8) {
    let c = nes.cpu.bus.cart.as_mut().unwrap();
    match mapper {
        4 => c.write(0xA001, 0x80),
        5 => {
            c.write(0x5102, 2);
            c.write(0x5103, 1);
        }
        19 => c.write(0xF800, 0x40),
        21 => c.write(0x9004, 0x01), // VRC4 $9002 bit 0: PRG RAM enable
        24 | 26 => c.write(0xB003, 0x80),
        69 => {
            c.write(0x8000, 8);
            c.write(0xA000, 0xC0);
        }
        _ => {}
    }
    c.write(0x6000, tag);
    c.write(0x7FFF, tag ^ 0xFF);
}

#[test]
fn save_ram_sizes_and_battery_flag() {
    for (m, sub, p, c, size) in BOARDS {
        let nes = Nes2A03::from_ines(&image(m, sub, true, p, c)).unwrap();
        assert_eq!(nes.save_ram().map(|v| v.len()), size, "mapper {m}");
        let plain = Nes2A03::from_ines(&image(m, sub, false, p, c)).unwrap();
        assert!(
            plain.save_ram().is_none(),
            "no battery, no .sav: mapper {m}"
        );
    }
}

#[test]
fn sav_files_round_trip_on_disk() {
    let dir = std::env::temp_dir().join(format!("pa_sav_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (m, sub, p, c, size) in BOARDS {
        let data = image(m, sub, true, p, c);
        let path = dir.join(format!("m{m}.sav"));
        let _ = std::fs::remove_file(&path);
        let mut a = Nes2A03::from_ines(&data).unwrap();
        assert!(!a.read_sav(&path).unwrap(), "missing file = first boot");
        poke_ram(&mut a, m, 0x40 | m);
        a.run_cycles(1000).unwrap();
        let wrote = a.write_sav(&path).unwrap();
        assert_eq!(wrote, size.is_some(), "mapper {m}");
        let Some(size) = size else { continue };
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), size);
        assert!(
            !path.with_extension("sav.tmp").exists(),
            "temporary renamed away"
        );
        if m != 5 {
            assert_eq!(
                (bytes[0], bytes[0x1FFF]),
                (0x40 | m, !(0x40 | m)),
                "mapper {m}"
            );
        }
        let mut b = Nes2A03::from_ines(&data).unwrap();
        assert!(b.read_sav(&path).unwrap());
        assert_eq!(b.save_ram().unwrap(), bytes, "mapper {m}");
        // Wrong size is refused and leaves RAM alone.
        std::fs::write(&path, &bytes[1..]).unwrap();
        let mut c2 = Nes2A03::from_ines(&data).unwrap();
        let before = c2.save_ram();
        let err = c2.read_sav(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(c2.save_ram(), before);
        assert_eq!(
            c2.load_save_ram(&bytes[1..]),
            Err(SaveRamError::WrongSize {
                expected: size,
                found: size - 1
            })
        );
    }
    let plain = image(2, 0, true, 8, 0);
    let mut n = Nes2A03::from_ines(&plain).unwrap();
    assert_eq!(n.load_save_ram(&[0; 16]), Err(SaveRamError::NoSaveRam));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn n163_sav_carries_the_internal_ram() {
    let data = image(19, 0, true, 8, 4);
    let mut a = Nes2A03::from_ines(&data).unwrap();
    {
        let c = a.cpu.bus.cart.as_mut().unwrap();
        c.write(0xF800, 0x80 | 0x10);
        c.write(0x4800, 0xC3);
    }
    let sav = a.save_ram().unwrap();
    assert_eq!(sav[0x2000 + 0x10], 0xC3);
    let mut b = Nes2A03::from_ines(&data).unwrap();
    b.load_save_ram(&sav).unwrap();
    let c = b.cpu.bus.cart.as_mut().unwrap();
    c.write(0xF800, 0x10);
    assert_eq!(c.read(0x4800), 0xC3);
}
