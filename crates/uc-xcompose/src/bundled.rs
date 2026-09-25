//! Built-in rules, and the default rule set that layers the user's files on top.
//!
//! - `data/en_US.UTF-8.Compose` is libX11's `nls/en_US.UTF-8/Compose.pre` after the
//!   build's preprocessing, regenerated with `cargo xtask data`. License:
//!   `data/COPYING.libX11`.
//! - `data/wincompose/Emoji.txt` and `WinCompose.txt` are WinCompose's own rules (emoji by
//!   name after Compose Compose, squared letters, and a few more), copied from
//!   <https://github.com/samhocevar/wincompose> `src/wincompose/rules`. License (WTFPL):
//!   `data/wincompose/COPYING`.

use std::path::{Path, PathBuf};

use crate::includes::{Includes, MapIncludes};
use crate::parser::{load, load_into, Diagnostic, Dialect, Loaded};
use crate::rules::RuleSet;

pub const EN_US_UTF8: &str = include_str!("../data/en_US.UTF-8.Compose");

/// The name `%L` expands to, and the file name in diagnostics about the bundled rules.
pub const EN_US_UTF8_PATH: &str = "<bundled>/en_US.UTF-8/Compose";

const WINCOMPOSE_FILES: [(&str, &str); 2] = [
    ("<bundled>/wincompose/Emoji.txt", include_str!("../data/wincompose/Emoji.txt")),
    ("<bundled>/wincompose/WinCompose.txt", include_str!("../data/wincompose/WinCompose.txt")),
];

/// The user's own rule files that WinCompose reads, in its order.
pub const USER_FILE_NAMES: [&str; 2] = [".XCompose", ".XCompose.txt"];

/// libX11's rules alone. The file has no includes and is known to load; a test checks
/// that it loads without a single diagnostic.
pub fn load_en_us() -> Loaded {
    load(EN_US_UTF8, EN_US_UTF8_PATH, &mut MapIncludes::default()).expect("the bundled Compose file loads")
}

/// The rule files in `home` that exist.
pub fn user_files(home: &Path) -> Vec<PathBuf> {
    USER_FILE_NAMES.iter().map(|name| home.join(name)).filter(|path| path.is_file()).collect()
}

/// The rules unicompose runs with, loaded in WinCompose's order so later files win:
/// libX11's en_US.UTF-8, WinCompose's emoji and extras (if `wincompose_rules`), then each
/// of `user_files`. The user's files may use WinCompose's key names. A user file that fails
/// to load is reported and its remaining lines are skipped; the rest still applies.
pub fn load_default(wincompose_rules: bool, user_files: &[PathBuf], includes: &mut dyn Includes) -> Loaded {
    let mut rules = load_en_us().rules;
    let mut diagnostics = Vec::new();
    if wincompose_rules {
        for (name, text) in WINCOMPOSE_FILES {
            diagnostics.extend(load_bundled(&mut rules, name, text));
        }
    }
    let includes = &mut AlreadyLoaded(includes);
    for path in user_files {
        let name = path.display().to_string();
        let result = std::fs::read_to_string(path)
            .map_err(|e| vec![Diagnostic::error(&name, 0, &format!("cannot read the file: {e}"))])
            .and_then(|text| {
                load_into(&mut rules, &text, &name, includes, Dialect::WinCompose).map_err(|e| e.diagnostics)
            });
        match result {
            Ok(found) | Err(found) => diagnostics.extend(found),
        }
    }
    Loaded { rules, diagnostics }
}

/// Includes for the user's files. The libX11 rules are already loaded underneath them, so
/// `include "%L"` (which Linux users put at the top) adds nothing, as in WinCompose,
/// instead of reloading them and reporting every rule as a duplicate.
struct AlreadyLoaded<'a>(&'a mut dyn Includes);

impl Includes for AlreadyLoaded<'_> {
    fn expand(&self, what: crate::Expand) -> Option<String> {
        self.0.expand(what)
    }

    fn read(&mut self, path: &str) -> std::io::Result<String> {
        if path == EN_US_UTF8_PATH {
            return Ok(String::new());
        }
        self.0.read(path)
    }
}

