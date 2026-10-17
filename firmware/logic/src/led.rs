//! User LED pattern selection (SPEC-controller.md §6).
//!
//! | State | Pattern |
//! |---|---|
//! | USB not configured | fast blink, 5 Hz |
//! | Configured, idle | slow blink, 1 Hz |
//! | Knob moving (a report sent recently) | solid on |
//! | Calibration learning (`CAL START`..`CAL STOP`) | blink, 10 Hz |
//!
//! The firmware binary drives a GPIO (active-low) from [`led_on`]; this
//! module only computes the boolean on/off state for a given millisecond
//! timestamp, so it stays hardware-free and host-testable.

/// Which LED pattern is currently active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedState {
    NotConfigured,
    Idle,
    Moving,
    Learning,
}

/// Picks the LED state from the raw conditions, in priority order:
/// not-configured (nothing else can be happening yet) > learning > moving >
/// idle.
pub fn select_led_state(usb_configured: bool, learning: bool, moving: bool) -> LedState {
    if !usb_configured {
        LedState::NotConfigured
    } else if learning {
        LedState::Learning
    } else if moving {
        LedState::Moving
    } else {
        LedState::Idle
    }
}

/// Whether the LED should be lit at `now_ms` for the given `state`.
///
/// `now_ms` is a free-running millisecond counter; blink patterns are a
/// simple `now_ms % period` square wave, so they don't need any state beyond
/// the current time (and are trivially in sync across calls).
pub fn led_on(state: LedState, now_ms: u32) -> bool {
    match state {
        LedState::NotConfigured => blink(now_ms, 5),
        LedState::Idle => blink(now_ms, 1),
        LedState::Moving => true,
        LedState::Learning => blink(now_ms, 10),
    }
}

/// 50%-duty square wave at `hz` Hz.
fn blink(now_ms: u32, hz: u32) -> bool {
    let period_ms = 1000 / hz;
    (now_ms % period_ms) < period_ms / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_not_configured_wins_over_everything() {
        assert_eq!(select_led_state(false, true, true), LedState::NotConfigured);
    }

    #[test]
    fn priority_learning_wins_over_moving_when_configured() {
        assert_eq!(select_led_state(true, true, true), LedState::Learning);
    }

    #[test]
    fn priority_moving_then_idle() {
        assert_eq!(select_led_state(true, false, true), LedState::Moving);
        assert_eq!(select_led_state(true, false, false), LedState::Idle);
    }

    #[test]
    fn moving_is_always_solid_on() {
        for t in [0, 1, 50, 99, 100, 999, 1000] {
            assert!(led_on(LedState::Moving, t));
        }
    }

    #[test]
    fn not_configured_blinks_at_5hz() {
        // period 200ms, on for first 100ms of each period.
        assert!(led_on(LedState::NotConfigured, 0));
        assert!(led_on(LedState::NotConfigured, 99));
        assert!(!led_on(LedState::NotConfigured, 100));
        assert!(!led_on(LedState::NotConfigured, 199));
        assert!(led_on(LedState::NotConfigured, 200));
    }

    #[test]
    fn idle_blinks_at_1hz() {
        // period 1000ms.
        assert!(led_on(LedState::Idle, 0));
        assert!(led_on(LedState::Idle, 499));
        assert!(!led_on(LedState::Idle, 500));
        assert!(!led_on(LedState::Idle, 999));
        assert!(led_on(LedState::Idle, 1000));
    }

    #[test]
    fn learning_blinks_at_10hz() {
        // period 100ms.
        assert!(led_on(LedState::Learning, 0));
        assert!(led_on(LedState::Learning, 49));
        assert!(!led_on(LedState::Learning, 50));
        assert!(!led_on(LedState::Learning, 99));
        assert!(led_on(LedState::Learning, 100));
    }
}
