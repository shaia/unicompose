use proptest::prelude::*;
use uc_xcompose::{load, MapIncludes};

use super::*;

const RETURN: Key = Key::Sym(sym::RETURN);
const ESCAPE: Key = Key::Sym(sym::ESCAPE);
const BACKSPACE: Key = Key::Sym(sym::BACKSPACE);

fn table(text: &str) -> Table {
    let loaded = load(text, "test", &mut MapIncludes::default()).unwrap();
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    Table::from_rules(&loaded.rules)
}

fn engine(table: Table) -> Engine {
    Engine::new(table, Options::default())
}

/// Taps Compose, then `keys`; returns the outcome of each key.
fn compose(engine: &mut Engine, keys: &[Key], now: Instant) -> Vec<Outcome> {
    assert_eq!(engine.compose(now), Outcome::Swallow);
    keys.iter().map(|&k| engine.key(k, now)).collect()
}

fn chars(text: &str) -> Vec<Key> {
    text.chars().map(Key::Char).collect()
}

/// `text` as keys, then Return.
fn entered(text: &str) -> Vec<Key> {
    let mut keys = chars(text);
    keys.push(RETURN);
    keys
}

fn typed(outcomes: &[Outcome]) -> Option<&str> {
    match outcomes.last()? {
        Outcome::Type(text) => Some(text),
        _ => None,
    }
}

fn is_invalid(outcomes: &[Outcome]) -> bool {
    matches!(outcomes.last(), Some(Outcome::Invalid(_)))
}

#[test]
fn keys_pass_through_when_not_composing() {
    let mut engine = engine(Table::bundled());
    assert_eq!(engine.key(Key::Char('o'), Instant::now()), Outcome::Pass);
    assert!(!engine.is_composing());
}

#[test]
fn familiar_sequences_type_their_symbol() {
    let mut engine = engine(Table::bundled());
    let now = Instant::now();
    for (keys, expected) in [("o\"", "ö"), ("->", "→"), ("<=", "≤"), ("oo", "°")] {
        let outcomes = compose(&mut engine, &chars(keys), now);
        assert_eq!(typed(&outcomes), Some(expected), "{keys}");
        assert!(outcomes[..outcomes.len() - 1].iter().all(|o| *o == Outcome::Swallow));
        assert!(!engine.is_composing());
    }
}

#[test]
fn unknown_sequence_is_invalid_and_returns_its_keys() {
    let mut engine = engine(Table::bundled());
    let outcomes = compose(&mut engine, &chars("o@"), Instant::now());
    assert_eq!(outcomes.last(), Some(&Outcome::Invalid(chars("o@"))));
    assert!(!engine.is_composing());
}

#[test]
fn escape_and_compose_cancel() {
    let mut engine = engine(Table::bundled());
    let now = Instant::now();
    assert_eq!(compose(&mut engine, &[Key::Char('o'), ESCAPE], now).last(), Some(&Outcome::Swallow));
    assert!(!engine.is_composing());
    compose(&mut engine, &chars("o"), now);
    assert_eq!(engine.compose(now), Outcome::Swallow);
    assert!(!engine.is_composing());
}

#[test]
fn backspace_takes_back_the_last_key() {
    let mut engine = engine(Table::bundled());
    let now = Instant::now();
    let keys = [Key::Char('e'), BACKSPACE, Key::Char('o'), Key::Char('"')];
    assert_eq!(typed(&compose(&mut engine, &keys, now)), Some("ö"));
    // With no keys left, Backspace cancels.
    compose(&mut engine, &[BACKSPACE], now);
    assert!(!engine.is_composing());
}

#[test]
fn hex_entry_types_any_code_point() {
    let mut engine = engine(Table::bundled());
    let now = Instant::now();
    assert_eq!(typed(&compose(&mut engine, &entered("u1D538"), now)), Some("𝔸"));
    assert_eq!(typed(&compose(&mut engine, &chars("U2192 "), now)), Some("→"));
}

#[test]
fn hex_entry_yields_to_rules_that_start_with_u() {
    let mut engine = engine(Table::bundled());
    assert_eq!(typed(&compose(&mut engine, &chars("u\""), Instant::now())), Some("ü"));
}

#[test]
fn hex_entry_rejects_bad_values_and_supports_backspace() {
    let mut engine = engine(Table::bundled());
    let now = Instant::now();
    assert!(is_invalid(&compose(&mut engine, &entered("ud800"), now)), "surrogate");
    assert!(is_invalid(&compose(&mut engine, &entered("u1"), now)), "control character");
    assert!(is_invalid(&compose(&mut engine, &chars("u1234567"), now)), "seven digits");
    let mut keys = chars("u2193");
    keys.extend([BACKSPACE, Key::Char('2'), RETURN]);
    assert_eq!(typed(&compose(&mut engine, &keys, now)), Some("→"));
}

