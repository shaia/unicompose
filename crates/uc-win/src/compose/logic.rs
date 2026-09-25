//! What the keyboard hook decides for each key event, without any Windows calls, so it
//! can be tested exhaustively.
//!
//! The Compose key counts only as a *tap*, with no other key pressed while it is down. If
//! another key comes first, the Compose key was a modifier (AltGr on UK and other
//! layouts): its press is given back to Windows ahead of that key, and everything carries
//! on as if nothing had been in the way.

use std::time::Instant;

use uc_engine::{Engine, Key, Outcome};
use uc_keysym::sym;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_LCONTROL, VK_PACKET, VK_RMENU};

use super::keys::ComposeKey;

/// The scan code Windows gives the fake Left Ctrl it sends ahead of Right Alt on layouts
/// with AltGr.
const FAKE_CTRL_SCAN: u32 = 0x21D;
/// Left Ctrl's real scan code, for giving back a fake Ctrl that turned out not to be one.
const CTRL_SCAN: u32 = 0x1D;

/// One low-level keyboard event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    pub vk: VIRTUAL_KEY,
    pub scan: u32,
    pub extended: bool,
    pub up: bool,
    /// Injected by unicompose itself.
    pub ours: bool,
}

/// Something to type once the hook has returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Injection {
    Text(String),
    /// Key events to replay as they were.
    Keys(Vec<KeyEvent>),
}

/// What to do with an event.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Decision {
    /// Hide the event from every application.
    pub swallow: bool,
    pub inject: Vec<Injection>,
}

impl Decision {
    fn pass() -> Self {
        Decision::default()
    }

    fn swallow() -> Self {
        Decision { swallow: true, inject: Vec::new() }
    }

    fn swallow_and(inject: Injection) -> Self {
        Decision { swallow: true, inject: vec![inject] }
    }
}

/// How the key in an event reads while composing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolved {
    /// Shift, Ctrl, Caps Lock and the like: they shape other keys and are not part of a
    /// sequence.
    Modifier,
    /// Pressed together with Ctrl, Alt or Windows: a shortcut, which ends the sequence.
    Shortcut,
    /// On the active layout, and on the reference layout when that differs.
    Key { key: Key, alt: Option<Key> },
}

/// Reads keys on the current keyboard layout.
pub(crate) trait Resolve {
    /// `altgr` asks for the key as typed with AltGr held, which Windows does not know
    /// about because the Compose key's press was swallowed.
    fn resolve(&mut self, event: &KeyEvent, altgr: bool) -> Resolved;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tap {
    Idle,
    /// The Compose key is down and nothing else has been pressed: its press, and whether
    /// Windows' fake Ctrl came with it (Right Alt on a layout with AltGr).
    Held(KeyEvent, bool),
    /// Another key was pressed while the Compose key was down.
    Modifier(GivenBack),
}

/// Whether the Compose key's press was replayed to Windows, which decides what happens to
/// its release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GivenBack {
    /// Kept hidden (it was part of a sequence), so its release is hidden too.
    No,
    /// Replayed, so its release passes.
    Yes,
    /// Replayed together with a Ctrl standing in for Windows' fake one. We pressed that
    /// Ctrl, so we release it: Windows' own fake Ctrl release is hidden, and ours follows
    /// the Compose key's release.
    WithCtrl,
}

pub(crate) struct Processor {
    engine: Engine,
    compose_key: ComposeKey,
    tap: Tap,
    /// Keys whose press was swallowed, so their release is swallowed too.
    swallowed: [bool; 256],
    /// A fake Ctrl was swallowed and Right Alt should follow.
    fake_ctrl_pending: bool,
    /// Drop the keys of a sequence that matched nothing instead of typing them.
    discard_invalid: bool,
}

impl Processor {
    pub fn new(engine: Engine, compose_key: ComposeKey, discard_invalid: bool) -> Self {
        Processor {
            engine,
            compose_key,
            tap: Tap::Idle,
            swallowed: [false; 256],
            fake_ctrl_pending: false,
            discard_invalid,
        }
    }

    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    #[cfg(test)]
    pub fn is_composing(&self) -> bool {
        self.engine.is_composing()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.engine.deadline()
    }

