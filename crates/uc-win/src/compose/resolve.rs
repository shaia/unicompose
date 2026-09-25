//! Reads what a key types on the foreground window's keyboard layout, and on a Latin
//! reference layout.

use std::ptr::null_mut;

use uc_engine::Key;
use uc_keysym::sym;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, GetKeyboardLayout, GetKeyboardLayoutList, MapVirtualKeyExW, ToUnicodeEx,
    HKL, MAPVK_VSC_TO_VK_EX, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END,
    VK_ESCAPE, VK_F1, VK_F24, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_MENU, VK_NEXT, VK_NUMLOCK, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN,
    VK_SCROLL, VK_SHIFT, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use super::logic::{KeyEvent, Resolve, Resolved};

/// Leave the kernel's keyboard state alone, so reading a key does not eat a pending dead
/// key in the application (Windows 10 1607 and later).
const TO_UNICODE_NO_STATE_CHANGE: u32 = 0x4;
/// Keys that type nothing get a keysym outside every real range, so they end a sequence.
const UNKNOWN_KEY: u32 = 0x2000_0000;

pub(crate) struct LayoutResolver {
    /// A Latin layout for hex digits and Latin sequences while another script is active.
    reference: Option<HKL>,
}

impl LayoutResolver {
    pub fn new() -> Self {
        let reference = find_reference_layout();
        match reference {
            Some(hkl) => tracing::debug!("reference layout {:08x}", hkl as usize),
            None => tracing::info!("no Latin keyboard layout installed; hex entry follows the active layout"),
        }
        LayoutResolver { reference }
    }
}

impl Resolve for LayoutResolver {
    fn resolve(&mut self, event: &KeyEvent, altgr: bool) -> Resolved {
        if is_modifier(event.vk) {
            return Resolved::Modifier;
        }
        let ctrl = is_down(VK_CONTROL);
        let alt = is_down(VK_MENU);
        let win = is_down(VK_LWIN) || is_down(VK_RWIN);
        // Ctrl and Alt together are AltGr; either alone, or Windows, makes a shortcut.
        if !altgr && (win || ctrl != alt) {
            return Resolved::Shortcut;
        }
        if let Some(keysym) = special_keysym(event) {
            return Resolved::Key { key: Key::Sym(keysym), alt: None };
        }
        let state = keyboard_state(altgr || (ctrl && alt));
        let active = foreground_layout();
        let key = to_char(event.vk, event.scan, &state, active)
            .map_or(Key::Sym(UNKNOWN_KEY | u32::from(event.vk)), Key::Char);
        let alt = self.reference.filter(|&hkl| hkl != active).and_then(|hkl| {
            let vk = reference_vk(event, hkl);
            to_char(vk, event.scan, &state, hkl).map(Key::Char)
        });
        Resolved::Key { key, alt }
    }
}

fn is_down(vk: VIRTUAL_KEY) -> bool {
    // SAFETY: GetAsyncKeyState has no preconditions; the high bit means "down".
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

fn is_modifier(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_SHIFT
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_CONTROL
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_MENU
            | VK_LMENU
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
            | VK_CAPITAL
            | VK_NUMLOCK
            | VK_SCROLL
    )
}

/// Keys whose meaning does not depend on the layout, as the keysyms rules use for them.
fn special_keysym(event: &KeyEvent) -> Option<u32> {
    Some(match event.vk {
        VK_RETURN if event.extended => sym::KP_ENTER,
        VK_RETURN => sym::RETURN,
        VK_ESCAPE => sym::ESCAPE,
        VK_BACK => sym::BACKSPACE,
        VK_TAB => sym::TAB,
        VK_DELETE => sym::DELETE,
        VK_HOME => 0xff50,
        VK_LEFT => 0xff51,
        VK_UP => 0xff52,
        VK_RIGHT => 0xff53,
        VK_DOWN => 0xff54,
        VK_PRIOR => 0xff55,
        VK_NEXT => 0xff56,
        VK_END => 0xff57,
        VK_INSERT => 0xff63,
        vk @ VK_F1..=VK_F24 => 0xffbe + u32::from(vk - VK_F1),
        _ => return None,
    })
}

