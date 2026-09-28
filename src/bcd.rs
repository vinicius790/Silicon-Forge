//! Packed BCD helpers (the ADC/SBC decimal arithmetic lives in `alu`).
//!
//! A packed byte holds two decimal digits, tens in bits 7–4. [`to_bcd`] and
//! [`from_bcd`] are the unchecked forms used by tests and tables; the checked forms
//! reject values that are not decimal.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BcdError {
    /// Binary value above 99.
    TooLarge(u8),
    /// A nibble above 9 (`$1A`, `$A0`).
    BadDigit(u8),
}

impl fmt::Display for BcdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(n) => write!(f, "{n} não cabe em dois dígitos BCD"),
            Self::BadDigit(b) => write!(f, "${b:02X} não é BCD válido"),
        }
    }
}

impl std::error::Error for BcdError {}

/// `n` modulo 100 as packed BCD.
pub fn to_bcd(n: u8) -> u8 {
    let n = n % 100;
    ((n / 10) << 4) | (n % 10)
}

/// Packed digits to binary; invalid nibbles are taken at face value (`$1A` is 20).
pub fn from_bcd(b: u8) -> u8 {
    (b >> 4) * 10 + (b & 0x0F)
}

pub fn to_bcd_checked(n: u8) -> Result<u8, BcdError> {
    if n > 99 {
        return Err(BcdError::TooLarge(n));
    }
    Ok(to_bcd(n))
}

pub fn is_valid(b: u8) -> bool {
    b >> 4 <= 9 && b & 0x0F <= 9
}

pub fn from_bcd_checked(b: u8) -> Result<u8, BcdError> {
    if !is_valid(b) {
        return Err(BcdError::BadDigit(b));
    }
    Ok(from_bcd(b))
}

/// Multi-byte packed BCD, most significant byte first, to binary.
pub fn decode_digits(bytes: &[u8]) -> Result<u64, BcdError> {
    bytes.iter().try_fold(0u64, |acc, &b| {
        Ok(acc
            .saturating_mul(100)
            .saturating_add(u64::from(from_bcd_checked(b)?)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_forms_cover_every_byte() {
        let valid = (0..=255u8).filter(|&b| is_valid(b)).count();
        assert_eq!(valid, 100);
        for n in 0..100u8 {
            assert_eq!(from_bcd_checked(to_bcd_checked(n).unwrap()), Ok(n));
        }
        assert_eq!(to_bcd_checked(100), Err(BcdError::TooLarge(100)));
        assert_eq!(from_bcd_checked(0x1A), Err(BcdError::BadDigit(0x1A)));
        assert_eq!(from_bcd_checked(0xA0), Err(BcdError::BadDigit(0xA0)));
        assert_eq!(to_bcd(142), 0x42, "modulo 100");
        assert_eq!(decode_digits(&[0x12, 0x34, 0x56]), Ok(123_456));
        assert_eq!(decode_digits(&[0x12, 0x3F]), Err(BcdError::BadDigit(0x3F)));
        assert_eq!(BcdError::BadDigit(0x3F).to_string(), "$3F não é BCD válido");
    }
}
