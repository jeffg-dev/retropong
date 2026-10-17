//! Calibration learning state driving `CAL START` / `CAL STOP`.
//!
//! While learning is active every raw (16-bit, post-filter) sample updates a
//! running min/max per axis. `CAL STOP` adopts the learned span into the live
//! [`AxisCal`](retrocontroller_protocol::cal::AxisCal), trimmed inward by
//! [`Learner::MARGIN`] raw counts on each end so the mechanical end stops
//! stay reachable even if the learned span was recorded a few counts short of
//! the pot's true rails.

use retrocontroller_protocol::cal::AxisCal;

/// Learns the min/max of one axis's raw samples during `CAL START`..`CAL
/// STOP`.
#[derive(Clone, Copy, Debug)]
pub struct Learner {
    min: u16,
    max: u16,
}

impl Learner {
    /// Raw counts trimmed inward from each end of the learned span when
    /// adopting it, so a knob that was swept *almost* to its physical limits
    /// during calibration still reaches 0 and `AXIS_MAX` afterwards.
    pub const MARGIN: u16 = 200;

    pub const fn new() -> Self {
        // No sample observed yet: min starts above any possible sample, max
        // below any possible sample, so the very first `observe` sets both.
        Self { min: u16::MAX, max: 0 }
    }

    /// Resets the learned range (called on `CAL START`).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Feeds one raw sample.
    pub fn observe(&mut self, raw: u16) {
        if raw < self.min {
            self.min = raw;
        }
        if raw > self.max {
            self.max = raw;
        }
    }

    /// Whether at least one sample has been observed since the last reset.
    pub fn has_samples(&self) -> bool {
        self.min <= self.max
    }

    /// Computes the [`AxisCal`] to adopt from the learned span (called on
    /// `CAL STOP`), preserving `prev_invert`. Returns `None` if nothing was
    /// learned, or if the learned (margin-adjusted) span is too small to be
    /// usable — the caller should respond `ERR` and leave the live
    /// calibration untouched in that case.
    pub fn finish(&self, prev_invert: bool) -> Option<AxisCal> {
        if !self.has_samples() {
            return None;
        }
        let min = self.min.saturating_add(Self::MARGIN);
        let max = self.max.saturating_sub(Self::MARGIN);
        if min >= max {
            return None;
        }
        let cal = AxisCal { min, max, invert: prev_invert };
        if cal.is_sane() {
            Some(cal)
        } else {
            None
        }
    }
}

impl Default for Learner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_learner_has_no_samples() {
        let l = Learner::new();
        assert!(!l.has_samples());
        assert_eq!(l.finish(false), None);
    }

    #[test]
    fn learns_min_and_max_and_applies_margin() {
        let mut l = Learner::new();
        for raw in [30000u16, 1000, 64000, 40000, 500] {
            l.observe(raw);
        }
        assert!(l.has_samples());
        let cal = l.finish(false).expect("sane span should adopt");
        assert_eq!(cal.min, 500 + Learner::MARGIN);
        assert_eq!(cal.max, 64000 - Learner::MARGIN);
        assert!(!cal.invert);
    }

    #[test]
    fn finish_preserves_previous_invert_flag() {
        let mut l = Learner::new();
        l.observe(1000);
        l.observe(64000);
        let cal = l.finish(true).unwrap();
        assert!(cal.invert);
    }

    #[test]
    fn tiny_span_is_rejected_as_insane() {
        let mut l = Learner::new();
        l.observe(30000);
        l.observe(30100); // span 100, nowhere near AxisCal::is_sane()'s 1024
        assert_eq!(l.finish(false), None);
    }

    #[test]
    fn span_that_only_survives_without_margin_is_still_rejected() {
        // Span of 1100 raw counts is "sane" pre-margin (>= 1024) but after
        // trimming 200 from each end (400 total) it's below the sanity floor,
        // and must be rejected rather than silently adopting a degenerate
        // calibration.
        let mut l = Learner::new();
        l.observe(10000);
        l.observe(11100);
        assert_eq!(l.finish(false), None);
    }

    #[test]
    fn reset_clears_previously_learned_range() {
        let mut l = Learner::new();
        l.observe(1000);
        l.observe(64000);
        l.reset();
        assert!(!l.has_samples());
        assert_eq!(l.finish(false), None);
    }

    #[test]
    fn single_repeated_sample_has_zero_span_and_is_rejected() {
        let mut l = Learner::new();
        for _ in 0..5 {
            l.observe(30000);
        }
        // has_samples() is true (min == max == 30000) but the span is zero.
        assert!(l.has_samples());
        assert_eq!(l.finish(false), None);
    }
}
