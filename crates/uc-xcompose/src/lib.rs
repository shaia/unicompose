//! XCompose files: the Linux format for Compose-key rules such as
//! `<Multi_key> <o> <quotedbl> : "ö"`.
//!
//! [`load`] reads a file into a [`RuleSet`], following `include` lines and resolving
//! conflicts exactly as libxkbcommon does, so a file means the same here as on Linux.
//! [`write`] turns rules back into text. [`bundled`] holds libX11's en_US.UTF-8 rules, the
//! default set, which is also what `include "%L"` reads.

pub mod bundled;
mod includes;
mod parser;
mod rules;
pub mod trie;
mod writer;

pub use includes::{Expand, FsIncludes, Includes, MapIncludes};
pub use parser::{load, load_into, Diagnostic, Dialect, LoadError, Loaded, Severity};
pub use rules::{Added, Output, Rule, RuleSet};
pub use writer::write;
