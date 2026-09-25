//! Writes rules back out in XCompose syntax.

use std::fmt::Write as _;

use crate::rules::{Output, RuleSet};

/// One line per rule, in keysym order: `<k1> <k2> : "text" keysym`.
pub fn write(rules: &RuleSet) -> String {
    let mut out = String::new();
    for rule in rules.rules() {
        let keys: Vec<String> = rule.keys.iter().map(|&k| format!("<{}>", uc_keysym::name(k))).collect();
        out.push_str(&keys.join(" "));
        out.push_str(" :");
        write_output(&mut out, &rule.output);
        out.push('\n');
    }
    out
}

fn write_output(out: &mut String, output: &Output) {
    if let Some(text) = &output.text {
        out.push_str(" \"");
        for c in text.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                // A line break would end the rule, and other controls are unreadable:
                // write their bytes as octal escapes.
                c if c.is_control() => {
                    for byte in c.encode_utf8(&mut [0; 4]).bytes() {
                        let _ = write!(out, "\\{byte:03o}");
                    }
                }
                c => out.push(c),
            }
        }
        out.push('"');
    }
    if let Some(name) = output.keysym.and_then(output_name) {
        out.push(' ');
        out.push_str(&name);
    }
}

/// How a keysym is written on the right-hand side, which takes only identifiers: names
/// such as `0` and `0x…` do not lex there, so digits fall back to the `U0030` form. A
/// keysym with neither form (never one read from a file) is left out.
fn output_name(keysym: uc_keysym::Keysym) -> Option<String> {
    let name = uc_keysym::name(keysym);
    if name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        return Some(name.into_owned());
    }
    let c = uc_keysym::to_char(keysym).filter(|c| !c.is_control())?;
    let unicode = format!("U{:04X}", u32::from(c));
    (uc_keysym::from_name(&unicode) == Some(keysym)).then_some(unicode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::includes::MapIncludes;
    use crate::load;

    #[test]
    fn writes_names_escapes_and_keysyms() {
        let mut rules = RuleSet::new();
        let multi = uc_keysym::sym::MULTI_KEY;
        rules.add(&[multi, 0x22, 0x5c], Output { text: Some("a\"b\\c\nd".into()), keysym: None });
        rules.add(&[multi, 0x0101_D538], Output { text: None, keysym: Some(0xe9) });
        assert_eq!(
            write(&rules),
            "<Multi_key> <quotedbl> <backslash> : \"a\\\"b\\\\c\\012d\"\n<Multi_key> <U1D538> : eacute\n"
        );
    }

    #[test]
    fn digit_keysyms_are_written_in_a_form_the_right_hand_side_accepts() {
        assert_eq!(output_name(0x30).as_deref(), Some("U0030"));
        assert_eq!(output_name(0xe9).as_deref(), Some("eacute"));
        assert_eq!(output_name(0x0100_2192).as_deref(), Some("U2192"));
    }

    #[test]
    fn output_parses_back_to_the_same_rules() {
        let original = crate::bundled::load_en_us().rules;
        let text = write(&original);
        let again = load(&text, "written", &mut MapIncludes::default()).unwrap();
        assert!(again.diagnostics.is_empty(), "{:?}", &again.diagnostics[..1]);
        assert_eq!(again.rules.rules(), original.rules());
    }
}
