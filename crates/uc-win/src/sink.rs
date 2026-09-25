//! Types text with `SendInput(KEYEVENTF_UNICODE)`, independent of the active keyboard layout.

use std::mem::size_of;

use uc_platform::{SinkError, TextSink};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CAPITAL, VK_LCONTROL, VK_LEFT,
    VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN,
};

use crate::quirks::{self, Quirk, QuirkSet};

/// Tags every event we inject, so our own keyboard hook can recognise and skip them.
pub const INJECTED_MARKER: usize = 0x5543_4F4D; // "UCOM"

/// An unassigned virtual key. Tapping it before releasing Alt or Win stops that release
/// from opening the menu bar or the Start menu (the same trick as AutoHotkey's
/// `A_MenuMaskKey`).
const VK_MASK: VIRTUAL_KEY = 0xE8;
const VK_U: VIRTUAL_KEY = b'U' as VIRTUAL_KEY;
const ZERO_WIDTH_SPACE: u16 = 0x200B;

/// Modifiers that turn a typed character into a shortcut, with their extended-key flag.
/// Shift is absent: it does not affect `VK_PACKET`.
const MODIFIERS: [(VIRTUAL_KEY, bool); 6] = [
    (VK_LCONTROL, false),
    (VK_RCONTROL, true),
    (VK_LMENU, false),
    (VK_RMENU, true),
    (VK_LWIN, true),
    (VK_RWIN, true),
];

/// Shift does matter to the keys some quirks press: arrows and GTK's Ctrl+Shift+U.
const SHIFTS: [(VIRTUAL_KEY, bool); 2] = [(VK_LSHIFT, false), (VK_RSHIFT, false)];

#[derive(Debug, Default)]
pub struct SendInputSink {
    quirks: QuirkSet,
    /// The foreground window last typed into and its quirk, so the window class is looked
    /// up only when focus moves.
    focus: Option<(usize, Option<Quirk>)>,
}

impl SendInputSink {
    pub fn new(quirks: QuirkSet) -> Self {
        Self { quirks, focus: None }
    }

    fn focused_quirk(&mut self) -> Option<Quirk> {
        if self.quirks == QuirkSet::NONE {
            return None;
        }
        let hwnd = quirks::foreground_window();
        match self.focus {
            Some((last, quirk)) if last == hwnd => quirk,
            _ => {
                let class = quirks::class_name(hwnd);
                let quirk = self.quirks.for_class(&class);
                match quirk {
                    Some(quirk) => tracing::debug!("focus moved to window class {class:?}: quirk {quirk}"),
                    None => tracing::debug!("focus moved to window class {class:?}"),
                }
                self.focus = Some((hwnd, quirk));
                quirk
            }
        }
    }
}

