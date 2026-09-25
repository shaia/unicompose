//! The Compose key's state machine.
//!
//! The platform layer reports two things: the Compose key was tapped ([`Engine::compose`]),
//! and some other key was pressed ([`Engine::key`]). Each call returns an [`Outcome`]:
//! whether to let the key through, swallow it, or type text instead. The engine does no IO
//! and reads no clock; callers pass the time in, and call [`Engine::tick`] when
//! [`Engine::deadline`] passes. That keeps it deterministic, so it is tested exhaustively.
//!
//! While composing:
//! - Keys walk the rule table. A key that completes a rule types its text.
//! - `u` then hex digits then Enter or Space types any code point (`u 1 d 5 3 8` → 𝔸).
//!   The digits count only where no rule continues with them.
//! - Escape or Compose cancels; Backspace takes back the last key.
//! - A key that fits no rule ends the sequence as [`Outcome::Invalid`].
//! - A sequence that is complete but could also continue waits for the next key or for
//!   the timeout.

mod table;

use std::time::{Duration, Instant};

use uc_keysym::sym;

pub use table::{Key, Table};
use uc_xcompose::trie::NodeId;

/// What to do with the key that was just reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Not ours: deliver the key as usual.
    Pass,
    /// Consumed; nothing to type yet.
    Swallow,
    /// Consumed; type this text.
    Type(String),
    /// Type this text (a waiting sequence that the key did not continue), then deliver the
    /// key as usual.
    TypeThenPass(String),
    /// Consumed; the sequence matched nothing. These are its keys after Compose, so the
    /// caller can type them instead, or beep.
    Invalid(Vec<Key>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Allow `u` + hex digits + Enter.
    pub hex: bool,
    /// How long a sequence that is complete but could continue waits for another key.
    pub ambiguity_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options { hex: true, ambiguity_timeout: Duration::from_millis(1000) }
    }
}

/// Longest hex entry: six digits reach U+10FFFF.
const MAX_HEX_DIGITS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Idle,
    /// Walking the table: `keys` so far (after Compose) lead to `node`. `deadline` is set
    /// while `node` has text but also children.
    Sequence {
        keys: Vec<Key>,
        node: NodeId,
        deadline: Option<Instant>,
    },
    /// `u` then these hex digits.
    Hex {
        keys: Vec<Key>,
        digits: String,
    },
}

#[derive(Debug, Clone)]
pub struct Engine {
    table: Table,
    options: Options,
    state: State,
}

impl Engine {
    pub fn new(table: Table, options: Options) -> Self {
        Engine { table, options, state: State::Idle }
    }

    /// Swaps in new rules. A sequence in progress is cancelled.
    pub fn set_table(&mut self, table: Table) {
        self.table = table;
        self.state = State::Idle;
    }

    pub fn is_composing(&self) -> bool {
        self.state != State::Idle
    }

    /// When `tick` should next be called, if a sequence is waiting on a timeout.
    pub fn deadline(&self) -> Option<Instant> {
        match self.state {
            State::Sequence { deadline, .. } => deadline,
            _ => None,
        }
    }

    /// Abandons any sequence in progress without typing, for example when focus moves.
    pub fn cancel(&mut self) {
        self.state = State::Idle;
    }

