//! Carries reports from the HID listener to the text sink: the part of the app both the
//! command line and the tray run.

use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use uc_codec::Report;
use uc_hid::{Device, HidEvent};
use uc_platform::TextSink;

use crate::describe;
use crate::device::Target;
use crate::record::Recorder;

pub type BoxError = Box<dyn Error + Send + Sync>;

/// Flags the caller flips while `run` is running.
#[derive(Debug, Clone, Default)]
pub struct Control {
    /// Ends `run` within the HID listener's read timeout.
    pub stop: Arc<AtomicBool>,
    /// Characters are dropped instead of typed. Recording continues.
    pub paused: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The device is absent: not plugged in yet, or unplugged.
    Waiting,
    Connected(Device),
}

/// Types what `target` sends until `ctl.stop` is set. Calls `on_status` with `Waiting`
/// first, then on every connect and disconnect.
pub fn run(
    target: &Target,
    sink: &mut dyn TextSink,
    recorder: Option<Recorder>,
    ctl: &Control,
    on_status: impl FnMut(Status),
) -> Result<(), BoxError> {
    tracing::info!("listening for {target}");
    let (tx, rx) = mpsc::channel();
    let stop = Arc::clone(&ctl.stop);
    let filter = target.filter;
    let listener =
        std::thread::Builder::new().name("hid".into()).spawn(move || uc_hid::listen(filter, &tx, &stop))?;
    // Ends when the listener returns and drops `tx`.
    dispatch(rx, target, sink, recorder, &ctl.paused, on_status);
    listener.join().map_err(|_| "HID listener panicked")??;
    Ok(())
}

fn dispatch(
    events: impl IntoIterator<Item = HidEvent>,
    target: &Target,
    sink: &mut dyn TextSink,
    mut recorder: Option<Recorder>,
    paused: &AtomicBool,
    mut on_status: impl FnMut(Status),
) {
    on_status(Status::Waiting);
    for event in events {
        if let Some(Err(e)) = recorder.as_mut().map(|r| r.write_event(&event)) {
            tracing::error!("recording stopped: {e}");
            recorder = None;
        }
        match event {
            HidEvent::Connected(device) => on_status(Status::Connected(device)),
            HidEvent::Disconnected => on_status(Status::Waiting),
            HidEvent::Report(bytes) => type_report(&bytes, target, sink, paused.load(Ordering::Relaxed)),
        }
    }
}

fn type_report(bytes: &[u8], target: &Target, sink: &mut dyn TextSink, paused: bool) {
    match target.decode(bytes) {
        Ok(Report::Codepoint(c)) if paused => tracing::debug!("paused, dropping {}", describe(c)),
        Ok(Report::Codepoint(c)) => {
            tracing::debug!("{}", describe(c));
            if let Err(e) = sink.type_text(c.encode_utf8(&mut [0; 4])) {
                tracing::warn!("cannot type {}: {e}", describe(c));
            }
        }
        Ok(Report::Unknown { command }) => tracing::debug!("ignoring report with command 0x{command:02x}"),
        Err(e) => tracing::warn!("ignoring report: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device;
    use uc_platform::SinkError;

    #[derive(Default)]
    struct Typed(String);

    impl TextSink for Typed {
        fn type_text(&mut self, text: &str) -> Result<(), SinkError> {
            self.0.push_str(text);
            Ok(())
        }
    }

    fn device() -> Device {
        Device {
            path: "test".into(),
            manufacturer: None,
            product: Some("Test pad".into()),
            vendor_id: 0x1209,
            product_id: 0x2211,
            usage_page: 0xFF60,
            usage: 0x61,
        }
    }

    fn replay(paused: bool) -> (String, Vec<Status>) {
        let events = vec![
            HidEvent::Connected(device()),
            HidEvent::Report(uc_codec::encode('α').to_vec()),
            HidEvent::Report(uc_codec::encode('𝔸').to_vec()),
            HidEvent::Disconnected,
        ];
        let target = Target::from(device::find(device::DEFAULT).unwrap());
        let mut sink = Typed::default();
        let mut statuses = Vec::new();
        dispatch(events, &target, &mut sink, None, &AtomicBool::new(paused), |s| statuses.push(s));
        (sink.0, statuses)
    }

    #[test]
    fn types_reports_and_reports_status_changes() {
        let (typed, statuses) = replay(false);
        assert_eq!(typed, "α𝔸");
        assert_eq!(statuses, [Status::Waiting, Status::Connected(device()), Status::Waiting]);
    }

    #[test]
    fn paused_types_nothing_but_still_tracks_the_device() {
        let (typed, statuses) = replay(true);
        assert_eq!(typed, "");
        assert_eq!(statuses, [Status::Waiting, Status::Connected(device()), Status::Waiting]);
    }
}
