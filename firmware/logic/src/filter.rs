//! Per-axis oversampling and smoothing.
//!
//! Pipeline for one knob: 12-bit ADC samples -> [`Oversampler`] (16x, ->
//! 16-bit raw) -> [`Ema`] (exponential moving average, still 16-bit raw) ->
//! `retrocontroller_protocol::cal::AxisCal::apply` (-> calibrated 0..=4095).
//! [`AxisFilter`] bundles the first two stages for convenience.

/// Number of 12-bit ADC samples accumulated into one 16-bit raw value.
pub const OVERSAMPLE_N: u32 = 16;

/// Scales a sum of [`OVERSAMPLE_N`] 12-bit samples (each `0..=4095`, so
/// `sum` is `0..=65520`) onto the full 16-bit range `0..=65535`.
///
/// Exact, overflow-free integer method: we want
/// `raw16 = round(sum * 65535 / 65520)`. Reducing `65535/65520` by their GCD
/// (15) gives the equal fraction `4369/4368`, so
///
/// ```text
/// raw16 = (sum * 4369 + 2184) / 4368        // 2184 = 4368 / 2, for round-to-nearest
/// ```
///
/// `sum <= 65520`, so `sum * 4369 <= 65520 * 4369 = 286_256_880`, which fits
/// comfortably in a `u32` (max ~4.29e9) with room to spare for the `+2184`
/// rounding term — no overflow, no intermediate `u64`, no floating point.
/// The endpoints are exact: `sum = 0 -> 0` and `sum = 65520 -> 65535`
/// (verified by the unit tests below).
pub const fn oversample_scale(sum: u32) -> u16 {
    ((sum * 4369 + 2184) / 4368) as u16
}

/// Accumulates [`OVERSAMPLE_N`] 12-bit ADC samples and reduces them to one
/// 16-bit raw value via [`oversample_scale`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Oversampler {
    sum: u32,
    count: u32,
}

impl Oversampler {
    pub const fn new() -> Self {
        Self { sum: 0, count: 0 }
    }

    /// Feeds one 12-bit ADC sample (values above 4095 are clamped, in case a
    /// caller ever hands us a raw HAL reading with a wider nominal range).
    /// Returns `Some(raw16)` once [`OVERSAMPLE_N`] samples have been
    /// accumulated, resetting the accumulator for the next batch.
    pub fn add(&mut self, sample12: u16) -> Option<u16> {
        self.sum += sample12.min(4095) as u32;
        self.count += 1;
        if self.count >= OVERSAMPLE_N {
            let raw = oversample_scale(self.sum);
            self.sum = 0;
            self.count = 0;
            Some(raw)
        } else {
            None
        }
    }
}

/// Exponential moving average over 16-bit raw values: `y += (x - y) / 4`.
///
/// The division is plain integer division (truncating toward zero, as `/`
/// does in Rust), so there is an inherent dead band: once `|x - y| < 4`, the
/// update rounds to zero and `y` stops moving. That is the standard
/// fixed-point EMA trade-off (no fractional accumulator is kept) and is
/// small relative to the 16-bit raw range (the residual is at most 3 counts,
/// under 0.005% of full scale, and shrinks further once mapped down to the
/// 12-bit calibrated axis). The first sample seeds `y` directly so the
/// filter doesn't ramp up from zero at boot.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ema {
    y: u16,
    seeded: bool,
}

impl Ema {
    pub const fn new() -> Self {
        Self { y: 0, seeded: false }
    }

    pub fn update(&mut self, x: u16) -> u16 {
        if !self.seeded {
            self.y = x;
            self.seeded = true;
        } else {
            let diff = x as i32 - self.y as i32;
            let delta = diff / 4;
            self.y = (self.y as i32 + delta).clamp(0, u16::MAX as i32) as u16;
        }
        self.y
    }

    pub fn value(&self) -> u16 {
        self.y
    }
}

/// One axis's oversample + EMA pipeline, for callers that just want to feed
/// raw ADC samples and get filtered 16-bit raw values back.
#[derive(Clone, Copy, Debug, Default)]
pub struct AxisFilter {
    oversampler: Oversampler,
    ema: Ema,
}