/// The key state to read a key with: the real Shift and Caps Lock, plus AltGr if asked.
fn keyboard_state(altgr: bool) -> [u8; 256] {
    let mut state = [0u8; 256];
    for vk in [VK_SHIFT, VK_LSHIFT, VK_RSHIFT] {
        if is_down(vk) {
            state[usize::from(vk)] = 0x80;
        }
    }
    // SAFETY: GetKeyState has no preconditions; the low bit is the toggle.
    if unsafe { GetKeyState(i32::from(VK_CAPITAL)) } & 1 != 0 {
        state[usize::from(VK_CAPITAL)] = 0x01;
    }
    if altgr {
        for vk in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU] {
            state[usize::from(vk)] = 0x80;
        }
    }
    state
}

fn foreground_layout() -> HKL {
    // SAFETY: both calls accept any window handle, including null; a null process-ID
    // pointer is allowed.
    unsafe {
        let window = GetForegroundWindow();
        let thread = if window.is_null() { 0 } else { GetWindowThreadProcessId(window, null_mut()) };
        GetKeyboardLayout(thread)
    }
}

/// The printable character `vk` types on `hkl`, if exactly one. A dead key counts as the
/// character it shows.
fn to_char(vk: VIRTUAL_KEY, scan: u32, state: &[u8; 256], hkl: HKL) -> Option<char> {
    let mut buf = [0u16; 8];
    // SAFETY: `state` has the 256 entries ToUnicodeEx reads, and `buf` is writable for the
    // length we pass.
    let n = unsafe {
        ToUnicodeEx(
            u32::from(vk),
            scan,
            state.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as i32,
            TO_UNICODE_NO_STATE_CHANGE,
            hkl,
        )
    };
    let len = match n {
        -1 => 1,
        n if n > 0 => n as usize,
        _ => return None,
    };
    let mut chars = char::decode_utf16(buf[..len.min(buf.len())].iter().copied()).filter_map(Result::ok);
    let c = chars.next()?;
    (chars.next().is_none() && !c.is_control()).then_some(c)
}

/// The key at the same position on the reference layout: what firmware built for that
/// layout meant when it sent this scan code.
fn reference_vk(event: &KeyEvent, hkl: HKL) -> VIRTUAL_KEY {
    if event.scan == 0 {
        return event.vk;
    }
    let scan = if event.extended { 0xE000 | event.scan } else { event.scan };
    // SAFETY: MapVirtualKeyExW takes any code and layout handle and returns 0 on failure.
    let vk = unsafe { MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK_EX, hkl) };
    VIRTUAL_KEY::try_from(vk).ok().filter(|&vk| vk != 0).unwrap_or(event.vk)
}

/// The first installed layout on which the A and U keys type `a` and `u`.
fn find_reference_layout() -> Option<HKL> {
    let mut layouts = [null_mut(); 64];
    // SAFETY: `layouts` is writable for the count we pass.
    let count = unsafe { GetKeyboardLayoutList(layouts.len() as i32, layouts.as_mut_ptr()) };
    let state = [0u8; 256];
    layouts[..usize::try_from(count).unwrap_or(0)].iter().copied().find(|&hkl| {
        to_char(b'A'.into(), 0x1E, &state, hkl) == Some('a')
            && to_char(b'U'.into(), 0x16, &state, hkl) == Some('u')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_independent_keys_are_keysyms() {
        let key = |vk, extended| special_keysym(&KeyEvent { vk, scan: 0, extended, up: false, ours: false });
        assert_eq!(key(VK_RETURN, false), Some(sym::RETURN));
        assert_eq!(key(VK_RETURN, true), Some(sym::KP_ENTER));
        assert_eq!(key(VK_UP, false), Some(0xff52));
        assert_eq!(key(VK_F1, false), Some(0xffbe));
        assert_eq!(key(b'A'.into(), false), None);
    }
}
