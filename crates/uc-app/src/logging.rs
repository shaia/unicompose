//! Log setup. The command line logs to stderr; the tray app has no console, so it logs
//! to daily files.

use std::io;
use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

/// Log files kept, one per day.
const KEEP_DAYS: usize = 7;
const PREFIX: &str = "unicompose";
const SUFFIX: &str = "log";

/// Verbosity from RUST_LOG, default info.
fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

pub fn init_stderr() {
    tracing_subscriber::fmt().with_env_filter(filter()).with_writer(io::stderr).init();
}

/// `%LOCALAPPDATA%\unicompose\logs`, or `None` if the variable is unset.
pub fn log_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("unicompose").join("logs"))
}

/// Logs to `unicompose.<date>.log` in `dir`, deleting all but the newest few. Log lines
/// are written by a background thread until the returned guard is dropped.
pub fn init_file(dir: &Path) -> Result<WorkerGuard, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dir)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(PREFIX)
        .filename_suffix(SUFFIX)
        .max_log_files(KEEP_DAYS)
        .build(dir)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt().with_env_filter(filter()).with_writer(writer).with_ansi(false).init();
    Ok(guard)
}

/// The most recently written log file in `dir`.
pub fn newest_log(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| is_log_name(&entry.file_name().to_string_lossy()))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .max()
        .map(|(_, path)| path)
}

fn is_log_name(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .and_then(|rest| rest.strip_suffix(SUFFIX))
        .is_some_and(|middle| middle.starts_with('.') && middle.ends_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_only_its_own_log_files() {
        assert!(is_log_name("unicompose.2026-09-25.log"));
        assert!(!is_log_name("unicompose.2026-09-25.txt"));
        assert!(!is_log_name("other.2026-09-25.log"));
    }
}
