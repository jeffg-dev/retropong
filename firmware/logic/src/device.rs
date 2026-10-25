//! The serial command handler: turns a parsed [`Command`] plus the device's
//! current readings into exactly one response line and an [`Effect`] the
//! firmware binary must carry out (persist calibration to flash, reboot into
//! BOOTSEL, or a plain reset).

use core::fmt::Write;

use retrocontroller_protocol::cal::{AxisCal, Calibration};
use retrocontroller_protocol::hid::AXIS_MAX;
use retrocontroller_protocol::serial::{self, Buttons, Command, Knob};
use retrocontroller_protocol::PROTOCOL_VERSION;

use crate::calibration::Learner;

/// Board identifier reported by `INFO` (SPEC-controller.md §5).
pub const BOARD: &str = "xiao-rp2350";
/// Device name reported by `INFO`.
pub const DEVICE_NAME: &str = "RetroController";
/// Response text for a calibration that can't be adopted (learned span too
/// small, or an explicit `CAL SET` with an insane range). Not part of
/// `retrocontroller_protocol::serial::ParseError` since it's a runtime
/// rejection, not a syntax error.
pub const ERR_CALIBRATION: &str = "ERR calibration";
/// Response text for `CAL STOP` issued without an active `CAL START`
/// learning session. Without this check, a stray or repeated `CAL STOP`
/// would silently re-adopt whatever stale min/max the learner last held
/// (from a previous, already-finished `CAL START`..`CAL STOP` cycle) and
/// mark the live calibration as unsaved even though nothing changed.
pub const ERR_NOT_LEARNING: &str = "ERR not learning";
/// Response text when a deferred flash write ([`Effect::SaveCal`] /
/// [`Effect::PersistCal`]) fails, completed via [`Device::finish_cal_save`] /
/// [`Device::finish_persist_cal`].
pub const ERR_FLASH: &str = "ERR flash";

/// A snapshot of the device's current readings, handed to [`Device::handle`]
/// so command responses (`RAW`, `VAL`) reflect the latest sampled values
/// without the logic crate needing to own the sampling pipeline itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Left knob, 16-bit filtered raw (`0..=65535`).
    pub raw_l: u16,
    /// Right knob, 16-bit filtered raw (`0..=65535`).
    pub raw_r: u16,
    /// Left knob, calibrated (`0..=AXIS_MAX`).
    pub val_l: u16,
    /// Right knob, calibrated (`0..=AXIS_MAX`).
    pub val_r: u16,
    /// Debounced buttons, logical order (see [`crate::buttons`]).
    pub buttons: Buttons,
}

/// A side effect for the firmware binary to carry out after [`Device::handle`]
/// returns. For [`Effect::Bootsel`] and [`Effect::Reboot`] the response line
/// has already been written to `out` by the time this is returned. For
/// [`Effect::SaveCal`] and [`Effect::PersistCal`] the response is
/// *deliberately not* written yet: the caller must attempt the flash write
/// first and then call [`Device::finish_cal_save`] / [`Device::finish_persist_cal`]
/// with the result before sending anything to the host, so a failed
/// erase/write/read-back is reported as `ERR flash` instead of an `OK` the
/// host would otherwise wrongly trust.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Nothing further to do.
    None,
    /// `CAL SAVE`: erase+write the calibration sector with this record, then
    /// read it back to confirm. Complete with [`Device::finish_cal_save`].
    SaveCal(Calibration),
    /// `CAL RESET` / `INVERT` / `CAL SWAP`: same flash write as [`Effect::SaveCal`], but
    /// the live calibration has already been updated to this record, and the
    /// pending response is the full `OK CAL ...` line rather than
    /// `OK CAL SAVE`. SWAP instead defers the live update until success.
    /// Complete with [`Device::finish_persist_cal`].
    PersistCal(Calibration),
    /// Flush the response, then reboot into the ROM USB bootloader.
    Bootsel,
    /// Flush the response, then perform a normal system reset.
    Reboot,
}

