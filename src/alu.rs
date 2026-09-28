//! ALU 6502: binary path and the documented decimal-mode behavior of each family.
//!
//! Decimal mode follows Bruce Clark's "Decimal Mode" tutorial (6502.org), which
//! defines the result for all 256x256x2 inputs, including invalid BCD:
//! - NMOS `ADC`: accumulator and C from sequence 1; N and V from the intermediate
//!   sum before the high-nibble adjust (sequence 2); Z from the **binary** sum.
//! - NMOS `SBC`: accumulator from sequence 3; N, V, Z and C equal binary `SBC`.
//! - 65C02 `ADC`: accumulator, C and V as NMOS; N and Z from the decimal result.
//! - 65C02 `SBC`: accumulator from sequence 4; C and V binary; N and Z from the result.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdcResult {
    pub value: u8,
    pub carry: bool,
    pub overflow: bool,
    pub negative: bool,
    pub zero: bool,
}

impl AdcResult {
    /// N and Z taken from `value` itself.
    fn from_value(value: u8, carry: bool, overflow: bool) -> Self {
        Self {
            value,
            carry,
            overflow,
            negative: value & 0x80 != 0,
            zero: value == 0,
        }
    }
}

pub fn adc_binary(a: u8, v: u8, carry_in: bool) -> AdcResult {
    let sum = a as u16 + v as u16 + u16::from(carry_in);
    let value = (sum & 0xFF) as u8;
    let overflow = (!(a ^ v) & (a ^ value) & 0x80) != 0;
    AdcResult::from_value(value, sum > 0xFF, overflow)
}

/// Sequences 1 and 2: `(value, carry, n, v)` of decimal `ADC`.
fn adc_decimal_core(a: u8, v: u8, carry_in: bool) -> (u8, bool, bool, bool) {
    let mut al = i16::from(a & 0x0F) + i16::from(v & 0x0F) + i16::from(carry_in);
    if al >= 0x0A {
        al = ((al + 0x06) & 0x0F) + 0x10;
    }
    // Sequence 2: high nibbles as signed values; N and V come from this sum.
    let signed = i16::from((a & 0xF0) as i8) + i16::from((v & 0xF0) as i8) + al;
    let negative = signed & 0x80 != 0;
    let overflow = !(-128..=127).contains(&signed);
    let mut sum = i16::from(a & 0xF0) + i16::from(v & 0xF0) + al;
    if sum >= 0xA0 {
        sum += 0x60;
    }
    ((sum & 0xFF) as u8, sum >= 0x100, negative, overflow)
}

/// NMOS `ADC`. `decimal` selects BCD.
pub fn adc(a: u8, v: u8, carry_in: bool, decimal: bool) -> AdcResult {
    let bin = adc_binary(a, v, carry_in);
    if !decimal {
        return bin;
    }
    let (value, carry, negative, overflow) = adc_decimal_core(a, v, carry_in);
    AdcResult {
        value,
        carry,
        overflow,
        negative,
        zero: bin.zero,
    }
}

/// 65C02 decimal `ADC`: N and Z are valid for the BCD result.
pub fn adc_cmos(a: u8, v: u8, carry_in: bool) -> AdcResult {
    let (value, carry, _, overflow) = adc_decimal_core(a, v, carry_in);
    AdcResult::from_value(value, carry, overflow)
}

/// NMOS `SBC`. `decimal` selects BCD; only the accumulator differs from binary.
pub fn sbc(a: u8, v: u8, carry_in: bool, decimal: bool) -> AdcResult {
    let bin = adc_binary(a, !v, carry_in);
    if !decimal {
        return bin;
    }
    let borrow = 1 - i16::from(carry_in);
    let mut al = i16::from(a & 0x0F) - i16::from(v & 0x0F) - borrow;
    if al < 0 {
        al = ((al - 0x06) & 0x0F) - 0x10;
    }
    let mut diff = i16::from(a & 0xF0) - i16::from(v & 0xF0) + al;
    if diff < 0 {
        diff -= 0x60;
    }
    AdcResult {
        value: (diff & 0xFF) as u8,
        ..bin
    }
}

/// 65C02 decimal `SBC` (sequence 4).
pub fn sbc_cmos(a: u8, v: u8, carry_in: bool) -> AdcResult {
    let bin = adc_binary(a, !v, carry_in);
    let borrow = 1 - i16::from(carry_in);
    let al = i16::from(a & 0x0F) - i16::from(v & 0x0F) - borrow;
    let mut diff = i16::from(a) - i16::from(v) - borrow;
    if diff < 0 {
        diff -= 0x60;
    }
    if al < 0 {
        diff -= 0x06;
    }
    AdcResult::from_value((diff & 0xFF) as u8, bin.carry, bin.overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_adc_ff_plus_1() {
        let r = adc(0xFF, 0x01, false, false);
        assert_eq!(r.value, 0x00);
        assert!(r.carry);
    }

    #[test]
    fn bcd_adc_9_plus_1() {
        let r = adc(0x09, 0x01, false, true);
        assert_eq!(r.value, 0x10);
        assert!(!r.carry);
    }

    #[test]
    fn bcd_adc_99_plus_1() {
        let r = adc(0x99, 0x01, false, true);
        assert_eq!(r.value, 0x00);
        assert!(r.carry);
        // NMOS: Z follows the binary sum $9A, N the intermediate $A0.
        assert!(!r.zero);
        assert!(r.negative);
        let c = adc_cmos(0x99, 0x01, false);
        assert!(c.zero && !c.negative);
    }

    #[test]
    fn bcd_valid_inputs_match_decimal_arithmetic() {
        for x in 0..100u8 {
            for y in 0..100u8 {
                for c in [false, true] {
                    let (a, v) = (crate::bcd::to_bcd(x), crate::bcd::to_bcd(y));
                    let sum = u16::from(x) + u16::from(y) + u16::from(c);
                    let add = adc(a, v, c, true);
                    assert_eq!(add.value, crate::bcd::to_bcd((sum % 100) as u8));
                    assert_eq!(add.carry, sum >= 100);
                    assert_eq!(adc_cmos(a, v, c).value, add.value);
                    let diff = i16::from(x) - i16::from(y) - i16::from(!c);
                    let sub = sbc(a, v, c, true);
                    assert_eq!(sub.value, crate::bcd::to_bcd(diff.rem_euclid(100) as u8));
                    assert_eq!(sub.carry, diff >= 0);
                    assert_eq!(sbc_cmos(a, v, c).value, sub.value);
                }
            }
        }
    }
}
