//! `embassy_usb::Handler` (device-level) and `RequestHandler` (HID
//! feature/idle requests) implementations.

use core::sync::atomic::{AtomicBool, Ordering};

use embassy_usb::class::hid::{HidProtocolMode, ReportId, RequestHandler};
use embassy_usb::control::OutResponse;
use embassy_usb::Handler;

/// Set by [`DeviceHandler`] whenever the host (de)configures the device.
/// Read by the LED task to pick the "USB not configured" 5 Hz pattern.
pub static USB_CONFIGURED: AtomicBool = AtomicBool::new(false);

pub fn is_configured() -> bool {
    USB_CONFIGURED.load(Ordering::Relaxed)
}

/// Tracks the device's configured state via the standard USB enumeration
/// callbacks; any of these firing except a positive `configured(true)` means
/// the device isn't ready yet.
#[derive(Default)]
pub struct DeviceHandler;

impl Handler for DeviceHandler {
    fn enabled(&mut self, _enabled: bool) {
        USB_CONFIGURED.store(false, Ordering::Relaxed);
    }

    fn reset(&mut self) {
        USB_CONFIGURED.store(false, Ordering::Relaxed);
    }

    fn addressed(&mut self, _addr: u8) {
        USB_CONFIGURED.store(false, Ordering::Relaxed);
    }

    fn configured(&mut self, configured: bool) {
        USB_CONFIGURED.store(configured, Ordering::Relaxed);
    }
}

/// Minimal HID `RequestHandler`: there are no vendor `GET_REPORT`/`SET_REPORT`
/// features to serve, so every hook just accepts with an empty/default
/// response.
#[derive(Default)]
pub struct PaddleRequestHandler;

impl RequestHandler for PaddleRequestHandler {
    fn get_report(&mut self, _id: ReportId, _buf: &mut [u8]) -> Option<usize> {
        None
    }

    fn set_report(&mut self, _id: ReportId, _data: &[u8]) -> OutResponse {
        OutResponse::Accepted
    }

    fn get_protocol(&self) -> HidProtocolMode {
        HidProtocolMode::Report
    }

    fn set_protocol(&mut self, _protocol: HidProtocolMode) -> OutResponse {
        OutResponse::Accepted
    }

    fn set_idle_ms(&mut self, _id: Option<ReportId>, _duration_ms: u32) {}

    fn get_idle_ms(&mut self, _id: Option<ReportId>) -> Option<u32> {
        None
    }
}
