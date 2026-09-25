//! The tray's icons, from the shared artwork in `uc_app::icon_art`.

use tray_icon::Icon;
use uc_app::icon_art::{self, Art};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    /// The Compose key is on, or the keyboard is connected: a green disc.
    Active,
    /// Nothing active: a grey ring.
    Waiting,
    /// Paused: an amber disc with a pause sign.
    Paused,
}

const SIZE: u32 = 32;

pub fn icon(kind: IconKind) -> Icon {
    let art = match kind {
        IconKind::Active => Art::Active,
        IconKind::Waiting => Art::Waiting,
        IconKind::Paused => Art::Paused,
    };
    Icon::from_rgba(icon_art::pixels(art, SIZE), SIZE, SIZE)
        .expect("the buffer holds SIZE x SIZE RGBA pixels")
}
