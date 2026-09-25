//! The Compose key on Windows: a low-level keyboard hook on a thread of its own.
//!
//! The hook decides in microseconds whether each key reaches applications
//! ([`logic::Processor`]). Anything to type is queued and typed by the same thread after
//! the hook returns, since Windows does not allow `SendInput` from inside the hook.
//!
//! Windows removes a low-level hook without notice if it is ever too slow. A watchdog
//! reinstalls the hook when there has been recent input but the hook has seen no key for
//! a long time. (Raw keyboard input would tell keys from mouse moves, but registering
//! for it, on any thread of the process, stalls the hook: Windows then waits out its
//! timeout on every key and lets each key through.)

mod keys;
mod logic;
mod resolve;

use std::cell::{Cell, RefCell};
use std::io;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use uc_engine::{Engine, Options, Table};
use uc_platform::TextSink;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, ProcessPowerThrottling, SetProcessInformation, SetThreadPriority,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_STATE, THREAD_PRIORITY_HIGHEST,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT,
    LLKHF_EXTENDED, LLKHF_UP, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WM_APP, WM_QUIT, WM_TIMER, WM_USER,
};

pub use keys::{ComposeKey, COMPOSE_KEYS};
use logic::{Injection, KeyEvent, Processor};
use resolve::LayoutResolver;

use crate::quirks::QuirkSet;
use crate::sink::{replay_keys, SendInputSink, INJECTED_MARKER};

/// Type what the hook queued.
const WM_APP_INJECT: u32 = WM_APP + 10;
/// Swap in the table in `Shared::next_table`.
const WM_APP_TABLE: u32 = WM_APP + 11;
/// Enabled or paused changed.
const WM_APP_STATE: u32 = WM_APP + 12;
/// How often the watchdog looks.
const WATCHDOG_PERIOD: Duration = Duration::from_secs(10);
/// Input this recent means someone is at the machine.
const WATCHDOG_RECENT_MS: u32 = 10_000;
/// A hook that has seen no key for this long while there is input gets reinstalled.
/// Input may be mouse only, which costs a needless reinstall this often at most.
const WATCHDOG_SILENT_MS: u32 = 300_000;

pub struct ComposeConfig {
    pub table: Table,
    pub key: ComposeKey,
    pub options: Options,
    /// Drop the keys of a sequence that matched nothing, instead of typing them.
    pub discard_invalid: bool,
    pub quirks: QuirkSet,
}

struct Shared {
    enabled: AtomicBool,
    paused: Arc<AtomicBool>,
    next_table: Mutex<Option<Table>>,
}

/// The running Compose key. Dropping it removes the hook and ends its thread.
pub struct Compose {
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
    key: ComposeKey,
}

impl Compose {
    /// Installs the hook. `paused` is shared with the rest of the app: while it is set,
    /// every key passes untouched.
    pub fn start(config: ComposeConfig, enabled: bool, paused: Arc<AtomicBool>) -> io::Result<Compose> {
        let shared =
            Arc::new(Shared { enabled: AtomicBool::new(enabled), paused, next_table: Mutex::new(None) });
        let key = config.key;
        let (ready, started) = mpsc::channel();
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("compose".into())
            .spawn(move || run(config, thread_shared, ready))?;
        let thread_id =
            started.recv().map_err(|_| io::Error::other("the compose thread ended while starting"))??;
        tracing::info!("Compose key is {key}{}", if enabled { "" } else { " (switched off)" });
        Ok(Compose { thread_id, thread: Some(thread), shared, key })
    }

    pub fn key(&self) -> ComposeKey {
        self.key
    }

    pub fn is_enabled(&self) -> bool {
        self.shared.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.shared.enabled.store(enabled, Ordering::Relaxed);
        self.post(WM_APP_STATE);
    }

    /// Call after changing `paused`, so a sequence in progress is dropped at once.
    pub fn paused_changed(&self) {
        self.post(WM_APP_STATE);
    }

    /// Replaces the rules; a sequence in progress is cancelled.
    pub fn set_table(&self, table: Table) {
        *self.shared.next_table.lock().unwrap_or_else(|e| e.into_inner()) = Some(table);
        self.post(WM_APP_TABLE);
    }

    fn post(&self, message: u32) {
        // SAFETY: posting a message with no pointers in it is sound for any thread ID.
        unsafe { PostThreadMessageW(self.thread_id, message, 0, 0) };
    }
}

impl Drop for Compose {
    fn drop(&mut self) {
        self.post(WM_QUIT);
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                tracing::error!("the compose thread panicked");
            }
        }
    }
}

