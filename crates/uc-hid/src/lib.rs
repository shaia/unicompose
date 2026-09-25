//! Listens for input reports on one Raw HID collection and survives unplug/replug.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use hidapi::{DeviceInfo, HidApi, HidDevice, HidError};

/// How often to look for the device while it is absent.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);
/// Upper bound on how long `stop` can go unnoticed.
const READ_TIMEOUT_MS: i32 = 250;
/// Room for a 64-byte report; QMK uses 32.
const READ_BUF_LEN: usize = 64;

/// Selects one HID top-level collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceFilter {
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
}

impl DeviceFilter {
    /// True if `device` is the collection this filter selects.
    pub fn matches(&self, device: &Device) -> bool {
        device.vendor_id == self.vendor_id
            && device.product_id == self.product_id
            && device.usage_page == self.usage_page
            && device.usage == self.usage
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub path: String,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage: u16,
}

impl From<&DeviceInfo> for Device {
    fn from(info: &DeviceInfo) -> Self {
        Self {
            path: info.path().to_string_lossy().into_owned(),
            manufacturer: info.manufacturer_string().map(str::to_owned),
            product: info.product_string().map(str::to_owned),
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            usage_page: info.usage_page(),
            usage: info.usage(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HidEvent {
    Connected(Device),
    Disconnected,
    /// One input report, without a report ID byte.
    Report(Vec<u8>),
}

/// Lists every HID collection, optionally narrowed to one vendor/product pair.
pub fn list_devices(vendor_product: Option<(u16, u16)>) -> Result<Vec<Device>, HidError> {
    let api = HidApi::new()?;
    Ok(api
        .device_list()
        .filter(|info| {
            vendor_product.is_none_or(|(vid, pid)| info.vendor_id() == vid && info.product_id() == pid)
        })
        .map(Device::from)
        .collect())
}

/// Sends events for the device matching `filter` until `stop` is set or `tx`'s receiver
/// is dropped. Waits for the device if it is absent, and reconnects after it is unplugged.
pub fn listen(filter: DeviceFilter, tx: &Sender<HidEvent>, stop: &AtomicBool) -> Result<(), HidError> {
    let mut api = HidApi::new()?;
    let mut warned_absent = false;
    while !stop.load(Ordering::Relaxed) {
        match open(&mut api, &filter) {
            Ok(Some((device, info))) => {
                warned_absent = false;
                tracing::info!(path = %info.path, "device connected");
                if tx.send(HidEvent::Connected(info)).is_err() {
                    return Ok(());
                }
                let receiver_alive = pump(&device, tx, stop);
                drop(device);
                if !receiver_alive || tx.send(HidEvent::Disconnected).is_err() {
                    return Ok(());
                }
                tracing::info!("device disconnected");
            }
            Ok(None) if !warned_absent => {
                warned_absent = true;
                tracing::info!(
                    "waiting for device {:04x}:{:04x} (usage page {:04x}, usage {:02x})",
                    filter.vendor_id,
                    filter.product_id,
                    filter.usage_page,
                    filter.usage
                );
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("cannot open device: {e}"),
        }
        sleep_unless_stopped(RECONNECT_INTERVAL, stop);
    }
    Ok(())
}

fn open(api: &mut HidApi, filter: &DeviceFilter) -> Result<Option<(HidDevice, Device)>, HidError> {
    // Enumerate only this VID/PID: a full refresh every RECONNECT_INTERVAL is wasteful.
    api.reset_devices()?;
    api.add_devices(filter.vendor_id, filter.product_id)?;
    // add_devices narrowed the list to this VID/PID, so at most a few collections remain.
    let Some(info) = api.device_list().find(|info| filter.matches(&Device::from(*info))) else {
        return Ok(None);
    };
    let device = info.open_device(api)?;
    Ok(Some((device, Device::from(info))))
}

/// Forwards reports until the device errors (unplugged) or `stop` is set.
/// Returns false if the receiver has gone away.
fn pump(device: &HidDevice, tx: &Sender<HidEvent>, stop: &AtomicBool) -> bool {
    let mut buf = [0u8; READ_BUF_LEN];
    while !stop.load(Ordering::Relaxed) {
        match device.read_timeout(&mut buf, READ_TIMEOUT_MS) {
            Ok(0) => {}
            Ok(n) => {
                if tx.send(HidEvent::Report(buf[..n].to_vec())).is_err() {
                    return false;
                }
            }
            Err(e) => {
                tracing::debug!("read failed: {e}");
                break;
            }
        }
    }
    true
}

fn sleep_unless_stopped(total: Duration, stop: &AtomicBool) {
    let deadline = Instant::now() + total;
    while !stop.load(Ordering::Relaxed) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(Duration::from_millis(READ_TIMEOUT_MS as u64)));
    }
}