/// The controller's command-handling state: live calibration, whether it
/// matches what's on flash, whether `STREAM` is on, and whether `CAL
/// START`..`CAL STOP` learning is in progress.
pub struct Device {
    pub cal: Calibration,
    pub live_cal_saved: bool,
    pub learning: bool,
    pub stream_on: bool,
    /// Reported by `INFO` as `fw=<fw>`; supplied by the caller (typically
    /// `env!("CARGO_PKG_VERSION")` of the firmware binary crate, not this
    /// logic crate).
    pub fw: &'static str,
    /// RP2350 OTP chip id, reported by `INFO` as 16 uppercase hex digits.
    pub uid: [u8; 8],
    pending_swap: Option<Calibration>,
    learn_l: Learner,
    learn_r: Learner,
}

impl Device {
    pub fn new(fw: &'static str, uid: [u8; 8], cal: Calibration, live_cal_saved: bool) -> Self {
        Self {
            cal,
            live_cal_saved,
            learning: false,
            stream_on: false,
            fw,
            uid,
            pending_swap: None,
            learn_l: Learner::new(),
            learn_r: Learner::new(),
        }
    }

    /// Called when the host connection goes away without a matching
    /// `STREAM OFF`/`CAL STOP` (DTR dropping, a USB bus reset, or a fresh
    /// `wait_connection()`): stops streaming and abandons any in-progress
    /// `CAL START` learning session. Without this, a host that vanishes
    /// mid-calibration (killed process, closed terminal, unplug/replug)
    /// leaves the controller in learning mode indefinitely -- the 10 Hz LED
    /// (SPEC-controller.md §6) stays on and the learners keep widening from
    /// whatever noise/motion happens afterward -- until some later host
    /// happens to send `CAL START` or `CAL STOP`, at which point a stray
    /// `CAL STOP` would adopt whatever span accumulated in the meantime.
    pub fn on_disconnect(&mut self) {
        self.stream_on = false;
        self.learning = false;
        self.learn_l.reset();
        self.learn_r.reset();
    }

    /// Feeds one filtered raw sample pair into the calibration learner, if
    /// `CAL START` learning is currently active. Call this once per filtered
    /// sample (i.e. whenever a new [`Snapshot::raw_l`]/`raw_r` pair is
    /// produced), independent of `handle`.
    pub fn observe_raw(&mut self, raw_l: u16, raw_r: u16) {
        if self.learning {
            self.learn_l.observe(raw_l);
            self.learn_r.observe(raw_r);
        }
    }

    /// Parses one line and handles it, writing the `OK .../ERR ...` response
    /// (including the trailing newline) to `out`.
    pub fn handle_line(&mut self, line: &str, state: &Snapshot, out: &mut impl Write) -> Effect {
        match serial::parse_command(line) {
            Ok(cmd) => self.handle(cmd, state, out),
            Err(e) => {
                let _ = writeln!(out, "{}", e.message());
                Effect::None
            }
        }
    }