/// Everything the hook callback needs. It lives in a thread-local, because a hook
/// callback takes no context argument; only the compose thread touches it.
struct HookState {
    processor: Processor,
    resolver: LayoutResolver,
    shared: Arc<Shared>,
    /// What to type once the callback has returned.
    pending: Vec<Injection>,
    /// Whether the last event was handled with composing on, to reset on the change.
    active: bool,
    /// The thread timer for the engine's ambiguity timeout, if set.
    engine_timer: usize,
    thread_id: u32,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
    /// `GetTickCount` when the hook last saw a key, for the watchdog.
    static LAST_HOOK_TICK: Cell<u32> = const { Cell::new(0) };
}

fn run(config: ComposeConfig, shared: Arc<Shared>, ready: Sender<io::Result<u32>>) {
    let thread_id = crate::app::current_thread_id();
    // SAFETY: MSG is plain data; peeking creates this thread's message queue, so posts
    // to it cannot be lost from here on.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        PeekMessageW(&mut msg, null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
    }
    keep_responsive();
    let processor =
        Processor::new(Engine::new(config.table, config.options), config.key, config.discard_invalid);
    let state = HookState {
        processor,
        resolver: LayoutResolver::new(),
        shared: Arc::clone(&shared),
        pending: Vec::new(),
        active: false,
        engine_timer: 0,
        thread_id,
    };
    STATE.with(|s| *s.borrow_mut() = Some(state));
    let mut hook = match install_hook() {
        Ok(hook) => hook,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    // SAFETY: GetTickCount has no preconditions.
    LAST_HOOK_TICK.set(unsafe { GetTickCount() });
    let _ = ready.send(Ok(thread_id));

    let mut sink = SendInputSink::new(config.quirks);
    // SAFETY: a thread timer (null window) with no callback just posts WM_TIMER here.
    let watchdog_timer = unsafe { SetTimer(null_mut(), 0, WATCHDOG_PERIOD.as_millis() as u32, None) };
    // SAFETY: MSG is plain data.
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: `msg` is a valid MSG to write into; 0 means WM_QUIT and -1 an error.
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        match msg.message {
            WM_APP_INJECT => inject_pending(&mut sink),
            WM_APP_TABLE => {
                let table = shared.next_table.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(table) = table {
                    with_state(|state| state.processor.engine_mut().set_table(table));
                    tracing::info!("compose rules reloaded");
                }
            }
            WM_APP_STATE => with_state(|state| {
                if !state.is_active() {
                    state.processor.reset();
                }
            }),
            WM_TIMER if msg.wParam == watchdog_timer => {
                if hook_went_silent() {
                    tracing::debug!("the keyboard hook has seen no key for a while; reinstalling it");
                    // SAFETY: `hook` is the handle SetWindowsHookExW returned.
                    unsafe { UnhookWindowsHookEx(hook) };
                    match install_hook() {
                        Ok(new) => hook = new,
                        Err(e) => {
                            tracing::error!("cannot reinstall the keyboard hook: {e}");
                            break;
                        }
                    }
                    // SAFETY: GetTickCount has no preconditions.
                    LAST_HOOK_TICK.set(unsafe { GetTickCount() });
                }
            }
            WM_TIMER => {
                let fired = with_state(|state| {
                    if msg.wParam != state.engine_timer {
                        return false;
                    }
                    // SAFETY: the timer is this thread's own.
                    unsafe { KillTimer(null_mut(), state.engine_timer) };
                    state.engine_timer = 0;
                    let decision = state.processor.tick(Instant::now());
                    state.pending.extend(decision.inject);
                    state.schedule_tick();
                    true
                });
                if fired {
                    inject_pending(&mut sink);
                }
            }
            _ => {
                // SAFETY: `msg` was just filled in by GetMessageW.
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }
    // SAFETY: the handles are ours and released once.
    unsafe {
        UnhookWindowsHookEx(hook);
        KillTimer(null_mut(), watchdog_timer);
    }
    STATE.with(|s| s.borrow_mut().take());
}

fn with_state<T: Default>(f: impl FnOnce(&mut HookState) -> T) -> T {
    STATE.with(|s| s.borrow_mut().as_mut().map(f).unwrap_or_default())
}

impl HookState {
    fn is_active(&self) -> bool {
        self.shared.enabled.load(Ordering::Relaxed) && !self.shared.paused.load(Ordering::Relaxed)
    }

    /// Handles one key event; true to hide it from applications.
    fn handle(&mut self, info: &KBDLLHOOKSTRUCT) -> bool {
        let active = self.is_active();
        if active != std::mem::replace(&mut self.active, active) {
            self.processor.reset();
        }
        if !active {
            return false;
        }
        let event = KeyEvent {
            vk: info.vkCode as u16,
            scan: info.scanCode,
            extended: info.flags & LLKHF_EXTENDED != 0,
            up: info.flags & LLKHF_UP != 0,
            ours: info.dwExtraInfo == INJECTED_MARKER,
        };
        let decision = self.processor.event(event, Instant::now(), &mut self.resolver);
        tracing::trace!(?event, ?decision, "key");
        if !decision.inject.is_empty() {
            self.pending.extend(decision.inject);
            // SAFETY: posting a message with no pointers in it is sound.
            unsafe { PostThreadMessageW(self.thread_id, WM_APP_INJECT, 0, 0) };
        }
        self.schedule_tick();
        decision.swallow
    }

    /// Keeps a timer running while the engine waits on a timeout.
    fn schedule_tick(&mut self) {
        match (self.processor.deadline(), self.engine_timer) {
            (Some(deadline), 0) => {
                let ms = deadline.saturating_duration_since(Instant::now()).as_millis().max(1);
                // SAFETY: a thread timer with no callback posts WM_TIMER to this thread.
                self.engine_timer =
                    unsafe { SetTimer(null_mut(), 0, u32::try_from(ms).unwrap_or(u32::MAX), None) };
            }
            (None, timer) if timer != 0 => {
                // SAFETY: the timer is this thread's own.
                unsafe { KillTimer(null_mut(), timer) };
                self.engine_timer = 0;
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: GetTickCount has no preconditions.
        LAST_HOOK_TICK.set(unsafe { GetTickCount() });
        // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT that lives for the
        // duration of this call.
        let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        // `try_borrow_mut`: never panic inside a hook, even if Windows re-entered it.
        let swallow = STATE.with(|s| match s.try_borrow_mut() {
            Ok(mut state) => state.as_mut().is_some_and(|state| state.handle(info)),
            Err(_) => false,
        });
        if swallow {
            return 1;
        }
    }
    // SAFETY: passing the event on unchanged, as every hook must.
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

/// Whether there has been recent input while the hook has seen no key for a long time.
fn hook_went_silent() -> bool {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: `info` has its size set, as GetLastInputInfo requires.
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return false;
    }
    // SAFETY: GetTickCount has no preconditions.
    let now = unsafe { GetTickCount() };
    silent(now, info.dwTime, LAST_HOOK_TICK.get())
}

/// The watchdog's rule, on `GetTickCount` values, which wrap after 49 days.
fn silent(now: u32, last_input: u32, last_hook: u32) -> bool {
    now.wrapping_sub(last_input) < WATCHDOG_RECENT_MS
        && last_input.wrapping_sub(last_hook) > WATCHDOG_SILENT_MS
}

fn install_hook() -> io::Result<HHOOK> {
    // SAFETY: `hook_proc` has the signature WH_KEYBOARD_LL expects, and a null module name
    // is this executable.
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), GetModuleHandleW(null()), 0) };
    if hook.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(hook)
}

