//! A set of compose rules with libxkbcommon's rules for conflicting sequences.

use std::borrow::Cow;

use uc_keysym::Keysym;

use crate::trie::Trie;

/// What a rule types: a string, a keysym, or both. When both are given, the string wins.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Output {
    pub text: Option<String>,
    pub keysym: Option<Keysym>,
}

impl Output {
    /// The text to type: the string, else the keysym's character. `None` if neither
    /// gives any text.
    pub fn text(&self) -> Option<Cow<'_, str>> {
        match (&self.text, self.keysym.and_then(uc_keysym::to_char)) {
            (Some(text), _) => Some(Cow::Borrowed(text)),
            (None, Some(c)) => Some(Cow::Owned(c.to_string())),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rule {
    pub keys: Vec<Keysym>,
    pub output: Output,
}

/// What `RuleSet::add` did with a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Added {
    New,
    /// Replaced the rule with the same keys.
    Replaced,
    /// Removed a shorter rule whose keys are a prefix of this one.
    ReplacedPrefix,
    /// Skipped: the same keys already give the same output.
    Duplicate,
    /// Skipped: longer rules start with these keys.
    IsPrefix,
}

impl Added {
    /// The warning libxkbcommon prints for this outcome, if any.
    pub fn warning(self) -> Option<&'static str> {
        match self {
            Added::New => None,
            Added::Replaced => Some("this compose sequence already exists; overriding"),
            Added::ReplacedPrefix => {
                Some("a sequence already exists which is a prefix of this sequence; overriding")
            }
            Added::Duplicate => Some("this compose sequence is a duplicate of another; skipping line"),
            Added::IsPrefix => Some("this compose sequence is a prefix of another; skipping line"),
        }
    }
}

/// Compose rules keyed by keysym sequence. No sequence is a prefix of another.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    trie: Trie<Keysym, Output>,
    len: usize,
}

impl RuleSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a rule the way libxkbcommon does: a later rule wins over an earlier one with
    /// the same keys or with a prefix of its keys, but a rule whose keys are a prefix of
    /// existing rules is skipped. `keys` must not be empty.
    pub fn add(&mut self, keys: &[Keysym], output: Output) -> Added {
        let (&last, init) = keys.split_last().expect("a rule has at least one key");
        let mut replaced_prefix = false;
        let mut node = self.trie.root();
        for &key in init {
            node = self.trie.child_or_insert(node, key);
            if self.trie.set_value(node, None).is_some() {
                self.len -= 1;
                replaced_prefix = true;
            }
        }
        let node = self.trie.child_or_insert(node, last);
        if let Some(existing) = self.trie.value(node) {
            if *existing == output {
                return Added::Duplicate;
            }
            // libxkbcommon overwrites only the parts the new rule gives, so a string
            // survives a later rule for the same keys that gives just a keysym.
            let merged = Output {
                text: output.text.or_else(|| existing.text.clone()),
                keysym: output.keysym.or(existing.keysym),
            };
            self.trie.set_value(node, Some(merged));
            return Added::Replaced;
        }
        if self.trie.has_children(node) {
            return Added::IsPrefix;
        }
        self.trie.set_value(node, Some(output));
        self.len += 1;
        if replaced_prefix {
            Added::ReplacedPrefix
        } else {
            Added::New
        }
    }

    pub fn get(&self, keys: &[Keysym]) -> Option<&Output> {
        self.trie.find(keys).and_then(|node| self.trie.value(node))
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every rule, in keysym order.
    pub fn rules(&self) -> Vec<Rule> {
        self.trie.entries().into_iter().map(|(keys, output)| Rule { keys, output: output.clone() }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Output {
        Output { text: Some(s.into()), keysym: None }
    }

    #[test]
    fn later_rule_with_the_same_keys_wins() {
        let mut set = RuleSet::new();
        assert_eq!(set.add(&[1, 2], text("a")), Added::New);
        assert_eq!(set.add(&[1, 2], text("a")), Added::Duplicate);
        assert_eq!(set.add(&[1, 2], text("b")), Added::Replaced);
        assert_eq!(set.get(&[1, 2]), Some(&text("b")));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn override_keeps_the_parts_the_new_rule_leaves_out() {
        let mut set = RuleSet::new();
        set.add(&[1], Output { text: Some("é".into()), keysym: Some(0xe9) });
        assert_eq!(set.add(&[1], Output { text: None, keysym: Some(0xe8) }), Added::Replaced);
        assert_eq!(set.get(&[1]), Some(&Output { text: Some("é".into()), keysym: Some(0xe8) }));
    }

    #[test]
    fn longer_rule_replaces_its_prefix_but_not_the_reverse() {
        let mut set = RuleSet::new();
        set.add(&[1, 2], text("short"));
        assert_eq!(set.add(&[1, 2, 3], text("long")), Added::ReplacedPrefix);
        assert_eq!(set.get(&[1, 2]), None);
        assert_eq!(set.add(&[1, 2], text("short again")), Added::IsPrefix);
        assert_eq!(set.add(&[1], text("shorter")), Added::IsPrefix);
        assert_eq!(set.len(), 1);
        assert_eq!(set.rules(), [Rule { keys: vec![1, 2, 3], output: text("long") }]);
    }

    #[test]
    fn output_text_prefers_the_string() {
        assert_eq!(text("x").text().as_deref(), Some("x"));
        let keysym_only = Output { text: None, keysym: Some(0x7e1) };
        assert_eq!(keysym_only.text().as_deref(), Some("α"));
        assert_eq!(Output { text: None, keysym: Some(0xfe51) }.text(), None);
    }
}
