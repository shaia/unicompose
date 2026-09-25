//! unicompose: a Compose key, and typing what a keyboard sends over Raw HID.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use uc_app::bridge::{self, BoxError, Control};
use uc_app::device::{self, DeviceArgs};
use uc_app::record::{self, Recorder};
use uc_app::settings::{Overrides, Settings};
use uc_app::typing;
use uc_app::{describe, logging};
use uc_codec::Report;
use uc_config::Config;

/// A Compose key, and typing what a keyboard sends over Raw HID into the focused window.
///
/// Settings live in %APPDATA%\unicompose\config.toml (see `unicompose settings`); the
/// flags below override them for one run. Log verbosity follows RUST_LOG (default: info).
/// For the tray icon, run unicompose-tray instead.
#[derive(Parser)]
#[command(version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    run: RunArgs,
}

#[derive(Subcommand)]
enum Command {
    /// List HID collections, marking the one the keyboard settings select.
    Devices {
        #[command(flatten)]
        device: DeviceArgs,
        /// List every HID collection, not just the profile's vendor and product.
        #[arg(long)]
        all: bool,
    },
    /// Decode a recording made with --record, one character per line.
    Decode {
        path: PathBuf,
        /// Device profile whose codec reads the recording [default: mathpad].
        #[arg(long, value_name = "NAME")]
        device: Option<String>,
    },
    /// Show where the settings file is, and the settings in effect with these flags.
    Settings {
        #[command(flatten)]
        overrides: Overrides,
    },
    /// Start unicompose-tray at login, passing on any flags given here.
    Autostart {
        action: AutostartAction,
        /// Start it as administrator, through a logon task, so it also works in windows
        /// that run as administrator. Asks for permission (UAC).
        #[arg(long)]
        elevated: bool,
        #[command(flatten)]
        overrides: Overrides,
    },
    /// Check Compose rules and report problems line by line. Without FILE, checks the
    /// rules the Compose key uses: the built-in ones plus your .XCompose.
    Rules {
        file: Option<PathBuf>,
        /// Print every rule, one per line, after includes and overrides are applied.
        #[arg(long)]
        dump: bool,
        /// Read FILE exactly as libxkbcommon (Linux) does, without WinCompose's extensions.
        #[arg(long, requires = "file")]
        strict: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum AutostartAction {
    Enable,
    Disable,
    Status,
}

#[derive(Args)]
struct RunArgs {
    #[command(flatten)]
    overrides: Overrides,
    /// Append every raw report to FILE.
    #[arg(long, value_name = "FILE")]
    record: Option<PathBuf>,
    /// Print characters instead of typing them. No Compose key.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> ExitCode {
    logging::init_stderr();
    let cli = Cli::parse();
    let result = match cli.command {
        None => run(cli.run),
        Some(Command::Devices { device, all }) => devices(&device, all),
        Some(Command::Decode { path, device }) => decode(&path, device.as_deref()),
        Some(Command::Settings { overrides }) => settings(&overrides),
        Some(Command::Autostart { action, elevated, overrides }) => autostart(action, elevated, &overrides),
        Some(Command::Rules { file, dump, strict }) => rules(file.as_deref(), dump, strict),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// The saved settings with `overrides` on top. A settings file that cannot be read is an
/// error here: on the command line, it is better to stop than to run with the defaults.
fn effective(overrides: &Overrides) -> Result<Config, BoxError> {
    let settings = Settings::load();
    if let Some(problem) = settings.problem {
        return Err(problem.into());
    }
    Ok(overrides.apply(&settings.saved))
}

fn run(args: RunArgs) -> Result<ExitCode, BoxError> {
    let config = effective(&args.overrides)?;
    let target = device::resolve(&config.keyboard)?;
    let mut sink = typing::make_sink(args.dry_run, &config.workarounds)?;
    let recorder = args.record.as_deref().map(Recorder::create).transpose()?;
    let ctl = Control::default();
    // The hook stays installed for as long as this is alive.
    let _compose = start_compose(&config, args.dry_run, &ctl)?;
    let on_ctrlc = Arc::clone(&ctl.stop);
    ctrlc::set_handler(move || on_ctrlc.store(true, Ordering::Relaxed))?;
    bridge::run(&target, &mut sink, recorder, &ctl, |_| {})?;
    Ok(ExitCode::SUCCESS)
}

/// The Compose key, unless printing instead of typing: a dry run must not swallow keys.
#[cfg(windows)]
fn start_compose(
    config: &Config,
    dry_run: bool,
    ctl: &Control,
) -> Result<Option<uc_app::compose::Started>, BoxError> {
    if dry_run {
        return Ok(None);
    }
    let quirks = typing::quirk_set(&config.workarounds);
    let started = uc_app::compose::start(&config.compose, quirks, Arc::clone(&ctl.paused))?;
    if started.as_ref().is_some_and(|s| s.held_for_wincompose) {
        tracing::warn!("quit WinCompose and start unicompose again to use its Compose key");
    }
    Ok(started)
}

#[cfg(not(windows))]
fn start_compose(_: &Config, _: bool, _: &Control) -> Result<Option<()>, BoxError> {
    Ok(None)
}

fn devices(args: &DeviceArgs, all: bool) -> Result<ExitCode, BoxError> {
    let overrides = Overrides { device: args.clone(), ..Overrides::default() };
    let target = device::resolve(&effective(&overrides)?.keyboard)?;
    let vendor_product = (!all).then_some((target.filter.vendor_id, target.filter.product_id));
    let devices = uc_hid::list_devices(vendor_product)?;
    println!("looking for {target}");
    if devices.is_empty() {
        eprintln!("no matching HID devices found");
        return Ok(ExitCode::FAILURE);
    }
    for d in devices {
        println!(
            "{}{:04x}:{:04x} usage {:04x}:{:02x}  {} / {}  {}",
            if target.filter.matches(&d) { "* " } else { "  " },
            d.vendor_id,
            d.product_id,
            d.usage_page,
            d.usage,
            d.manufacturer.as_deref().unwrap_or("?"),
            d.product.as_deref().unwrap_or("?"),
            d.path,
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Prints `U+XXXX<tab>char` per decoded character; fails if any report does not decode.
fn decode(path: &Path, profile: Option<&str>) -> Result<ExitCode, BoxError> {
    let profile = device::find(profile.unwrap_or(device::DEFAULT))?;
    let mut code = ExitCode::SUCCESS;
    for (index, report) in record::read(path)?.iter().enumerate() {
        match (profile.decode)(report) {
            Ok(Report::Codepoint(c)) => println!("{}", describe(c)),
            Ok(Report::Unknown { command }) => {
                eprintln!("report {}: unknown command 0x{command:02x}", index + 1)
            }
            Err(e) => {
                eprintln!("report {}: {e}", index + 1);
                code = ExitCode::FAILURE;
            }
        }
    }
    Ok(code)
}

fn settings(overrides: &Overrides) -> Result<ExitCode, BoxError> {
    match Config::default_path() {
        Some(path) => eprintln!("settings file: {}", path.display()),
        None => eprintln!("APPDATA is not set, so settings are not saved"),
    }
    print!("{}", effective(overrides)?.to_toml());
    Ok(ExitCode::SUCCESS)
}

fn rules(path: Option<&Path>, dump: bool, strict: bool) -> Result<ExitCode, BoxError> {
    use uc_xcompose::{bundled, load_into, Dialect, FsIncludes, RuleSet};
    let includes = &mut FsIncludes::from_env();
    let loaded = match path {
        Some(path) => {
            let text = std::fs::read_to_string(path)?;
            let dialect = if strict { Dialect::Xkbcommon } else { Dialect::WinCompose };
            let mut rules = RuleSet::new();
            match load_into(&mut rules, &text, &path.display().to_string(), includes, dialect) {
                Ok(diagnostics) => uc_xcompose::Loaded { rules, diagnostics },
                Err(e) => {
                    e.diagnostics.iter().for_each(|d| eprintln!("{d}"));
                    return Ok(ExitCode::FAILURE);
                }
            }
        }
        None => {
            let files = includes.home.as_deref().map(bundled::user_files).unwrap_or_default();
            files.iter().for_each(|f| eprintln!("reading {}", f.display()));
            bundled::load_default(true, &files, includes)
        }
    };
    loaded.diagnostics.iter().for_each(|d| eprintln!("{d}"));
    if dump {
        print!("{}", uc_xcompose::write(&loaded.rules));
    }
    eprintln!("{} rules, {} warnings or errors", loaded.rules.len(), loaded.diagnostics.len());
    Ok(ExitCode::SUCCESS)
}

#[cfg(windows)]
fn autostart(action: AutostartAction, elevated: bool, overrides: &Overrides) -> Result<ExitCode, BoxError> {
    use uc_app::autostart::{self, State};
    match action {
        AutostartAction::Enable => {
            // Refuse a command line the tray would reject at every login.
            let config = effective(overrides)?;
            device::resolve(&config.keyboard)?;
            uc_app::compose::compose_key(&config.compose)?;
            let exe = autostart::tray_exe_next_to_current()?;
            let args = overrides.to_args();
            if elevated {
                let command = autostart::enable_elevated(&exe, &args)?;
                println!("starts at login as administrator: {command}");
            } else {
                let command = autostart::enable(&exe, &args)?;
                println!("starts at login: {command}");
            }
        }
        AutostartAction::Disable => {
            autostart::disable()?;
            println!("does not start at login");
        }
        AutostartAction::Status => {
            match autostart::state()? {
                State::Off => {}
                State::On(command) => println!("starts at login: {command}"),
                State::Disabled(command) => println!("switched off in Task Manager: {command}"),
            }
            match autostart::elevated_command()? {
                Some(command) => println!("starts at login as administrator: {command}"),
                None if autostart::state()? == State::Off => println!("does not start at login"),
                None => {}
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(windows))]
fn autostart(_: AutostartAction, _: bool, _: &Overrides) -> Result<ExitCode, BoxError> {
    Err("start at login is only implemented on Windows so far".into())
}