    /// Handles an already-parsed command, writing the response line
    /// (including the trailing newline) to `out` and returning the [`Effect`]
    /// the caller must carry out. Exception: for a command that returns
    /// [`Effect::SaveCal`] or [`Effect::PersistCal`], `out` is left untouched
    /// here — see [`Effect`]'s docs for why the response is deferred.
    pub fn handle(&mut self, cmd: Command, state: &Snapshot, out: &mut impl Write) -> Effect {
        match cmd {
            Command::Info => {
                let _ = write!(
                    out,
                    "OK INFO name={} fw={} proto={} board={} uid=",
                    DEVICE_NAME, self.fw, PROTOCOL_VERSION, BOARD
                );
                for b in &self.uid {
                    let _ = write!(out, "{b:02X}");
                }
                let _ = writeln!(out, " axes=2 max={AXIS_MAX}");
                Effect::None
            }
            Command::Raw => {
                let _ = writeln!(out, "OK RAW {} {}", state.raw_l, state.raw_r);
                Effect::None
            }
            Command::Val => {
                let _ = writeln!(out, "OK VAL {} {}", state.val_l, state.val_r);
                Effect::None
            }
            Command::Buttons => {
                let _ = writeln!(out, "OK BTN {} {}", state.buttons.pressed, state.buttons.seen);
                Effect::None
            }
            Command::Stream(on) => {
                self.stream_on = on;
                let _ = writeln!(out, "OK STREAM {}", if on { "ON" } else { "OFF" });
                Effect::None
            }
            Command::CalShow => {
                self.write_cal_line(out);
                Effect::None
            }
            Command::CalStart => {
                self.learning = true;
                self.learn_l.reset();
                self.learn_r.reset();
                let _ = writeln!(out, "OK CAL START");
                Effect::None
            }
            Command::CalStop => {
                if !self.learning {
                    // No active CAL START..CAL STOP session: reject rather
                    // than re-running `finish()` against whatever stale
                    // min/max the learners last held, which would silently
                    // re-adopt an old learned span (or spuriously clear
                    // `live_cal_saved`) with no new sweep behind it.
                    let _ = writeln!(out, "{ERR_NOT_LEARNING}");
                    return Effect::None;
                }
                self.learning = false;
                let learned =
                    (self.learn_l.finish(self.cal.left.invert), self.learn_r.finish(self.cal.right.invert));
                // Always clear the learners on the way out of a learning
                // session, whether or not the span was adopted, so nothing
                // stale can be reused if `CAL STOP` is ever reached again
                // without a new `CAL START` (belt-and-braces alongside the
                // `!self.learning` guard above).
                self.learn_l.reset();
                self.learn_r.reset();
                match learned {
                    (Some(left), Some(right)) => {
                        self.cal.left = left;
                        self.cal.right = right;
                        self.live_cal_saved = false;
                        self.write_cal_line(out);
                    }
                    _ => {
                        let _ = writeln!(out, "{ERR_CALIBRATION}");
                    }
                }
                Effect::None
            }
            Command::CalSet { lmin, lmax, rmin, rmax } => {
                let left = AxisCal { min: lmin, max: lmax, invert: self.cal.left.invert };
                let right = AxisCal { min: rmin, max: rmax, invert: self.cal.right.invert };
                if left.is_sane() && right.is_sane() {
                    self.cal.left = left;
                    self.cal.right = right;
                    self.live_cal_saved = false;
                    self.write_cal_line(out);
                } else {
                    let _ = writeln!(out, "{ERR_CALIBRATION}");
                }
                Effect::None
            }
            Command::CalSave => {
                // Neither the `OK`/`ERR` line nor `live_cal_saved` is decided
                // yet: the caller must attempt the flash write and call
                // `finish_cal_save` with the outcome before sending any
                // response (see `Effect`'s docs).
                Effect::SaveCal(self.cal)
            }
            Command::CalSwap(on) => {
                let candidate = Calibration { swap: on, ..self.cal };
                self.pending_swap = Some(candidate);
                Effect::PersistCal(candidate)
            }
            Command::CalReset => {
                self.cal = Calibration::default();
                Effect::PersistCal(self.cal)
            }
            Command::Invert { knob, on } => {
                match knob {
                    Knob::Left => self.cal.left.invert = on,
                    Knob::Right => self.cal.right.invert = on,
                }
                // SPEC-controller.md §1: "flip it in firmware with `INVERT
                // L|R 1` (persisted)" -- so, like `CAL RESET`, this both
                // updates the live calibration immediately and persists it,
                // with the response deferred until the write is attempted.
                Effect::PersistCal(self.cal)
            }
            Command::Bootsel => {
                let _ = writeln!(out, "OK BOOTSEL");
                Effect::Bootsel
            }
            Command::Reboot => {
                let _ = writeln!(out, "OK REBOOT");
                Effect::Reboot
            }
        }
    }

