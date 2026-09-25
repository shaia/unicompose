//! Where settings come from: `config.toml`, created on the first run from WinCompose's
//! settings if it is installed, with command-line flags on top for one run.

use std::path::PathBuf;

use clap::Args;
use uc_config::Config;

use crate::compose::{ComposeArgs, WinComposeSettings};
use crate::device::DeviceArgs;
use crate::typing::QuirkArgs;

/// Every flag that overrides a setting.
#[derive(Args, Debug, Clone, Default)]
pub struct Overrides {
    #[command(flatten)]
    pub device: DeviceArgs,
    #[command(flatten)]
    pub quirks: QuirkArgs,
    #[command(flatten)]
    pub compose: ComposeArgs,
}

impl Overrides {
    /// `saved` with these flags applied.
    pub fn apply(&self, saved: &Config) -> Config {
        let mut config = saved.clone();
        self.device.apply(&mut config.keyboard);
        self.quirks.apply(&mut config.workarounds);
        self.compose.apply(&mut config.compose);
        config
    }

    /// The flags again, for the login entry.
    pub fn to_args(&self) -> Vec<String> {
        [self.device.to_args(), self.quirks.to_args(), self.compose.to_args()].concat()
    }

    /// Which sections the flags touch, so the Settings window can say they win.
    pub fn sections(&self) -> Sections {
        Sections {
            keyboard: !self.device.is_empty(),
            workarounds: !self.quirks.is_empty(),
            compose: !self.compose.is_empty(),
        }
    }
}

/// Sections of the settings, by name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sections {
    pub keyboard: bool,
    pub workarounds: bool,
    pub compose: bool,
}

/// The settings as saved, and where.
#[derive(Debug, Clone)]
pub struct Settings {
    pub saved: Config,
    /// `None` if `APPDATA` is not set: settings then last for this run only.
    pub path: Option<PathBuf>,
    /// Why the saved file could not be used, if it could not. It is then left alone and
    /// the defaults apply.
    pub problem: Option<String>,
}

impl Settings {
    /// Reads `config.toml`. If there is none, creates it: from WinCompose's settings when
    /// WinCompose is installed, else from the defaults.
    pub fn load() -> Settings {
        let Some(path) = Config::default_path() else {
            return Settings { saved: Config::default(), path: None, problem: None };
        };
        match Config::load(&path) {
            Ok(Some(saved)) => Settings { saved, path: Some(path), problem: None },
            Ok(None) => {
                let (saved, source) = match WinComposeSettings::read() {
                    Some(wincompose) => (wincompose.to_config(), "from WinCompose's settings"),
                    None => (Config::default(), "with the defaults"),
                };
                match saved.save(&path) {
                    Ok(()) => tracing::info!("created {} {source}", path.display()),
                    Err(e) => tracing::warn!("cannot create {}: {e}", path.display()),
                }
                Settings { saved, path: Some(path), problem: None }
            }
            Err(e) => {
                tracing::error!("{e}; using the defaults");
                Settings { saved: Config::default(), path: Some(path), problem: Some(e.to_string()) }
            }
        }
    }

    /// Saves new settings. A file that could not be read is replaced only now, when the
    /// user saves on purpose.
    pub fn save(&mut self, saved: Config) -> std::io::Result<()> {
        if let Some(path) = &self.path {
            saved.save(path)?;
        }
        self.saved = saved;
        self.problem = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        overrides: Overrides,
    }

    fn parse(args: &[&str]) -> Overrides {
        Cli::parse_from(std::iter::once("test").chain(args.iter().copied())).overrides
    }

    #[test]
    fn flags_win_over_saved_settings_and_nothing_else_changes() {
        let mut saved = Config::default();
        saved.compose.key = "capslock".into();
        saved.workarounds.office_font = true;
        let config = parse(&["--compose-key", "menu", "--vid", "1234"]).apply(&saved);
        assert_eq!(config.compose.key, "menu");
        assert_eq!(config.keyboard.vid.as_deref(), Some("1234"));
        assert!(config.workarounds.office_font);
        assert_eq!(parse(&[]).apply(&saved), saved);
    }

    #[test]
    fn sections_touched_by_flags() {
        assert_eq!(parse(&[]).sections(), Sections::default());
        let sections = parse(&["--no-quirks", "--pid", "1"]).sections();
        assert!(sections.keyboard && sections.workarounds && !sections.compose);
    }
}
