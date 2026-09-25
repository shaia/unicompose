//! Device profiles: which HID collection to open, and how to read its reports.
//!
//! A supported keyboard is a row in `PROFILES`, not something the rest of the app knows
//! about. Mathpad is the example that ships with it. Firmware that sends the same reports
//! needs one more row, and a keyboard that differs only in its IDs needs none: the
//! `[keyboard]` settings, or the `--vid`/`--pid`/`--usage-page`/`--usage` flags, reach it
//! without a rebuild.

use std::fmt;

use clap::Args;
use uc_codec::{DecodeError, Report};
use uc_config::KeyboardSection;
use uc_hid::DeviceFilter;

/// Reads one input report as it arrives from the device.
pub type Decoder = fn(&[u8]) -> Result<Report, DecodeError>;

pub struct Profile {
    pub name: &'static str,
    /// The HID collection to open.
    pub filter: DeviceFilter,
    pub decode: Decoder,
}

/// The profile used when `--device` is not given.
pub const DEFAULT: &str = "mathpad";

/// QMK's Raw HID collection: vendor-defined usage page 0xFF60, usage 0x61.
const QMK_RAW_HID: (u16, u16) = (0xFF60, 0x61);

pub const PROFILES: &[Profile] = &[Profile {
    // Summa-Cogni Mathpad, with its OS switch in the MAC position. 0x1209 is pid.codes.
    name: "mathpad",
    filter: DeviceFilter {
        vendor_id: 0x1209,
        product_id: 0x2211,
        usage_page: QMK_RAW_HID.0,
        usage: QMK_RAW_HID.1,
    },
    decode: uc_codec::decode,
}];

/// Command-line overrides of the `[keyboard]` settings.
#[derive(Args, Debug, Clone, Default)]
pub struct DeviceArgs {
    /// Device profile to read reports with [default: from settings, else mathpad].
    #[arg(long, value_name = "NAME")]
    device: Option<String>,
    /// Vendor ID in hex, replacing the profile's.
    #[arg(long, value_name = "HEX", value_parser = parse_hex16)]
    vid: Option<u16>,
    /// Product ID in hex, replacing the profile's.
    #[arg(long, value_name = "HEX", value_parser = parse_hex16)]
    pid: Option<u16>,
    /// HID usage page in hex, replacing the profile's.
    #[arg(long, value_name = "HEX", value_parser = parse_hex16)]
    usage_page: Option<u16>,
    /// HID usage in hex, replacing the profile's.
    #[arg(long, value_name = "HEX", value_parser = parse_hex16)]
    usage: Option<u16>,
}

/// A profile with the overrides from settings and flags applied.
#[derive(Debug, Clone)]
pub struct Target {
    pub name: &'static str,
    pub filter: DeviceFilter,
    decode: Decoder,
}

/// The keyboard the settings describe: a profile, with any of its IDs replaced.
pub fn resolve(keyboard: &KeyboardSection) -> Result<Target, String> {
    let mut target = Target::from(find(&keyboard.profile)?);
    let filter = &mut target.filter;
    let fields = [
        ("vid", &keyboard.vid, &mut filter.vendor_id),
        ("pid", &keyboard.pid, &mut filter.product_id),
        ("usage_page", &keyboard.usage_page, &mut filter.usage_page),
        ("usage", &keyboard.usage, &mut filter.usage),
    ];
    for (name, text, value) in fields {
        if let Some(text) = text.as_deref().filter(|t| !t.trim().is_empty()) {
            *value = parse_hex16(text.trim()).map_err(|e| format!("keyboard {name}: {e}"))?;
        }
    }
    Ok(target)
}

impl DeviceArgs {
    /// Replaces the settings these flags give.
    pub fn apply(&self, keyboard: &mut KeyboardSection) {
        if let Some(name) = &self.device {
            keyboard.profile = name.clone();
        }
        let fields = [
            (self.vid, &mut keyboard.vid),
            (self.pid, &mut keyboard.pid),
            (self.usage_page, &mut keyboard.usage_page),
            (self.usage, &mut keyboard.usage),
        ];
        for (flag, setting) in fields {
            if let Some(value) = flag {
                *setting = Some(format!("{value:04x}"));
            }
        }
    }

    /// Whether any flag was given.
    pub fn is_empty(&self) -> bool {
        self.device.is_none()
            && self.vid.is_none()
            && self.pid.is_none()
            && self.usage_page.is_none()
            && self.usage.is_none()
    }

    /// The flags that select the same target again, for a command line stored elsewhere
    /// (the login entry).
    pub fn to_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(name) = &self.device {
            args.extend(["--device".to_owned(), name.clone()]);
        }
        let hex = [
            ("--vid", self.vid),
            ("--pid", self.pid),
            ("--usage-page", self.usage_page),
            ("--usage", self.usage),
        ];
        for (flag, value) in hex {
            if let Some(value) = value {
                args.extend([flag.to_owned(), format!("{value:04x}")]);
            }
        }
        args
    }
}