    /// Forgets any sequence in progress, for example when composing is paused.
    pub fn reset(&mut self) {
        self.engine.cancel();
        self.tap = Tap::Idle;
        self.fake_ctrl_pending = false;
    }

    pub fn tick(&mut self, now: Instant) -> Decision {
        match self.engine.tick(now) {
            Some(Outcome::Type(text)) => Decision { swallow: false, inject: vec![Injection::Text(text)] },
            _ => Decision::pass(),
        }
    }

    pub fn event(&mut self, event: KeyEvent, now: Instant, resolve: &mut dyn Resolve) -> Decision {
        if event.ours || event.vk == VK_PACKET {
            return Decision::pass();
        }
        let compose_is_ralt = self.compose_key.vk == VK_RMENU;
        if compose_is_ralt && event.vk == VK_LCONTROL && event.scan == FAKE_CTRL_SCAN {
            return self.fake_ctrl(event);
        }
        let is_compose_press = event.vk == self.compose_key.vk && !event.up;
        let fake_ctrl = std::mem::take(&mut self.fake_ctrl_pending);
        if fake_ctrl && !is_compose_press {
            // The swallowed Ctrl was not followed by Right Alt after all: give it back
            // ahead of this event.
            return Decision::swallow_and(Injection::Keys(vec![ctrl_press(), event]));
        }
        if event.vk == self.compose_key.vk {
            return self.compose_event(event, fake_ctrl, now);
        }
        if let (Tap::Held(press, fake_ctrl), false) = (self.tap, event.up) {
            // A key while the Compose key is down: it is a modifier. Outside a sequence,
            // replay its press ahead of this key so AltGr works. A replayed Right Alt gets
            // no fake Ctrl from Windows, so replay that too, or AltGr+4 becomes Alt+4.
            if !self.engine.is_composing() {
                self.tap = Tap::Modifier(if fake_ctrl { GivenBack::WithCtrl } else { GivenBack::Yes });
                let mut replay = if fake_ctrl { vec![ctrl_press()] } else { Vec::new() };
                replay.extend([press, event]);
                return Decision::swallow_and(Injection::Keys(replay));
            }
            self.tap = Tap::Modifier(GivenBack::No);
        }
        let index = usize::from(event.vk as u8);
        if event.up {
            return if std::mem::take(&mut self.swallowed[index]) {
                Decision::swallow()
            } else {
                Decision::pass()
            };
        }
        if !self.engine.is_composing() {
            return Decision::pass();
        }
        let altgr = compose_is_ralt && self.tap == Tap::Modifier(GivenBack::No);
        match resolve.resolve(&event, altgr) {
            Resolved::Modifier => Decision::pass(),
            Resolved::Shortcut => {
                self.engine.cancel();
                Decision::pass()
            }
            Resolved::Key { key, alt } => {
                let outcome = self.engine.key_or(key, alt, now);
                self.apply(outcome, Some(event))
            }
        }
    }

    fn fake_ctrl(&mut self, event: KeyEvent) -> Decision {
        match (event.up, self.tap) {
            (false, Tap::Idle) => {
                self.fake_ctrl_pending = true;
                Decision::swallow()
            }
            (true, Tap::Held(..) | Tap::Modifier(GivenBack::No | GivenBack::WithCtrl)) => Decision::swallow(),
            // AltGr in use: this is the system's own fake Ctrl for the replayed Right Alt.
            _ => Decision::pass(),
        }
    }

    fn compose_event(&mut self, event: KeyEvent, fake_ctrl: bool, now: Instant) -> Decision {
        match (event.up, self.tap) {
            (false, Tap::Idle) => {
                self.tap = Tap::Held(event, fake_ctrl);
                Decision::swallow()
            }
            // Auto-repeat while held.
            (false, Tap::Held(..)) => Decision::swallow(),
            (false, Tap::Modifier(GivenBack::No)) => Decision::swallow(),
            (false, Tap::Modifier(_)) => Decision::pass(),
            (true, Tap::Held(..)) => {
                self.tap = Tap::Idle;
                let outcome = self.engine.compose(now);
                self.apply(outcome, None)
            }
            (true, Tap::Modifier(given_back)) => {
                self.tap = Tap::Idle;
                match given_back {
                    GivenBack::No => Decision::swallow(),
                    GivenBack::Yes => Decision::pass(),
                    GivenBack::WithCtrl => {
                        let release = KeyEvent { up: true, ..ctrl_press() };
                        Decision { swallow: false, inject: vec![Injection::Keys(vec![release])] }
                    }
                }
            }
            (true, Tap::Idle) => Decision::pass(),
        }
    }

