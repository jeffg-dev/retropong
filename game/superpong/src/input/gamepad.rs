//! Generic gamepad knob source, via `gilrs`. Second-priority input source:
//! used when no RetroController device is open over hidapi. A
//! RetroController that gilrs also sees (as a generic HID gamepad, over its
//! evdev node) is skipped here only while hidapi already has the real
//! device open, so the two sources never fight over it; if hidapi can't
//! open it at all (e.g. a missing udev rule for `/dev/hidraw*`), gilrs is
//! left free to drive it instead, since Linux's evdev nodes are commonly
//! readable without any extra rule (task review finding #7).

use gilrs::{Axis, Button, EventType, GamepadId, Gilrs, GilrsBuilder};
use retrocontroller_protocol::usb::{PID, VID};

/// Reads knob values from the first connected gamepad that is not a
/// RetroController device.
pub struct GamepadSource {
    gilrs: Option<Gilrs>,
    active: Option<GamepadId>,
    /// Buttons pressed since the last [`GamepadSource::take_pressed`], for
    /// the menu (SPEC §13.6), in order.
    pressed: Vec<Button>,
}

impl GamepadSource {
    /// Creates the source. Failure to initialise `gilrs` (rare; platform
    /// backend issues) leaves the source permanently inactive rather than
    /// panicking.
    pub fn new() -> Self {
        // Default filters include a centre deadzone and jitter smoothing
        // tuned for sticks/buttons, not for a paddle knob that should read
        // its full range; disable them (see SPEC / research notes).
        let gilrs = GilrsBuilder::new().with_default_filters(false).build().ok();
        Self { gilrs, active: None, pressed: Vec::new() }
    }

    /// The buttons pressed on any gamepad since the last call, oldest first.
    /// A RetroController's own buttons are left out: they mean something
    /// else (see [`crate::menu::PaddleButton`]) and arrive over hidapi.
    pub fn take_pressed(&mut self) -> Vec<Button> {
        std::mem::take(&mut self.pressed)
    }

    /// Drains pending events (to keep gilrs' cached state fresh, and to
    /// collect button presses), picks the first connected gamepad (skipping
    /// a RetroController only when `skip_retrocontroller` is set, i.e.
    /// hidapi already has it open -- see module docs), and returns its knob
    /// reading, or `None` if there isn't one.
    pub fn poll(&mut self, skip_retrocontroller: bool) -> Option<(f32, f32)> {
        let gilrs = self.gilrs.as_mut()?;
        while let Some(event) = gilrs.next_event() {
            if let EventType::ButtonPressed(button, _) = event.event {
                let pad = gilrs.gamepad(event.id);
                if pad.vendor_id() != Some(VID) || pad.product_id() != Some(PID) {
                    self.pressed.push(button);
                }
            }
        }

        self.active = None;
        for (id, gp) in gilrs.gamepads() {
            if !gp.is_connected() {
                continue;
            }
            let is_retrocontroller = gp.vendor_id() == Some(VID) && gp.product_id() == Some(PID);
            if is_retrocontroller && skip_retrocontroller {
                continue;
            }
            self.active = Some(id);
            break;
        }

        let id = self.active?;
        let gp = self.gilrs.as_ref()?.gamepad(id);
        let x = gp.value(Axis::LeftStickX);
        let y = gp.value(Axis::LeftStickY);
        // gilrs already inverts Y (IS_Y_AXIS_REVERSED), so a knob at its
        // "up" end reads y = +1.0 here; the spec's mapping formula accounts
        // for that: right knob 0.0 (top) <- y = +1.0.
        let left = ((x + 1.0) / 2.0).clamp(0.0, 1.0);
        let right = ((1.0 - y) / 2.0).clamp(0.0, 1.0);
        Some((left, right))
    }
}
