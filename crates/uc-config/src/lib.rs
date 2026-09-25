//! unicompose's settings, kept in `%APPDATA%\unicompose\config.toml`.
//!
//! The Settings window edits this file, and command-line flags override it for one run.
//! Every field has a default, so a file may list only what differs.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub compose: ComposeSection,
    pub keyboard: KeyboardSection,
    pub workarounds: WorkaroundSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ComposeSection {
    pub enabled: bool,
    /// ralt, lalt, rctrl, lctrl, rwin, lwin, menu, capslock, scrolllock, pause, insert
    /// or printscreen.
    pub key: String,
    /// WinCompose's emoji and extra rules.
    pub wincompose_rules: bool,
    /// `u` + hex digits + Enter types any code point.
    pub hex_entry: bool,
    /// Drop the keys of a sequence that matches nothing, instead of typing them.
    pub discard_invalid: bool,
}

impl Default for ComposeSection {
    fn default() -> Self {
        ComposeSection {
            enabled: true,
            key: "ralt".to_owned(),
            wincompose_rules: true,
            hex_entry: true,
            discard_invalid: false,
        }
    }
}

/// The Raw HID keyboard: a profile, and optional overrides of its IDs as hex strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyboardSection {
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_page: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

impl Default for KeyboardSection {
    fn default() -> Self {
        KeyboardSection { profile: "mathpad".to_owned(), vid: None, pid: None, usage_page: None, usage: None }
    }
}

/// Per-application workarounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkaroundSection {
    pub gtk_astral: bool,
    pub office_font: bool,
}

impl Default for WorkaroundSection {
    fn default() -> Self {
        WorkaroundSection { gtk_astral: true, office_font: false }
    }
}

/// A config file that exists but cannot be used.
#[derive(Debug)]
pub enum ConfigError {
    Read(PathBuf, io::Error),
    Parse(PathBuf, String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read(path, e) => write!(f, "cannot read {}: {e}", path.display()),
            ConfigError::Parse(path, e) => write!(f, "{} is not valid: {e}", path.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

const HEADER: &str = "\
# unicompose settings. The Settings window rewrites this file; command-line flags
# override it for one run. Delete a line to go back to its default.

";

impl Config {
    /// `%APPDATA%\unicompose\config.toml`, or `None` if `APPDATA` is not set.
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("unicompose").join("config.toml"))
    }

    pub fn from_toml(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_toml(&self) -> String {
        let body = toml::to_string_pretty(self).expect("settings always serialize");
        format!("{HEADER}{body}")
    }

    /// The config at `path`, or `None` if there is no file.
    pub fn load(path: &Path) -> Result<Option<Config>, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(ConfigError::Read(path.to_owned(), e)),
        };
        Config::from_toml(&text).map(Some).map_err(|e| ConfigError::Parse(path.to_owned(), e))
    }

    /// Writes the file, creating its folder. Written to a temporary file first, so a crash
    /// never leaves half a config.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("toml.tmp");
        std::fs::write(&temp, self.to_toml())?;
        std::fs::rename(&temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let mut config = Config::default();
        config.compose.key = "capslock".into();
        config.keyboard.vid = Some("1234".into());
        config.workarounds.office_font = true;
        let text = config.to_toml();
        assert!(text.starts_with("# unicompose settings"));
        assert_eq!(Config::from_toml(&text).unwrap(), config);
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let config = Config::from_toml("[compose]\nkey = \"menu\"\n").unwrap();
        assert_eq!(config.compose.key, "menu");
        assert!(config.compose.enabled);
        assert_eq!(config.keyboard, KeyboardSection::default());
        assert_eq!(Config::from_toml("").unwrap(), Config::default());
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        let error = Config::from_toml("[compose]\nenabled = true\nkee = \"ralt\"\n").unwrap_err();
        assert!(error.contains("kee") && error.contains('3'), "{error}");
        let error = Config::from_toml("[compose]\nenabled = \"yes\"\n").unwrap_err();
        assert!(error.contains("enabled") || error.contains("bool"), "{error}");
    }

    #[test]
    fn save_and_load() {
        let dir = std::env::temp_dir().join(format!("uc-config-{}", std::process::id()));
        let path = dir.join("sub").join("config.toml");
        assert!(Config::load(&path).unwrap().is_none());
        let mut config = Config::default();
        config.compose.enabled = false;
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), Some(config));
        std::fs::write(&path, "[nope]\n").unwrap();
        assert!(matches!(Config::load(&path), Err(ConfigError::Parse(..))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
