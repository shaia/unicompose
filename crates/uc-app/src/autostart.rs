//! The login entry: it runs the tray app with the flags that pick the keyboard and quirks.

use std::path::{Path, PathBuf};

/// Name of the value under the `Run` key.
pub const NAME: &str = "unicompose";

/// The tray app's file name.
pub const TRAY_EXE: &str = if cfg!(windows) { "unicompose-tray.exe" } else { "unicompose-tray" };

/// `"<exe>" args…`, quoting every argument that contains whitespace.
pub fn command_line(exe: &Path, args: &[String]) -> String {
    let args = join_args(args);
    let exe = format!("\"{}\"", exe.display());
    if args.is_empty() {
        exe
    } else {
        format!("{exe} {args}")
    }
}

/// Arguments joined with spaces, quoting any that contain whitespace.
pub fn join_args(args: &[String]) -> String {
    let quoted: Vec<String> = args
        .iter()
        .map(|arg| if arg.contains(char::is_whitespace) { format!("\"{arg}\"") } else { arg.clone() })
        .collect();
    quoted.join(" ")
}

/// The tray app, installed next to the running program.
pub fn tray_exe_next_to_current() -> Result<PathBuf, String> {
    let current = std::env::current_exe().map_err(|e| format!("cannot find this program's path: {e}"))?;
    let tray = current.with_file_name(TRAY_EXE);
    if !tray.is_file() {
        return Err(format!("{} not found; it belongs next to {}", tray.display(), current.display()));
    }
    Ok(tray)
}

#[cfg(windows)]
pub use uc_win::autostart::State;

#[cfg(windows)]
pub fn state() -> std::io::Result<State> {
    uc_win::autostart::LOGIN.state(NAME)
}

/// Starts `exe args…` at login and returns the stored command line. Replaces the
/// administrator logon task if there is one, which shows a UAC prompt.
#[cfg(windows)]
pub fn enable(exe: &Path, args: &[String]) -> std::io::Result<String> {
    let command = command_line(exe, args);
    uc_win::autostart::LOGIN.enable(NAME, &command)?;
    uc_win::logon_task::delete(NAME)?;
    Ok(command)
}

/// Stops both kinds of login start. Deleting the administrator task shows a UAC prompt.
#[cfg(windows)]
pub fn disable() -> std::io::Result<()> {
    uc_win::autostart::LOGIN.disable(NAME)?;
    uc_win::logon_task::delete(NAME)
}

/// The administrator logon task's command line, if the task exists.
#[cfg(windows)]
pub fn elevated_command() -> std::io::Result<Option<String>> {
    uc_win::logon_task::command(NAME)
}

/// Starts `exe args…` as administrator at login, through a logon task, so it can type
/// into windows that run as administrator. Shows a UAC prompt. Replaces the ordinary
/// entry, so only one copy starts.
#[cfg(windows)]
pub fn enable_elevated(exe: &Path, args: &[String]) -> std::io::Result<String> {
    let arguments = join_args(args);
    uc_win::logon_task::create(NAME, exe, &arguments)?;
    uc_win::autostart::LOGIN.disable(NAME)?;
    Ok(command_line(exe, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_quotes_the_exe_and_spaced_args() {
        let args = ["--device".to_owned(), "my pad".to_owned(), "--vid".to_owned(), "1234".to_owned()];
        assert_eq!(
            command_line(Path::new(r"C:\Program Files\unicompose\unicompose-tray.exe"), &args),
            r#""C:\Program Files\unicompose\unicompose-tray.exe" --device "my pad" --vid 1234"#
        );
    }
}