    /// Turns the engine's answer into a decision. `event` is the key press that led to
    /// it, or `None` for the Compose key.
    fn apply(&mut self, outcome: Outcome, event: Option<KeyEvent>) -> Decision {
        let mut mark_swallowed = || {
            if let Some(event) = event {
                self.swallowed[usize::from(event.vk as u8)] = true;
            }
        };
        match outcome {
            Outcome::Pass => Decision::pass(),
            Outcome::Swallow => {
                mark_swallowed();
                Decision::swallow()
            }
            Outcome::Type(text) => {
                mark_swallowed();
                Decision::swallow_and(Injection::Text(text))
            }
            Outcome::TypeThenPass(text) => {
                // Typed text goes out after the hook returns, so the key must follow it
                // rather than pass now. Its release passes as usual.
                let mut inject = vec![Injection::Text(text)];
                inject.extend(event.map(|e| Injection::Keys(vec![e])));
                Decision { swallow: true, inject }
            }
            Outcome::Invalid(keys) => {
                mark_swallowed();
                let text = replay(&keys);
                if self.discard_invalid || text.is_empty() {
                    Decision::swallow()
                } else {
                    Decision::swallow_and(Injection::Text(text))
                }
            }
        }
    }
}

/// Left Ctrl pressed, standing in for the fake Ctrl Windows sends ahead of AltGr.
fn ctrl_press() -> KeyEvent {
    KeyEvent { vk: VK_LCONTROL, scan: CTRL_SCAN, extended: false, up: false, ours: false }
}

