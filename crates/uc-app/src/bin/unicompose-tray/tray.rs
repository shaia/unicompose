//! The tray icon and its menu. The main thread owns them and runs the message loop; a
//! `dispatch` thread runs the bridge and reports device status back through a channel,
//! and the Compose key's hook runs on a thread of its own. Both restart when their
//! settings change.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use clap::Parser;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use uc_app::bridge::{self, BoxError, Control, Status};
use uc_app::compose::{self, WINCOMPOSE_EXE};
use uc_app::device::{self, Target};
use uc_app::settings::{Overrides, Settings};
use uc_app::{autostart, logging, typing};
use uc_config::Config;
use uc_win::app;
use uc_win::compose::Compose;
use uc_win::dialog::{self, Handler, Values};

use crate::icons::{self, IconKind};
use crate::settings_window::{self, Context, Login, EDIT_RULES, RELOAD_RULES};

const TITLE: &str = "unicompose";
/// One tray per login session: two would both open the keyboard and type everything twice.
const INSTANCE: &str = "Local\\unicompose-tray";
/// Windows cuts tray tooltips at 127 characters.
const TOOLTIP_MAX_CHARS: usize = 127;
/// How often to look for WinCompose while it holds the Compose key off.
const WINCOMPOSE_CHECK: Duration = Duration::from_secs(5);

/// A Compose key, and typing what a keyboard sends over Raw HID, from a tray icon.
///
/// Settings are in %APPDATA%\unicompose\config.toml, which the Settings window edits; the
/// flags below override them for this run. Left-click the icon to pause or resume;
/// right-click for the menu. The log is in %LOCALAPPDATA%\unicompose\logs, and its
/// verbosity follows RUST_LOG (default: info).
#[derive(Parser)]
#[command(name = "unicompose-tray", version)]
struct TrayArgs {
    #[command(flatten)]
    overrides: Overrides,
}

pub fn main() -> ExitCode {
    // There is no console: report problems in a message box until the log is open.
    let args = match TrayArgs::try_parse() {
        Ok(args) => args,
        Err(e) => {
            let failed = e.use_stderr();
            app::message_box(TITLE, &e.render().to_string(), failed);
            return if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS };
        }
    };
    let Some(log_dir) = logging::log_dir() else {
        app::message_box(TITLE, "LOCALAPPDATA is not set, so there is nowhere to write the log.", true);
        return ExitCode::FAILURE;
    };
    let _log_writer = match logging::init_file(&log_dir) {
        Ok(guard) => guard,
        Err(e) => {
            app::message_box(TITLE, &format!("Cannot write the log in {}: {e}", log_dir.display()), true);
            return ExitCode::FAILURE;
        }
    };
    match run(args, log_dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            app::message_box(TITLE, &e.to_string(), true);
            ExitCode::FAILURE
        }
    }
}

/// What the dispatch thread tells the tray.
enum Update {
    Status(Status),
    Failed(String),
}

