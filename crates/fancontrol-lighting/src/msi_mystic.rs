//! MSI Mystic Light, USB `0DB0:0076` (MSI X870 / B850 boards).
//!
//! Protocol facts from <https://github.com/nmelo/msi-mystic-light-x870e> (MIT),
//! tested there on MSI PRO X870E-P WIFI only. One HID feature report per JARGB header:
//!
//! | Offset | Value |
//! |--------|-------|
//! | 0 | `0x51` report id |
//! | 1 | `0x09` fixed |
//! | 2 | `0x04` ARGB type |
//! | 3 | header index (0 = JARGB1 ...) |
//! | 4..=5 | `0` |
//! | 6 | LED count |
//! | 7.. | R, G, B per LED |
//!
//! Safety rule (spec 08): only this direct color report is ever sent. No mode,
//! effect, or save-to-flash command.

use crate::{LightingError, Result, Rgb};

pub const VID: u16 = 0x0DB0;
pub const PID: u16 = 0x0076;
pub const REPORT_ID: u8 = 0x51;
/// Feature report length, report id byte included.
pub const REPORT_LEN: usize = 761;
/// JARGB headers addressed by the report (JARGB1..JARGB3).
pub const ARGB_HEADERS: u8 = 3;
const DATA_OFFSET: usize = 7;
pub const MAX_LEDS: usize = (REPORT_LEN - DATA_OFFSET) / 3;

/// Build the direct color frame for one JARGB header.
pub fn direct_frame(header: u8, colors: &[Rgb]) -> Result<Vec<u8>> {
    if header >= ARGB_HEADERS {
        return Err(LightingError::HeaderOutOfRange {
            header,
            max: ARGB_HEADERS,
        });
    }
    if colors.len() > MAX_LEDS {
        return Err(LightingError::TooManyLeds {
            count: colors.len(),
            max: MAX_LEDS,
        });
    }
    let mut buf = vec![0u8; REPORT_LEN];
    buf[0] = REPORT_ID;
    buf[1] = 0x09;
    buf[2] = 0x04;
    buf[3] = header;
    // MAX_LEDS (251) fits in a byte.
    buf[6] = colors.len() as u8;
    for (i, c) in colors.iter().enumerate() {
        let at = DATA_OFFSET + i * 3;
        buf[at] = c.r;
        buf[at + 1] = c.g;
        buf[at + 2] = c.b;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_layout_matches_reference() {
        let colors = [Rgb::new(0xFF, 0x10, 0x01); 60];
        let f = direct_frame(1, &colors).unwrap();
        assert_eq!(f.len(), REPORT_LEN);
        assert_eq!(&f[..7], &[0x51, 0x09, 0x04, 1, 0, 0, 60]);
        assert_eq!(&f[7..10], &[0xFF, 0x10, 0x01]);
        assert_eq!(&f[184..187], &[0xFF, 0x10, 0x01]);
        assert!(f[187..].iter().all(|&b| b == 0));
    }

    #[test]
    fn max_leds_fill_the_report() {
        assert_eq!(MAX_LEDS, 251);
        let f = direct_frame(0, &[Rgb::new(1, 2, 3); MAX_LEDS]).unwrap();
        assert_eq!(&f[REPORT_LEN - 3..], &[1, 2, 3]);
    }

    #[test]
    fn rejects_bad_header_and_led_count() {
        assert!(direct_frame(ARGB_HEADERS, &[]).is_err());
        assert!(direct_frame(0, &[Rgb::BLACK; MAX_LEDS + 1]).is_err());
    }
}
