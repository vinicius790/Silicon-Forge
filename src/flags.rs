//! Registrador de status (P) do MOS 6502.
//!
//! Bits: N V - B D I Z C
//! O bit 5 (unused) é sempre lido como 1 em PHP/BRK no hardware real;
//! este núcleo força esse comportamento ao empilhar P.

/// Máscaras de bit do status register.
pub mod bits {
    pub const C: u8 = 1 << 0; // Carry
    pub const Z: u8 = 1 << 1; // Zero
    pub const I: u8 = 1 << 2; // IRQ disable
    pub const D: u8 = 1 << 3; // Decimal (BCD) — suportado parcialmente
    pub const B: u8 = 1 << 4; // Break (só no stack frame)
    pub const U: u8 = 1 << 5; // Unused (sempre 1 no stack)
    pub const V: u8 = 1 << 6; // Overflow
    pub const N: u8 = 1 << 7; // Negative
}

/// Status register com helpers tipados.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status(pub u8);

impl Status {
    pub fn new() -> Self {
        // Após reset, tipicamente I=1; demais 0. Bit U lógico = 1 quando empilhado.
        Self(bits::I | bits::U)
    }

    pub fn raw(self) -> u8 {
        self.0
    }

    pub fn get(self, mask: u8) -> bool {
        self.0 & mask != 0
    }

    pub fn set(&mut self, mask: u8, on: bool) {
        if on {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
    }

    /// Atualiza Z e N a partir de um valor de 8 bits.
    pub fn set_zn(&mut self, value: u8) {
        self.set(bits::Z, value == 0);
        self.set(bits::N, value & 0x80 != 0);
    }

    /// Valor empilhado por PHP/BRK: força B=1 e U=1.
    pub fn to_stack_php(self) -> u8 {
        self.0 | bits::B | bits::U
    }

    /// Valor empilhado por interrupção de hardware: B=0, U=1.
    pub fn to_stack_irq(self) -> u8 {
        (self.0 & !bits::B) | bits::U
    }

    /// Restaura P do stack (ignora B; força U=1).
    pub fn from_stack(byte: u8) -> Self {
        Self((byte & !bits::B) | bits::U)
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{}-{}{}{}{}{}",
            if self.get(bits::N) { 'N' } else { 'n' },
            if self.get(bits::V) { 'V' } else { 'v' },
            if self.get(bits::B) { 'B' } else { 'b' },
            if self.get(bits::D) { 'D' } else { 'd' },
            if self.get(bits::I) { 'I' } else { 'i' },
            if self.get(bits::Z) { 'Z' } else { 'z' },
            if self.get(bits::C) { 'C' } else { 'c' },
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusParseError(pub String);

impl std::fmt::Display for StatusParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "status inválido: {}", self.0)
    }
}

impl std::error::Error for StatusParseError {}

/// Inverse of `Display`: eight characters `NV-BDIZC`, upper case = set, lower = clear,
/// the third character `-` (bit 5 is then set, as the CPU always reads it).
impl std::str::FromStr for Status {
    type Err = StatusParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() != 8 || chars[2] != '-' {
            return Err(StatusParseError(s.into()));
        }
        let mut status = Status(bits::U);
        for (ch, (letter, mask)) in chars.iter().filter(|&&c| c != '-').zip([
            ('N', bits::N),
            ('V', bits::V),
            ('B', bits::B),
            ('D', bits::D),
            ('I', bits::I),
            ('Z', bits::Z),
            ('C', bits::C),
        ]) {
            if ch.to_ascii_uppercase() != letter {
                return Err(StatusParseError(s.into()));
            }
            status.set(mask, ch.is_ascii_uppercase());
        }
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_parses_back_for_every_value_and_rejects_garbage() {
        for raw in 0..=255u8 {
            let status = Status(raw | bits::U);
            assert_eq!(
                status.to_string().parse::<Status>(),
                Ok(status),
                "{raw:02X}"
            );
        }
        for bad in ["NV-BDIZ", "NVxBDIZC", "VN-BDIZC", "NV-BDIZCC", "nv-bdizq"] {
            assert!(bad.parse::<Status>().is_err(), "{bad}");
        }
        assert_eq!(Status::from_stack(0xFF).to_stack_irq() & bits::B, 0);
        assert_eq!(Status::new().to_stack_php(), bits::I | bits::U | bits::B);
    }
}