/// Types what the hook queued, outside the hook callback.
fn inject_pending(sink: &mut SendInputSink) {
    let pending = with_state(|state| std::mem::take(&mut state.pending));
    for injection in pending {
        let result = match injection {
            Injection::Text(text) => sink.type_text(&text),
            Injection::Keys(events) => {
                let events: Vec<_> = events.iter().map(|e| (e.vk, e.scan, e.extended, e.up)).collect();
                replay_keys(&events)
            }
        };
        if let Err(e) = result {
            tracing::warn!("cannot type compose output: {e}");
        }
    }
}

/// Runs the compose thread ahead of ordinary work, and keeps Windows from slowing it
/// down as a background process (EcoQoS): a late hook gets removed.
fn keep_responsive() {
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: both calls take the current process or thread pseudo-handle, and `state`
    // is the struct ProcessPowerThrottling expects, with its size.
    unsafe {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchdog_reinstalls_only_a_hook_silent_through_recent_input() {
        let minute = 60_000;
        assert!(silent(10 * minute, 10 * minute - 1000, 0), "input now, no key for ten minutes");
        assert!(!silent(10 * minute, 10 * minute - 1000, 9 * minute), "a key a minute ago");
        assert!(!silent(10 * minute, minute, 0), "nobody at the machine");
        assert!(silent(5 * minute, 5 * minute, u32::MAX - 10 * minute), "across the tick counter's wrap");
    }
}