/// The text a failed sequence's keys would have typed without Compose.
fn replay(keys: &[Key]) -> String {
    keys.iter()
        .filter_map(|key| match *key {
            Key::Char(c) => Some(c),
            Key::Sym(sym::RETURN | sym::KP_ENTER) => Some('\n'),
            Key::Sym(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uc_engine::{Options, Table};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        VK_CAPITAL, VK_LSHIFT, VK_OEM_7, VK_RETURN, VK_SPACE,
    };

    const VK_4: VIRTUAL_KEY = b'4' as VIRTUAL_KEY;
    const VK_CTRL_C: VIRTUAL_KEY = b'C' as VIRTUAL_KEY;

    /// A US-like layout: letters and digits type themselves, OEM_7 types `"`, and Ctrl
    /// held with `C` is a shortcut.
    struct Us {
        calls: Vec<bool>,
    }

    impl Resolve for Us {
        fn resolve(&mut self, event: &KeyEvent, altgr: bool) -> Resolved {
            self.calls.push(altgr);
            let key = match event.vk {
                VK_LSHIFT | VK_CAPITAL => return Resolved::Modifier,
                VK_CTRL_C => return Resolved::Shortcut,
                VK_4 if altgr => Key::Char('€'),
                VK_OEM_7 => Key::Char('"'),
                VK_RETURN => Key::Sym(sym::RETURN),
                VK_SPACE => Key::Char(' '),
                vk => Key::Char(char::from(vk as u8).to_ascii_lowercase()),
            };
            Resolved::Key { key, alt: None }
        }
    }

    fn processor(key: &str) -> Processor {
        let table = uc_xcompose::bundled::load_en_us().rules;
        let mut engine_table = Table::from_rules(&table);
        if key == "euro" {
            let loaded = uc_xcompose::load(
                "<Multi_key> <EuroSign> <EuroSign> : \"€€\"\n",
                "test",
                &mut uc_xcompose::MapIncludes::default(),
            )
            .unwrap();
            engine_table = Table::from_rules(&loaded.rules);
        }
        Processor::new(Engine::new(engine_table, Options::default()), ComposeKey::default(), false)
    }

    fn ev(vk: VIRTUAL_KEY, up: bool) -> KeyEvent {
        KeyEvent { vk, scan: 0, extended: false, up, ours: false }
    }

    fn fake_ctrl(up: bool) -> KeyEvent {
        KeyEvent { vk: VK_LCONTROL, scan: FAKE_CTRL_SCAN, extended: false, up, ours: false }
    }

    /// Runs `events` and returns each decision.
    fn run(p: &mut Processor, events: &[KeyEvent]) -> Vec<Decision> {
        let mut us = Us { calls: Vec::new() };
        events.iter().map(|&e| p.event(e, Instant::now(), &mut us)).collect()
    }

    /// Right Alt as Windows reports it: extended, scan code 0x38.
    fn ralt(up: bool) -> KeyEvent {
        KeyEvent { vk: VK_RMENU, scan: 0x38, extended: true, up, ours: false }
    }

    fn ralt_down() -> KeyEvent {
        ralt(false)
    }

    fn tap_compose() -> Vec<KeyEvent> {
        vec![fake_ctrl(false), ralt(false), fake_ctrl(true), ralt(true)]
    }

    fn press(vk: VIRTUAL_KEY) -> [KeyEvent; 2] {
        [ev(vk, false), ev(vk, true)]
    }

    #[test]
    fn keys_pass_when_not_composing() {
        let mut p = processor("");
        assert!(run(&mut p, &press(b'A' as VIRTUAL_KEY)).iter().all(|d| *d == Decision::pass()));
    }

    #[test]
    fn a_tap_composes_and_every_event_of_the_sequence_is_hidden() {
        let mut p = processor("");
        let mut events = tap_compose();
        events.extend(press(b'O' as VIRTUAL_KEY));
        events.extend(press(VK_OEM_7));
        let decisions = run(&mut p, &events);
        assert!(decisions.iter().all(|d| d.swallow), "{decisions:#?}");
        let typed: Vec<&Injection> = decisions.iter().flat_map(|d| &d.inject).collect();
        assert_eq!(typed, [&Injection::Text("ö".into())]);
        assert!(!p.is_composing());
    }

    #[test]
    fn right_alt_with_a_key_is_altgr_and_is_given_back() {
        let mut p = processor("");
        let decisions = run(
            &mut p,
            &[fake_ctrl(false), ralt(false), ev(VK_4, false), ev(VK_4, true), fake_ctrl(true), ralt(true)],
        );
        assert!(decisions[0].swallow && decisions[1].swallow);
        let replay = vec![ctrl_press(), ralt_down(), ev(VK_4, false)];
        assert_eq!(decisions[2], Decision::swallow_and(Injection::Keys(replay)));
        // Windows now has AltGr down: the key's release passes. We pressed the Ctrl, so
        // we release it: Windows' fake Ctrl release is hidden, and ours follows Right Alt's.
        assert_eq!(decisions[3], Decision::pass());
        assert_eq!(decisions[4], Decision::swallow());
        let ctrl_release = KeyEvent { up: true, ..ctrl_press() };
        assert_eq!(
            decisions[5],
            Decision { swallow: false, inject: vec![Injection::Keys(vec![ctrl_release])] }
        );
        assert!(!p.is_composing());
    }

    #[test]
    fn right_alt_without_a_fake_ctrl_is_plain_alt_and_replays_alone() {
        // US layout: Right Alt is Alt, and Windows sends no fake Ctrl.
        let mut p = processor("");
        let decisions = run(&mut p, &[ralt(false), ev(VK_4, false)]);
        assert_eq!(decisions[1], Decision::swallow_and(Injection::Keys(vec![ralt_down(), ev(VK_4, false)])));
    }

    #[test]
    fn altgr_characters_can_be_part_of_a_sequence() {
        let mut p = processor("euro");
        let mut events = tap_compose();
        for _ in 0..2 {
            events.extend([fake_ctrl(false), ralt(false), ev(VK_4, false), ev(VK_4, true)]);
            events.extend([fake_ctrl(true), ralt(true)]);
        }
        let mut us = Us { calls: Vec::new() };
        let decisions: Vec<Decision> = events.iter().map(|&e| p.event(e, Instant::now(), &mut us)).collect();
        assert!(decisions.iter().all(|d| d.swallow), "nothing reaches the application: {decisions:#?}");
        assert_eq!(us.calls, [true, true], "both 4s are read with AltGr");
        assert_eq!(decisions.last().unwrap().inject, [], "the sequence ends on the second 4");
        assert!(decisions.iter().any(|d| d.inject == [Injection::Text("€€".into())]));
    }

    #[test]
    fn auto_repeat_of_the_held_compose_key_is_hidden() {
        let mut p = processor("");
        let decisions = run(&mut p, &[ralt(false), ralt(false), ralt(true)]);
        assert!(decisions.iter().all(|d| d.swallow));
        assert!(p.is_composing());
    }

    #[test]
    fn a_failed_sequence_types_its_keys() {
        let mut p = processor("");
        let mut events = tap_compose();
        events.extend(press(b'O' as VIRTUAL_KEY));
        events.extend(press(b'Q' as VIRTUAL_KEY));
        let injected: Vec<Injection> = run(&mut p, &events).into_iter().flat_map(|d| d.inject).collect();
        assert_eq!(injected, [Injection::Text("oq".into())]);

        let mut p =
            Processor::new(Engine::new(Table::bundled(), Options::default()), ComposeKey::default(), true);
        assert!(run(&mut p, &events).iter().all(|d| d.inject.is_empty()), "discarded");
    }

    #[test]
    fn a_shortcut_ends_the_sequence_and_passes() {
        let mut p = processor("");
        let mut events = tap_compose();
        events.push(ev(VK_CTRL_C, false));
        assert_eq!(run(&mut p, &events).last(), Some(&Decision::pass()));
        assert!(!p.is_composing());
    }

    #[test]
    fn modifiers_pass_while_composing() {
        let mut p = processor("");
        let mut events = tap_compose();
        events.push(ev(VK_LSHIFT, false));
        assert_eq!(run(&mut p, &events).last(), Some(&Decision::pass()));
        assert!(p.is_composing());
    }

    #[test]
    fn our_own_events_and_packets_always_pass() {
        let mut p = processor("");
        run(&mut p, &tap_compose());
        let ours = KeyEvent { ours: true, ..ev(b'O' as VIRTUAL_KEY, false) };
        let packet = ev(VK_PACKET, false);
        assert_eq!(run(&mut p, &[ours, packet]), [Decision::pass(), Decision::pass()]);
        assert!(p.is_composing(), "they are not part of the sequence");
    }

    #[test]
    fn a_fake_ctrl_without_right_alt_is_given_back() {
        let mut p = processor("");
        let a = ev(b'A' as VIRTUAL_KEY, false);
        let decisions = run(&mut p, &[fake_ctrl(false), a]);
        assert!(decisions[0].swallow);
        assert_eq!(decisions[1], Decision::swallow_and(Injection::Keys(vec![ctrl_press(), a])));
    }

    #[test]
    fn hex_entry_through_the_hook() {
        let mut p = processor("");
        let mut events = tap_compose();
        for vk in [b'U', b'2', b'1', b'9', b'2'] {
            events.extend(press(vk as VIRTUAL_KEY));
        }
        events.extend(press(VK_RETURN));
        let injected: Vec<Injection> = run(&mut p, &events).into_iter().flat_map(|d| d.inject).collect();
        assert_eq!(injected, [Injection::Text("→".into())]);
    }

    #[test]
    fn a_key_that_ends_an_ambiguous_wait_is_replayed_after_the_text() {
        let mut p = processor("");
        let mut events = tap_compose();
        events.extend(press(b'U' as VIRTUAL_KEY));
        events.push(ev(b'A' as VIRTUAL_KEY, false)); // `u a` is ă, or the start of hex
        let z = ev(b'Z' as VIRTUAL_KEY, false);
        events.push(z);
        let decisions = run(&mut p, &events);
        let last = decisions.last().unwrap();
        assert!(last.swallow);
        assert_eq!(last.inject, [Injection::Text("ă".into()), Injection::Keys(vec![z])]);
        // Its release passes, as the replayed press will reach the application.
        assert_eq!(run(&mut p, &[ev(b'Z' as VIRTUAL_KEY, true)]), [Decision::pass()]);
    }

    #[test]
    fn caps_lock_as_compose_key_taps_without_toggling() {
        let caps = *super::super::keys::COMPOSE_KEYS.iter().find(|k| k.name() == "capslock").unwrap();
        let mut p = Processor::new(Engine::new(Table::bundled(), Options::default()), caps, false);
        let decisions = run(&mut p, &press(VK_CAPITAL));
        assert!(decisions.iter().all(|d| d.swallow));
        assert!(p.is_composing());
    }
}
