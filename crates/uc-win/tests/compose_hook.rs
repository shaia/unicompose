//! Runs the real Compose key: installs the keyboard hook, presses keys the way a
//! keyboard does, and checks the characters a window receives.
//!
//! Ignored by default, like `focused_window`: it takes keyboard focus and switches
//! layout. Run with `cargo test -p uc-win --test compose_hook -- --ignored` while at the
//! desk.
#![cfg(windows)]

mod common;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use common::{send_keys, Layout, Window};
use uc_engine::{Options, Table};
use uc_win::compose::{Compose, ComposeConfig, ComposeKey};
use uc_win::QuirkSet;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_RETURN, VK_RMENU};

const UK: &str = "00000809";
const HEBREW: &str = "0000040D";

fn tap(vk: VIRTUAL_KEY) -> [(VIRTUAL_KEY, bool, bool); 2] {
    [(vk, false, false), (vk, false, true)]
}

/// Right Alt tapped on its own: the Compose key.
fn compose() -> [(VIRTUAL_KEY, bool, bool); 2] {
    [(VK_RMENU, true, false), (VK_RMENU, true, true)]
}

fn keys(text: &str) -> Vec<(VIRTUAL_KEY, bool, bool)> {
    text.bytes().flat_map(|b| tap(VIRTUAL_KEY::from(b.to_ascii_uppercase()))).collect()
}

fn sequence(parts: &[&[(VIRTUAL_KEY, bool, bool)]]) -> Vec<(VIRTUAL_KEY, bool, bool)> {
    parts.concat()
}

fn text(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

#[test]
#[ignore = "takes keyboard focus; run with --ignored on an interactive desktop"]
fn compose_key_types_through_the_hook() {
    // RUST_LOG=uc_win=trace shows every key event and what the hook decided.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let window = Window::create();
    window.take_focus();
    let config = ComposeConfig {
        table: Table::bundled(),
        key: ComposeKey::default(),
        options: Options::default(),
        discard_invalid: false,
        quirks: QuirkSet::NONE,
    };
    let _compose = Compose::start(config, true, Arc::new(AtomicBool::new(false))).expect("install the hook");

    let _uk = Layout::activate(UK);
    let degree = sequence(&[&compose(), &keys("oo")]);
    assert_eq!(text(&window.chars_from(1, || send_keys(&degree))), "°", "Compose o o");

    let arrow = sequence(&[&compose(), &keys("u2192"), &tap(VK_RETURN)]);
    assert_eq!(text(&window.chars_from(1, || send_keys(&arrow))), "→", "Compose u 2192 Enter");

    let invalid = sequence(&[&compose(), &keys("oq")]);
    assert_eq!(text(&window.chars_from(2, || send_keys(&invalid))), "oq", "a failed sequence types its keys");

    // Windows adds its fake Left Ctrl (scan 0x21D) to this Right Alt, as it does for a
    // physical one on a layout with AltGr.
    let four = VIRTUAL_KEY::from(b'4');
    let altgr = [(VK_RMENU, true, false), (four, false, false), (four, false, true), (VK_RMENU, true, true)];
    // One event at a time, as a person types: unicompose replays the held key after the
    // hook returns, so a whole batch would overtake the replay.
    let typed = window.chars_from(1, || {
        for event in altgr {
            send_keys(&[event]);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    });
    assert_eq!(text(&typed), "€", "Right Alt held is still AltGr");

    drop(_uk);
    let _hebrew = Layout::activate(HEBREW);
    let control = window.chars_from(1, || send_keys(&keys("a")));
    assert_eq!(text(&control), "ש", "Hebrew is not active");
    let arrow = sequence(&[&compose(), &keys("u2192"), &tap(VK_RETURN)]);
    assert_eq!(
        text(&window.chars_from(1, || send_keys(&arrow))),
        "→",
        "hex entry reads the keys on a Latin layout while Hebrew is active"
    );
}