/// Loads a bundled file, keeping only errors: its warnings are upstream's own duplicate
/// sequences, where the later one wins as it does in WinCompose.
fn load_bundled(rules: &mut RuleSet, name: &str, text: &str) -> Vec<Diagnostic> {
    load_into(rules, text, name, &mut MapIncludes::default(), Dialect::WinCompose)
        .expect("the bundled WinCompose files load")
        .into_iter()
        .filter(|d| d.severity == crate::Severity::Error)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uc_keysym::sym::MULTI_KEY;

    fn text(rules: &RuleSet, keys: &[&str]) -> Option<String> {
        let keys: Vec<u32> = keys.iter().map(|k| uc_keysym::from_name(k).unwrap()).collect();
        rules.get(&[&[MULTI_KEY][..], &keys].concat()).and_then(|o| o.text.clone())
    }

    #[test]
    fn loads_cleanly_with_thousands_of_rules() {
        let loaded = load_en_us();
        let shown: Vec<String> = loaded.diagnostics.iter().take(5).map(ToString::to_string).collect();
        assert!(loaded.diagnostics.is_empty(), "{} diagnostics, first: {shown:#?}", loaded.diagnostics.len());
        assert!(loaded.rules.len() > 4000, "{} rules", loaded.rules.len());
    }

    #[test]
    fn familiar_sequences() {
        let rules = load_en_us().rules;
        assert_eq!(text(&rules, &["o", "quotedbl"]).as_deref(), Some("ö"));
        assert_eq!(text(&rules, &["minus", "greater"]).as_deref(), Some("→"));
        assert_eq!(text(&rules, &["less", "equal"]).as_deref(), Some("≤"));
    }

    #[test]
    fn wincompose_rules_load_without_errors_and_add_emoji() {
        let loaded = load_default(true, &[], &mut MapIncludes::default());
        let errors: Vec<String> = loaded
            .diagnostics
            .iter()
            .filter(|d| d.severity == crate::Severity::Error)
            .take(5)
            .map(ToString::to_string)
            .collect();
        assert!(errors.is_empty(), "{errors:#?}");
        let en_us = load_en_us().rules.len();
        assert!(loaded.rules.len() > en_us + 1000, "{} rules vs {en_us}", loaded.rules.len());
        let grin = ["Multi_key", "w", "i", "n", "k", "i", "n", "g"];
        assert_eq!(text(&loaded.rules, &grin).as_deref(), Some("😉"));
        assert_eq!(text(&loaded.rules, &["P", "minus"]).as_deref(), Some("₽"));
    }

    #[test]
    fn user_files_come_last_and_a_broken_one_is_reported() {
        let dir = std::env::temp_dir().join(format!("uc-xcompose-user-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".XCompose");
        std::fs::write(
            &file,
            "\u{feff}include \"%L\"\n<Multi_key> <o> <quotedbl> : \"mine\"\n<Multi_key> <!> <!> : \"‼\"\n",
        )
        .unwrap();
        let mut includes = crate::FsIncludes::from_env();
        let loaded = load_default(false, &user_files(&dir), &mut includes);
        assert_eq!(text(&loaded.rules, &["o", "quotedbl"]).as_deref(), Some("mine"));
        assert_eq!(text(&loaded.rules, &["exclam", "exclam"]).as_deref(), Some("‼"));
        // The two overrides are reported; include "%L" adds no duplicates of its own.
        let messages: Vec<&str> = loaded.diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages, ["this compose sequence already exists; overriding"; 2]);

        std::fs::write(&file, "include \"/no/such/file\"\n").unwrap();
        let loaded = load_default(false, &user_files(&dir), &mut includes);
        assert!(loaded.diagnostics.iter().any(|d| d.message.contains("/no/such/file")));
        assert_eq!(text(&loaded.rules, &["o", "quotedbl"]).as_deref(), Some("ö"), "the defaults still apply");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
