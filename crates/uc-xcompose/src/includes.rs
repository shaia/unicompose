//! Where `include` lines lead: the `%` expansions and the files they name.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;

use crate::bundled;

/// The `%` escapes in an include path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expand {
    /// `%H`: the home directory.
    Home,
    /// `%L`: the Compose file for the current locale.
    LocaleFile,
    /// `%S`: the system's X locale directory.
    SystemDir,
}

pub trait Includes {
    /// The text a `%` escape stands for, or `None` if it has no value here.
    fn expand(&self, what: Expand) -> Option<String>;
    /// Reads an included file.
    fn read(&mut self, path: &str) -> io::Result<String>;
}

/// Includes from the file system. `%L` is the bundled en_US.UTF-8 file, since Windows has
/// no system Compose files.
#[derive(Debug, Clone, Default)]
pub struct FsIncludes {
    pub home: Option<PathBuf>,
    pub system_dir: Option<PathBuf>,
}

impl FsIncludes {
    /// `%H` from `HOME`, or `USERPROFILE` on Windows.
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        FsIncludes { home, system_dir: None }
    }
}

impl Includes for FsIncludes {
    fn expand(&self, what: Expand) -> Option<String> {
        match what {
            Expand::Home => self.home.as_ref().map(|p| p.to_string_lossy().into_owned()),
            Expand::LocaleFile => Some(bundled::EN_US_UTF8_PATH.to_owned()),
            Expand::SystemDir => self.system_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
        }
    }

    fn read(&mut self, path: &str) -> io::Result<String> {
        if path == bundled::EN_US_UTF8_PATH {
            return Ok(bundled::EN_US_UTF8.to_owned());
        }
        std::fs::read_to_string(path)
    }
}

/// Includes from memory, for tests and for text that never touches the disk.
#[derive(Debug, Clone, Default)]
pub struct MapIncludes {
    home: Option<String>,
    locale_file: Option<String>,
    files: HashMap<String, String>,
}

impl MapIncludes {
    #[must_use]
    pub fn with_home(mut self, home: &str) -> Self {
        self.home = Some(home.to_owned());
        self
    }

    /// `%L` names `path`.
    #[must_use]
    pub fn with_locale_file(mut self, path: &str) -> Self {
        self.locale_file = Some(path.to_owned());
        self
    }

    #[must_use]
    pub fn with_file(mut self, path: &str, text: &str) -> Self {
        self.files.insert(path.to_owned(), text.to_owned());
        self
    }
}

impl Includes for MapIncludes {
    fn expand(&self, what: Expand) -> Option<String> {
        match what {
            Expand::Home => self.home.clone(),
            Expand::LocaleFile => self.locale_file.clone(),
            Expand::SystemDir => None,
        }
    }

    fn read(&mut self, path: &str) -> io::Result<String> {
        self.files.get(path).cloned().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }
}