fn run(args: TrayArgs, log_dir: PathBuf) -> Result<(), BoxError> {
    let Some(_instance) = app::SingleInstance::acquire(INSTANCE)? else {
        tracing::info!("unicompose-tray is already running; exiting");
        return Ok(());
    };
    tracing::info!(
        "unicompose-tray {} starting{}",
        env!("CARGO_PKG_VERSION"),
        if app::is_elevated() { " as administrator" } else { "" }
    );
    let settings = Settings::load();
    if let Some(problem) = &settings.problem {
        app::message_box(
            TITLE,
            &format!("{problem}\n\nunicompose is using the default settings until you fix or save them."),
            true,
        );
    }
    let (tx, rx) = mpsc::channel();
    let mut tray = Tray::new(settings, args.overrides, log_dir, tx)?;
    app::run_message_loop(|| {
        if !tray.poll(&rx) {
            app::quit();
        }
    });
    tray.shut_down();
    tracing::info!("unicompose-tray exiting");
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Link {
    Waiting,
    Connected(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ComposeState {
    /// Switched off in the settings or by `--no-compose`.
    None,
    On,
    /// Switched off from the menu, for now.
    Off,
    /// Off until WinCompose quits.
    HeldForWinCompose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    paused: bool,
    link: Link,
    compose: ComposeState,
}

/// What the tray shows for a state.
#[derive(Debug, PartialEq, Eq)]
struct View {
    icon: IconKind,
    tooltip: String,
    /// The keyboard's status line.
    status: String,
    /// The Compose key's menu line.
    compose: String,
}

/// `target` names the keyboard while it is absent, as in `mathpad (1209:2211)`, and
/// `key` the Compose key, as in "Right Alt".
fn view(state: &State, target: &str, key: &str) -> View {
    let status = match &state.link {
        Link::Waiting => format!("Waiting for {target}"),
        Link::Connected(device) => format!("Connected: {device}"),
        Link::Failed(error) => format!("Stopped: {error}"),
    };
    let compose = match state.compose {
        ComposeState::HeldForWinCompose => format!("Compose key ({key}): off while WinCompose runs"),
        ComposeState::None => "Compose key: off in settings".to_owned(),
        _ => format!("Compose key ({key})"),
    };
    let composing = state.compose == ComposeState::On;
    let icon = match (&state.link, state.paused) {
        (_, true) => IconKind::Paused,
        (Link::Connected(_), false) => IconKind::Active,
        _ if composing => IconKind::Active,
        _ => IconKind::Waiting,
    };
    let paused = if state.paused { " (paused)" } else { "" };
    let compose_part = if composing { format!("Compose: {key}; ") } else { String::new() };
    let tooltip =
        format!("{TITLE}{paused}: {compose_part}{status}").chars().take(TOOLTIP_MAX_CHARS).collect();
    View { icon, tooltip, status, compose }
}

/// The dispatch thread: types what the keyboard sends.
struct Dispatch {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct Menus {
    status: MenuItem,
    compose: CheckMenuItem,
    pause: CheckMenuItem,
    settings: MenuItem,
    login: CheckMenuItem,
    login_elevated: CheckMenuItem,
    reload: MenuItem,
    open_log: MenuItem,
    open_log_folder: MenuItem,
    quit: MenuItem,
}

struct Tray {
    icon: TrayIcon,
    menus: Menus,
    state: State,
    shown: Option<IconKind>,
    settings: Settings,
    overrides: Overrides,
    /// The saved settings with the flags applied: what is running.
    config: Config,
    target: Option<Target>,
    paused: Arc<AtomicBool>,
    dispatch: Option<Dispatch>,
    updates: Sender<Update>,
    compose: Option<Compose>,
    /// When WinCompose was last looked for, while it holds the Compose key off.
    wincompose_checked: Instant,
    log_dir: PathBuf,
    packaged: bool,
}

impl Tray {
    fn new(
        settings: Settings,
        overrides: Overrides,
        log_dir: PathBuf,
        updates: Sender<Update>,
    ) -> Result<Self, BoxError> {
        let packaged = app::is_packaged();
        let (login_on, elevated_on) = login_state();
        let menus = Menus {
            status: MenuItem::new("", false, None),
            compose: CheckMenuItem::new("", true, false, None),
            pause: CheckMenuItem::new("Pause", true, false, None),
            settings: MenuItem::new("Settings\u{2026}", true, None),
            login: CheckMenuItem::new("Start at login", !packaged, login_on, None),
            login_elevated: CheckMenuItem::new(
                "Start at login as administrator",
                !packaged,
                elevated_on,
                None,
            ),
            reload: MenuItem::new("Reload Compose sequences", true, None),
            open_log: MenuItem::new("Open log", true, None),
            open_log_folder: MenuItem::new("Open log folder", true, None),
            quit: MenuItem::new("Quit", true, None),
        };
        let menu = Menu::new();
        menu.append_items(&[
            &menus.status,
            &PredefinedMenuItem::separator(),
            &menus.compose,
            &menus.pause,
            &menus.settings,
            &PredefinedMenuItem::separator(),
            &menus.login,
            &menus.login_elevated,
            &PredefinedMenuItem::separator(),
            &menus.reload,
            &menus.open_log,
            &menus.open_log_folder,
            &PredefinedMenuItem::separator(),
            &menus.quit,
        ])?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            // Left click pauses; the menu is on the right button.
            .with_menu_on_left_click(false)
            .build()?;
        let config = overrides.apply(&settings.saved);
        let mut tray = Tray {
            icon,
            menus,
            state: State { paused: false, link: Link::Waiting, compose: ComposeState::None },
            shown: None,
            settings,
            overrides,
            config,
            target: None,
            paused: Arc::new(AtomicBool::new(false)),
            dispatch: None,
            updates,
            compose: None,
            wincompose_checked: Instant::now(),
            log_dir,
            packaged,
        };
        tray.start_dispatch();
        tray.start_compose();
        tray.refresh();
        Ok(tray)
    }

    fn start_dispatch(&mut self) {
        let target = match device::resolve(&self.config.keyboard) {
            Ok(target) => target,
            Err(e) => {
                tracing::error!("{e}");
                self.target = None;
                self.state.link = Link::Failed(e);
                return;
            }
        };
        let mut sink = match typing::make_sink(false, &self.config.workarounds) {
            Ok(sink) => sink,
            Err(e) => {
                self.state.link = Link::Failed(e);
                return;
            }
        };
        let stop = Arc::new(AtomicBool::new(false));
        let ctl = Control { stop: Arc::clone(&stop), paused: Arc::clone(&self.paused) };
        let tx = self.updates.clone();
        let main_thread = app::current_thread_id();
        let thread_target = target.clone();
        let spawned = std::thread::Builder::new().name("dispatch".into()).spawn(move || {
            let notify = |update| {
                // The tray is gone only when the app is quitting.
                let _ = tx.send(update);
                app::wake(main_thread);
            };
            let result =
                bridge::run(&thread_target, &mut sink, None, &ctl, |status| notify(Update::Status(status)));
            if let Err(e) = result {
                tracing::error!("stopped: {e}");
                notify(Update::Failed(e.to_string()));
            }
        });
        match spawned {
            Ok(thread) => {
                self.dispatch = Some(Dispatch { stop, thread });
                self.target = Some(target);
                self.state.link = Link::Waiting;
            }
            Err(e) => self.state.link = Link::Failed(e.to_string()),
        }
    }

    fn stop_dispatch(&mut self) {
        if let Some(dispatch) = self.dispatch.take() {
            dispatch.stop.store(true, Ordering::Relaxed);
            if dispatch.thread.join().is_err() {
                tracing::error!("the dispatch thread panicked");
            }
        }
    }

    fn start_compose(&mut self) {
        let quirks = typing::quirk_set(&self.config.workarounds);
        match compose::start(&self.config.compose, quirks, Arc::clone(&self.paused)) {
            Ok(Some(started)) => {
                self.state.compose = if started.held_for_wincompose {
                    ComposeState::HeldForWinCompose
                } else {
                    ComposeState::On
                };
                self.compose = Some(started.compose);
            }
            Ok(None) => self.state.compose = ComposeState::None,
            Err(e) => {
                tracing::error!("cannot start the Compose key: {e}");
                app::message_box(TITLE, &format!("Cannot start the Compose key: {e}"), true);
                self.state.compose = ComposeState::None;
            }
        }
    }

    fn stop_compose(&mut self) {
        // Dropping it removes the hook and ends its thread.
        self.compose = None;
    }

    fn shut_down(&mut self) {
        self.stop_compose();
        self.stop_dispatch();
    }

    /// Handles everything that arrived since the last call. Returns false on Quit.
    fn poll(&mut self, updates: &Receiver<Update>) -> bool {
        let before = self.state.clone();
        while let Ok(update) = updates.try_recv() {
            self.state.link = match update {
                Update::Status(Status::Waiting) => Link::Waiting,
                Update::Status(Status::Connected(device)) => {
                    let fallback = self.target.as_ref().map_or("keyboard", |t| t.name);
                    let name = device.product.unwrap_or_else(|| fallback.to_owned());
                    Link::Connected(format!("{name} ({:04x}:{:04x})", device.vendor_id, device.product_id))
                }
                Update::Failed(error) => Link::Failed(error),
            };
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                self.toggle_pause();
            }
        }
        while let Ok(MenuEvent { id }) = MenuEvent::receiver().try_recv() {
            let menus = &self.menus;
            if id == *menus.pause.id() {
                self.toggle_pause();
            } else if id == *menus.compose.id() {
                self.toggle_compose();
            } else if id == *menus.settings.id() {
                self.open_settings();
            } else if id == *menus.login.id() {
                self.set_login(if login_state().0 { Login::Off } else { Login::Normal });
            } else if id == *menus.login_elevated.id() {
                self.set_login(if login_state().1 { Login::Off } else { Login::Elevated });
            } else if id == *menus.reload.id() {
                self.reload_rules(true);
            } else if id == *menus.open_log.id() {
                let path = logging::newest_log(&self.log_dir).unwrap_or_else(|| self.log_dir.clone());
                open(&path);
            } else if id == *menus.open_log_folder.id() {
                open(&self.log_dir);
            } else if id == *menus.quit.id() {
                return false;
            }
        }
        self.watch_wincompose();
        if self.state != before {
            self.refresh();
        }
        true
    }

    fn toggle_pause(&mut self) {
        self.state.paused = !self.state.paused;
        self.paused.store(self.state.paused, Ordering::Relaxed);
        if let Some(compose) = &self.compose {
            compose.paused_changed();
        }
        tracing::info!("{}", if self.state.paused { "paused" } else { "resumed" });
    }

    /// Switches a running Compose key on or off for now, without saving it.
    fn toggle_compose(&mut self) {
        let Some(compose) = &self.compose else {
            // Off in the settings: the menu cannot start it; the Settings window can.
            self.refresh();
            return;
        };
        let on = self.state.compose != ComposeState::On;
        if on && self.state.compose == ComposeState::HeldForWinCompose {
            tracing::warn!("Compose key switched on while WinCompose runs: both will act on it");
        }
        compose.set_enabled(on);
        self.state.compose = if on { ComposeState::On } else { ComposeState::Off };
        tracing::info!("Compose key {}", if on { "on" } else { "off" });
    }

    /// Switches the Compose key on once WinCompose has quit.
    fn watch_wincompose(&mut self) {
        if self.state.compose != ComposeState::HeldForWinCompose
            || self.wincompose_checked.elapsed() < WINCOMPOSE_CHECK
        {
            return;
        }
        self.wincompose_checked = Instant::now();
        if let Some(compose) = self.compose.as_ref().filter(|_| !app::process_running(WINCOMPOSE_EXE)) {
            compose.set_enabled(true);
            self.state.compose = ComposeState::On;
            tracing::info!("WinCompose has quit; Compose key on");
        }
    }

    /// Rereads the sequences. With `report`, says how it went in a message box.
    fn reload_rules(&mut self, report: bool) {
        let (table, problems) = compose::load_table(&self.config.compose);
        let count = table.len();
        if let Some(compose) = &self.compose {
            compose.set_table(table);
        }
        if report {
            let mut text = format!("{count} Compose sequences loaded.");
            if !problems.is_empty() {
                text.push_str(&format!("\n\n{} problems in your sequences:\n", problems.len()));
                for problem in problems.iter().take(8) {
                    text.push_str(&format!("\n{problem}"));
                }
            }
            app::message_box(TITLE, &text, !problems.is_empty());
        }
    }

    fn set_login(&mut self, login: Login) {
        if let Err(e) = apply_login(login, &self.overrides) {
            tracing::error!("cannot change start at login: {e}");
            app::message_box(TITLE, &format!("Cannot change start at login: {e}"), true);
        }
        self.sync_login();
    }

    /// Shows what Windows says, whatever happened.
    fn sync_login(&self) {
        let (login_on, elevated_on) = login_state();
        self.menus.login.set_checked(login_on);
        self.menus.login_elevated.set_checked(elevated_on);
    }

    fn open_settings(&mut self) {
        let (login_on, elevated_on) = login_state();
        let login = match (login_on, elevated_on) {
            (_, true) => Login::Elevated,
            (true, false) => Login::Normal,
            (false, false) => Login::Off,
        };
        let status = view(&self.state, &self.target_label(), "").status;
        let rules_file = rules_file().map(|p| p.display().to_string()).unwrap_or_default();
        let context = Context {
            overridden: self.overrides.sections(),
            packaged: self.packaged,
            login,
            keyboard_status: &status,
            rules_file: &rules_file,
        };
        let form = settings_window::form(&self.settings.saved, &context);
        let mut handler = SettingsHandler { tray: self, login };
        if let Err(e) = dialog::show(&form, &mut handler) {
            tracing::error!("cannot show the settings: {e}");
        }
        self.refresh();
        self.sync_login();
    }

    /// Saves `saved` and restarts whatever its changes touch.
    fn apply_settings(&mut self, saved: Config) -> Result<(), String> {
        if saved != self.settings.saved {
            self.settings.save(saved).map_err(|e| format!("Cannot save the settings: {e}"))?;
            tracing::info!("settings saved");
        }
        let config = self.overrides.apply(&self.settings.saved);
        let old = std::mem::replace(&mut self.config, config);
        let workarounds_changed = old.workarounds != self.config.workarounds;
        if workarounds_changed || old.keyboard != self.config.keyboard {
            self.stop_dispatch();
            self.start_dispatch();
        }
        if workarounds_changed || old.compose != self.config.compose {
            self.stop_compose();
            self.start_compose();
        }
        self.refresh();
        Ok(())
    }

    fn target_label(&self) -> String {
        self.target.as_ref().map_or_else(|| "the keyboard".to_owned(), Target::label)
    }

    fn refresh(&mut self) {
        let key = self.compose.as_ref().map_or("off", |c| c.key().label());
        let view = view(&self.state, &self.target_label(), key);
        if self.shown != Some(view.icon) {
            match self.icon.set_icon(Some(icons::icon(view.icon))) {
                Ok(()) => self.shown = Some(view.icon),
                Err(e) => tracing::warn!("cannot set the tray icon: {e}"),
            }
        }
        if let Err(e) = self.icon.set_tooltip(Some(&view.tooltip)) {
            tracing::warn!("cannot set the tray tooltip: {e}");
        }
        self.menus.status.set_text(&view.status);
        self.menus.compose.set_text(&view.compose);
        self.menus.compose.set_enabled(self.compose.is_some());
        // A click on a check item toggles its mark by itself; keep it in step with the state.
        self.menus.compose.set_checked(self.state.compose == ComposeState::On);
        self.menus.pause.set_checked(self.state.paused);
    }
}

/// The Settings window's handler: applies the settings to the running tray.
struct SettingsHandler<'a> {
    tray: &'a mut Tray,
    /// How the app started at login when the window opened.
    login: Login,
}

impl Handler for SettingsHandler<'_> {
    fn apply(&mut self, values: &Values) -> Result<(), String> {
        let (saved, login) = settings_window::read(values, &self.tray.settings.saved)?;
        self.tray.apply_settings(saved)?;
        if let Some(login) = login.filter(|&l| l != self.login) {
            apply_login(login, &self.tray.overrides)
                .map_err(|e| format!("Cannot change start at login: {e}"))?;
            self.login = login;
        }
        Ok(())
    }

    fn button(&mut self, id: &'static str) {
        match id {
            EDIT_RULES => edit_rules(),
            RELOAD_RULES => self.tray.reload_rules(true),
            _ => {}
        }
    }
}

/// `%USERPROFILE%\.XCompose`, the file "Edit my sequences" opens.
fn rules_file() -> Option<PathBuf> {
    uc_xcompose::FsIncludes::from_env().home.map(|home| home.join(".XCompose"))
}

/// Opens the user's sequences in Notepad, creating the file with a short guide first.
fn edit_rules() {
    let Some(path) = rules_file() else { return };
    if !path.exists() {
        if let Err(e) = std::fs::write(&path, settings_window::RULES_TEMPLATE) {
            app::message_box(TITLE, &format!("Cannot create {}: {e}", path.display()), true);
            return;
        }
    }
    // Notepad, rather than whatever opens files with no extension.
    if let Err(e) = std::process::Command::new("notepad.exe").arg(&path).spawn() {
        app::message_box(TITLE, &format!("Cannot open {}: {e}", path.display()), true);
    }
}

fn apply_login(login: Login, overrides: &Overrides) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let args = overrides.to_args();
    match login {
        Login::Off => autostart::disable()?,
        Login::Normal => tracing::info!("starts at login: {}", autostart::enable(&exe, &args)?),
        Login::Elevated => {
            tracing::info!("starts at login as administrator: {}", autostart::enable_elevated(&exe, &args)?)
        }
    }
    Ok(())
}

