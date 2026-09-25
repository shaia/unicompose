//! Property tests: the parser survives any input, and written rules parse back unchanged.

use proptest::prelude::*;
use uc_xcompose::{load, write, MapIncludes, Output, RuleSet};

/// Fragments that make random text look like Compose files, so the parser gets past
/// the first token more often than pure noise would.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("<Multi_key>".to_owned()),
        Just("<a>".to_owned()),
        Just("<U1D538>".to_owned()),
        Just("<0xff20>".to_owned()),
        Just(" : ".to_owned()),
        Just("\"x\"".to_owned()),
        Just("\"\\\\\\\"\\x41\\101\\q\"".to_owned()),
        Just("eacute".to_owned()),
        Just("include \"%H/x\"".to_owned()),
        Just("!Ctrl ~Shift None".to_owned()),
        Just("# comment".to_owned()),
        Just("\n".to_owned()),
        "\\PC{0,6}",
        "[<>:\"\\\\%!~#\n ]{1,4}",
    ]
}

fn keysym() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(uc_keysym::sym::MULTI_KEY),
        0x20u32..0x7f,
        0xa0u32..0x100,
        Just(0xfe51),
        (0x100u32..0x2_0000)
            .prop_filter("scalar", |cp| char::from_u32(*cp).is_some())
            .prop_map(|cp| 0x0100_0000 + cp),
    ]
}

fn output() -> impl Strategy<Value = Output> {
    (proptest::option::of("\\PC{0,4}|[\"\\\\\n\t]{1,3}"), proptest::option::of(keysym()))
        .prop_filter("some output", |(text, keysym)| text.is_some() || keysym.is_some())
        .prop_filter("fits a line", |(text, _)| text.as_ref().is_none_or(|t| t.len() * 4 <= 255))
        .prop_map(|(text, keysym)| Output { text, keysym })
}

proptest! {
    #[test]
    fn any_text_loads_or_fails_cleanly(parts in prop::collection::vec(fragment(), 0..40)) {
        let text = parts.concat();
        let mut includes = MapIncludes::default().with_home("/h").with_file("/h/x", &text);
        match load(&text, "random", &mut includes) {
            Ok(loaded) => {
                for rule in loaded.rules.rules() {
                    prop_assert!(!rule.keys.is_empty() && rule.keys.len() <= 10);
                }
            }
            Err(e) => prop_assert!(!e.diagnostics.is_empty()),
        }
    }

    #[test]
    fn written_rules_parse_back_unchanged(
        rules in prop::collection::vec((prop::collection::vec(keysym(), 1..5), output()), 1..30)
    ) {
        let mut set = RuleSet::new();
        for (keys, output) in rules {
            set.add(&keys, output);
        }
        let text = write(&set);
        let again = load(&text, "written", &mut MapIncludes::default()).unwrap();
        prop_assert!(again.diagnostics.is_empty(), "{:?}\n{}", again.diagnostics, text);
        prop_assert_eq!(again.rules.rules(), set.rules());
    }
}
