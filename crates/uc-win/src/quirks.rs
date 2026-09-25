//! Per-application workarounds, carried over from WinCompose (`Composer.SendString`).
//!
//! A quirk is chosen by the window class of the foreground top-level window.

use std::fmt;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quirk {
    /// GTK 2/3 on Windows crashes on surrogate pairs sent through `VK_PACKET`, so characters
    /// outside the BMP go through GTK's own Ctrl+Shift+U hex entry instead.
    GtkAstral,
    /// Word and Outlook can switch font on some symbol insertions. Typing the text before a
    /// zero-width space keeps the surrounding font, but leaves the ZWSP in the document, so
    /// this one is off unless asked for.
    OfficeFont,
}

impl Quirk {
    pub const ALL: [Quirk; 2] = [Quirk::GtkAstral, Quirk::OfficeFont];

    pub fn name(self) -> &'static str {
        match self {
            Quirk::GtkAstral => "gtk-astral",
            Quirk::OfficeFont => "office-font",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|quirk| quirk.name() == name)
    }

    fn on_by_default(self) -> bool {
        match self {
            Quirk::GtkAstral => true,
            Quirk::OfficeFont => false,
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

impl fmt::Display for Quirk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Foreground window classes, matched exactly, and the quirk each one needs.
const WINDOW_CLASSES: &[(&str, Quirk)] = &[
    ("gdkWindowToplevel", Quirk::GtkAstral),
    // XChat and HexChat rename GTK's top-level window class.
    ("xchatWindowToplevel", Quirk::GtkAstral),
    ("hexchatWindowToplevel", Quirk::GtkAstral),
    ("OpusApp", Quirk::OfficeFont),        // Word
    ("rctrl_renwnd32", Quirk::OfficeFont), // Outlook
];

/// The quirks that are switched on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuirkSet(u8);

impl QuirkSet {
    pub const NONE: QuirkSet = QuirkSet(0);

    pub fn defaults() -> Self {
        Quirk::ALL.into_iter().filter(|q| q.on_by_default()).fold(Self::NONE, |set, q| set.with(q))
    }

    #[must_use]
    pub fn with(self, quirk: Quirk) -> Self {
        QuirkSet(self.0 | quirk.bit())
    }

    pub fn contains(self, quirk: Quirk) -> bool {
        self.0 & quirk.bit() != 0
    }

    /// The enabled quirk for a window of class `class`, if any.
    pub fn for_class(self, class: &str) -> Option<Quirk> {
        WINDOW_CLASSES.iter().find(|(name, _)| *name == class).map(|&(_, q)| q).filter(|&q| self.contains(q))
    }
}

impl Default for QuirkSet {
    fn default() -> Self {
        Self::defaults()
    }
}

/// The foreground window as an address, so it can be compared and sent across threads.
/// Zero when no window has the foreground.
pub(crate) fn foreground_window() -> usize {
    // SAFETY: GetForegroundWindow has no preconditions.
    unsafe { GetForegroundWindow() as usize }
}

/// The window class of `hwnd`, or an empty string if the window has gone.
pub(crate) fn class_name(hwnd: usize) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: `buf` is writable for its full length, which is what we pass. A stale handle
    // makes the call fail and return 0; it never dereferences the handle itself.
    let len = unsafe { GetClassNameW(hwnd as HWND, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_gtk_but_not_office() {
        let set = QuirkSet::defaults();
        assert_eq!(set.for_class("gdkWindowToplevel"), Some(Quirk::GtkAstral));
        assert_eq!(set.for_class("hexchatWindowToplevel"), Some(Quirk::GtkAstral));
        assert_eq!(set.for_class("OpusApp"), None);
        assert_eq!(set.with(Quirk::OfficeFont).for_class("OpusApp"), Some(Quirk::OfficeFont));
    }

    #[test]
    fn unknown_classes_and_an_empty_set_match_nothing() {
        assert_eq!(QuirkSet::defaults().for_class("Notepad"), None);
        assert_eq!(QuirkSet::NONE.for_class("gdkWindowToplevel"), None);
    }

    #[test]
    fn names_round_trip() {
        for quirk in Quirk::ALL {
            assert_eq!(Quirk::from_name(quirk.name()), Some(quirk));
        }
        assert_eq!(Quirk::from_name("nope"), None);
    }
}
