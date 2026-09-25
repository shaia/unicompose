//! Windows backends. Empty on other platforms.
#![cfg(windows)]

pub mod app;
pub mod autostart;
pub mod compose;
pub mod dialog;
pub mod logon_task;
mod quirks;
mod sink;

pub use quirks::{Quirk, QuirkSet};
pub use sink::{SendInputSink, INJECTED_MARKER};
