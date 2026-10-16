//! Plain 8-bit RGB color.

use crate::{LightingError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse `RRGGBB` or `#RRGGBB`.
    pub fn from_hex(s: &str) -> Result<Self> {
        let hex = s.trim().trim_start_matches('#');
        if hex.len() != 6 || !hex.is_ascii() {
            return Err(LightingError::InvalidColor(s.to_string()));
        }
        let byte = |i: usize| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| LightingError::InvalidColor(s.to_string()))
        };
        Ok(Self::new(byte(0)?, byte(2)?, byte(4)?))
    }

    pub fn to_hex(self) -> String {
        format!("{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(Rgb::from_hex("ff8000").unwrap(), Rgb::new(255, 128, 0));
        assert_eq!(Rgb::from_hex("#00FF7f").unwrap(), Rgb::new(0, 255, 127));
    }

    #[test]
    fn rejects_bad_hex() {
        for bad in ["", "fff", "gg0000", "ff00000", "é0000"] {
            assert!(Rgb::from_hex(bad).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(Rgb::new(1, 0xAB, 0xFF).to_hex(), "01ABFF");
    }
}
