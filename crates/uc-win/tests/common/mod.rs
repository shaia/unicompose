//! A window that records the characters it receives, for tests that type into it.
#![allow(dead_code)] // each test binary uses a different part

use std::cell::RefCell;
use std::io;
use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ActivateKeyboardLayout, GetKeyboardLayoutList, LoadKeyboardLayoutW, SendInput, SetFocus,
    UnloadKeyboardLayout, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KLF_NOTELLSHELL, VIRTUAL_KEY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
    GetWindowThreadProcessId, PeekMessageW, RegisterClassW, SetForegroundWindow, TranslateMessage,
    UnregisterClassW, CW_USEDEFAULT, MSG, PM_REMOVE, WM_CHAR, WNDCLASSW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

const CLASS: &str = "uc-win-focused-window-test";
/// How long to wait for injected input to arrive.
pub const DELIVERY_TIMEOUT: Duration = Duration::from_secs(2);
/// How long to keep listening afterwards, to catch unexpected extra characters.
const SETTLE: Duration = Duration::from_millis(100);

thread_local! {
    /// UTF-16 units from every WM_CHAR the test window has received.
    static RECEIVED: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
}

pub struct Window {
    hwnd: HWND,
    instance: HINSTANCE,
}

impl Window {
    pub fn create() -> Self {
        let class = wide(CLASS);
        // SAFETY: a null module name returns the handle of this executable.
        let instance = unsafe { GetModuleHandleW(null()) };
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };
        // SAFETY: `wc` and the class name it points to outlive the call.
        let atom = unsafe { RegisterClassW(&wc) };
        assert_ne!(atom, 0, "RegisterClassW: {}", io::Error::last_os_error());
        // SAFETY: the class is registered; every pointer is valid or null.
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                class.as_ptr(),
                class.as_ptr(),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                480,
                160,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        assert!(!hwnd.is_null(), "CreateWindowExW: {}", io::Error::last_os_error());
        pump();
        Self { hwnd, instance }
    }

    /// SetForegroundWindow is refused unless this process owns the foreground, so first
    /// attach to the input of the thread that does.
    pub fn take_focus(&self) {
        // SAFETY: GetForegroundWindow has no preconditions.
        let before = unsafe { GetForegroundWindow() };
        // SAFETY: plain Win32 calls on a live window; the attachment is undone before returning.
        let granted = unsafe {
            let me = GetCurrentThreadId();
            let owner = GetWindowThreadProcessId(before, null_mut());
            let attached = owner != 0 && owner != me && AttachThreadInput(me, owner, 1) != 0;
            let granted = SetForegroundWindow(self.hwnd) != 0;
            SetFocus(self.hwnd);
            if attached {
                AttachThreadInput(me, owner, 0);
            }
            granted
        };
        // Activation completes asynchronously; meanwhile the foreground window reads as null.
        let deadline = Instant::now() + DELIVERY_TIMEOUT;
        while !self.is_foreground() && Instant::now() < deadline {
            pump_briefly();
        }
        assert!(
            self.is_foreground(),
            "cannot take focus (SetForegroundWindow granted: {granted}). {}",
            if before.is_null() {
                "No window had focus to begin with: run from an unlocked desktop you are using."
            } else {
                "Another window kept it: is it elevated?"
            }
        );
    }

    pub fn is_foreground(&self) -> bool {
        // SAFETY: GetForegroundWindow has no preconditions.
        unsafe { GetForegroundWindow() == self.hwnd }
    }

    /// Runs `inject` while this window has focus and returns the WM_CHAR units it produced.
    pub fn chars_from(&self, expected_len: usize, inject: impl FnOnce()) -> Vec<u16> {
        pump();
        RECEIVED.take();
        assert!(self.is_foreground(), "test window lost focus; refusing to type");
        inject();
        let deadline = Instant::now() + DELIVERY_TIMEOUT;
        while RECEIVED.with_borrow(Vec::len) < expected_len && Instant::now() < deadline {
            pump_briefly();
        }
        let settled = Instant::now() + SETTLE;
        while Instant::now() < settled {
            pump_briefly();
        }
        RECEIVED.take()
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: the window and its class were created by `create` and are released once.
        unsafe {
            DestroyWindow(self.hwnd);
            UnregisterClassW(wide(CLASS).as_ptr(), self.instance);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_CHAR {
        RECEIVED.with_borrow_mut(|units| units.push(wparam as u16));
        return 0;
    }
    // SAFETY: forwards the arguments the system called us with.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Activates a keyboard layout on this thread. On drop, restores the previous layout and
/// unloads this one if the test loaded it.
pub struct Layout {
    previous: HKL,
    loaded: Option<HKL>,
}

impl Layout {
    pub fn activate(klid: &str) -> Self {
        let installed = installed_layouts();
        // SAFETY: the KLID is a NUL-terminated wide string that outlives the call.
        let hkl = unsafe { LoadKeyboardLayoutW(wide(klid).as_ptr(), KLF_NOTELLSHELL) };
        assert!(!hkl.is_null(), "LoadKeyboardLayoutW({klid}): {}", io::Error::last_os_error());
        let loaded = (!installed.contains(&hkl)).then_some(hkl);
        // SAFETY: `hkl` was just loaded.
        let previous = unsafe { ActivateKeyboardLayout(hkl, 0) };
        let layout = Self { previous, loaded };
        assert!(!previous.is_null(), "ActivateKeyboardLayout({klid}): {}", io::Error::last_os_error());
        pump();
        layout
    }
}

impl Drop for Layout {
    fn drop(&mut self) {
        // SAFETY: restores a handle the system gave us, and unloads only a layout we loaded.
        unsafe {
            if !self.previous.is_null() {
                ActivateKeyboardLayout(self.previous, 0);
            }
            if let Some(hkl) = self.loaded {
                UnloadKeyboardLayout(hkl);
            }
        }
    }
}

fn installed_layouts() -> Vec<HKL> {
    // SAFETY: a zero-length query writes nothing and returns the count.
    let count = unsafe { GetKeyboardLayoutList(0, null_mut()) };
    let mut layouts = vec![null_mut(); usize::try_from(count).unwrap_or(0)];
    // SAFETY: `layouts` has room for `count` handles.
    let written = unsafe { GetKeyboardLayoutList(count, layouts.as_mut_ptr()) };
    layouts.truncate(usize::try_from(written).unwrap_or(0));
    layouts
}

/// Presses and releases `vk` like a physical key, so the active layout decides what it types.
pub fn send_key(vk: VIRTUAL_KEY) {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    };
    let inputs = [key(0), key(KEYEVENTF_KEYUP)];
    // SAFETY: `inputs` holds two initialised INPUT structs of the size passed.
    let sent = unsafe { SendInput(2, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    assert_eq!(sent, 2, "SendInput: {}", io::Error::last_os_error());
}

pub fn pump() {
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid out-pointer; a null HWND selects every message of this thread.
    while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
        // SAFETY: `msg` was just filled in by PeekMessageW.
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub fn pump_briefly() {
    pump();
    std::thread::sleep(Duration::from_millis(5));
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// Sends key events the way a keyboard would, without unicompose's marker:
/// `(vk, extended, up)`.
pub fn send_keys(events: &[(VIRTUAL_KEY, bool, bool)]) {
    let events: Vec<_> = events.iter().map(|&(vk, extended, up)| (vk, 0, extended, up)).collect();
    send_scanned(&events);
}

/// Like `send_keys`, with a scan code: `(vk, scan, extended, up)`.
pub fn send_scanned(events: &[(VIRTUAL_KEY, u16, bool, bool)]) {
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
                    ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 },
                },
            }
        })
        .collect();
    // SAFETY: `inputs` holds initialised INPUT structs of the size passed.
    let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    assert_eq!(sent as usize, inputs.len(), "SendInput: {}", io::Error::last_os_error());
}
