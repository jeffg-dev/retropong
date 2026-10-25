//! Physical knob sources: RetroController HID, then gamepad, otherwise centred.
mod gamepad;
mod hid;
use crate::menu::PaddleButton;
use crate::settings::Settings;
pub use gamepad::GamepadSource;
pub use hid::HidSource;
pub struct InputManager {
    hid: HidSource,
    gamepad: GamepadSource,
}
impl InputManager {
    pub fn new() -> Self {
        Self { hid: HidSource::new(), gamepad: GamepadSource::new() }
    }
    pub fn poll(&mut self, settings: &Settings) -> (f32, f32) {
        let hid = self.hid.poll();
        let gamepad = self.gamepad.poll(self.hid.connected());
        hid.or(gamepad).map(|(l, r)| settings.apply(l, r)).unwrap_or((0.5, 0.5))
    }
    pub fn take_buttons(&mut self) -> Vec<gilrs::Button> {
        self.gamepad.take_pressed()
    }
    /// The RetroController's own button presses, read over hidapi.
    pub fn take_paddle_buttons(&mut self) -> Vec<PaddleButton> {
        self.hid.take_pressed()
    }
}
