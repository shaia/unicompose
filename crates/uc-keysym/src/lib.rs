//! X11 keysyms: the names XCompose files use for keys, their numeric values, and the
//! character each one types.
//!
//! Names and values follow libxkbcommon's `xkb_keysym_from_name` and
//! `xkb_keysym_get_name`, so rules resolve the same way they do on Linux. The table is
//! generated from xorgproto's `keysymdef.h` by `cargo xtask data`.

use std::borrow::Cow;

mod generated;

use generated::{BY_NAME, BY_VALUE, TO_UNICODE};

/// A keysym value.
pub type Keysym = u32;

/// Keysyms the compose engine treats specially.
pub mod sym {
    use super::Keysym;

    pub const BACKSPACE: Keysym = 0xff08;
    pub const TAB: Keysym = 0xff09;
    pub const RETURN: Keysym = 0xff0d;
    pub const ESCAPE: Keysym = 0xff1b;
    /// The Compose key.
    pub const MULTI_KEY: Keysym = 0xff20;
    pub const KP_ENTER: Keysym = 0xff8d;
    pub const DELETE: Keysym = 0xffff;
}

/// Keysyms `0x01000000 + code point` stand for that code point directly.
const UNICODE_OFFSET: u32 = 0x0100_0000;

/// Looks up a keysym by name: a name from `keysymdef.h`, `U` followed by a hex code
/// point (`U2192`), or a `0x` hex value. Names are case-sensitive.
pub fn from_name(name: &str) -> Option<Keysym> {
    if let Ok(index) = BY_NAME.binary_search_by(|(n, _)| (*n).cmp(name)) {
        return Some(BY_NAME[index].1);
    }
    if let Some(hex) = name.strip_prefix('U').filter(|hex| is_hex(hex)) {
        let cp = u32::from_str_radix(hex, 16).ok()?;
        return match cp {
            0..0x20 | 0x7f..0xa0 => None,
            0x20..0x100 => Some(cp),
            0x100..=0x10_ffff => Some(UNICODE_OFFSET + cp),
            _ => None,
        };
    }
    if let Some(hex) = name.strip_prefix("0x").filter(|hex| is_hex(hex)) {
        return u32::from_str_radix(hex, 16).ok().filter(|&value| value <= 0x1fff_ffff);
    }
    None
}

fn is_hex(text: &str) -> bool {
    !text.is_empty() && text.len() <= 8 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The name a keysym is written as: its `keysymdef.h` name, `U` + hex for a Unicode
/// keysym without one, or its `0x` hex value.
pub fn name(keysym: Keysym) -> Cow<'static, str> {
    if let Ok(index) = BY_VALUE.binary_search_by_key(&keysym, |&(value, _)| value) {
        return Cow::Borrowed(BY_VALUE[index].1);
    }
    match keysym.checked_sub(UNICODE_OFFSET) {
        Some(cp @ 0x100..=0x10_ffff) => Cow::Owned(format!("U{cp:04X}")),
        _ => Cow::Owned(format!("0x{keysym:08x}")),
    }
}

/// The character a keysym stands for exactly, if any. Control keys such as Return map to
/// their control character, as in libxkbcommon.
pub fn to_char(keysym: Keysym) -> Option<char> {
    match keysym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(keysym),
        _ if keysym >= UNICODE_OFFSET => char::from_u32(keysym - UNICODE_OFFSET),
        _ => TO_UNICODE
            .binary_search_by_key(&keysym, |&(value, _)| value)
            .ok()
            .and_then(|index| char::from_u32(TO_UNICODE[index].1)),
    }
}

/// The keysym for a character: its Latin-1 value, or `0x01000000 + code point`.
pub fn from_char(c: char) -> Keysym {
    match u32::from(c) {
        cp @ (0x20..=0x7e | 0xa0..=0xff) => cp,
        cp => UNICODE_OFFSET + cp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn tables_are_sorted_for_binary_search() {
        assert!(BY_NAME.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(BY_VALUE.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(TO_UNICODE.windows(2).all(|w| w[0].0 < w[1].0));
    }

    #[test]
    fn named_keysyms() {
        assert_eq!(from_name("Multi_key"), Some(sym::MULTI_KEY));
        assert_eq!(from_name("dead_acute"), Some(0xfe51));
        assert_eq!(from_name("quotedbl"), Some(0x22));
        assert_eq!(from_name("Greek_alpha"), Some(0x7e1));
        assert_eq!(from_name("multi_key"), None, "names are case-sensitive");
        assert_eq!(name(0xfe51), "dead_acute");
        assert_eq!(name(sym::MULTI_KEY), "Multi_key");
    }

    #[test]
    fn unicode_names() {
        assert_eq!(from_name("U2192"), Some(0x0100_2192));
        assert_eq!(from_name("U1D538"), Some(0x0101_D538));
        assert_eq!(from_name("U00e9"), Some(0xe9), "Latin-1 code points are their own keysym");
        assert_eq!(from_name("U0007"), None, "control characters have no keysym");
        assert_eq!(from_name("U110000"), None);
        assert_eq!(from_name("U"), Some(0x55), "the letter U is a name, not an empty code point");
        assert_eq!(from_name("Uzz"), None);
        assert_eq!(name(0x0101_D538), "U1D538");
    }

    #[test]
    fn hex_names() {
        assert_eq!(from_name("0xff20"), Some(sym::MULTI_KEY));
        assert_eq!(from_name("0x"), None);
        assert_eq!(name(0x1234_5678 & 0x0fff_ffff), "0x02345678");
    }

    #[test]
    fn characters() {
        assert_eq!(to_char(0x22), Some('"'));
        assert_eq!(to_char(0xe9), Some('é'));
        assert_eq!(to_char(0x7e1), Some('α'));
        assert_eq!(to_char(0x0101_D538), Some('𝔸'));
        assert_eq!(to_char(sym::RETURN), Some('\r'));
        assert_eq!(to_char(0xfe51), None, "dead keys type nothing by themselves");
        assert_eq!(to_char(0xffb0), None, "keypad keys map only approximately");
        assert_eq!(to_char(0x0100_D800), None);
    }

    proptest! {
        #[test]
        fn every_char_round_trips_through_its_keysym(c in any::<char>().prop_filter("printable", |c| !c.is_control())) {
            prop_assert_eq!(to_char(from_char(c)), Some(c));
        }

        #[test]
        fn names_round_trip(index in 0..BY_VALUE.len()) {
            let (value, name_) = BY_VALUE[index];
            prop_assert_eq!(from_name(name_), Some(value));
            prop_assert_eq!(name(value), name_);
        }

        #[test]
        fn lookups_never_panic(text in "\\PC{0,12}", value in any::<u32>()) {
            let _ = from_name(&text);
            let _ = to_char(value);
            let _ = name(value);
        }
    }
}
