//! ALU properties over all 256 x 256 x 2 inputs.

use mos6502::alu::{adc, adc_binary, adc_cmos, sbc, sbc_cmos};
use mos6502::bcd::{from_bcd, is_valid, to_bcd};

fn all() -> impl Iterator<Item = (u8, u8, bool)> {
    (0..=255u8).flat_map(|a| (0..=255u8).flat_map(move |v| [(a, v, false), (a, v, true)]))
}

#[test]
fn binary_adc_and_sbc_match_wide_arithmetic() {
    for (a, v, c) in all() {
        let r = adc_binary(a, v, c);
        let wide = u16::from(a) + u16::from(v) + u16::from(c);
        let signed = i16::from(a as i8) + i16::from(v as i8) + i16::from(c);
        assert_eq!((r.value, r.carry), (wide as u8, wide > 0xFF));
        assert_eq!(r.overflow, !(-128..=127).contains(&signed));
        assert_eq!((r.zero, r.negative), (r.value == 0, r.value & 0x80 != 0));
        let s = sbc(a, v, c, false);
        let diff = i16::from(a) - i16::from(v) - i16::from(!c);
        let sdiff = i16::from(a as i8) - i16::from(v as i8) - i16::from(!c);
        assert_eq!(
            (s.value, s.carry),
            (diff as u8, diff >= 0),
            "{a:02X}-{v:02X}"
        );
        assert_eq!(s.overflow, !(-128..=127).contains(&sdiff));
    }
}

#[test]
fn decimal_results_are_bcd_arithmetic_for_valid_inputs() {
    for (a, v, c) in all().filter(|&(a, v, _)| is_valid(a) && is_valid(v)) {
        let (x, y) = (u16::from(from_bcd(a)), u16::from(from_bcd(v)));
        let sum = x + y + u16::from(c);
        let add = adc(a, v, c, true);
        assert_eq!(
            (add.value, add.carry),
            (to_bcd((sum % 100) as u8), sum > 99)
        );
        assert_eq!(adc_cmos(a, v, c).value, add.value);
        let diff = x as i16 - y as i16 - i16::from(!c);
        let sub = sbc(a, v, c, true);
        assert_eq!(
            (sub.value, sub.carry),
            (to_bcd(diff.rem_euclid(100) as u8), diff >= 0)
        );
        assert_eq!(sbc_cmos(a, v, c).value, sub.value);
    }
}

#[test]
fn flag_sources_per_family_hold_for_every_input() {
    for (a, v, c) in all() {
        let bin = adc_binary(a, v, c);
        let nmos = adc(a, v, c, true);
        assert_eq!(nmos.zero, bin.zero, "NMOS ADC Z is binary");
        let cmos = adc_cmos(a, v, c);
        assert_eq!(
            (cmos.zero, cmos.negative),
            (cmos.value == 0, cmos.value & 0x80 != 0)
        );
        assert_eq!(
            (cmos.carry, cmos.overflow, cmos.value),
            (nmos.carry, nmos.overflow, nmos.value)
        );
        let sbin = sbc(a, v, c, false);
        let sn = sbc(a, v, c, true);
        assert_eq!(
            (sn.carry, sn.overflow, sn.zero, sn.negative),
            (sbin.carry, sbin.overflow, sbin.zero, sbin.negative)
        );
        let sc = sbc_cmos(a, v, c);
        assert_eq!((sc.carry, sc.overflow), (sbin.carry, sbin.overflow));
        assert_eq!(
            (sc.zero, sc.negative),
            (sc.value == 0, sc.value & 0x80 != 0)
        );
    }
}