#[test]
fn hex_entry_works_without_any_u_rules_and_can_be_turned_off() {
    let now = Instant::now();
    let mut engine = engine(table("<Multi_key> <a> <b> : \"x\"\n"));
    assert_eq!(typed(&compose(&mut engine, &entered("u3b1"), now)), Some("α"));

    let mut engine = Engine::new(Table::bundled(), Options { hex: false, ..Options::default() });
    assert!(is_invalid(&compose(&mut engine, &chars("u1"), now)));
}

#[test]
fn keysym_forms_of_a_character_match_the_same_key() {
    let mut engine = engine(table("<Multi_key> <Greek_alpha> <U2192> : \"x\"\n"));
    assert_eq!(typed(&compose(&mut engine, &chars("α→"), Instant::now())), Some("x"));
}

#[test]
fn u_and_a_hex_digit_wait_between_the_rule_and_hex_entry() {
    let now = Instant::now();
    let mut engine = engine(Table::bundled());
    // `u a` alone is ă, after the timeout or before an unrelated key.
    assert_eq!(compose(&mut engine, &chars("ua"), now), [Outcome::Swallow, Outcome::Swallow]);
    assert_eq!(engine.tick(engine.deadline().unwrap()), Some(Outcome::Type("ă".into())));
    assert_eq!(compose(&mut engine, &chars("uaz"), now).last(), Some(&Outcome::TypeThenPass("ă".into())));
    // More digits, or Enter, make it hex: U+A9 is ©; U+A is a control character.
    assert_eq!(typed(&compose(&mut engine, &entered("ua9"), now)), Some("©"));
    assert!(is_invalid(&compose(&mut engine, &entered("ua"), now)));
    assert_eq!(typed(&compose(&mut engine, &entered("uE9"), now)), Some("é"));
}

#[test]
fn compose_twice_continues_into_rules_that_start_with_it() {
    let now = Instant::now();
    let mut engine = engine(table("<Multi_key> <Multi_key> <o> <k> : \"🆗\"\n<Multi_key> <o> <o> : \"°\"\n"));
    assert_eq!(engine.compose(now), Outcome::Swallow);
    assert_eq!(engine.compose(now), Outcome::Swallow);
    assert!(engine.is_composing());
    let outcomes: Vec<Outcome> = chars("ok").into_iter().map(|k| engine.key(k, now)).collect();
    assert_eq!(typed(&outcomes), Some("🆗"));
    // Without such rules, a second Compose cancels.
    let mut plain = Engine::new(Table::bundled(), Options::default());
    plain.compose(now);
    assert_eq!(plain.compose(now), Outcome::Swallow);
    assert!(!plain.is_composing());
}

#[test]
fn reference_layout_reading_is_used_only_where_the_active_one_fits_nothing() {
    let now = Instant::now();
    let mut engine = engine(Table::bundled());
    // Hebrew: the U key types ו and the E key ק; digits and Enter are the same.
    engine.compose(now);
    for (active, reference) in [('ו', 'u'), ('1', '1'), ('ק', 'e'), ('9', '9')] {
        assert_eq!(engine.key_or(Key::Char(active), Some(Key::Char(reference)), now), Outcome::Swallow);
    }
    assert_eq!(engine.key_or(RETURN, Some(RETURN), now), Outcome::Type("\u{1E9}".into()));
    // A Latin sequence typed on Hebrew: the O key types ם, which fits no rule.
    engine.compose(now);
    engine.key_or(Key::Char('ם'), Some(Key::Char('o')), now);
    assert_eq!(engine.key_or(Key::Char('ם'), Some(Key::Char('o')), now), Outcome::Type("°".into()));
    // The active reading wins when it fits.
    engine.compose(now);
    engine.key_or(Key::Char('-'), Some(Key::Char('/')), now);
    assert_eq!(engine.key_or(Key::Char('>'), Some(Key::Char('.')), now), Outcome::Type("→".into()));
}

/// A table where `<Multi_key> α` is complete but `<Multi_key> α b` continues it.
/// libxkbcommon's rules never keep both for one keysym, but two keysyms that type the
/// same character can produce this.
fn ambiguous() -> Table {
    table("<Multi_key> <Greek_alpha> : \"short\"\n<Multi_key> <U03B1> <b> : \"long\"\n")
}

