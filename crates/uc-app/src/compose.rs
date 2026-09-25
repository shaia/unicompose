//! The Compose key: which key, which rules, and whether WinCompose is in the way.
//!
//! The rules are the ones WinCompose reads, in its order: libX11's, WinCompose's emoji and
//! extras, then the user's `.XCompose` and `.XCompose.txt`. WinCompose's own settings seed
//! `config.toml` the first time unicompose runs (see `settings`).

use std::path::PathBuf;

use clap::Args;
use uc_config::{ComposeSection, Config};
use uc_engine::Table;
use uc_xcompose::{Diagnostic, FsIncludes};

/// WinCompose's executable, to detect it running.
pub const WINCOMPOSE_EXE: &str = "wincompose.exe";

/// Command-line overrides of the `[compose]` settings.
#[derive(Args, Debug, Clone, Default)]
pub struct ComposeArgs {
    /// The Compose key: ralt, lalt, rctrl, lctrl, rwin, lwin, menu, capslock, scrolllock,
    /// pause, insert or printscreen [default: from settings, else ralt].
    #[arg(long, value_name = "KEY")]
    compose_key: Option<String>,
    /// No Compose key: only type what the keyboard sends over Raw HID.
    #[arg(long, conflicts_with = "compose_key")]
    no_compose: bool,
    /// Drop the keys of a sequence that matches nothing, instead of typing them.
    #[arg(long)]
    discard_invalid: bool,
    /// Leave out WinCompose's emoji and extra rules.
    #[arg(long)]
    no_wincompose_rules: bool,
}

impl ComposeArgs {
    /// Replaces the settings these flags give.
    pub fn apply(&self, compose: &mut ComposeSection) {
        if let Some(key) = &self.compose_key {
            compose.key = key.clone();
        }
        if self.no_compose {
            compose.enabled = false;
        }
        if self.discard_invalid {
            compose.discard_invalid = true;
        }
        if self.no_wincompose_rules {
            compose.wincompose_rules = false;
        }
    }

    /// Whether any flag was given.
    pub fn is_empty(&self) -> bool {
        self.to_args().is_empty()
    }

    /// The flags that give the same Compose key again, for the login entry.
    pub fn to_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(key) = &self.compose_key {
            args.extend(["--compose-key".to_owned(), key.clone()]);
        }
        for (set, flag) in [
            (self.no_compose, "--no-compose"),
            (self.discard_invalid, "--discard-invalid"),
            (self.no_wincompose_rules, "--no-wincompose-rules"),
        ] {
            if set {
                args.push(flag.to_owned());
            }
        }
        args
    }
}

/// WinCompose's `%APPDATA%\WinCompose\settings.ini`.
#[derive(Debug, Clone, Default)]
pub struct WinComposeSettings {
    text: String,
}

impl WinComposeSettings {
    pub fn read() -> Option<Self> {
        let path = PathBuf::from(std::env::var_os("APPDATA")?).join("WinCompose").join("settings.ini");
        std::fs::read_to_string(path).ok().map(|text| WinComposeSettings { text })
    }

