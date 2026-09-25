//! Codec for Unicode input reports carried over Raw HID.
//!
//! A keyboard can send a character the host's layout cannot produce as a vendor-defined
//! HID report instead of as keystrokes. This codec reads the framing that Mathpad's QMK
//! firmware uses, which any firmware is free to copy; nothing here is specific to that
//! keyboard, which the app knows only as one entry in its device table.
//!
//! Reports are 32 bytes and start with command byte `0x01`. Mathpad's firmware
//! (`firmware/mathpad/mp1a/keymaps/default/modes/unicode/unicode_mode.c`) sends two
//! encodings of the payload:
//!
//! - binary (`send_raw_hid_unicode`, used for symbols): bytes 1..5 are the code point as a
//!   big-endian `u32`, so byte 1 is always `0x00` for a valid code point;
//! - ASCII (`send_unicode_via_rawhid`): bytes 1..5 are four hex digits, BMP only.
//!
//! Byte 1 distinguishes them: an ASCII hex digit can never be the top byte of a valid
//! code point.

use std::fmt;

/// Raw HID report length. QMK's buffer is 32 bytes; `decode` accepts any length.
pub const REPORT_LEN: usize = 32;

const CMD_UNICODE: u8 = 0x01;

/// A decoded Raw HID report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Report {
    /// Type this character.
    Codepoint(char),
    /// A command this codec does not know; newer firmware may send these.
    Unknown { command: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    TooShort {
        len: usize,
    },
    /// Not a Unicode scalar value (surrogate or above U+10FFFF).
    InvalidCodepoint(u32),
    /// A control character; never a symbol, and unsafe to inject blindly.
    Control(char),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { len } => write!(f, "report too short ({len} bytes)"),
            Self::InvalidCodepoint(cp) => write!(f, "invalid code point 0x{cp:X}"),
            Self::Control(c) => write!(f, "control character U+{:04X}", u32::from(*c)),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Decodes one Raw HID report as read from the device (without a report ID byte).
pub fn decode(report: &[u8]) -> Result<Report, DecodeError> {
    let Some(&command) = report.first() else {
        return Err(DecodeError::TooShort { len: 0 });
    };
    if command != CMD_UNICODE {
        return Ok(Report::Unknown { command });
    }
    let Some(payload) = report.get(1..5) else {
        return Err(DecodeError::TooShort { len: report.len() });
    };
    let cp = match ascii_hex(payload) {
        Some(cp) => cp,
        None => u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]),
    };
    let c = char::from_u32(cp).ok_or(DecodeError::InvalidCodepoint(cp))?;
    if c.is_control() {
        return Err(DecodeError::Control(c));
    }
    Ok(Report::Codepoint(c))
}

/// Encodes `c` the way the firmware's binary path does. Used to fake a device in tests.
pub fn encode(c: char) -> [u8; REPORT_LEN] {
    let mut report = [0u8; REPORT_LEN];
    report[0] = CMD_UNICODE;
    report[1..5].copy_from_slice(&u32::from(c).to_be_bytes());
    report
}

fn ascii_hex(payload: &[u8]) -> Option<u32> {
    if !payload[0].is_ascii_hexdigit() {
        return None;
    }
    let digits = std::str::from_utf8(payload).ok()?;
    u32::from_str_radix(digits, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn encode_ascii(c: char) -> [u8; REPORT_LEN] {
        let mut report = [0u8; REPORT_LEN];
        report[0] = CMD_UNICODE;
        report[1..5].copy_from_slice(format!("{:04x}", u32::from(c)).as_bytes());
        report
    }

    #[test]
    fn decodes_binary_firmware_report() {
        // α as sent by send_raw_hid_unicode.
        let mut report = [0u8; REPORT_LEN];
        report[..5].copy_from_slice(&[0x01, 0x00, 0x00, 0x03, 0xB1]);
        assert_eq!(decode(&report), Ok(Report::Codepoint('α')));
    }

    #[test]
    fn decodes_astral_math_letter() {
        assert_eq!(decode(&encode('𝔸')), Ok(Report::Codepoint('𝔸')));
    }

    #[test]
    fn decodes_ascii_firmware_report() {
        let mut report = [0u8; REPORT_LEN];
        report[..5].copy_from_slice(b"\x0100b0");
        assert_eq!(decode(&report), Ok(Report::Codepoint('°')));
    }

    #[test]
    fn rejects_empty_and_short_reports() {
        assert_eq!(decode(&[]), Err(DecodeError::TooShort { len: 0 }));
        assert_eq!(decode(&[0x01, 0, 0]), Err(DecodeError::TooShort { len: 3 }));
    }

    #[test]
    fn rejects_all_zero_payload_as_control() {
        let mut report = [0u8; REPORT_LEN];
        report[0] = CMD_UNICODE;
        assert_eq!(decode(&report), Err(DecodeError::Control('\0')));
    }

    #[test]
    fn rejects_surrogates_and_out_of_range() {
        let mut report = [0u8; REPORT_LEN];
        report[..5].copy_from_slice(&[0x01, 0x00, 0x00, 0xD8, 0x00]);
        assert_eq!(decode(&report), Err(DecodeError::InvalidCodepoint(0xD800)));
        report[..5].copy_from_slice(&[0x01, 0x00, 0x11, 0x00, 0x00]);
        assert_eq!(decode(&report), Err(DecodeError::InvalidCodepoint(0x11_0000)));
    }

    #[test]
    fn passes_unknown_commands_through() {
        assert_eq!(decode(&[0x02; REPORT_LEN]), Ok(Report::Unknown { command: 0x02 }));
    }

    proptest! {
        #[test]
        fn binary_round_trips(c in any::<char>().prop_filter("printable", |c| !c.is_control())) {
            prop_assert_eq!(decode(&encode(c)), Ok(Report::Codepoint(c)));
        }

        #[test]
        fn ascii_round_trips_for_bmp(c in any::<char>().prop_filter("printable BMP", |c| {
            !c.is_control() && u32::from(*c) <= 0xFFFF
        })) {
            prop_assert_eq!(decode(&encode_ascii(c)), Ok(Report::Codepoint(c)));
        }

        #[test]
        fn never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let _ = decode(&bytes);
        }
    }
}