impl TextSink for SendInputSink {
    fn type_text(&mut self, text: &str) -> Result<(), SinkError> {
        let quirk = self.focused_quirk();
        let held = held_modifiers(releases_shift(quirk, text));
        let capslock = quirk == Some(Quirk::GtkAstral) && capslock_on();
        let inputs: Vec<INPUT> = plan(text, &held, quirk, capslock).into_iter().map(to_input).collect();
        send(&inputs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stroke {
    Key { vk: VIRTUAL_KEY, extended: bool, up: bool },
    Unit { unit: u16, up: bool },
}

fn held_modifiers(with_shift: bool) -> Vec<(VIRTUAL_KEY, bool)> {
    let shifts: &[_] = if with_shift { &SHIFTS } else { &[] };
    MODIFIERS
        .iter()
        .chain(shifts)
        .copied()
        // SAFETY: GetAsyncKeyState has no preconditions; the high bit means "down".
        .filter(|&(vk, _)| unsafe { GetAsyncKeyState(i32::from(vk)) } < 0)
        .collect()
}

fn capslock_on() -> bool {
    // SAFETY: GetKeyState has no preconditions; the low bit is the toggle state.
    unsafe { GetKeyState(i32::from(VK_CAPITAL)) & 1 != 0 }
}

fn is_astral(c: char) -> bool {
    u32::from(c) > 0xFFFF
}

/// Whether a held Shift would change what `quirk` types for `text`.
fn releases_shift(quirk: Option<Quirk>, text: &str) -> bool {
    match quirk {
        Some(Quirk::OfficeFont) => true,
        Some(Quirk::GtkAstral) => text.chars().any(is_astral),
        None => false,
    }
}

/// Builds the keystrokes for `text`, first releasing any `held` modifiers.
///
/// Modifiers are released but deliberately not pressed again: on AltGr layouts, Windows
/// adds a fake LCtrl to every RAlt event, so re-pressing both would leave LCtrl stuck
/// down after the user lets go. A later physical key-up of an already-released key is
/// harmless.
fn plan(text: &str, held: &[(VIRTUAL_KEY, bool)], quirk: Option<Quirk>, capslock_on: bool) -> Vec<Stroke> {
    let mut strokes = Vec::with_capacity(text.len() * 4 + held.len() + 6);
    if !held.is_empty() {
        tap(&mut strokes, VK_MASK, false);
        for &(vk, extended) in held {
            strokes.push(Stroke::Key { vk, extended, up: true });
        }
    }
    let office = quirk == Some(Quirk::OfficeFont);
    if office {
        // Type in front of a zero-width space, which carries the surrounding font.
        unit(&mut strokes, ZERO_WIDTH_SPACE);
        tap(&mut strokes, VK_LEFT, true);
    }
    let mut units = [0u16; 2];
    for c in text.chars() {
        match c {
            '\r' => {}
            '\n' => tap(&mut strokes, VK_RETURN, false),
            c if is_astral(c) && quirk == Some(Quirk::GtkAstral) => {
                gtk_hex_entry(&mut strokes, c, capslock_on)
            }
            c => {
                // Each UTF-16 unit, surrogates included, becomes its own WM_CHAR.
                for &u in c.encode_utf16(&mut units).iter() {
                    unit(&mut strokes, u);
                }
            }
        }
    }
    if office {
        tap(&mut strokes, VK_RIGHT, true);
    }
    strokes
}

/// GTK's Unicode entry: Ctrl+Shift+U, the hex digits, Enter. Caps Lock would turn the
/// entry off, so it is toggled off around it. The digits go as Unicode units rather than
/// virtual keys, because on Hebrew and similar layouts `VK_A`..`VK_F` type other letters.
fn gtk_hex_entry(strokes: &mut Vec<Stroke>, c: char, capslock_on: bool) {
    if capslock_on {
        tap(strokes, VK_CAPITAL, false);
    }
    strokes.push(Stroke::Key { vk: VK_LCONTROL, extended: false, up: false });
    strokes.push(Stroke::Key { vk: VK_LSHIFT, extended: false, up: false });
    tap(strokes, VK_U, false);
    strokes.push(Stroke::Key { vk: VK_LSHIFT, extended: false, up: true });
    strokes.push(Stroke::Key { vk: VK_LCONTROL, extended: false, up: true });
    for digit in format!("{:x}", u32::from(c)).bytes() {
        unit(strokes, u16::from(digit));
    }
    tap(strokes, VK_RETURN, false);
    if capslock_on {
        tap(strokes, VK_CAPITAL, false);
    }
}

fn tap(strokes: &mut Vec<Stroke>, vk: VIRTUAL_KEY, extended: bool) {
    strokes.push(Stroke::Key { vk, extended, up: false });
    strokes.push(Stroke::Key { vk, extended, up: true });
}

fn unit(strokes: &mut Vec<Stroke>, unit: u16) {
    strokes.push(Stroke::Unit { unit, up: false });
    strokes.push(Stroke::Unit { unit, up: true });
}

/// Replays key events as a keyboard would send them: `(vk, scan, extended, up)`. They
/// carry our marker, so our hook lets them through.
pub(crate) fn replay_keys(events: &[(VIRTUAL_KEY, u32, bool, bool)]) -> Result<(), SinkError> {
    let inputs: Vec<INPUT> = events
        .iter()
        .map(|&(vk, scan, extended, up)| {
            let mut flags = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
            if up {
                flags |= KEYEVENTF_KEYUP;
            }
            INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: vk,
                        wScan: (scan & 0xFF) as u16,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: INJECTED_MARKER,
                    },
                },
            }
        })
        .collect();
    send(&inputs)
}

