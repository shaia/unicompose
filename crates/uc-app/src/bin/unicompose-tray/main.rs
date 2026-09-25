//! unicompose-tray: types what a keyboard sends over Raw HID, from a tray icon that shows
//! whether the keyboard is connected, pauses typing with one click, opens the log and
//! starts the app at login. Windows only so far.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(windows)]
mod icons;
#[cfg(windows)]
mod settings_window;
#[cfg(windows)]
mod tray;

#[cfg(windows)]
fn main() -> ExitCode {
    tray::main()
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("unicompose-tray is only implemented on Windows so far; run unicompose --dry-run instead");
    ExitCode::FAILURE
}
