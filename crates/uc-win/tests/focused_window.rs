//! Types into a real window and checks the WM_CHAR stream it receives, under the current
//! keyboard layout and under layouts where the same keys type something else.
//!
//! Ignored by default: it takes keyboard focus and briefly switches layout. It checks focus
//! before every injection and fails rather than type into another window.
//! Run with `cargo test -p uc-win -- --ignored`.
#![cfg(windows)]

mod common;

use common::{send_key, Layout, Window};
use uc_platform::TextSink;
use uc_win::SendInputSink;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;

/// ASCII, a digit, BMP, an astral math letter (a surrogate pair) and Enter.
const TEXT: &str = "a1α𝔸\n";
/// What a Unicode window receives for TEXT: one WM_CHAR per UTF-16 unit, Enter as CR.
const EXPECTED: [u16; 6] = [0x61, 0x31, 0x3B1, 0xD835, 0xDD38, 0x0D];

/// A layout to type under. Pressing `vk` on it types `control`, which differs from UK/US,
/// so the control press proves the layout is really active.
struct Case {
    name: &'static str,
    klid: &'static str,
    vk: VIRTUAL_KEY,
    control: u16,
}

const CASES: [Case; 2] = [
    Case { name: "Hebrew", klid: "0000040D", vk: b'A' as VIRTUAL_KEY, control: 0x05E9 }, // ש
    Case { name: "French AZERTY", klid: "0000040C", vk: b'1' as VIRTUAL_KEY, control: 0x26 }, // &
];

#[test]
#[ignore = "takes keyboard focus; run with --ignored on an interactive desktop"]
fn types_the_same_text_under_every_layout() {
    let window = Window::create();
    window.take_focus();
    let mut sink = SendInputSink::default();

    let typed = window.chars_from(EXPECTED.len(), || sink.type_text(TEXT).unwrap());
    assert_eq!(typed, EXPECTED, "under the current layout");

    for case in &CASES {
        let _layout = Layout::activate(case.klid);
        let control = window.chars_from(1, || send_key(case.vk));
        assert_eq!(control, [case.control], "{} is not active (is Caps Lock on?)", case.name);
        let typed = window.chars_from(EXPECTED.len(), || sink.type_text(TEXT).unwrap());
        assert_eq!(typed, EXPECTED, "under {}", case.name);
    }
}
