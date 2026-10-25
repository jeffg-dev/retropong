//! Optional momentary buttons (hardware/README.md): debouncing and wiring
//! detection.
//!
//! Each button switches its pin to GND against the RP2350's internal pull-up,
//! so an unwired pin reads exactly like an idle, wired button. A button is
//! therefore only *known* to exist once it has been pressed ("seen"); until
//! then it simply reports released, which is also all an absent button can
//! ever do.
//!
//! A pin must also read released for [`DEBOUNCE_SAMPLES`] in a row before any
//! press counts ("armed"). A pin held low from boot -- a button held while
//! plugging in, or a wiring that ties the pad to GND -- therefore reports
//! released instead of a press that never ends.
//!
//! Like the axes, buttons are sampled per physical pin and routed to logical
//! players by the calibration's wheel swap, so each button follows the knob
//! it sits beside.

use retrocontroller_protocol::hid::{BUTTON_LEFT, BUTTON_RIGHT};
use retrocontroller_protocol::serial::Buttons as ButtonState;

/// Consecutive agreeing samples needed to accept a new level. At the 1 kHz
/// sampling rate this is 5 ms, longer than typical contact bounce.
pub const DEBOUNCE_SAMPLES: u8 = 5;

/// One debounced, active-low button input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Button {
    pressed: bool,
    run: u8,
    armed: bool,
    seen: bool,
}

impl Button {
    pub const fn new() -> Self {
        Self { pressed: false, run: 0, armed: false, seen: false }
    }

    /// Feeds one pin sample (`true` = pin low, contact closed).
    pub fn update(&mut self, closed: bool) {
        // Until armed, count consecutive releases; afterwards, consecutive
        // samples that disagree with the debounced state.
        let counts = if self.armed { closed != self.pressed } else { !closed };
        if !counts {
            self.run = 0;
            return;
        }
        self.run += 1;
        if self.run < DEBOUNCE_SAMPLES {
            return;
        }
        self.run = 0;
        if self.armed {
            self.pressed = closed;
            self.seen |= closed;
        } else {
            self.armed = true;
        }
    }

    /// The debounced state, `false` until armed.
    pub fn pressed(&self) -> bool {
        self.pressed
    }

    /// Whether this button has been pressed since boot.
    pub fn seen(&self) -> bool {
        self.seen
    }
}

/// The two physical button inputs, beside the A0 and A1 knobs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub a0: Button,
    pub a1: Button,
}

impl Buttons {
    pub const fn new() -> Self {
        Self { a0: Button::new(), a1: Button::new() }
    }

    /// Feeds one sample per physical pin (`true` = pin low).
    pub fn update(&mut self, a0_closed: bool, a1_closed: bool) {
        self.a0.update(a0_closed);
        self.a1.update(a1_closed);
    }

    /// The state in logical player order, applying the wheel swap.
    pub fn logical(&self, swap: bool) -> ButtonState {
        let (left, right) = if swap { (self.a1, self.a0) } else { (self.a0, self.a1) };
        let mask = |l: bool, r: bool| (if l { BUTTON_LEFT } else { 0 }) | (if r { BUTTON_RIGHT } else { 0 });
        ButtonState { pressed: mask(left.pressed(), right.pressed()), seen: mask(left.seen(), right.seen()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(b: &mut Button, closed: bool, n: u8) {
        for _ in 0..n {
            b.update(closed);
        }
    }

    fn armed() -> Button {
        let mut b = Button::new();
        feed(&mut b, false, DEBOUNCE_SAMPLES);
        b
    }

    #[test]
    fn unwired_pin_never_reports_or_detects_a_button() {
        let mut b = Button::new();
        feed(&mut b, false, 200);
        assert!(!b.pressed() && !b.seen());
    }

    #[test]
    fn press_is_debounced_and_marks_the_button_seen() {
        let mut b = armed();
        feed(&mut b, true, DEBOUNCE_SAMPLES - 1);
        assert!(!b.pressed(), "a press shorter than the debounce window is bounce");
        b.update(true);
        assert!(b.pressed() && b.seen());
        feed(&mut b, false, DEBOUNCE_SAMPLES);
        assert!(!b.pressed() && b.seen(), "seen persists after release");
    }

    #[test]
    fn bounce_resets_the_debounce_count() {
        let mut b = armed();
        for _ in 0..20 {
            feed(&mut b, true, DEBOUNCE_SAMPLES - 1);
            b.update(false);
        }
        assert!(!b.pressed() && !b.seen());
    }

    #[test]
    fn pin_low_from_boot_is_ignored_until_released() {
        let mut b = Button::new();
        feed(&mut b, true, 200);
        assert!(!b.pressed() && !b.seen(), "a pin grounded from boot is not a held button");
        feed(&mut b, false, DEBOUNCE_SAMPLES);
        feed(&mut b, true, DEBOUNCE_SAMPLES);
        assert!(b.pressed() && b.seen());
    }

    #[test]
    fn logical_state_follows_the_wheel_swap() {
        let mut bs = Buttons::new();
        for _ in 0..DEBOUNCE_SAMPLES {
            bs.update(false, false);
        }
        for _ in 0..DEBOUNCE_SAMPLES {
            bs.update(true, false);
        }
        assert_eq!(bs.logical(false), ButtonState { pressed: BUTTON_LEFT, seen: BUTTON_LEFT });
        assert_eq!(bs.logical(true), ButtonState { pressed: BUTTON_RIGHT, seen: BUTTON_RIGHT });
    }
}