    /// Completes a deferred `CAL SAVE` ([`Effect::SaveCal`]) after the caller
    /// has attempted the flash write. Must be called (and its output sent)
    /// before any other response for that command. Writes `OK CAL SAVE` and
    /// marks the calibration saved on success, or `ERR flash` and leaves it
    /// unsaved on failure.
    pub fn finish_cal_save(&mut self, ok: bool, out: &mut impl Write) {
        self.live_cal_saved = ok;
        if ok {
            let _ = writeln!(out, "OK CAL SAVE");
        } else {
            let _ = writeln!(out, "{ERR_FLASH}");
        }
    }

    /// Completes a deferred `CAL RESET` / `INVERT` / `CAL SWAP` ([`Effect::PersistCal`])
    /// after the caller has attempted the flash write. Must be called (and
    /// its output sent) before any other response for that command. Writes
    /// the full `OK CAL ...` line and marks the calibration saved on success,
    /// or `ERR flash` and leaves it unsaved on failure (the live calibration
    /// itself was already updated for RESET/INVERT; SWAP commits only on success).
    pub fn finish_persist_cal(&mut self, ok: bool, out: &mut impl Write) {
        if let Some(candidate) = self.pending_swap.take() {
            if ok {
                self.cal = candidate;
            }
        }
        self.live_cal_saved = ok;
        if ok {
            self.write_cal_line(out);
        } else {
            let _ = writeln!(out, "{ERR_FLASH}");
        }
    }

    fn write_cal_line(&self, out: &mut impl Write) {
        let _ = writeln!(
            out,
            "OK CAL {} {} {} {} {} {} {} {}",
            self.cal.left.min,
            self.cal.left.max,
            self.cal.right.min,
            self.cal.right.max,
            self.cal.left.invert as u8,
            self.cal.right.invert as u8,
            self.live_cal_saved as u8,
            self.cal.swap as u8
        );
    }
}

