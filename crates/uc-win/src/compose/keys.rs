//! Keys that can act as the Compose key.

use std::fmt;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_APPS, VK_CAPITAL, VK_INSERT, VK_LCONTROL, VK_LMENU, VK_LWIN, VK_PAUSE, VK_RCONTROL,
    VK_RMENU, VK_RWIN, VK_SCROLL, VK_SNAPSHOT,
};

/// A key that is tapped to start a sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposeKey {
    pub(crate) vk: VIRTUAL_KEY,
    name: &'static str,
    label: &'static str,
    /// WinCompose's name for it in `settings.ini`, without the `VK.` prefix.
    wincompose: &'static str,
}

/// Every key that can be the Compose key. The first is the default, as in WinCompose.
pub const COMPOSE_KEYS: &[ComposeKey] = &[
    ComposeKey { vk: VK_RMENU, name: "ralt", label: "Right Alt", wincompose: "RMENU" },
    ComposeKey { vk: VK_LMENU, name: "lalt", label: "Left Alt", wincompose: "LMENU" },
    ComposeKey { vk: VK_RCONTROL, name: "rctrl", label: "Right Ctrl", wincompose: "RCONTROL" },
    ComposeKey { vk: VK_LCONTROL, name: "lctrl", label: "Left Ctrl", wincompose: "LCONTROL" },
    ComposeKey { vk: VK_RWIN, name: "rwin", label: "Right Windows", wincompose: "RWIN" },
    ComposeKey { vk: VK_LWIN, name: "lwin", label: "Left Windows", wincompose: "LWIN" },
    ComposeKey { vk: VK_APPS, name: "menu", label: "Menu", wincompose: "APPS" },
    ComposeKey { vk: VK_CAPITAL, name: "capslock", label: "Caps Lock", wincompose: "CAPITAL" },
    ComposeKey { vk: VK_SCROLL, name: "scrolllock", label: "Scroll Lock", wincompose: "SCROLL" },
    ComposeKey { vk: VK_PAUSE, name: "pause", label: "Pause", wincompose: "PAUSE" },
    ComposeKey { vk: VK_INSERT, name: "insert", label: "Insert", wincompose: "INSERT" },
    ComposeKey { vk: VK_SNAPSHOT, name: "printscreen", label: "Print Screen", wincompose: "SNAPSHOT" },
];

impl ComposeKey {
    pub fn from_name(name: &str) -> Option<ComposeKey> {
        COMPOSE_KEYS.iter().copied().find(|key| key.name.eq_ignore_ascii_case(name))
    }

    /// The command-line name, such as `ralt`.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The name people know, such as "Right Alt".
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// The Compose key set in WinCompose's `settings.ini` text, such as
    /// `compose_key=VK.RMENU`. WinCompose allows several; the first known one is used.
    pub fn from_wincompose_settings(ini: &str) -> Option<ComposeKey> {
        let value = ini.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "compose_key").then_some(value)
        })?;
        value
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(|part| part.trim().strip_prefix("VK."))
            .find_map(|vk| COMPOSE_KEYS.iter().copied().find(|key| key.wincompose == vk))
    }
}

impl Default for ComposeKey {
    fn default() -> Self {
        COMPOSE_KEYS[0]
    }
}

impl fmt::Display for ComposeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_found_case_insensitively() {
        for key in COMPOSE_KEYS {
            assert_eq!(ComposeKey::from_name(&key.name.to_uppercase()), Some(*key));
        }
        assert_eq!(ComposeKey::default().name(), "ralt");
        assert_eq!(ComposeKey::from_name("hyper"), None);
    }

    #[test]
    fn reads_wincompose_settings() {
        let ini = "[composing]\ncompose_key=VK.RMENU\nled_key=VK.COMPOSE\n";
        assert_eq!(ComposeKey::from_wincompose_settings(ini).map(|k| k.name()), Some("ralt"));
        let several = "[composing]\ncompose_key = VK.NONSENSE, VK.CAPITAL\n";
        assert_eq!(ComposeKey::from_wincompose_settings(several).map(|k| k.name()), Some("capslock"));
        assert_eq!(ComposeKey::from_wincompose_settings("[composing]\n"), None);
    }
}