#[test]
fn ambiguous_sequence_waits_then_types_on_timeout() {
    let mut engine = engine(ambiguous());
    let start = Instant::now();
    assert_eq!(compose(&mut engine, &chars("α"), start), [Outcome::Swallow]);
    let deadline = engine.deadline().expect("waiting");
    assert_eq!(deadline, start + Options::default().ambiguity_timeout);
    assert_eq!(engine.tick(deadline - Duration::from_millis(1)), None);
    assert_eq!(engine.tick(deadline), Some(Outcome::Type("short".into())));
    assert!(!engine.is_composing());
}

#[test]
fn ambiguous_sequence_continues_or_types_before_an_unrelated_key() {
    let mut engine = engine(ambiguous());
    let now = Instant::now();
    assert_eq!(typed(&compose(&mut engine, &chars("αb"), now)), Some("long"));
    assert_eq!(compose(&mut engine, &chars("αz"), now).last(), Some(&Outcome::TypeThenPass("short".into())));
    compose(&mut engine, &chars("α"), now);
    assert_eq!(engine.compose(now), Outcome::Type("short".into()), "Compose does not lose it");
}

#[test]
fn of_two_colliding_keysyms_the_higher_wins() {
    let t = table("<Multi_key> <Greek_alpha> : \"named\"\n<Multi_key> <U03B1> : \"unicode\"\n");
    assert_eq!(t.lookup(&[Key::Sym(sym::MULTI_KEY), Key::Char('α')]), Some("unicode"));
    assert_eq!(t.len(), 1);
}

#[test]
fn every_bundled_compose_rule_can_be_typed() {
    let table = Table::bundled();
    let mut engine = engine(table.clone());
    let now = Instant::now();
    let rules = uc_xcompose::bundled::load_en_us().rules.rules();
    let mut checked = 0;
    for rule in rules.iter().filter(|r| r.keys[0] == sym::MULTI_KEY) {
        let keys: Vec<Key> = rule.keys.iter().map(|&k| Key::from_keysym(k)).collect();
        let mut outcomes = compose(&mut engine, &keys[1..], now);
        // `u` + a hex digit (`u a` is ă) waits in case more digits follow.
        if let Some(deadline) = engine.deadline() {
            outcomes.extend(engine.tick(deadline));
        }
        assert_eq!(typed(&outcomes), table.lookup(&keys), "{rule:?}");
        checked += 1;
    }
    assert!(checked > 3000, "only {checked} rules start with Multi_key");
}

fn any_key() -> impl Strategy<Value = Key> {
    prop_oneof![
        4 => prop::sample::select(chars("ou\"'-<>=aeU1234dD bz~^α")),
        1 => any::<char>().prop_map(Key::Char),
        1 => prop::sample::select(vec![RETURN, ESCAPE, BACKSPACE, Key::Sym(0xfe51), Key::Sym(sym::KP_ENTER)]),
    ]
}

#[derive(Debug, Clone)]
enum Action {
    Compose,
    Key(Key),
    Wait(u64),
}

fn any_action() -> impl Strategy<Value = Action> {
    prop_oneof![
        1 => Just(Action::Compose),
        6 => any_key().prop_map(Action::Key),
        1 => (0u64..2000).prop_map(Action::Wait),
    ]
}

/// Random input never panics, keys pass while idle, finished sequences end, and timeouts
/// fire exactly when due. The tables are built once, outside the generated cases.
#[test]
fn random_input_keeps_the_engine_consistent() {
    let tables = [Table::bundled(), ambiguous()];
    let strategy = prop::collection::vec(any_action(), 0..60);
    let mut runner = proptest::test_runner::TestRunner::default();
    let result = runner.run(&strategy, |actions| {
        for table in &tables {
            let mut engine = engine(table.clone());
            let mut now = Instant::now();
            for action in &actions {
                let was_composing = engine.is_composing();
                match *action {
                    Action::Compose => {
                        let _ = engine.compose(now);
                    }
                    Action::Key(key) => {
                        let outcome = engine.key(key, now);
                        if !was_composing {
                            prop_assert_eq!(&outcome, &Outcome::Pass);
                        }
                        if matches!(
                            outcome,
                            Outcome::Type(_) | Outcome::TypeThenPass(_) | Outcome::Invalid(_)
                        ) {
                            prop_assert!(!engine.is_composing());
                        }
                    }
                    Action::Wait(ms) => {
                        now += Duration::from_millis(ms);
                        if let Some(deadline) = engine.deadline() {
                            prop_assert_eq!(engine.tick(now).is_some(), now >= deadline);
                        }
                    }
                }
            }
            let _ = engine.key(ESCAPE, now);
            prop_assert!(!engine.is_composing(), "Escape always gets back to idle");
        }
        Ok(())
    });
    if let Err(e) = result {
        panic!("{e}");
    }
}
