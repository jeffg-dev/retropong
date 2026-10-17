//! Output hysteresis (SPEC-controller.md §3) plus the keep-alive deadline.
//!
//! A calibrated axis value is reported when:
//! - it differs from the last *reported* value by >= [`JUMP_THRESHOLD`] counts
//!   (kills 1-LSB jitter while still catching real motion immediately), OR
//! - the *current* value itself (the "candidate") has been the sampled value,
//!   continuously, for at least [`STABLE_MS`], and it differs from the last
//!   reported value (catches slow drift that never takes a >=2 step, without
//!   mistaking rapid jitter between two adjacent counts for a settled value:
//!   every time the sampled value changes, even by 1, the candidate and its
//!   timer reset), OR
//! - at least [`KEEPALIVE_MS`] have passed since the last report, regardless
//!   of whether the value changed (so hosts always see a live device).
//!
//! [`Hysteresis::update`] distinguishes the last case from the first two via
//! [`Report::Changed`] vs. [`Report::KeepAlive`], so a caller driving a
//! "knob moving" indicator (SPEC-controller.md §6) from "a report was sent
//! recently" doesn't mistake a periodic keep-alive (which fires at least
//! every [`KEEPALIVE_MS`], faster than the LED's own "recently" window) for
//! actual motion.

/// Minimum absolute change (vs. the last reported value) that is reported
/// immediately.
pub const JUMP_THRESHOLD: u16 = 2;
/// How long the sampled value must stay exactly at one candidate reading
/// before a value differing from the last reported one is reported as
/// settled drift.
pub const STABLE_MS: u32 = 50;
/// Maximum gap between reports, changed or not.
pub const KEEPALIVE_MS: u32 = 50;

/// A report emitted by [`Hysteresis::update`], distinguishing an actual value
/// change from a periodic keep-alive with no change so callers can tell them
/// apart (e.g. to drive a "knob moving" indicator only on real motion).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    /// The value changed (a big jump, settled drift, or the very first
    /// sample).
    Changed(u16),
    /// The keep-alive deadline was reached with no value change to report.
    KeepAlive(u16),
}

impl Report {
    /// The reported value, regardless of why it was reported.
    pub fn value(self) -> u16 {
        match self {
            Report::Changed(v) | Report::KeepAlive(v) => v,
        }
    }

    /// Whether this report reflects an actual value change (as opposed to a
    /// keep-alive of an unchanged value).
    pub fn is_change(self) -> bool {
        matches!(self, Report::Changed(_))
    }
}

/// Per-axis hysteresis + keep-alive state machine.
///
/// `now_ms` is expected to be a free-running millisecond counter (e.g. from
/// `embassy_time::Instant::now().as_millis()` truncated to `u32`, which is
/// good for ~49.7 days between wraps). Arithmetic uses `wrapping_sub` so a
/// single wraparound between two calls still yields a sane (small) elapsed
/// time rather than panicking or reporting a huge gap.
#[derive(Clone, Copy, Debug)]
pub struct Hysteresis {
    last: Option<u16>,
    last_report_ms: u32,
    /// The most recently sampled value, and the timestamp at which it first
    /// became the sampled value. Reset (both fields) every time `update` sees
    /// a `val` different from this candidate, so `candidate_ms` always marks
    /// "how long has the signal been sitting exactly at `candidate`",
    /// independent of when the last report went out.
    candidate: u16,
    candidate_ms: u32,
}

impl Hysteresis {
    pub const fn new() -> Self {
        Self { last: None, last_report_ms: 0, candidate: 0, candidate_ms: 0 }
    }

    /// Feeds one filtered/calibrated sample. Returns `Some(report)` exactly
    /// when a report (HID write or serial `S` line) should be emitted;
    /// `Report::Changed` vs. `Report::KeepAlive` tells the caller whether the
    /// value actually changed or this is just the periodic keep-alive.
    pub fn update(&mut self, val: u16, now_ms: u32) -> Option<Report> {
        let Some(last) = self.last else {
            self.report(val, now_ms);
            return Some(Report::Changed(val));
        };

        // The candidate is the value currently being sampled. Any change at
        // all -- including bouncing back to a value seen a moment ago --
        // restarts its timer: only a value that has been the sampled reading
        // *continuously* for STABLE_MS counts as settled, so rapid jitter
        // between two adjacent counts (which keeps resetting the candidate)
        // never accumulates enough elapsed time to be reported as drift.
        if val != self.candidate {
            self.candidate = val;
            self.candidate_ms = now_ms;
        }
        let candidate_elapsed = now_ms.wrapping_sub(self.candidate_ms);
        let since_report = now_ms.wrapping_sub(self.last_report_ms);

        let big_jump = val.abs_diff(last) >= JUMP_THRESHOLD;
        let settled_drift = val != last && candidate_elapsed >= STABLE_MS;
        let keepalive = since_report >= KEEPALIVE_MS;

        if big_jump || settled_drift {
            self.report(val, now_ms);
            Some(Report::Changed(val))
        } else if keepalive {
            // Repeat the last *reported* value, not the currently sampled
            // one: `val` here is exactly the jitter/drift `big_jump` and
            // `settled_drift` above just decided isn't worth reporting, so
            // adopting it (as `self.report(val, ..)` used to) would put
            // unreported jitter on the wire and, by moving `last`, would
            // also implicitly reset the >= 2 count jump comparison to
            // measure from a value the host was never told about. Only
            // `last_report_ms` moves, so the KEEPALIVE_MS deadline is still
            // measured from *this* report.
            self.last_report_ms = now_ms;
            Some(Report::KeepAlive(last))
        } else {
            None
        }
    }

