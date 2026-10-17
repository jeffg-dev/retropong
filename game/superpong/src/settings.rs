//! Runtime settings changed through the pause menu.
//!
//! These start from the CLI flags (see `main.rs`) but change live via
//! the menu, unlike the CLI-only testing flags (`--demo`,
//! `--knobs`, ...).

use crate::audio::{volume_gain, DEFAULT_VOLUME};

/// How the picture is shown (SPEC §13.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum DisplayMode {
    /// Sharp square pixels, as a modern monitor shows an image: the console's
    /// colours and nothing else.
    Pixel,
    /// Classic video: the picture as a 1970s colour TV showed it (fine
    /// vertical stripes, scanlines, phosphor mask, curved screen).
    Crt,
}

impl DisplayMode {
    /// The other mode.
    pub fn toggled(self) -> Self {
        match self {
            Self::Pixel => Self::Crt,
            Self::Crt => Self::Pixel,
        }
    }
}

/// Live, player-toggleable display/input settings.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Colour picture (default on) vs. black-and-white.
    pub color: bool,
    /// Pixel or classic-video picture (default CRT).
    pub display: DisplayMode,
    /// All sound muted (CLI `--mute`), whatever the volume.
    pub muted: bool,
    /// Master volume, `0..=VOLUME_MAX` (the menu's slider; 0 is silent).
    pub volume: u8,
    /// Left knob reading is inverted (`1.0 - v`) before reaching the core.
    pub invert_left: bool,
    /// Right knob reading is inverted (`1.0 - v`) before reaching the core.
    pub invert_right: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            color: true,
            display: DisplayMode::Crt,
            muted: false,
            volume: DEFAULT_VOLUME,
            invert_left: true,
            invert_right: true,
        }
    }
}

impl Settings {
    /// Playback gain for the sounds: the volume's, or 0 while muted.
    pub fn gain(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            volume_gain(self.volume)
        }
    }

    /// Applies independent `invert_left`/`invert_right` corrections to a raw
    /// `(left, right)` knob pair before it is handed to the core.
    ///
    /// These are corrections for a physical device (a backwards
    /// potentiometer), so
    /// [`InputManager::poll`](crate::input::InputManager::poll) applies this
    /// only to HID/gamepad readings, never to the headless `--demo`/
    /// `--knobs` testing paths in `main.rs` (task review finding #10).
    pub fn apply(&self, left: f32, right: f32) -> (f32, f32) {
        let l = if self.invert_left { 1.0 - left } else { left };
        let r = if self.invert_right { 1.0 - right } else { right };
        (l, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_starts_as_crt_and_toggles_between_the_two_modes() {
        let display = Settings::default().display;
        assert_eq!(display, DisplayMode::Crt);
        assert_eq!(display.toggled(), DisplayMode::Pixel);
        assert_eq!(display.toggled().toggled(), DisplayMode::Crt);
    }

    #[test]
    fn knobs_default_to_clockwise_up() {
        let settings = Settings::default();
        assert!(settings.invert_left && settings.invert_right);
        let (left, right) = settings.apply(0.2, 0.7);
        assert!((left - 0.8).abs() < 1e-6);
        assert!((right - 0.3).abs() < 1e-6);
    }

    #[test]
    fn mute_silences_any_volume() {
        let settings = Settings::default();
        assert_eq!(settings.gain(), volume_gain(DEFAULT_VOLUME));
        assert_eq!(Settings { muted: true, ..settings }.gain(), 0.0);
        assert_eq!(Settings { volume: 0, ..settings }.gain(), 0.0);
        assert_eq!(Settings { volume: crate::audio::VOLUME_MAX, ..settings }.gain(), 1.0);
    }
}
