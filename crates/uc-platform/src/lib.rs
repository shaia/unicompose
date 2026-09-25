//! Traits shared between the platform backends and the app.

use std::fmt;

/// Types text into whatever window has keyboard focus.
pub trait TextSink {
    fn type_text(&mut self, text: &str) -> Result<(), SinkError>;
}

impl<T: TextSink + ?Sized> TextSink for Box<T> {
    fn type_text(&mut self, text: &str) -> Result<(), SinkError> {
        (**self).type_text(text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinkError(pub String);

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SinkError {}