    pub fn from_text(text: &str) -> Self {
        WinComposeSettings { text: text.to_owned() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// A `key=True` or `key=False` setting.
    pub fn flag(&self, key: &str) -> Option<bool> {
        self.text.lines().find_map(|line| {
            let (k, v) = line.split_once('=')?;
            if k.trim() != key {
                return None;
            }
            match v.trim() {
                v if v.eq_ignore_ascii_case("true") => Some(true),
                v if v.eq_ignore_ascii_case("false") => Some(false),
                _ => None,
            }
        })
    }

    /// Settings with WinCompose's choices in place of the defaults.
    pub fn to_config(&self) -> Config {
        let mut config = Config::default();
        let compose = &mut config.compose;
        #[cfg(windows)]
        if let Some(key) = uc_win::compose::ComposeKey::from_wincompose_settings(&self.text) {
            compose.key = key.name().to_owned();
        }
        compose.wincompose_rules = self.flag("use_emoji_rules").unwrap_or(compose.wincompose_rules);
        compose.hex_entry = self.flag("unicode_input").unwrap_or(compose.hex_entry);
        compose.discard_invalid = self.flag("discard_on_invalid").unwrap_or(compose.discard_invalid);
        config.workarounds.office_font = self.flag("insert_zwsp").unwrap_or(config.workarounds.office_font);
        config
    }
}

/// The rules, freshly read. Problems in the user's files are returned, not fatal.
pub fn load_table(compose: &ComposeSection) -> (Table, Vec<Diagnostic>) {
    let includes = &mut FsIncludes::from_env();
    let user_files = includes.home.as_deref().map(uc_xcompose::bundled::user_files).unwrap_or_default();
    for file in &user_files {
        tracing::info!("reading Compose rules from {}", file.display());
    }
    let loaded = uc_xcompose::bundled::load_default(compose.wincompose_rules, &user_files, includes);
    for diagnostic in loaded.diagnostics.iter().take(20) {
        tracing::warn!("{diagnostic}");
    }
    if loaded.diagnostics.len() > 20 {
        tracing::warn!("{} more problems in the Compose rules", loaded.diagnostics.len() - 20);
    }
    let table = Table::from_rules(&loaded.rules);
    tracing::info!("{} Compose sequences", table.len());
    (table, loaded.diagnostics)
}

#[cfg(windows)]
pub use windows::{compose_key, start, Started};

#[cfg(windows)]
mod windows {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use uc_config::ComposeSection;
    use uc_engine::Options;
    use uc_win::compose::{Compose, ComposeConfig, ComposeKey, COMPOSE_KEYS};
    use uc_win::QuirkSet;

    use super::{load_table, WINCOMPOSE_EXE};

    /// A running Compose key.
    pub struct Started {
        pub compose: Compose,
        /// Started switched off because WinCompose was running: two hooks on the same key
        /// would both act on it.
        pub held_for_wincompose: bool,
    }

    /// The Compose key the settings name.
    pub fn compose_key(compose: &ComposeSection) -> Result<ComposeKey, String> {
        ComposeKey::from_name(&compose.key).ok_or_else(|| {
            let known: Vec<&str> = COMPOSE_KEYS.iter().map(|k| k.name()).collect();
            format!("unknown Compose key '{}'; known keys: {}", compose.key, known.join(", "))
        })
    }

    /// Installs the Compose key, unless it is switched off. `paused` is the app's pause
    /// switch.
    pub fn start(
        compose: &ComposeSection,
        quirks: QuirkSet,
        paused: Arc<AtomicBool>,
    ) -> Result<Option<Started>, Box<dyn std::error::Error + Send + Sync>> {
        if !compose.enabled {
            return Ok(None);
        }
        let key = compose_key(compose)?;
        let (table, _) = load_table(compose);
        let options = Options { hex: compose.hex_entry, ..Options::default() };
        let held_for_wincompose = uc_win::app::process_running(WINCOMPOSE_EXE);
        if held_for_wincompose {
            tracing::warn!("WinCompose is running, so the Compose key stays off until it quits");
        }
        let config = ComposeConfig { table, key, options, discard_invalid: compose.discard_invalid, quirks };
        let compose = Compose::start(config, !held_for_wincompose, paused)?;
        Ok(Some(Started { compose, held_for_wincompose }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        compose: ComposeArgs,
    }

    fn parse(args: &[&str]) -> ComposeArgs {
        Cli::parse_from(std::iter::once("test").chain(args.iter().copied())).compose
    }

    #[test]
    fn to_args_round_trips() {
        for args in [&[][..], &["--compose-key", "capslock", "--discard-invalid"][..], &["--no-compose"][..]]
        {
            assert_eq!(parse(args).to_args(), args);
        }
    }

    #[test]
    fn flags_override_the_settings() {
        let mut compose = ComposeSection::default();
        parse(&["--compose-key", "menu", "--no-wincompose-rules"]).apply(&mut compose);
        assert_eq!(compose.key, "menu");
        assert!(!compose.wincompose_rules);
        assert!(compose.enabled);
        parse(&["--no-compose"]).apply(&mut compose);
        assert!(!compose.enabled);
    }

    #[test]
    fn wincompose_settings_become_a_config() {
        let settings = WinComposeSettings::from_text(
            "[composing]\ncompose_key=VK.CAPITAL\ndiscard_on_invalid=True\nunicode_input = False\n",
        );
        assert_eq!(settings.flag("use_emoji_rules"), None);
        let config = settings.to_config();
        assert!(config.compose.discard_invalid);
        assert!(!config.compose.hex_entry);
        assert!(config.compose.wincompose_rules, "missing settings keep the defaults");
        #[cfg(windows)]
        assert_eq!(config.compose.key, "capslock");
    }

    #[cfg(windows)]
    #[test]
    fn unknown_compose_keys_name_the_known_ones() {
        let compose = ComposeSection { key: "hyper".into(), ..Default::default() };
        assert!(compose_key(&compose).unwrap_err().contains("ralt"));
    }
}