    /// The Compose key was tapped: starts a sequence. In a sequence, it continues rules
    /// that use Compose twice (WinCompose's emoji names); otherwise it types a waiting
    /// complete sequence, or cancels.
    pub fn compose(&mut self, now: Instant) -> Outcome {
        let multi = Key::Sym(sym::MULTI_KEY);
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Idle => {
                let root = self.table.root();
                let node = self.table.child(root, multi).unwrap_or(root);
                self.state = self.enter(Vec::new(), node, now);
                Outcome::Swallow
            }
            State::Sequence { keys, node, deadline } if self.table.child(node, multi).is_some() => {
                self.sequence_key(keys, node, deadline, multi, now)
            }
            State::Sequence { node, deadline: Some(_), .. } => self.type_node(node),
            _ => Outcome::Swallow,
        }
    }

    /// A key other than Compose was pressed.
    pub fn key(&mut self, key: Key, now: Instant) -> Outcome {
        self.key_or(key, None, now)
    }

    /// Like `key`, with a second reading of the same key on a reference layout. `alt` is
    /// used where `key` fits nothing and `alt` does: on Hebrew, the key that types `ו`
    /// still starts hex entry as `u`, and Latin sequences keep working.
    pub fn key_or(&mut self, key: Key, alt: Option<Key>, now: Instant) -> Outcome {
        match std::mem::replace(&mut self.state, State::Idle) {
            State::Idle => Outcome::Pass,
            State::Sequence { keys, node, deadline } => {
                let key = match alt {
                    Some(alt)
                        if !self.sequence_accepts(&keys, node, key)
                            && self.sequence_accepts(&keys, node, alt) =>
                    {
                        alt
                    }
                    _ => key,
                };
                self.sequence_key(keys, node, deadline, key, now)
            }
            State::Hex { keys, digits } => {
                let key = match alt {
                    Some(alt) if !hex_accepts(key) && hex_accepts(alt) => alt,
                    _ => key,
                };
                self.hex_key(keys, digits, key, now)
            }
        }
    }

    /// Whether `key` continues the sequence `keys` at `node`, by a rule or by hex entry.
    fn sequence_accepts(&self, keys: &[Key], node: NodeId, key: Key) -> bool {
        if self.table.child(node, key).is_some() {
            return true;
        }
        self.options.hex
            && match keys {
                [] => is_u(key),
                [u] if is_u(*u) => hex_digit(key).is_some(),
                _ => self.is_hex_prefix(keys) && hex_accepts(key),
            }
    }

    /// `u` then one hex digit, which is also a complete rule in some tables (`u a` is ă).
    fn is_hex_prefix(&self, keys: &[Key]) -> bool {
        self.options.hex && matches!(keys, [u, d] if is_u(*u) && hex_digit(*d).is_some())
    }

    /// Types a waiting sequence once its timeout has passed.
    pub fn tick(&mut self, now: Instant) -> Option<Outcome> {
        match self.state {
            State::Sequence { node, deadline: Some(deadline), .. } if now >= deadline => {
                self.state = State::Idle;
                Some(self.type_node(node))
            }
            _ => None,
        }
    }

    fn sequence_key(
        &mut self,
        mut keys: Vec<Key>,
        node: NodeId,
        deadline: Option<Instant>,
        key: Key,
        now: Instant,
    ) -> Outcome {
        match key {
            Key::Sym(sym::ESCAPE) => return Outcome::Swallow,
            Key::Sym(sym::BACKSPACE) => {
                if keys.pop().is_some() {
                    self.state = self.rewalk(keys, now);
                }
                return Outcome::Swallow;
            }
            _ => {}
        }
        if let Some(next) = self.table.child(node, key) {
            keys.push(key);
            if self.table.has_children(next) || self.is_hex_prefix(&keys) {
                self.state = self.enter(keys, next, now);
                return Outcome::Swallow;
            }
            return self.type_node(next);
        }
        if self.is_hex_prefix(&keys) && hex_accepts(key) {
            // `u a` could be ă or the start of a code point: more digits make it hex.
            let digits = hex_digit(keys[1]).map(String::from).unwrap_or_default();
            return self.hex_key(keys, digits, key, now);
        }
        if deadline.is_some() {
            // Complete, and this key does not continue it: type it, then handle the key
            // as if no sequence had been in progress.
            return match self.type_node(node) {
                Outcome::Type(text) => Outcome::TypeThenPass(text),
                _ => Outcome::Pass,
            };
        }
        if self.options.hex {
            let after_u = keys.len() == 1 && is_u(keys[0]);
            if keys.is_empty() && is_u(key) {
                // No rule starts with `u` in this table; it can still start hex entry.
                keys.push(key);
                self.state = State::Hex { keys, digits: String::new() };
                return Outcome::Swallow;
            }
            if let Some(digit) = hex_digit(key).filter(|_| after_u) {
                keys.push(key);
                self.state = State::Hex { keys, digits: digit.to_string() };
                return Outcome::Swallow;
            }
        }
        keys.push(key);
        Outcome::Invalid(keys)
    }

    fn hex_key(&mut self, mut keys: Vec<Key>, mut digits: String, key: Key, now: Instant) -> Outcome {
        match key {
            Key::Sym(sym::ESCAPE) => Outcome::Swallow,
            Key::Sym(sym::BACKSPACE) => {
                keys.pop();
                if digits.pop().is_some() && !digits.is_empty() {
                    self.state = State::Hex { keys, digits };
                } else {
                    // Out of digits: back to `u`, or to just after Compose.
                    self.state = self.rewalk(keys, now);
                }
                Outcome::Swallow
            }
            Key::Sym(sym::RETURN | sym::KP_ENTER) | Key::Char(' ') => {
                let c = u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .filter(|c| !c.is_control());
                match c {
                    Some(c) => Outcome::Type(c.to_string()),
                    None => {
                        keys.push(key);
                        Outcome::Invalid(keys)
                    }
                }
            }
            _ => match hex_digit(key) {
                Some(digit) if digits.len() < MAX_HEX_DIGITS => {
                    keys.push(key);
                    digits.push(digit);
                    self.state = State::Hex { keys, digits };
                    Outcome::Swallow
                }
                _ => {
                    keys.push(key);
                    Outcome::Invalid(keys)
                }
            },
        }
    }

    /// The state for having typed `keys` after Compose; used after Backspace.
    fn rewalk(&self, keys: Vec<Key>, now: Instant) -> State {
        let root = self.table.root();
        let start = self.table.child(root, Key::Sym(sym::MULTI_KEY)).unwrap_or(root);
        match keys.iter().try_fold(start, |node, &key| self.table.child(node, key)) {
            Some(node) => self.enter(keys, node, now),
            None => {
                // Only hex entry leaves the table: `u` + digits.
                let digits: String = keys.iter().skip(1).filter_map(|&k| hex_digit(k)).collect();
                State::Hex { keys, digits }
            }
        }
    }

    fn enter(&self, keys: Vec<Key>, node: NodeId, now: Instant) -> State {
        let waits =
            self.table.text(node).is_some() && (self.table.has_children(node) || self.is_hex_prefix(&keys));
        State::Sequence { keys, node, deadline: waits.then(|| now + self.options.ambiguity_timeout) }
    }

    fn type_node(&mut self, node: NodeId) -> Outcome {
        self.state = State::Idle;
        match self.table.text(node) {
            Some(text) => Outcome::Type(text.to_owned()),
            None => Outcome::Swallow,
        }
    }
}

/// Keys hex entry takes: a digit, or Enter or Space to finish.
fn hex_accepts(key: Key) -> bool {
    hex_digit(key).is_some() || matches!(key, Key::Sym(sym::RETURN | sym::KP_ENTER) | Key::Char(' '))
}

fn is_u(key: Key) -> bool {
    matches!(key, Key::Char('u' | 'U'))
}

fn hex_digit(key: Key) -> Option<char> {
    match key {
        Key::Char(c) if c.is_ascii_hexdigit() => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