    fn report(&mut self, val: u16, now_ms: u32) {
        self.last = Some(val);
        self.last_report_ms = now_ms;
        // Re-anchor the candidate on every report so it doesn't inherit
        // elapsed time accrued before the report went out.
        self.candidate = val;
        self.candidate_ms = now_ms;
    }

    /// The last value actually reported, if any.
    pub fn last_reported(&self) -> Option<u16> {
        self.last
    }
}

impl Default for Hysteresis {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sample_always_reports() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
    }

    #[test]
    fn one_lsb_jitter_is_suppressed() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        // Jitter of +/-1 for a short while (well under the 50ms settle time
        // and the 50ms keep-alive) must not produce a report.
        for (t, v) in [(5, 1001), (10, 1000), (15, 1001), (20, 1000), (25, 1001)] {
            assert_eq!(h.update(v, t), None, "t={t} v={v} should be suppressed");
        }
    }

    #[test]
    fn two_count_move_is_reported_immediately() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        assert_eq!(h.update(1002, 1), Some(Report::Changed(1002)));
        // A move the other way by exactly the threshold also reports.
        assert_eq!(h.update(1000, 2), Some(Report::Changed(1000)));
    }

    #[test]
    fn slow_drift_reported_after_50ms_stable() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        // Drift to 1001 (a 1-count move, below JUMP_THRESHOLD) and hold it.
        assert_eq!(h.update(1001, 10), None);
        assert_eq!(h.update(1001, 30), None);
        assert_eq!(h.update(1001, 49), None, "must not fire before 50ms have elapsed");
        assert_eq!(h.update(1001, 60), Some(Report::Changed(1001)), "must fire once stable for >= 50ms");
    }

    #[test]
    fn returning_to_last_value_still_gets_a_keepalive_at_50ms() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        // Bounces to 1001 and back to 1000; neither step is a >=2 jump, and
        // since it ends up equal to `last` there's no "settled drift" to
        // report either. But the 50ms keep-alive deadline (from the t=0
        // report) still applies regardless of these no-op wiggles, and since
        // the value didn't actually change it must come back as `KeepAlive`,
        // not `Changed`.
        assert_eq!(h.update(1001, 10), None);
        assert_eq!(h.update(1000, 40), None);
        assert_eq!(h.update(1000, 49), None, "not yet 50ms since the last report");
        assert_eq!(h.update(1000, 50), Some(Report::KeepAlive(1000)), "keep-alive deadline reached");
    }

    #[test]
    fn keepalive_fires_even_without_change() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        assert_eq!(h.update(1000, 10), None);
        assert_eq!(h.update(1000, 49), None);
        assert_eq!(
            h.update(1000, 50),
            Some(Report::KeepAlive(1000)),
            "keep-alive must fire at the 50ms deadline, and must be tagged KeepAlive, not Changed"
        );
    }

    #[test]
    fn keepalive_clock_restarts_after_any_report() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        assert_eq!(h.update(1002, 40), Some(Report::Changed(1002))); // big jump report resets the clock
        assert_eq!(h.update(1002, 80), None, "only 40ms since the last report");
        assert_eq!(h.update(1002, 90), Some(Report::KeepAlive(1002)), "50ms since the jump report");
    }

    #[test]
    fn large_negative_move_is_also_reported() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(5000, 0), Some(Report::Changed(5000)));
        assert_eq!(h.update(100, 1), Some(Report::Changed(100)));
    }

    /// Regression test for the settle timer being measured from the last
    /// *report* instead of from when the new value itself first appeared:
    /// `report()` always re-anchored `anchor`/`anchor_ms` to the just-reported
    /// value, so `anchor == last` held by induction and the "moving anchor"
    /// never actually moved independently, making `stable_elapsed` equal
    /// `since_report`. A value that had only just changed could then be
    /// reported as "settled drift" far sooner than [`STABLE_MS`].
    #[test]
    fn settle_timer_starts_when_the_new_value_first_appears_not_at_the_last_report() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        // Below JUMP_THRESHOLD, so a single sample of 1001 must not report by
        // itself.
        assert_eq!(h.update(1001, 45), None);
        // Only 5ms after 1001 first appeared. The 50ms keep-alive deadline
        // (measured from the t=0 report) lands exactly here, so some report
        // is due -- but it must be a `KeepAlive` of the last *reported*
        // value (1000), not a `Changed` "settled drift": 1001 itself has not
        // been held for STABLE_MS yet, and a keep-alive must never adopt an
        // unreported value onto the wire. (Before the fix this fired as
        // `Changed(1001)`, because the stability window was actually
        // measured from the last report at t=0, not from t=45 when 1001
        // first appeared.)
        assert_eq!(
            h.update(1001, 50),
            Some(Report::KeepAlive(1000)),
            "1001 has only been the current value for 5ms; must not be adopted yet"
        );
        // 1001 has now been the sampled value continuously since t=45, so at
        // t=95 (50ms later) it has genuinely settled and must be reported as
        // `Changed` -- demonstrating the settled-drift path is reachable now
        // that the keep-alive above didn't secretly adopt 1001 early.
        assert_eq!(
            h.update(1001, 95),
            Some(Report::Changed(1001)),
            "1001 has been stable for 50ms and must now be reported as settled drift"
        );
    }

    /// Regression test: the keep-alive must repeat the last *reported* value,
    /// not adopt whatever the sampled value happens to be at the keep-alive
    /// deadline. Before the fix, `update`'s keep-alive branch called
    /// `self.report(val, now_ms)`, which both put unreported 1-LSB jitter on
    /// the wire (as `KeepAlive(1001)`) and moved `last`/`candidate` to that
    /// unreported value -- letting a knob that never actually moved by >= 2
    /// counts still make the wire value flicker between adjacent counts at
    /// roughly the keep-alive rate.
    #[test]
    fn keepalive_repeats_the_last_reported_value_not_the_current_sample() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        // 1-LSB jitter, well under both the 50ms settle window and the 50ms
        // keep-alive deadline; none of these report on their own.
        assert_eq!(h.update(1001, 10), None);
        assert_eq!(h.update(1000, 20), None);
        assert_eq!(h.update(1001, 49), None);
        // The keep-alive deadline (50ms since the t=0 report) lands while
        // the sampled value happens to be 1001. It must repeat 1000, the
        // last value actually reported -- not adopt 1001.
        assert_eq!(
            h.update(1001, 50),
            Some(Report::KeepAlive(1000)),
            "keep-alive must repeat the last reported value, not adopt the currently sampled one"
        );
        assert_eq!(
            h.last_reported(),
            Some(1000),
            "a keep-alive must not change what counts as the last reported value"
        );
    }

    /// Regression test: a value bouncing between two adjacent counts every
    /// few ms (e.g. post-EMA raw noise landing exactly on a calibrated
    /// rounding boundary) must never be tagged `Changed` after the first
    /// sample -- only the periodic `KeepAlive`. Before the fix this produced
    /// dozens of spurious `Changed` reports per second (see
    /// `settle_timer_starts_when_the_new_value_first_appears_not_at_the_last_report`
    /// for why), which firmware's "knob moving" LED logic treats as motion.
    #[test]
    fn rounding_boundary_jitter_never_produces_a_changed_report_after_the_first() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        let mut saw_changed_after_first = false;
        for t in 1..10_000u32 {
            let v = if (t / 7) % 2 == 0 { 1000 } else { 1001 };
            if let Some(Report::Changed(_)) = h.update(v, t) {
                saw_changed_after_first = true;
            }
        }
        assert!(
            !saw_changed_after_first,
            "1-LSB rounding-boundary jitter must never report Changed after the first sample"
        );
    }

    /// Regression test for the "knob moving" LED being stuck solid-on
    /// (SPEC-controller.md §6): a caller must be able to tell a keep-alive
    /// report (fired at least every `KEEPALIVE_MS`, i.e. faster than the
    /// LED's own "recently" window) apart from a real change, by matching on
    /// `Report::Changed` vs `Report::KeepAlive` rather than treating every
    /// `Some(_)` as motion.
    #[test]
    fn holding_a_constant_value_yields_only_keepalive_reports_after_the_first() {
        let mut h = Hysteresis::new();
        let mut saw_a_later_change = false;
        let mut t = 0u32;
        for i in 0..40u32 {
            t = i * 10; // 1kHz-ish sampling, well above the keep-alive rate
            if let Some(report) = h.update(1234, t) {
                if i == 0 {
                    assert_eq!(report, Report::Changed(1234), "the very first sample is a change");
                } else if report.is_change() {
                    saw_a_later_change = true;
                } else {
                    assert_eq!(report, Report::KeepAlive(1234));
                }
            }
        }
        let _ = t;
        assert!(!saw_a_later_change, "a constant value must never report Changed after the first sample");
    }

    /// Acceptance scenario: 1-count jitter at 1 kHz for 10 seconds must never
    /// move `last_reported` (every jitter step is < [`JUMP_THRESHOLD`] and the
    /// candidate resets every single sample, so it can never sit still for
    /// [`STABLE_MS`] either), and every keep-alive along the way must carry
    /// the original reported value, at exactly the 50ms cadence.
    #[test]
    fn jitter_at_1khz_for_10s_never_changes_last_reported() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        let mut keepalives = 0u32;
        for t in 1..=10_000u32 {
            // Alternates every millisecond: never stable, never a >=2 jump.
            let v = if t % 2 == 0 { 1000 } else { 1001 };
            match h.update(v, t) {
                Some(Report::Changed(val)) => {
                    panic!("unexpected Changed({val}) at t={t} from 1-count jitter");
                }
                Some(Report::KeepAlive(val)) => {
                    assert_eq!(val, 1000, "keep-alive must carry the original reported value, t={t}");
                    keepalives += 1;
                }
                None => {}
            }
        }
        assert_eq!(keepalives, 200, "one keep-alive every 50ms over 10s of idle jitter");
        assert_eq!(h.last_reported(), Some(1000));
    }

    /// Acceptance scenario: a slow turn of +1 count every 300ms for 30
    /// seconds (well under [`JUMP_THRESHOLD`] per step, but each step then
    /// sits still far longer than [`STABLE_MS`]) must produce one `Changed`
    /// report per step via the settled-drift path, and the wire value must
    /// end up tracking the knob -- even though a keep-alive (300ms is a
    /// multiple of the 50ms keep-alive period) fires at the exact instant
    /// each step lands, before the new value has settled. This is the
    /// "settled-drift path reachable" regression: if a keep-alive ever
    /// adopted the just-changed candidate (instead of repeating the last
    /// reported value untouched), the settled-drift comparison above would
    /// see `val == last` and this slow turn would never report at all.
    #[test]
    fn slow_turn_one_count_every_300ms_settles_into_changed_reports() {
        let mut h = Hysteresis::new();
        let start: u16 = 1000;
        assert_eq!(h.update(start, 0), Some(Report::Changed(start)));
        let mut changes = 0u32;
        let mut last_changed_value = start;
        for t in 1..=30_000u32 {
            let val = start + (t / 300) as u16;
            if let Some(Report::Changed(v)) = h.update(val, t) {
                changes += 1;
                last_changed_value = v;
            }
        }
        assert_eq!(changes, 99, "one settled-drift Changed per +1 step, ~99 over 30s at 300ms/step");
        assert_eq!(h.last_reported(), Some(last_changed_value));
        assert_eq!(h.last_reported(), Some(start + 99));
    }

    /// Acceptance scenario: a >= [`JUMP_THRESHOLD`] step reports immediately,
    /// with no need to wait out [`STABLE_MS`].
    #[test]
    fn five_count_step_is_reported_immediately() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1000, 0), Some(Report::Changed(1000)));
        assert_eq!(h.update(1005, 1), Some(Report::Changed(1005)));
    }

    /// Acceptance scenario: while idle (no value change at all), keep-alives
    /// fire at exactly the [`KEEPALIVE_MS`] cadence, never as `Changed`.
    #[test]
    fn keepalive_cadence_is_50ms_while_idle() {
        let mut h = Hysteresis::new();
        assert_eq!(h.update(1234, 0), Some(Report::Changed(1234)));
        let mut keepalives = 0u32;
        for t in 1..=1_000u32 {
            match h.update(1234, t) {
                Some(Report::KeepAlive(val)) => {
                    assert_eq!(val, 1234);
                    keepalives += 1;
                }
                Some(Report::Changed(_)) => {
                    panic!("idle value must never report Changed after the first sample")
                }
                None => {}
            }
        }
        assert_eq!(keepalives, 20, "1000ms of idle at a 50ms cadence is exactly 20 keep-alives");
    }
}