/// Formats the unsolicited `STREAM` line: `S l_raw r_raw l r btn seen`.
pub fn format_stream_line(state: &Snapshot, out: &mut impl Write) {
    let _ = writeln!(
        out,
        "S {} {} {} {} {} {}",
        state.raw_l, state.raw_r, state.val_l, state.val_r, state.buttons.pressed, state.buttons.seen
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buf::FixedBuf;

    const UID: [u8; 8] = [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];

    fn device() -> Device {
        Device::new("1.2.3", UID, Calibration::default(), true)
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            raw_l: 12345,
            raw_r: 54321,
            val_l: 111,
            val_r: 222,
            buttons: Buttons { pressed: 1, seen: 3 },
        }
    }

    fn run(d: &mut Device, line: &str) -> (heapless_line::Line, Effect) {
        let mut buf: FixedBuf<160> = FixedBuf::new();
        let eff = d.handle_line(line, &snapshot(), &mut buf);
        (heapless_line::Line::from(buf.as_str()), eff)
    }

    /// A tiny fixed-capacity owned-string stand-in, since there's no `alloc`
    /// here and tests want to compare the exact response text after the
    /// `FixedBuf` it came from has gone out of scope.
    mod heapless_line {
        pub struct Line {
            buf: [u8; 160],
            len: usize,
        }
        impl Line {
            pub fn from(s: &str) -> Self {
                let mut buf = [0u8; 160];
                buf[..s.len()].copy_from_slice(s.as_bytes());
                Self { buf, len: s.len() }
            }
            pub fn as_str(&self) -> &str {
                core::str::from_utf8(&self.buf[..self.len]).unwrap()
            }
        }
    }

    #[test]
    fn info_response_matches_the_protocol_table_exactly() {
        let mut d = device();
        let (line, eff) = run(&mut d, "INFO");
        assert_eq!(
            line.as_str(),
            "OK INFO name=RetroController fw=1.2.3 proto=1 board=xiao-rp2350 uid=0123456789ABCDEF axes=2 max=4095\n"
        );
        assert_eq!(eff, Effect::None);
    }

    #[test]
    fn raw_and_val_report_the_snapshot() {
        let mut d = device();
        let (line, _) = run(&mut d, "RAW");
        assert_eq!(line.as_str(), "OK RAW 12345 54321\n");
        let (line, _) = run(&mut d, "VAL");
        assert_eq!(line.as_str(), "OK VAL 111 222\n");
        let (line, _) = run(&mut d, "BTN");
        assert_eq!(line.as_str(), "OK BTN 1 3\n");
    }

    #[test]
    fn stream_on_and_off() {
        let mut d = device();
        let (line, eff) = run(&mut d, "STREAM ON");
        assert_eq!(line.as_str(), "OK STREAM ON\n");
        assert_eq!(eff, Effect::None);
        assert!(d.stream_on);
        let (line, _) = run(&mut d, "STREAM OFF");
        assert_eq!(line.as_str(), "OK STREAM OFF\n");
        assert!(!d.stream_on);
    }

    #[test]
    fn cal_show_reports_defaults() {
        // AxisCal::default() = { min: DEFAULT_MARGIN (1000), max: RAW_MAX -
        // DEFAULT_MARGIN (64535), invert: false } for both axes.
        let mut d = device();
        let (line, _) = run(&mut d, "CAL SHOW");
        assert_eq!(line.as_str(), "OK CAL 1000 64535 1000 64535 0 0 1 0\n");
    }

    #[test]
    fn cal_start_then_stop_adopts_learned_range() {
        let mut d = device();
        let (line, eff) = run(&mut d, "CAL START");
        assert_eq!(line.as_str(), "OK CAL START\n");
        assert_eq!(eff, Effect::None);
        assert!(d.learning);

        for raw in [1000u16, 500, 64000, 64500] {
            d.observe_raw(raw, raw);
        }

        let (line, eff) = run(&mut d, "CAL STOP");
        assert!(!d.learning);
        assert_eq!(eff, Effect::None);
        assert_eq!(line.as_str(), "OK CAL 700 64300 700 64300 0 0 0 0\n");
        assert!(!d.live_cal_saved);
    }

    #[test]
    fn cal_stop_with_no_samples_is_an_error_and_leaves_cal_untouched() {
        let mut d = device();
        let before = d.cal;
        run(&mut d, "CAL START");
        let (line, eff) = run(&mut d, "CAL STOP");
        assert_eq!(line.as_str(), "ERR calibration\n");
        assert_eq!(eff, Effect::None);
        assert_eq!(d.cal, before);
    }

    #[test]
    fn cal_stop_without_a_preceding_start_is_rejected() {
        let mut d = device();
        let before = d.cal;
        let (line, eff) = run(&mut d, "CAL STOP");
        assert_eq!(line.as_str(), "ERR not learning\n");
        assert_eq!(eff, Effect::None);
        assert_eq!(d.cal, before);
        assert!(d.live_cal_saved, "an unrelated STOP must not touch the saved flag");
    }

    /// Regression test: a second `CAL STOP` with no new `CAL START` in
    /// between used to re-run the learner's stale min/max (from the *first*
    /// cycle) and spuriously clear `live_cal_saved` even though the live
    /// calibration hadn't changed since it was last saved.
    #[test]
    fn repeated_cal_stop_does_not_readopt_stale_learned_data_or_clear_saved() {
        let mut d = device();
        run(&mut d, "CAL START");
        for raw in [1000u16, 500, 64000, 64500] {
            d.observe_raw(raw, raw);
        }
        let (_, eff) = run(&mut d, "CAL STOP");
        assert_eq!(eff, Effect::None);
        let cal_after_first_stop = d.cal;

        // Simulate the operator then saving that calibration.
        d.live_cal_saved = true;

        // A stray second STOP (no new START, no new samples) must be
        // rejected outright, not silently re-adopt the same/stale span and
        // report the calibration as unsaved again.
        let (line, eff) = run(&mut d, "CAL STOP");
        assert_eq!(line.as_str(), "ERR not learning\n");
        assert_eq!(eff, Effect::None);
        assert_eq!(d.cal, cal_after_first_stop);
        assert!(d.live_cal_saved, "a rejected repeat STOP must not clear the saved flag");
    }

    #[test]
    fn cal_set_valid_range_is_adopted() {
        let mut d = device();
        let (line, eff) = run(&mut d, "CAL SET 1000 64000 2000 63000");
        assert_eq!(line.as_str(), "OK CAL 1000 64000 2000 63000 0 0 0 0\n");
        assert_eq!(eff, Effect::None);
        assert!(!d.live_cal_saved);
        assert_eq!(d.cal.left, AxisCal { min: 1000, max: 64000, invert: false });
    }

    #[test]
    fn cal_set_insane_range_is_rejected() {
        let mut d = device();
        let before = d.cal;
        let (line, _) = run(&mut d, "CAL SET 1000 1100 2000 63000");
        assert_eq!(line.as_str(), "ERR calibration\n");
        assert_eq!(d.cal, before);
    }

    /// Regression test: `handle()` must NOT report `OK CAL SAVE` or mark the
    /// calibration saved until the caller has actually attempted the flash
    /// write via `finish_cal_save` — otherwise a failed write is reported to
    /// the host as a success.
    #[test]
    fn cal_save_defers_the_response_until_finish_cal_save() {
        let mut d = device();
        d.live_cal_saved = false;
        let cal_before = d.cal;
        let (line, eff) = run(&mut d, "CAL SAVE");
        assert_eq!(line.as_str(), "", "the response must be deferred, not written by handle() itself");
        assert_eq!(eff, Effect::SaveCal(cal_before));
        assert!(!d.live_cal_saved, "must not be marked saved before the flash write is attempted");

        // Success path.
        let mut buf: FixedBuf<160> = FixedBuf::new();
        d.finish_cal_save(true, &mut buf);
        assert_eq!(buf.as_str(), "OK CAL SAVE\n");
        assert!(d.live_cal_saved);

        // Failure path must report ERR, not OK, and leave the flag unsaved.
        let mut buf: FixedBuf<160> = FixedBuf::new();
        d.finish_cal_save(false, &mut buf);
        assert_eq!(buf.as_str(), "ERR flash\n");
        assert!(!d.live_cal_saved);
    }

    /// Regression test: same deferred-response requirement as
    /// `cal_save_defers_the_response_until_finish_cal_save`, but for `CAL
    /// RESET`, whose pending response is the full `OK CAL ...` line rather
    /// than `OK CAL SAVE`. The live calibration is reset immediately either
    /// way; only whether it's reported *saved* waits on the flash result.
    #[test]
    fn cal_reset_restores_defaults_immediately_but_defers_the_saved_response() {
        let mut d = device();
        run(&mut d, "CAL SET 1000 64000 2000 63000");
        run(&mut d, "INVERT L 1");
        let (line, eff) = run(&mut d, "CAL RESET");
        assert_eq!(d.cal, Calibration::default(), "live cal resets immediately");
        assert_eq!(line.as_str(), "", "the response must be deferred, not written by handle() itself");
        assert_eq!(eff, Effect::PersistCal(Calibration::default()));

        let mut buf: FixedBuf<160> = FixedBuf::new();
        d.finish_persist_cal(true, &mut buf);
        assert_eq!(buf.as_str(), "OK CAL 1000 64535 1000 64535 0 0 1 0\n");
        assert!(d.live_cal_saved);

        let mut buf: FixedBuf<160> = FixedBuf::new();
        d.finish_persist_cal(false, &mut buf);
        assert_eq!(buf.as_str(), "ERR flash\n");
        assert!(!d.live_cal_saved);
    }

    /// Regression test for SPEC-controller.md §1 ("flip it in firmware with
    /// `INVERT L|R 1` (persisted)"): `INVERT` must persist to flash like
    /// `CAL RESET`, and — same as `CAL SAVE`/`CAL RESET` — must not claim
    /// success (or mark saved) before the write is actually attempted.
    #[test]
    fn invert_flips_the_flag_immediately_and_persists() {
        let mut d = device();
        assert!(d.live_cal_saved);
        let (line, eff) = run(&mut d, "INVERT L 1");
        assert!(d.cal.left.invert, "the live flag flips immediately");
        assert!(!d.cal.right.invert);
        assert_eq!(line.as_str(), "", "the response must be deferred, not written by handle() itself");
        assert_eq!(eff, Effect::PersistCal(d.cal));

        let mut buf: FixedBuf<160> = FixedBuf::new();
        d.finish_persist_cal(true, &mut buf);
        assert_eq!(buf.as_str(), "OK CAL 1000 64535 1000 64535 1 0 1 0\n");
        assert!(d.live_cal_saved);

        let (_, eff) = run(&mut d, "INVERT RIGHT ON");
        assert!(d.cal.right.invert);
        assert_eq!(eff, Effect::PersistCal(d.cal));
        let mut buf: FixedBuf<160> = FixedBuf::new();
        d.finish_persist_cal(true, &mut buf);
        assert_eq!(buf.as_str(), "OK CAL 1000 64535 1000 64535 1 1 1 0\n");
    }

    #[test]
    fn invert_persist_failure_reports_err_flash_and_leaves_saved_flag_false() {
        let mut d = device();
        run(&mut d, "INVERT L 1");
        let mut buf: FixedBuf<160> = FixedBuf::new();
        d.finish_persist_cal(false, &mut buf);
        assert_eq!(buf.as_str(), "ERR flash\n");
        assert!(!d.live_cal_saved);
    }

    /// Regression test: without `on_disconnect`, a lost connection left
    /// `learning` (and `stream_on`) set, so the controller stayed in `CAL
    /// START` learning mode indefinitely once the host that started it went
    /// away without a `CAL STOP`.
    #[test]
    fn on_disconnect_stops_streaming_and_leaves_learning_mode() {
        let mut d = device();
        run(&mut d, "STREAM ON");
        run(&mut d, "CAL START");
        assert!(d.stream_on);
        assert!(d.learning);

        d.on_disconnect();

        assert!(!d.stream_on, "a lost connection must stop an unattended STREAM");
        assert!(!d.learning, "a lost connection must not leave the controller stuck in learning mode");
    }

    /// Regression test: `on_disconnect` must also clear the learners
    /// themselves, not just the `learning` flag -- otherwise a fresh `CAL
    /// START` after a reconnect would still be polluted by samples observed
    /// before the disconnect (a stray `CAL STOP` later would then adopt a
    /// span mixing two unrelated sweeps).
    #[test]
    fn on_disconnect_discards_in_progress_learning_samples() {
        let mut d = device();
        run(&mut d, "CAL START");
        d.observe_raw(1000, 1000);
        d.observe_raw(60000, 60000);

        d.on_disconnect();

        run(&mut d, "CAL START");
        d.observe_raw(2000, 2000);
        d.observe_raw(4000, 4000);
        let (line, eff) = run(&mut d, "CAL STOP");
        assert_eq!(eff, Effect::None);
        assert_eq!(
            line.as_str(),
            "OK CAL 2200 3800 2200 3800 0 0 0 0\n",
            "the pre-disconnect samples (1000/60000) must not still be in the learners"
        );
    }

    #[test]
    fn bootsel_and_reboot_effects() {
        let mut d = device();
        let (line, eff) = run(&mut d, "BOOTSEL");
        assert_eq!(line.as_str(), "OK BOOTSEL\n");
        assert_eq!(eff, Effect::Bootsel);

        let (line, eff) = run(&mut d, "REBOOT");
        assert_eq!(line.as_str(), "OK REBOOT\n");
        assert_eq!(eff, Effect::Reboot);
    }

    #[test]
    fn unknown_and_malformed_commands_report_parse_errors() {
        let mut d = device();
        let (line, eff) = run(&mut d, "FROB");
        assert_eq!(line.as_str(), "ERR unknown command\n");
        assert_eq!(eff, Effect::None);

        let (line, _) = run(&mut d, "");
        assert_eq!(line.as_str(), "ERR empty\n");

        let (line, _) = run(&mut d, "STREAM");
        assert_eq!(line.as_str(), "ERR bad argument\n");
    }

    #[test]
    fn swap_is_transactional_and_survives_calibration_and_reboot() {
        let mut d = device();
        let original = d.cal;
        let (line, effect) = run(&mut d, "CAL SWAP ON");
        assert_eq!(line.as_str(), "");
        let Effect::PersistCal(candidate) = effect else {
            panic!("expected persistence");
        };
        assert!(candidate.swap);
        assert_eq!(d.cal, original, "assignment waits for persistence");
        let mut out: FixedBuf<160> = FixedBuf::new();
        d.finish_persist_cal(false, &mut out);
        assert_eq!(out.as_str(), "ERR flash\n");
        assert_eq!(d.cal, original, "failed write rolls back assignment");
        assert!(!d.live_cal_saved, "failed erase may have destroyed flash");

        run(&mut d, "CAL SWAP ON");
        let mut out: FixedBuf<160> = FixedBuf::new();
        d.finish_persist_cal(true, &mut out);
        assert!(d.cal.swap && d.live_cal_saved);
        run(&mut d, "INVERT L 1");
        run(&mut d, "CAL SET 1000 64000 2000 63000");
        assert!(d.cal.swap && d.cal.left.invert);
        run(&mut d, "CAL START");
        d.observe_raw(1000, 2000);
        d.observe_raw(64000, 63000);
        run(&mut d, "CAL STOP");
        assert!(d.cal.swap && d.cal.left.invert);
        let (_, Effect::SaveCal(saved)) = run(&mut d, "CAL SAVE") else {
            panic!("expected save");
        };
        let restored = Calibration::from_bytes(&saved.to_bytes()).unwrap();
        assert!(Device::new("test", UID, restored, true).cal.swap);
        run(&mut d, "CAL SWAP OFF");
        let mut out: FixedBuf<160> = FixedBuf::new();
        d.finish_persist_cal(true, &mut out);
        assert!(!d.cal.swap);
        d.cal.swap = true;
        run(&mut d, "CAL RESET");
        assert!(!d.cal.swap);
    }

    #[test]
    fn val_and_stream_use_assigned_outputs_but_raw_keeps_wiring_order() {
        let mut d = device();
        d.cal.swap = true;
        let (val_l, val_r) = d.cal.apply(1000, 64535);
        let sample = Snapshot { raw_l: 1000, raw_r: 64535, val_l, val_r, buttons: Buttons::default() };
        let mut out: FixedBuf<160> = FixedBuf::new();
        d.handle(Command::Val, &sample, &mut out);
        assert_eq!(out.as_str(), "OK VAL 4095 0\n");
        let mut out: FixedBuf<160> = FixedBuf::new();
        format_stream_line(&sample, &mut out);
        assert_eq!(out.as_str(), "S 1000 64535 4095 0 0 0\n");
    }

    #[test]
    fn stream_line_format() {
        let mut buf: FixedBuf<64> = FixedBuf::new();
        format_stream_line(&snapshot(), &mut buf);
        assert_eq!(buf.as_str(), "S 12345 54321 111 222 1 3\n");
        let old = serial::StreamSample::parse(buf.as_str()).unwrap();
        assert_eq!(old.buttons, Some(Buttons { pressed: 1, seen: 3 }));
    }
}