fn send(inputs: &[INPUT]) -> Result<(), SinkError> {
    if inputs.is_empty() {
        return Ok(());
    }
    let count = u32::try_from(inputs.len()).map_err(|_| SinkError("too many events".into()))?;
    // SAFETY: `inputs` is a live, initialised slice of `count` INPUT structs, and the size
    // argument matches the struct SendInput expects.
    let sent = unsafe { SendInput(count, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if sent != count {
        // Also fails silently (sent == count) when UIPI blocks an elevated target.
        return Err(SinkError(format!(
            "SendInput inserted {sent} of {count} events: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

fn to_input(stroke: Stroke) -> INPUT {
    let (vk, scan, mut flags, up) = match stroke {
        Stroke::Key { vk, extended, up } => (vk, 0, if extended { KEYEVENTF_EXTENDEDKEY } else { 0 }, up),
        Stroke::Unit { unit, up } => (0, unit, KEYEVENTF_UNICODE, up),
    };
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: INJECTED_MARKER },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units(strokes: &[Stroke]) -> Vec<u16> {
        strokes
            .iter()
            .filter_map(|s| match *s {
                Stroke::Unit { unit, up: false } => Some(unit),
                _ => None,
            })
            .collect()
    }

    fn down(vk: VIRTUAL_KEY) -> Stroke {
        Stroke::Key { vk, extended: false, up: false }
    }

    fn up(vk: VIRTUAL_KEY) -> Stroke {
        Stroke::Key { vk, extended: false, up: true }
    }

    fn key_taps(strokes: &[Stroke]) -> Vec<Stroke> {
        strokes.iter().copied().filter(|s| matches!(s, Stroke::Key { .. })).collect()
    }

    #[test]
    fn bmp_char_is_one_down_up_pair() {
        assert_eq!(
            plan("α", &[], None, false),
            [Stroke::Unit { unit: 0x03B1, up: false }, Stroke::Unit { unit: 0x03B1, up: true }]
        );
    }

    #[test]
    fn astral_char_is_sent_as_surrogate_pair_in_order() {
        let strokes = plan("𝔸", &[], None, false);
        assert_eq!(strokes.len(), 4);
        assert_eq!(units(&strokes), [0xD835, 0xDD38]);
    }

    #[test]
    fn newline_becomes_return_and_cr_is_dropped() {
        assert_eq!(plan("\r\n", &[], None, false), [down(VK_RETURN), up(VK_RETURN)]);
    }

    #[test]
    fn held_modifiers_are_masked_then_released_and_not_restored() {
        let strokes = plan("≤", &[(VK_RMENU, true), (VK_LCONTROL, false)], None, false);
        assert_eq!(
            strokes[..4],
            [
                down(VK_MASK),
                up(VK_MASK),
                Stroke::Key { vk: VK_RMENU, extended: true, up: true },
                up(VK_LCONTROL),
            ]
        );
        assert_eq!(strokes.len(), 6);
        assert!(strokes[4..].iter().all(|s| matches!(s, Stroke::Unit { unit: 0x2264, .. })));
    }

    #[test]
    fn gtk_quirk_leaves_bmp_text_alone() {
        assert_eq!(plan("α≤", &[], Some(Quirk::GtkAstral), true), plan("α≤", &[], None, false));
        assert!(!releases_shift(Some(Quirk::GtkAstral), "α≤"));
    }

    #[test]
    fn gtk_quirk_types_astral_chars_through_hex_entry() {
        let strokes = plan("𝔸", &[], Some(Quirk::GtkAstral), false);
        assert_eq!(
            key_taps(&strokes),
            [
                down(VK_LCONTROL),
                down(VK_LSHIFT),
                down(VK_U),
                up(VK_U),
                up(VK_LSHIFT),
                up(VK_LCONTROL),
                down(VK_RETURN),
                up(VK_RETURN)
            ]
        );
        assert_eq!(String::from_utf16(&units(&strokes)).unwrap(), "1d538");
        assert!(releases_shift(Some(Quirk::GtkAstral), "a𝔸"));
    }

    #[test]
    fn gtk_quirk_turns_caps_lock_off_around_the_entry() {
        let strokes = plan("𝔸", &[], Some(Quirk::GtkAstral), true);
        assert_eq!(strokes[..2], [down(VK_CAPITAL), up(VK_CAPITAL)]);
        assert_eq!(strokes[strokes.len() - 2..], [down(VK_CAPITAL), up(VK_CAPITAL)]);
    }

    #[test]
    fn office_quirk_types_in_front_of_a_zero_width_space() {
        let strokes = plan("≤", &[], Some(Quirk::OfficeFont), false);
        let left = Stroke::Key { vk: VK_LEFT, extended: true, up: false };
        let right = Stroke::Key { vk: VK_RIGHT, extended: true, up: false };
        assert_eq!(units(&strokes), [ZERO_WIDTH_SPACE, 0x2264]);
        assert_eq!(strokes[2], left);
        assert_eq!(strokes[strokes.len() - 2], right);
        assert!(releases_shift(Some(Quirk::OfficeFont), "≤"));
    }

    #[test]
    fn every_injected_event_carries_the_marker() {
        for stroke in plan("a\n𝔸", &[(VK_LWIN, true)], Some(Quirk::GtkAstral), true) {
            let input = to_input(stroke);
            // SAFETY: to_input always fills the `ki` member.
            let ki = unsafe { input.Anonymous.ki };
            assert_eq!(ki.dwExtraInfo, INJECTED_MARKER);
        }
    }
}
