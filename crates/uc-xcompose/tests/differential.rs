//! Compares our reading of a Compose file with libxkbcommon's.
//!
//! Ignored by default: it needs a dump of libxkbcommon's table, made on Linux with
//! `tools/xkbcommon_dump.py`. For the bundled file:
//!
//! ```sh
//! python3 tools/xkbcommon_dump.py crates/uc-xcompose/data/en_US.UTF-8.Compose > target/xkbcommon.dump
//! UC_XKBCOMMON_DUMP=target/xkbcommon.dump cargo test -p uc-xcompose --test differential -- --ignored
//! ```
//!
//! `UC_COMPOSE_FILE` compares another file instead of the bundled one.

use std::collections::BTreeMap;

use uc_xcompose::{load, FsIncludes, Output};

/// Rules where we knowingly differ from libxkbcommon, as their key sequences in the
/// dump's hex form. Empty: there are no known differences.
const ALLOWED: &[&str] = &[];

type Table = BTreeMap<String, (Option<u32>, Option<String>)>;

fn ours(text: &str) -> Table {
    let loaded = load(text, "compose", &mut FsIncludes::from_env()).expect("the file loads");
    loaded
        .rules
        .rules()
        .into_iter()
        .map(|rule| {
            let keys: Vec<String> = rule.keys.iter().map(|k| format!("{k:x}")).collect();
            let Output { text, keysym } = rule.output;
            (keys.join(" "), (keysym, text.filter(|t| !t.is_empty())))
        })
        .collect()
}

fn theirs(dump: &str) -> Table {
    dump.lines()
        .map(|line| {
            let mut fields = line.split('\t');
            let (Some(keys), Some(keysym), Some(text)) = (fields.next(), fields.next(), fields.next()) else {
                panic!("malformed dump line: {line:?}");
            };
            let keysym = (keysym != "-").then(|| u32::from_str_radix(keysym, 16).expect("hex keysym"));
            (keys.to_owned(), (keysym, json_string(text)))
        })
        .collect()
}

/// Decodes the JSON the dump script writes: `null` or an ASCII string with `\uXXXX`
/// escapes, surrogate pairs included.
fn json_string(json: &str) -> Option<String> {
    if json == "null" {
        return None;
    }
    let inner = json.strip_prefix('"').and_then(|s| s.strip_suffix('"')).expect("a JSON string");
    let mut units = Vec::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            units.push(c as u16);
            continue;
        }
        let unit = match chars.next().expect("escape") {
            'u' => u16::from_str_radix(&chars.by_ref().take(4).collect::<String>(), 16).expect("\\u escape"),
            'n' => 0x0a,
            't' => 0x09,
            'r' => 0x0d,
            'b' => 0x08,
            'f' => 0x0c,
            other => other as u16,
        };
        units.push(unit);
    }
    Some(String::from_utf16(&units).expect("valid UTF-16"))
}

#[test]
#[ignore = "needs UC_XKBCOMMON_DUMP from tools/xkbcommon_dump.py"]
fn matches_libxkbcommon() {
    let dump_path = std::env::var("UC_XKBCOMMON_DUMP").expect("set UC_XKBCOMMON_DUMP to a dump file");
    let dump = std::fs::read_to_string(&dump_path).expect("read the dump");
    let (text, min_rules) = match std::env::var("UC_COMPOSE_FILE") {
        Ok(path) => (std::fs::read_to_string(path).expect("read UC_COMPOSE_FILE"), 1),
        // Guards against comparing with a dump of the wrong file.
        Err(_) => (uc_xcompose::bundled::EN_US_UTF8.to_owned(), 4000),
    };
    let (ours, theirs) = (ours(&text), theirs(&dump));
    assert!(theirs.len() >= min_rules, "the dump has only {} rules", theirs.len());

    let mut differences = Vec::new();
    for key in ours.keys().chain(theirs.keys()).collect::<std::collections::BTreeSet<_>>() {
        if ALLOWED.contains(&key.as_str()) {
            continue;
        }
        let (a, b) = (ours.get(key), theirs.get(key));
        if a != b {
            differences.push(format!("{key}: ours {a:?}, libxkbcommon {b:?}"));
        }
    }
    assert!(
        differences.is_empty(),
        "{} of {} rules differ; first ones:\n{}",
        differences.len(),
        theirs.len(),
        differences[..differences.len().min(20)].join("\n")
    );
    println!("{} rules match libxkbcommon", ours.len());
}
