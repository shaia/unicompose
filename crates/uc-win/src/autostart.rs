//! Start at login through the per-user `Run` key, the same mechanism Task Manager's
//! Startup page lists and lets the user switch off.

use std::ffi::c_void;
use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY,
    RRF_RT_REG_SZ,
};

/// A pair of HKCU subkeys: the command lines to run, and Explorer's record of which of
/// them the user switched off in Task Manager.
#[derive(Debug, Clone, Copy)]
pub struct RunKey {
    pub run: &'static str,
    pub approved: &'static str,
}

/// The keys Windows reads at login.
pub const LOGIN: RunKey = RunKey {
    run: r"Software\Microsoft\Windows\CurrentVersion\Run",
    approved: r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
};

/// Whether an entry runs at login, and with what command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Off,
    /// Registered, but switched off in Task Manager.
    Disabled(String),
    On(String),
}

impl RunKey {
    pub fn state(&self, name: &str) -> io::Result<State> {
        let Some(command) = self.command(name)? else {
            return Ok(State::Off);
        };
        let approved = read(self.approved, name, RRF_RT_REG_BINARY)?.is_none_or(|bytes| approved(&bytes));
        Ok(if approved { State::On(command) } else { State::Disabled(command) })
    }

    pub fn command(&self, name: &str) -> io::Result<Option<String>> {
        let Some(bytes) = read(self.run, name, RRF_RT_REG_SZ)? else {
            return Ok(None);
        };
        let units: Vec<u16> = bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        let text = String::from_utf16_lossy(&units);
        Ok(Some(text.trim_end_matches('\0').to_owned()))
    }

    /// Registers `command` to run at login. Also clears a Task Manager "disabled" mark,
    /// since the user has just asked for the opposite.
    pub fn enable(&self, name: &str, command: &str) -> io::Result<()> {
        let data: Vec<u16> = command.encode_utf16().chain([0]).collect();
        let subkey = wide(self.run);
        let value = wide(name);
        // SAFETY: every pointer is to a live, NUL-terminated wide string or to `data`,
        // whose byte length we pass. RegSetKeyValueW creates the subkey if needed.
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                REG_SZ,
                data.as_ptr().cast::<c_void>(),
                u32::try_from(data.len() * 2).map_err(|_| io::Error::other("command line too long"))?,
            )
        };
        check(status)?;
        delete(self.approved, name)
    }

    pub fn disable(&self, name: &str) -> io::Result<()> {
        delete(self.run, name)?;
        delete(self.approved, name)
    }
}

/// Explorer writes 12 bytes; an odd first byte (0x03, 0x07) means switched off.
fn approved(bytes: &[u8]) -> bool {
    bytes.first().is_none_or(|b| b & 1 == 0)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn check(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

/// Reads an HKCU value of the type `flags` allows; `None` if the key or value is missing.
fn read(subkey: &str, name: &str, flags: u32) -> io::Result<Option<Vec<u8>>> {
    let subkey = wide(subkey);
    let value = wide(name);
    loop {
        let mut len = 0u32;
        // SAFETY: the strings are live and NUL-terminated; a null data pointer asks for
        // the size only.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                flags,
                null_mut(),
                null_mut(),
                &mut len,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status)?;
        let mut buf = vec![0u8; len as usize];
        // SAFETY: `buf` is writable for `len` bytes, which is what we pass.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                flags,
                null_mut(),
                buf.as_mut_ptr().cast::<c_void>(),
                &mut len,
            )
        };
        match status {
            ERROR_SUCCESS => {
                buf.truncate(len as usize);
                return Ok(Some(buf));
            }
            ERROR_FILE_NOT_FOUND => return Ok(None),
            // The value grew between the two calls; ask for its size again.
            ERROR_MORE_DATA => continue,
            status => return Err(io::Error::from_raw_os_error(status as i32)),
        }
    }
}

/// Deletes an HKCU value; a missing key or value is not an error.
fn delete(subkey: &str, name: &str) -> io::Result<()> {
    let subkey = wide(subkey);
    let value = wide(name);
    // SAFETY: both strings are live and NUL-terminated.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey.as_ptr(), value.as_ptr()) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Registry::{RegDeleteTreeW, REG_BINARY};

    const SCRATCH: &str = r"Software\unicompose-test";

    /// A throwaway pair of keys under HKCU, deleted when dropped.
    struct Scratch(RunKey);

    impl Scratch {
        fn new(name: &'static str) -> Self {
            // Leaked so the keys can be `'static`, like the real ones; a few bytes per test.
            let run = Box::leak(format!(r"{SCRATCH}-{name}\Run").into_boxed_str());
            let approved = Box::leak(format!(r"{SCRATCH}-{name}\Approved").into_boxed_str());
            Scratch(RunKey { run, approved })
        }

        fn mark_disabled(&self, name: &str) {
            let data = [3u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            let subkey = wide(self.0.approved);
            let value = wide(name);
            // SAFETY: live NUL-terminated strings and a 12-byte buffer whose length we pass.
            let status = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    subkey.as_ptr(),
                    value.as_ptr(),
                    REG_BINARY,
                    data.as_ptr().cast(),
                    12,
                )
            };
            check(status).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let parent = self.0.run.trim_end_matches(r"\Run");
            let key = wide(parent);
            // SAFETY: a live NUL-terminated string naming a key under our scratch root.
            unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key.as_ptr()) };
        }
    }

    #[test]
    fn enable_then_disable_round_trips() {
        let scratch = Scratch::new("round-trip");
        let key = scratch.0;
        assert_eq!(key.state("app").unwrap(), State::Off);
        let command = r#""C:\Program Files\ü\app.exe" --vid 1209"#;
        key.enable("app", command).unwrap();
        assert_eq!(key.state("app").unwrap(), State::On(command.into()));
        key.disable("app").unwrap();
        assert_eq!(key.state("app").unwrap(), State::Off);
        key.disable("app").unwrap();
    }

    #[test]
    fn task_manager_mark_is_reported_and_cleared_by_enable() {
        let scratch = Scratch::new("approved");
        let key = scratch.0;
        key.enable("app", "app.exe").unwrap();
        scratch.mark_disabled("app");
        assert_eq!(key.state("app").unwrap(), State::Disabled("app.exe".into()));
        key.enable("app", "app.exe").unwrap();
        assert_eq!(key.state("app").unwrap(), State::On("app.exe".into()));
    }

    #[test]
    fn approved_bytes() {
        assert!(approved(&[2, 0, 0]));
        assert!(approved(&[6]));
        assert!(approved(&[]));
        assert!(!approved(&[3, 0, 0]));
        assert!(!approved(&[7]));
    }
}