impl From<&Profile> for Target {
    fn from(profile: &Profile) -> Self {
        Target { name: profile.name, filter: profile.filter, decode: profile.decode }
    }
}

impl Target {
    /// Profile name and IDs, for status text: `mathpad (1209:2211)`.
    pub fn label(&self) -> String {
        format!("{} ({:04x}:{:04x})", self.name, self.filter.vendor_id, self.filter.product_id)
    }

    pub fn decode(&self, report: &[u8]) -> Result<Report, DecodeError> {
        (self.decode)(report)
    }
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The name comes last: the overrides may have left nothing of the profile but its codec.
        let DeviceFilter { vendor_id, product_id, usage_page, usage } = self.filter;
        write!(
            f,
            "{vendor_id:04x}:{product_id:04x} usage {usage_page:04x}:{usage:02x} ({} profile)",
            self.name
        )
    }
}

pub fn find(name: &str) -> Result<&'static Profile, String> {
    PROFILES.iter().find(|profile| profile.name == name).ok_or_else(|| {
        let known: Vec<&str> = PROFILES.iter().map(|profile| profile.name).collect();
        format!("unknown device profile '{name}'; known profiles: {}", known.join(", "))
    })
}

pub fn parse_hex16(text: &str) -> Result<u16, String> {
    let digits = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text);
    u16::from_str_radix(digits, 16).map_err(|_| format!("'{text}' is not a 16-bit hex number"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        device: DeviceArgs,
    }

    fn parse(args: &[&str]) -> DeviceArgs {
        Cli::parse_from(std::iter::once("test").chain(args.iter().copied())).device
    }

    /// The target for the default settings with `args` on top.
    fn target(args: &[&str]) -> Result<Target, String> {
        let mut keyboard = KeyboardSection::default();
        parse(args).apply(&mut keyboard);
        resolve(&keyboard)
    }

    #[test]
    fn defaults_to_the_example_profile() {
        let target = target(&[]).unwrap();
        assert_eq!(target.name, DEFAULT);
        assert_eq!(target.filter, find(DEFAULT).unwrap().filter);
    }

    #[test]
    fn overrides_replace_only_the_fields_given() {
        let profile = find(DEFAULT).unwrap();
        let target = target(&["--vid", "0x1234", "--usage", "42"]).unwrap();
        assert_eq!(target.filter.vendor_id, 0x1234);
        assert_eq!(target.filter.usage, 0x42);
        assert_eq!(target.filter.product_id, profile.filter.product_id);
        assert_eq!(target.filter.usage_page, profile.filter.usage_page);
    }

    #[test]
    fn to_args_round_trips() {
        for args in [&[][..], &["--device", "mathpad", "--vid", "0x1234", "--usage-page", "ff00"][..]] {
            let first = parse(args);
            let again = parse(&first.to_args().iter().map(String::as_str).collect::<Vec<_>>());
            assert_eq!(again.to_args(), first.to_args());
            let (mut a, mut b) = (KeyboardSection::default(), KeyboardSection::default());
            first.apply(&mut a);
            again.apply(&mut b);
            assert_eq!(resolve(&a).unwrap().filter, resolve(&b).unwrap().filter);
        }
        assert!(parse(&[]).to_args().is_empty());
    }

    #[test]
    fn unknown_profile_names_the_known_ones() {
        let error = target(&["--device", "nope"]).unwrap_err();
        assert!(error.contains("nope") && error.contains(DEFAULT), "{error}");
    }

    #[test]
    fn settings_ids_are_hex_and_named_in_errors() {
        let keyboard =
            KeyboardSection { vid: Some("0x1d50".into()), pid: Some(" ".into()), ..Default::default() };
        assert_eq!(resolve(&keyboard).unwrap().filter.vendor_id, 0x1d50);
        let bad = KeyboardSection { usage: Some("xyz".into()), ..Default::default() };
        assert!(resolve(&bad).unwrap_err().contains("usage"));
    }

    #[test]
    fn rejects_hex_that_does_not_fit_16_bits() {
        assert_eq!(parse_hex16("ff60"), Ok(0xFF60));
        assert!(parse_hex16("10000").is_err());
        assert!(parse_hex16("zz").is_err());
    }

    #[test]
    fn every_profile_has_a_unique_name() {
        let mut names: Vec<&str> = PROFILES.iter().map(|profile| profile.name).collect();
        names.sort_unstable();
        let unique = names.len();
        names.dedup();
        assert_eq!(names.len(), unique);
        assert!(find(DEFAULT).is_ok(), "the default profile must exist");
    }
}
