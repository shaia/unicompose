//! Chooses how characters reach the focused window.

use clap::Args;
use uc_config::WorkaroundSection;
use uc_platform::{SinkError, TextSink};

use crate::describe;

/// A text sink that can be handed to the thread that types.
pub type Sink = Box<dyn TextSink + Send>;

/// The workaround names flags use, as `uc_win::Quirk::name` spells them.
pub const QUIRK_NAMES: [&str; 2] = ["gtk-astral", "office-font"];

/// Command-line overrides of the `[workarounds]` settings.
#[derive(Args, Debug, Clone, Default)]
pub struct QuirkArgs {
    /// Turn on a per-application workaround: gtk-astral or office-font. Repeatable.
    #[arg(long = "quirk", value_name = "NAME", value_parser = QUIRK_NAMES)]
    quirks: Vec<String>,
    /// Turn off every per-application workaround, including gtk-astral.
    #[arg(long, conflicts_with = "quirks")]
    no_quirks: bool,
}

impl QuirkArgs {
    /// The flags that select the same quirks again, for the login entry.
    pub fn to_args(&self) -> Vec<String> {
        let mut args: Vec<String> =
            self.quirks.iter().flat_map(|q| ["--quirk".to_owned(), q.clone()]).collect();
        if self.no_quirks {
            args.push("--no-quirks".to_owned());
        }
        args
    }

    /// Replaces the settings these flags give.
    pub fn apply(&self, workarounds: &mut WorkaroundSection) {
        if self.no_quirks {
            *workarounds = WorkaroundSection { gtk_astral: false, office_font: false };
        }
        for name in &self.quirks {
            match name.as_str() {
                "gtk-astral" => workarounds.gtk_astral = true,
                "office-font" => workarounds.office_font = true,
                _ => unreachable!("clap accepts only QUIRK_NAMES"),
            }
        }
    }

    /// Whether any flag was given.
    pub fn is_empty(&self) -> bool {
        self.quirks.is_empty() && !self.no_quirks
    }
}

/// The workarounds switched on in the settings.
#[cfg(windows)]
pub fn quirk_set(workarounds: &WorkaroundSection) -> uc_win::QuirkSet {
    use uc_win::{Quirk, QuirkSet};
    let mut set = QuirkSet::NONE;
    for (on, quirk) in
        [(workarounds.gtk_astral, Quirk::GtkAstral), (workarounds.office_font, Quirk::OfficeFont)]
    {
        if on {
            set = set.with(quirk);
        }
    }
    set
}

pub fn make_sink(dry_run: bool, workarounds: &WorkaroundSection) -> Result<Sink, String> {
    if dry_run {
        return Ok(Box::new(PrintSink));
    }
    #[cfg(windows)]
    return Ok(Box::new(uc_win::SendInputSink::new(quirk_set(workarounds))));
    #[cfg(not(windows))]
    {
        let _ = workarounds;
        Err("typing is only implemented on Windows so far; use --dry-run".into())
    }
}

/// Prints each character instead of typing it.
pub struct PrintSink;

impl TextSink for PrintSink {
    fn type_text(&mut self, text: &str) -> Result<(), SinkError> {
        text.chars().for_each(|c| println!("{}", describe(c)));
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
        quirks: QuirkArgs,
    }

    fn parse(args: &[&str]) -> QuirkArgs {
        Cli::parse_from(std::iter::once("test").chain(args.iter().copied())).quirks
    }

    #[test]
    fn to_args_round_trips() {
        for args in [&[][..], &["--quirk", "office-font"][..], &["--no-quirks"][..]] {
            assert_eq!(parse(args).to_args(), args);
        }
    }

    #[test]
    fn quirk_and_no_quirks_conflict() {
        assert!(Cli::try_parse_from(["test", "--quirk", "office-font", "--no-quirks"]).is_err());
    }

    fn applied(args: &[&str]) -> WorkaroundSection {
        let mut workarounds = WorkaroundSection::default();
        parse(args).apply(&mut workarounds);
        workarounds
    }

    #[test]
    fn flags_override_the_settings() {
        assert_eq!(applied(&[]), WorkaroundSection::default());
        assert_eq!(applied(&["--no-quirks"]), WorkaroundSection { gtk_astral: false, office_font: false });
        assert!(applied(&["--quirk", "office-font"]).office_font);
        assert!(Cli::try_parse_from(["test", "--quirk", "nope"]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn names_match_the_windows_quirks_and_defaults_agree() {
        use uc_win::{Quirk, QuirkSet};
        let names: Vec<&str> = Quirk::ALL.iter().map(|q| q.name()).collect();
        assert_eq!(names, QUIRK_NAMES);
        assert_eq!(quirk_set(&WorkaroundSection::default()), QuirkSet::defaults());
    }
}