/// Whether the app starts at login normally, and as administrator.
fn login_state() -> (bool, bool) {
    let normal = autostart::state().map(|s| matches!(s, autostart::State::On(_)));
    let elevated = autostart::elevated_command().map(|c| c.is_some());
    for error in [normal.as_ref().err(), elevated.as_ref().err()].into_iter().flatten() {
        tracing::warn!("cannot read the login entry: {error}");
    }
    (normal.unwrap_or(false), elevated.unwrap_or(false))
}

fn open(path: &std::path::Path) {
    if let Err(e) = app::open(path) {
        tracing::error!("{e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "mathpad (1209:2211)";

    fn state(paused: bool, link: Link, compose: ComposeState) -> State {
        State { paused, link, compose }
    }

    fn show(state: &State) -> View {
        view(state, TARGET, "Right Alt")
    }

    #[test]
    fn waiting_without_compose_shows_the_target_and_a_grey_ring() {
        let view = show(&state(false, Link::Waiting, ComposeState::None));
        assert_eq!(view.icon, IconKind::Waiting);
        assert_eq!(view.status, "Waiting for mathpad (1209:2211)");
        assert_eq!(view.tooltip, "unicompose: Waiting for mathpad (1209:2211)");
        assert_eq!(view.compose, "Compose key: off in settings");
    }

    #[test]
    fn the_compose_key_alone_makes_the_icon_active() {
        let view = show(&state(false, Link::Waiting, ComposeState::On));
        assert_eq!(view.icon, IconKind::Active);
        assert_eq!(view.tooltip, "unicompose: Compose: Right Alt; Waiting for mathpad (1209:2211)");
        assert_eq!(view.compose, "Compose key (Right Alt)");
    }

    #[test]
    fn wincompose_holding_the_key_is_shown() {
        let view = show(&state(false, Link::Waiting, ComposeState::HeldForWinCompose));
        assert_eq!(view.icon, IconKind::Waiting);
        assert_eq!(view.compose, "Compose key (Right Alt): off while WinCompose runs");
    }

    #[test]
    fn connected_is_active_until_paused() {
        let link = Link::Connected("Mathpad (1209:2211)".into());
        assert_eq!(show(&state(false, link.clone(), ComposeState::None)).icon, IconKind::Active);
        let paused = show(&state(true, link, ComposeState::None));
        assert_eq!(paused.icon, IconKind::Paused);
        assert_eq!(paused.tooltip, "unicompose (paused): Connected: Mathpad (1209:2211)");
    }

    #[test]
    fn failure_is_shown_and_the_tooltip_fits_windows_limit() {
        let view = show(&state(false, Link::Failed("x".repeat(200)), ComposeState::None));
        assert_eq!(view.icon, IconKind::Waiting);
        assert!(view.status.starts_with("Stopped: x"));
        assert_eq!(view.tooltip.chars().count(), TOOLTIP_MAX_CHARS);
    }

    #[test]
    fn login_entry_gets_only_the_flags_given() {
        let args = TrayArgs::parse_from(["unicompose-tray", "--vid", "1234", "--compose-key", "menu"]);
        assert_eq!(args.overrides.to_args(), ["--vid", "1234", "--compose-key", "menu"]);
        assert!(TrayArgs::parse_from(["unicompose-tray"]).overrides.to_args().is_empty());
    }
}
