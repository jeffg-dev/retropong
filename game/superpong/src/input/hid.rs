//! RetroController knob source: reads the paddle controller's USB HID
//! report directly via `hidapi`. Highest-priority input source when present.

use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use retrocontroller_protocol::hid::PaddleReport;
use retrocontroller_protocol::usb::{PID, VID};

/// How often we retry enumeration while no device is open.
const PROBE_INTERVAL: Duration = Duration::from_millis(500);

/// Reads knob values from a RetroController paddle device over hidapi,
/// re-enumerating on a timer whenever no device is open or the open one
/// stops responding. Never panics: every hidapi failure just leaves the
/// source disconnected until the next probe.
pub struct HidSource {
    api: Option<HidApi>,
    device: Option<HidDevice>,
    last: (f32, f32),
    next_probe: Instant,
    /// Human-readable reason the source is unavailable, for diagnostics. Only
    /// ever set for a real failure (a `HidApi`/backend error, a permission
    /// problem, a mid-game disconnect) -- never for the ordinary "nothing
    /// plugged in" case, which is not worth warning about on every
    /// keyboard/gamepad-only run (task review finding #2/#5).
    unavailable_reason: Option<String>,
}

impl HidSource {
    /// Creates the source. Constructing the underlying `HidApi` can fail
    /// (e.g. missing permissions); that is reported via
    /// internal diagnostics rather than propagated.
    pub fn new() -> Self {
        let mut unavailable_reason = None;
        let api = match HidApi::new() {
            Ok(api) => Some(api),
            Err(e) => {
                unavailable_reason = Some(format!("hidapi: {e}"));
                None
            }
        };
        let mut source =
            Self { api, device: None, last: (0.5, 0.5), next_probe: Instant::now(), unavailable_reason };
        source.try_connect();
        source
    }

    /// Whether a device is currently open.
    pub fn connected(&self) -> bool {
        self.device.is_some()
    }

    fn try_connect(&mut self) {
        let Some(api) = self.api.as_mut() else { return };
        // `refresh_devices()` is `reset_devices()` + `add_devices(0, 0)`,
        // i.e. a full re-enumeration (and, on some backends, opening every
        // HID node on the system to read its descriptor) just to find our
        // one device. We re-probe on a timer while nothing is plugged in,
        // so filter by our VID/PID up front instead (task review
        // finding #11).
        if let Err(e) = api.reset_devices().and_then(|_| api.add_devices(VID, PID)) {
            self.unavailable_reason = Some(format!("hidapi: {e}"));
            return;
        }
        let api: &HidApi = &*api;
        // Prefer a collection explicitly reporting usage page 1 / usage 5
        // (Generic Desktop / Game Pad); some backends (Linux libusb) can't
        // report usage at all, so fall back to any match (already filtered
        // to our VID/PID above).
        let candidates: Vec<&hidapi::DeviceInfo> = api.device_list().collect();
        let best = candidates
            .iter()
            .find(|d| d.usage_page() == 0x01 && d.usage() == 0x05)
            .or_else(|| candidates.first())
            .copied();
        let Some(info) = best else {
            // Not an error -- just nothing plugged in right now. But don't
            // stomp a still-fresh disconnect note: if a device just dropped
            // out from under us, "not found" is *why*, and the HUD hint is
            // only useful if it survives more than one probe interval
            // (task review finding #1). It's cleared below on a successful
            // reconnect, or replaced above if a real hidapi error occurs.
            if self.unavailable_reason.as_deref() != Some("RetroController disconnected") {
                self.unavailable_reason = None;
            }
            return;
        };
        match info.open_device(api) {
            Ok(dev) => {
                if let Err(e) = dev.set_blocking_mode(false) {
                    self.unavailable_reason = Some(format!("hidapi: {e}"));
                    return;
                }
                self.device = Some(dev);
                self.unavailable_reason = None;
            }
            Err(e) => {
                self.unavailable_reason = Some(format!("hidapi: {e}"));
            }
        }
    }

    /// Drains all queued reports (keeping only the latest) and returns the
    /// current knob reading, or `None` if no device is open. On a read
    /// error the device is dropped and re-enumeration is scheduled.
    pub fn poll(&mut self) -> Option<(f32, f32)> {
        if self.device.is_none() {
            if Instant::now() >= self.next_probe {
                self.next_probe = Instant::now() + PROBE_INTERVAL;
                self.try_connect();
            }
            return None;
        }

        let dev = self.device.as_ref().expect("checked Some above");
        let mut buf = [0u8; 64];
        let mut latest: Option<PaddleReport> = None;
        loop {
            match dev.read_timeout(&mut buf, 0) {
                Ok(0) => break,
                Ok(n) => {
                    if let Some(report) = PaddleReport::from_bytes(&buf[..n]) {
                        latest = Some(report);
                    }
                }
                Err(_) => {
                    self.device = None;
                    self.next_probe = Instant::now() + PROBE_INTERVAL;
                    self.unavailable_reason = Some("RetroController disconnected".to_string());
                    return None;
                }
            }
        }
        if let Some(report) = latest {
            self.last = (report.x_unit(), report.y_unit());
        }
        Some(self.last)
    }
}
