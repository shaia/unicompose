//! The unicompose app, shared by the `unicompose` command line and the `unicompose-tray`
//! tray app.

pub mod autostart;
pub mod bridge;
pub mod compose;
pub mod device;
pub mod icon_art;
pub mod logging;
pub mod record;
pub mod settings;
pub mod typing;

/// `U+XXXX<tab>char`, the form characters are printed and logged in.
pub fn describe(c: char) -> String {
    format!("U+{:04X}\t{c}", u32::from(c))
}
