//! Raw report recordings: one report per line as `<ms since start>\t<hex bytes>`,
//! with `#` comment lines for connect/disconnect.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use uc_hid::HidEvent;

const HEADER: &str = "# unicompose raw HID recording v1";

pub struct Recorder {
    out: Box<dyn Write>,
    start: Instant,
}

impl Recorder {
    /// Appends to `path`, creating it if needed.
    pub fn create(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Self::new(Box::new(BufWriter::new(file)))
    }

    pub fn new(mut out: Box<dyn Write>) -> io::Result<Self> {
        writeln!(out, "{HEADER}")?;
        Ok(Self { out, start: Instant::now() })
    }

    pub fn comment(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.out, "# {text}")?;
        self.out.flush()
    }

    pub fn report(&mut self, bytes: &[u8]) -> io::Result<()> {
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        writeln!(self.out, "{}\t{hex}", self.start.elapsed().as_millis())?;
        // Flush per report so a crash or Ctrl+C never loses the interesting part.
        self.out.flush()
    }

    pub fn write_event(&mut self, event: &HidEvent) -> io::Result<()> {
        match event {
            HidEvent::Connected(device) => self.comment(&format!("connected {}", device.path)),
            HidEvent::Disconnected => self.comment("disconnected"),
            HidEvent::Report(bytes) => self.report(bytes),
        }
    }
}

/// Reads the reports from a recording, ignoring timestamps and comments.
pub fn read(path: &Path) -> io::Result<Vec<Vec<u8>>> {
    parse(BufReader::new(File::open(path)?))
}

fn parse(input: impl BufRead) -> io::Result<Vec<Vec<u8>>> {
    let mut reports = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let invalid =
            |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("line {}: {what}", index + 1));
        let (_, hex) = line.split_once('\t').ok_or_else(|| invalid("expected <ms>\\t<hex>"))?;
        if hex.len() % 2 != 0 || !hex.is_ascii() {
            return Err(invalid("odd-length or non-ASCII hex"));
        }
        let report = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|_| invalid("bad hex digit"))?;
        reports.push(report);
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A `Write` whose contents stay readable after the Recorder takes ownership.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn recording_round_trips() {
        let buf = Shared::default();
        let mut recorder = Recorder::new(Box::new(buf.clone())).unwrap();
        let alpha = uc_codec::encode('α').to_vec();
        let math_a = uc_codec::encode('𝔸').to_vec();
        recorder.comment("connected").unwrap();
        recorder.report(&alpha).unwrap();
        recorder.report(&math_a).unwrap();

        let text = buf.0.lock().unwrap().clone();
        assert_eq!(parse(&text[..]).unwrap(), [alpha, math_a]);
    }

    #[test]
    fn rejects_malformed_lines_with_line_number() {
        let err = parse(&b"# ok\n12\t0g\n"[..]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().starts_with("line 2:"), "{err}");
    }
}