impl AxisFilter {
    pub const fn new() -> Self {
        Self { oversampler: Oversampler::new(), ema: Ema::new() }
    }

    /// Feeds one 12-bit ADC sample. Returns `Some(raw16)` (the freshly
    /// updated EMA output) whenever an oversampled batch completes.
    pub fn add_sample(&mut self, sample12: u16) -> Option<u16> {
        self.oversampler.add(sample12).map(|raw| self.ema.update(raw))
    }

    /// The current filtered 16-bit raw value (0 until the first batch
    /// completes).
    pub fn value(&self) -> u16 {
        self.ema.value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversample_scale_endpoints_are_exact() {
        assert_eq!(oversample_scale(0), 0);
        assert_eq!(oversample_scale(16 * 4095), 65535);
    }

    #[test]
    fn oversample_scale_is_monotonic_and_in_range() {
        let mut prev = oversample_scale(0);
        let mut sum = 0u32;
        while sum <= 16 * 4095 {
            let v = oversample_scale(sum);
            assert!(v as u32 <= 65535);
            assert!(v >= prev);
            prev = v;
            sum += 137; // coarse stride, still covers the whole domain
        }
    }

    #[test]
    fn oversampler_all_zero_samples_gives_zero() {
        let mut o = Oversampler::new();
        let mut last = None;
        for _ in 0..OVERSAMPLE_N {
            last = o.add(0);
        }
        assert_eq!(last, Some(0));
    }

    #[test]
    fn oversampler_all_max_samples_gives_65535() {
        let mut o = Oversampler::new();
        let mut last = None;
        for _ in 0..OVERSAMPLE_N {
            last = o.add(4095);
        }
        assert_eq!(last, Some(65535));
    }

    #[test]
    fn oversampler_only_completes_every_n_samples() {
        let mut o = Oversampler::new();
        for _ in 0..OVERSAMPLE_N - 1 {
            assert_eq!(o.add(2000), None);
        }
        assert!(o.add(2000).is_some());
        // Accumulator resets: next batch starts fresh.
        for _ in 0..OVERSAMPLE_N - 1 {
            assert_eq!(o.add(0), None);
        }
        assert_eq!(o.add(0), Some(0));
    }

    #[test]
    fn ema_seeds_on_first_sample() {
        let mut e = Ema::new();
        assert_eq!(e.update(12345), 12345);
    }

    #[test]
    fn ema_converges_toward_target_within_deadband() {
        let mut e = Ema::new();
        e.update(0);
        for _ in 0..200 {
            e.update(60000);
        }
        // Truncating fixed-point division leaves a small residual (< 4).
        assert!(60000 - e.value() < 4, "y={} should be within 4 of 60000", e.value());
    }

    #[test]
    fn ema_converges_downward_too() {
        let mut e = Ema::new();
        e.update(60000);
        for _ in 0..200 {
            e.update(100);
        }
        assert!(e.value() as i32 - 100 < 4, "y={} should be within 4 of 100", e.value());
    }

    #[test]
    fn ema_does_not_overshoot_or_go_negative() {
        let mut e = Ema::new();
        e.update(3);
        for _ in 0..50 {
            let y = e.update(0);
            assert!(y <= 3);
        }
        // Truncating division dead-bands out once |x - y| < 4, so a target of
        // 0 starting from y=3 settles at 3 rather than reaching 0 exactly
        // (diff=3 -> delta=3/4=0). This never goes negative or overshoots,
        // which is what this test actually guards.
        assert_eq!(e.value(), 3);
    }

    #[test]
    fn axis_filter_end_to_end_reaches_full_scale() {
        let mut f = AxisFilter::new();
        let mut last = 0;
        for _ in 0..(OVERSAMPLE_N * 100) {
            if let Some(v) = f.add_sample(4095) {
                last = v;
            }
        }
        assert_eq!(last, 65535);
        assert_eq!(f.value(), 65535);
    }
}
