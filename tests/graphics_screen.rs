//! Screen addressing, PPM export and palette matching over every pixel and color.

use mos6502::graphics::{nearest_color, Screen, RETRO_PALETTE};

#[test]
fn every_pixel_and_color_is_addressed_once() {
    let mut s = Screen::new();
    for y in 0..32 {
        for x in 0..32 {
            s.set_pixel(x, y, (x + y * 3) as u8 | 0xF0);
        }
    }
    for y in 0..34 {
        for x in 0..34 {
            let want = (x < 32 && y < 32).then(|| ((x + y * 3) as u8) & 0x0F);
            assert_eq!(s.pixel(x, y), want);
        }
    }
    s.set_pixel(32, 0, 1);
    s.set_pixel(0, 32, 1);
    assert_eq!(s.pixel(0, 0), Some(0), "off-screen writes ignored");
}

#[test]
fn ppm_is_exact_for_each_scale() {
    let mut s = Screen::new();
    s.set_pixel(31, 31, 7);
    for scale in [1usize, 2, 5] {
        let ppm = s.to_ppm(scale).unwrap();
        let side = 32 * scale;
        let header = format!("P6\n{side} {side}\n255\n");
        assert!(ppm.starts_with(header.as_bytes()));
        assert_eq!(ppm.len(), header.len() + side * side * 3);
        let last = &ppm[ppm.len() - 3..];
        let (r, g, b) = RETRO_PALETTE[7];
        assert_eq!(last, [r, g, b]);
        let first_of_block = header.len() + ((side - scale) * side + side - scale) * 3;
        assert_eq!(&ppm[first_of_block..first_of_block + 3], [r, g, b]);
        assert_eq!(&ppm[first_of_block - 3..first_of_block], [0, 0, 0]);
    }
    assert!(s.to_ppm(0).is_err() && s.to_ppm(65).is_err());
}

#[test]
fn nearest_color_maps_the_palette_to_itself_and_every_grey_somewhere_sane() {
    for (i, &(r, g, b)) in RETRO_PALETTE.iter().enumerate() {
        assert_eq!(nearest_color(r, g, b), i as u8);
    }
    for v in 0..=255u8 {
        let idx = nearest_color(v, v, v);
        let (r, g, b) = RETRO_PALETTE[usize::from(idx)];
        assert!(r == g && g == b, "grey {v} -> {idx}");
    }
}
