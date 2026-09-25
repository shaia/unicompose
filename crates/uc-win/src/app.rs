//! What a tray app needs from Windows besides the icon itself: a message loop other
//! threads can wake, a single-instance guard, and the shell's "open".

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, APPMODEL_ERROR_NO_PACKAGE, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, GetCurrentThreadId, OpenProcessToken,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MessageBoxW, PostQuitMessage, PostThreadMessageW, SetTimer,
    TranslateMessage, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MSG, SW_SHOWNORMAL, WM_APP,
};

/// Posted to a thread to make its message loop run the callback.
const WM_APP_WAKE: u32 = WM_APP + 1;
/// Message loops also wake this often. A modal loop, such as an open tray menu, swallows
/// thread messages, so a wake posted then would otherwise wait for the next input.
const TICK_MS: u32 = 1000;

pub fn current_thread_id() -> u32 {
    // SAFETY: GetCurrentThreadId has no preconditions.
    unsafe { GetCurrentThreadId() }
}

/// Makes the message loop on `thread_id` run its callback soon. Returns false if that
/// thread has no message queue (its loop has not started, or has ended).
pub fn wake(thread_id: u32) -> bool {
    // SAFETY: posting a message with no pointers in it is sound for any thread ID.
    unsafe { PostThreadMessageW(thread_id, WM_APP_WAKE, 0, 0) != 0 }
}

/// Runs this thread's message loop until `quit` is called on it, calling `on_message`
/// after every message: window messages, wakes and a periodic tick alike.
pub fn run_message_loop(mut on_message: impl FnMut()) {
    // SAFETY: a thread timer (null window) with no callback just posts WM_TIMER here.
    let timer = unsafe { SetTimer(null_mut(), 0, TICK_MS, None) };
    // SAFETY: MSG is plain data, and all-zero is a valid value for it.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: `msg` is a valid MSG to write into; GetMessageW returns 0 on WM_QUIT and -1
    // on error, and both end the loop.
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        // SAFETY: `msg` was just filled in by GetMessageW.
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        on_message();
    }
    if timer != 0 {
        // SAFETY: `timer` is the thread timer created above.
        unsafe { KillTimer(null_mut(), timer) };
    }
}

/// Ends the message loop running on this thread once it next looks for a message.
pub fn quit() {
    // SAFETY: PostQuitMessage has no preconditions.
    unsafe { PostQuitMessage(0) };
}

/// Held by the one running instance; released when dropped or when the process exits.
#[derive(Debug)]
pub struct SingleInstance(HANDLE);

// SAFETY: a mutex handle can be closed from any thread.
unsafe impl Send for SingleInstance {}

impl SingleInstance {
    /// Returns `None` if another process already holds `name`, such as
    /// `Local\unicompose-tray` (one per login session).
    pub fn acquire(name: &str) -> io::Result<Option<Self>> {
        let name = wide(OsStr::new(name));
        // SAFETY: `name` is live and NUL-terminated; null attributes mean the defaults.
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            // An elevated instance owns the mutex, and we may not open it: that is one too.
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                return Ok(None);
            }
            return Err(error);
        }
        // SAFETY: GetLastError has no preconditions; CreateMutexW sets it on success too.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: `handle` is the handle CreateMutexW just returned.
            unsafe { CloseHandle(handle) };
            return Ok(None);
        }
        Ok(Some(SingleInstance(handle)))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        // SAFETY: we own this handle and close it once.
        unsafe { CloseHandle(self.0) };
    }
}

/// Whether a process with this executable name (such as `wincompose.exe`, compared
/// without regard to case) is running in any session.
pub fn process_running(exe_name: &str) -> bool {
    // SAFETY: a snapshot of all processes; PROCESSENTRY32W is plain data with its size set,
    // and the handle is closed before returning.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more && !found {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            found = String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(exe_name);
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

/// Whether this process runs from an MSIX package. Packaged apps start at login through
/// their package's startup task, and their registry writes are kept private to the
/// package, so the Run key and the logon task do not apply.
pub fn is_packaged() -> bool {
    let mut len = 0u32;
    // SAFETY: a zero length with a null buffer only asks whether there is a package name.
    let status = unsafe { GetCurrentPackageFullName(&mut len, null_mut()) };
    status != APPMODEL_ERROR_NO_PACKAGE
}

/// Whether this process runs as administrator.
pub fn is_elevated() -> bool {
    // SAFETY: the pseudo-handle of this process needs no closing; the token handle is
    // closed, and TOKEN_ELEVATION is plain data of the size passed.
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// Opens a file or folder the way Explorer would on double-click.
pub fn open(path: &Path) -> io::Result<()> {
    let verb = wide(OsStr::new("open"));
    let file = wide(path.as_os_str());
    // SAFETY: both strings are live and NUL-terminated; the other pointers may be null.
    let result =
        unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), file.as_ptr(), null(), null(), SW_SHOWNORMAL) };
    // ShellExecuteW reports success as a value above 32.
    if result as usize > 32 {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "cannot open {} (ShellExecute error {})",
            path.display(),
            result as usize
        )))
    }
}

/// Shows a message box: the only way a program without a console can report an error
/// before its log exists.
pub fn message_box(title: &str, text: &str, error: bool) {
    let title = wide(OsStr::new(title));
    let text = wide(OsStr::new(text));
    let icon = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    // SAFETY: both strings are live and NUL-terminated; no owner window.
    unsafe { MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | icon) };
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_of_the_same_name_fails_until_the_first_is_dropped() {
        let name = format!("Local\\unicompose-test-{}", std::process::id());
        let first = SingleInstance::acquire(&name).unwrap();
        assert!(first.is_some());
        assert!(SingleInstance::acquire(&name).unwrap().is_none());
        drop(first);
        assert!(SingleInstance::acquire(&name).unwrap().is_some());
    }

    #[test]
    fn finds_this_test_process_by_name() {
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().to_uppercase();
        assert!(process_running(&name));
        assert!(!process_running("no-such-process-unicompose.exe"));
    }

    #[test]
    fn a_test_binary_is_not_packaged() {
        assert!(!is_packaged());
    }

    #[test]
    fn wake_runs_the_callback_and_quit_ends_the_loop() {
        let (tx, rx) = std::sync::mpsc::channel();
        let looper = std::thread::spawn(move || {
            let mut calls = 0;
            tx.send(current_thread_id()).unwrap();
            run_message_loop(|| {
                calls += 1;
                quit();
            });
            calls
        });
        let tid = rx.recv().unwrap();
        // The loop's queue exists once it first calls GetMessageW; retry until then.
        while !wake(tid) {
            std::thread::yield_now();
        }
        assert!(looper.join().unwrap() >= 1);
    }
}
