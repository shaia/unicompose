//! The rule table the engine walks, keyed by what a key types rather than by keysym.

use std::sync::Arc;

use uc_keysym::Keysym;
use uc_xcompose::trie::{NodeId, Trie};
use uc_xcompose::RuleSet;

/// A key as the engine sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// A key that types this printable character on the active layout.
    Char(char),
    /// Any other key: Compose, dead keys, Return, Escape, Backspace.
    Sym(Keysym),
}

impl Key {
    /// How a rule's keysym is matched: by the printable character it stands for, so
    /// `<Greek_alpha>` and `<U03B1>` are the same key and match whatever layout types α;
    /// otherwise by keysym.
    pub fn from_keysym(keysym: Keysym) -> Key {
        match uc_keysym::to_char(keysym) {
            Some(c) if !c.is_control() => Key::Char(c),
            _ => Key::Sym(keysym),
        }
    }
}

/// Compose rules ready for the engine. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Table {
    trie: Arc<Trie<Key, String>>,
    len: usize,
}

impl Table {
    /// Builds the table from parsed rules. Rules that type no text are left out. When two
    /// rules come to the same keys, such as `<Greek_alpha>` and `<U03B1>`, the one listed
    /// later by `RuleSet::rules` (the higher keysym) wins.
    pub fn from_rules(rules: &RuleSet) -> Table {
        let mut trie = Trie::new();
        let mut len = 0;
        for rule in rules.rules() {
            let Some(text) = rule.output.text() else { continue };
            let node = rule
                .keys
                .iter()
                .fold(trie.root(), |node, &k| trie.child_or_insert(node, Key::from_keysym(k)));
            if trie.set_value(node, Some(text.into_owned())).is_none() {
                len += 1;
            }
        }
        Table { trie: Arc::new(trie), len }
    }

    /// libX11's en_US.UTF-8 rules.
    pub fn bundled() -> Table {
        Table::from_rules(&uc_xcompose::bundled::load_en_us().rules)
    }

    /// Number of sequences.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The text for a complete sequence of keys, including the leading Compose key.
    pub fn lookup(&self, keys: &[Key]) -> Option<&str> {
        self.trie.find(keys).and_then(|node| self.text(node))
    }

    pub(crate) fn root(&self) -> NodeId {
        self.trie.root()
    }

    pub(crate) fn child(&self, node: NodeId, key: Key) -> Option<NodeId> {
        self.trie.child(node, key)
    }

    pub(crate) fn has_children(&self, node: NodeId) -> bool {
        self.trie.has_children(node)
    }

    pub(crate) fn text(&self, node: NodeId) -> Option<&str> {
        self.trie.value(node).map(String::as_str)
    }
}
