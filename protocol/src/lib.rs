//! Shared protocol definitions for the RetroController paddle controller.
//!
//! This crate is the single source of truth for everything the three pieces
//! of software (firmware, host toolkit, game) must agree on:
//!
//! * USB identifiers and strings ([`usb`])
//! * the HID input report layout ([`hid`])
//! * the line-oriented text protocol spoken over the CDC-ACM serial port ([`serial`])
//! * potentiometer calibration math and its persisted form ([`cal`])
//!
//! It is `#![no_std]` so the firmware can use it directly. Enable the `std`
//! feature on the host for a few conveniences (it currently only enables
//! `std`-based tests).

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]

/// Firmware / protocol version advertised in `INFO` responses.
pub const PROTOCOL_VERSION: u8 = 1;

/// USB identity of the controller.
pub mod usb {
    /// pid.codes vendor id. `0x1209:0x0001` is the pid.codes *test* PID, which is
    /// acceptable for a hobby device that is never sold. Change both if you
    /// ever obtain your own PID.
    pub const VID: u16 = 0x1209;
    /// pid.codes test PID (see [`VID`]).
    pub const PID: u16 = 0x0001;
    pub const MANUFACTURER: &str = "RetroController";
    pub const PRODUCT: &str = "RetroController Paddles";
    /// Volume label of the RP2350 ROM bootloader mass-storage device.
    pub const BOOTLOADER_VOLUME: &str = "RP2350";
    /// USB VID/PID of the RP2350 ROM bootloader.
    pub const BOOTLOADER_VID: u16 = 0x2e8a;
    pub const BOOTLOADER_PID: u16 = 0x000f;
}

/// HID input report layout.
///
/// The device enumerates as a *Game Pad* (Generic Desktop usage 0x05) with two
/// 16-bit absolute axes (X = left knob, Y = right knob) and eight buttons.
/// Bits [`BUTTON_LEFT`] and [`BUTTON_RIGHT`] carry the optional momentary
/// buttons, in logical (post-swap) player order; the other bits are always 0.
/// A controller wired without buttons simply never sets them.
pub mod hid {
    /// Maximum logical axis value. Axes run `0..=AXIS_MAX`.
    pub const AXIS_MAX: u16 = 4095;
    /// Size in bytes of the input report (no report id prefix on the wire).
    pub const REPORT_LEN: usize = 5;
    /// HID poll interval requested in the endpoint descriptor, in ms.
    pub const POLL_MS: u8 = 1;
    /// Button bit for the left player's button (HID Button 1).
    pub const BUTTON_LEFT: u8 = 1 << 0;
    /// Button bit for the right player's button (HID Button 2).
    pub const BUTTON_RIGHT: u8 = 1 << 1;

    /// One input report. Little-endian on the wire: `x_lo x_hi y_lo y_hi buttons`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
    pub struct PaddleReport {
        /// Left knob, `0..=AXIS_MAX`. 0 = fully counter-clockwise.
        pub x: u16,
        /// Right knob, `0..=AXIS_MAX`. 0 = fully counter-clockwise.
        pub y: u16,
        /// Button bitmask ([`BUTTON_LEFT`], [`BUTTON_RIGHT`]); 1 = pressed.
        pub buttons: u8,
    }

    impl PaddleReport {
        pub const fn new(x: u16, y: u16) -> Self {
            Self { x, y, buttons: 0 }
        }

        pub const fn with_buttons(x: u16, y: u16, buttons: u8) -> Self {
            Self { x, y, buttons }
        }

        pub fn to_bytes(&self) -> [u8; REPORT_LEN] {
            let x = self.x.min(AXIS_MAX).to_le_bytes();
            let y = self.y.min(AXIS_MAX).to_le_bytes();
            [x[0], x[1], y[0], y[1], self.buttons]
        }

        /// Parses a report. Accepts buffers longer than [`REPORT_LEN`] (extra bytes
        /// are ignored) so callers can pass a whole HID read buffer.
        pub fn from_bytes(b: &[u8]) -> Option<Self> {
            if b.len() < REPORT_LEN {
                return None;
            }
            Some(Self {
                x: u16::from_le_bytes([b[0], b[1]]).min(AXIS_MAX),
                y: u16::from_le_bytes([b[2], b[3]]).min(AXIS_MAX),
                buttons: b[4],
            })
        }

        /// Left knob as `0.0..=1.0`.
        pub fn x_unit(&self) -> f32 {
            self.x.min(AXIS_MAX) as f32 / AXIS_MAX as f32
        }

        /// Right knob as `0.0..=1.0`.
        pub fn y_unit(&self) -> f32 {
            self.y.min(AXIS_MAX) as f32 / AXIS_MAX as f32
        }
    }
}

/// Potentiometer calibration.
///
/// The firmware oversamples the 12-bit ADC and produces a 16-bit *raw* value
/// per knob (`0..=65535`, full ADC span). Calibration maps the mechanically
/// reachable raw range `[min, max]` onto the HID axis range `0..=AXIS_MAX`,
/// optionally inverting the direction.
pub mod cal {
    use super::hid::AXIS_MAX;

    /// Raw values are 16-bit.
    pub const RAW_MAX: u16 = 65535;
    /// Default margin trimmed from each end of the raw span so a pot that does
    /// not quite reach the rails still hits 0 and AXIS_MAX. About 1.5%.
    pub const DEFAULT_MARGIN: u16 = 1000;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct AxisCal {
        pub min: u16,
        pub max: u16,
        pub invert: bool,
    }

    impl Default for AxisCal {
        fn default() -> Self {
            Self { min: DEFAULT_MARGIN, max: RAW_MAX - DEFAULT_MARGIN, invert: false }
        }
    }

    impl AxisCal {
        /// Maps a raw 16-bit reading onto `0..=AXIS_MAX`, clamping at the ends.
        pub fn apply(&self, raw: u16) -> u16 {
            let (min, max) = if self.min < self.max { (self.min, self.max) } else { (self.max, self.min) };
            let span = (max - min) as u32;
            let r = raw.clamp(min, max) as u32 - min as u32;
            // Round-to-nearest scaling; a zero span maps to 0.
            let v = (r * AXIS_MAX as u32 + span / 2).checked_div(span).unwrap_or(0) as u16;
            if self.invert {
                AXIS_MAX - v
            } else {
                v
            }
        }

        /// Returns `true` if the calibration is usable (a non-trivial span).
        pub fn is_sane(&self) -> bool {
            let lo = self.min.min(self.max);
            let hi = self.min.max(self.max);
            hi - lo >= 1024
        }
    }

    /// Calibration for both knobs plus a persisted-format header.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
    pub struct Calibration {
        pub left: AxisCal,
        pub right: AxisCal,
        /// Exchange logical outputs after physical-axis calibration.
        pub swap: bool,
    }

    impl Calibration {
        /// Calibrate physical inputs first, then assign logical left/right.
        pub fn apply(&self, raw_l: u16, raw_r: u16) -> (u16, u16) {
            let pair = (self.left.apply(raw_l), self.right.apply(raw_r));
            if self.swap {
                (pair.1, pair.0)
            } else {
                pair
            }
        }

        pub const MAGIC: [u8; 4] = *b"RCC1";
        /// Serialized size in bytes.
        pub const LEN: usize = 16;

        /// Serializes to a fixed 16-byte record:
        /// `magic[4] lmin[2] lmax[2] rmin[2] rmax[2] flags[1] pad[2] crc8[1]`. Flags bits 0/1 invert physical axes;
        /// bit 2 swaps logical outputs (zero in legacy records).
        pub fn to_bytes(&self) -> [u8; Self::LEN] {
            let mut b = [0u8; Self::LEN];
            b[0..4].copy_from_slice(&Self::MAGIC);
            b[4..6].copy_from_slice(&self.left.min.to_le_bytes());
            b[6..8].copy_from_slice(&self.left.max.to_le_bytes());
            b[8..10].copy_from_slice(&self.right.min.to_le_bytes());
            b[10..12].copy_from_slice(&self.right.max.to_le_bytes());
            b[12] = (self.left.invert as u8) | ((self.right.invert as u8) << 1) | ((self.swap as u8) << 2);
            b[15] = crc8(&b[..15]);
            b
        }

        /// Parses a record produced by [`Self::to_bytes`]; `None` if the magic or
        /// checksum is wrong or the values are insane (e.g. erased flash).
        pub fn from_bytes(b: &[u8]) -> Option<Self> {
            if b.len() < Self::LEN || b[0..4] != Self::MAGIC || crc8(&b[..15]) != b[15] {
                return None;
            }
            let c = Self {
                swap: b[12] & 4 != 0,
                left: AxisCal {
                    min: u16::from_le_bytes([b[4], b[5]]),
                    max: u16::from_le_bytes([b[6], b[7]]),
                    invert: b[12] & 1 != 0,
                },
                right: AxisCal {
                    min: u16::from_le_bytes([b[8], b[9]]),
                    max: u16::from_le_bytes([b[10], b[11]]),
                    invert: b[12] & 2 != 0,
                },
            };
            if c.left.is_sane() && c.right.is_sane() {
                Some(c)
            } else {
                None
            }
        }
    }

    /// CRC-8 (poly 0x07, init 0), used for the calibration record.
    pub fn crc8(data: &[u8]) -> u8 {
        let mut crc: u8 = 0;
        for &byte in data {
            crc ^= byte;
            for _ in 0..8 {
                crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
            }
        }
        crc
    }
}

/// Line-oriented text protocol over the CDC-ACM serial port.
///
/// Every command is one ASCII line terminated by `\n` (a preceding `\r` is
/// ignored). Keywords are case-insensitive. Every command produces exactly one
/// response line starting with `OK` or `ERR`, except that `STREAM ON` additionally
/// starts periodic unsolicited `S ...` lines until `STREAM OFF`.
///
/// | Command                         | Response                                                    |
/// |---------------------------------|-------------------------------------------------------------|
/// | `INFO`                          | `OK INFO name=<s> fw=<semver> proto=<n> board=<s> uid=<hex> axes=2 max=4095` |
/// | `RAW`                           | `OK RAW <l_raw> <r_raw>` (16-bit oversampled ADC)           |
/// | `VAL`                           | `OK VAL <l> <r>` (calibrated 0..4095, what HID reports)     |
/// | `STREAM ON` / `STREAM OFF`      | `OK STREAM ON|OFF`; then `S <l_raw> <r_raw> <l> <r> <btn> <seen>` at [`STREAM_HZ`] |
/// | `BTN`                           | `OK BTN <btn> <seen>` (button bitmasks: pressed now, ever pressed since boot) |
/// | `CAL SHOW`                      | `OK CAL <lmin> <lmax> <rmin> <rmax> <linv> <rinv> <saved> <swap>`  |
/// | `CAL START`                     | `OK CAL START` (begins learning min/max from live raw)      |
/// | `CAL STOP`                      | `OK CAL <...>` (adopts learned range into the live cal) / `ERR calibration` (nothing learned, or too small) / `ERR not learning` (no `CAL START` in progress) |
/// | `CAL SET <lmin> <lmax> <rmin> <rmax>` | `OK CAL <...>` / `ERR calibration` (span too small)   |
/// | `CAL SAVE`                      | `OK CAL SAVE` (persists live cal to flash) / `ERR flash` (erase/write/read-back failed) |
/// | `CAL SWAP ON|OFF`              | `OK CAL <...>` (assignment persisted after flash success) / `ERR flash` |
/// | `CAL RESET`                     | `OK CAL <...>` (defaults, also persisted) / `ERR flash`     |
/// | `INVERT L|R 0|1`                | `OK CAL <...>` (also persisted) / `ERR flash`               |
/// | `BOOTSEL`                       | `OK BOOTSEL` then reboots into the USB bootloader           |
/// | `REBOOT`                        | `OK REBOOT` then resets                                     |
/// | anything else                   | `ERR unknown command` / `ERR bad argument`                  |
pub mod serial {
    /// Rate of unsolicited `S` lines while streaming.
    pub const STREAM_HZ: u32 = 50;
    /// Maximum accepted command line length (bytes, excluding newline).
    pub const MAX_LINE: usize = 64;
    /// Serial baud rate is irrelevant for CDC-ACM but hosts must pick one.
    pub const NOMINAL_BAUD: u32 = 115_200;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Knob {
        Left,
        Right,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Command {
        Info,
        Raw,
        Val,
        Buttons,
        Stream(bool),
        CalShow,
        CalStart,
        CalStop,
        CalSet { lmin: u16, lmax: u16, rmin: u16, rmax: u16 },
        CalSave,
        CalReset,
        CalSwap(bool),
        Invert { knob: Knob, on: bool },
        Bootsel,
        Reboot,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum ParseError {
        Empty,
        UnknownCommand,
        BadArgument,
    }

    impl ParseError {
        /// The `ERR ...` line to send back.
        pub const fn message(&self) -> &'static str {
            match self {
                ParseError::Empty => "ERR empty",
                ParseError::UnknownCommand => "ERR unknown command",
                ParseError::BadArgument => "ERR bad argument",
            }
        }
    }

    fn eq_ic(a: &str, b: &str) -> bool {
        a.len() == b.len() && a.bytes().zip(b.bytes()).all(|(x, y)| x.eq_ignore_ascii_case(&y))
    }

    fn parse_u16(s: Option<&str>) -> Result<u16, ParseError> {
        s.ok_or(ParseError::BadArgument)?.parse::<u16>().map_err(|_| ParseError::BadArgument)
    }

    fn parse_bool(s: Option<&str>) -> Result<bool, ParseError> {
        match s {
            Some(v) if eq_ic(v, "1") || eq_ic(v, "ON") || eq_ic(v, "TRUE") => Ok(true),
            Some(v) if eq_ic(v, "0") || eq_ic(v, "OFF") || eq_ic(v, "FALSE") => Ok(false),
            _ => Err(ParseError::BadArgument),
        }
    }

    /// Parses one command line (without the trailing newline; `\r` and
    /// surrounding whitespace are tolerated).
    pub fn parse_command(line: &str) -> Result<Command, ParseError> {
        let mut it = line
            .trim_matches(|c: char| c == '\r' || c == '\n' || c == ' ' || c == '\t')
            .split_ascii_whitespace();
        let kw = it.next().ok_or(ParseError::Empty)?;
        let cmd = if eq_ic(kw, "INFO") {
            Command::Info
        } else if eq_ic(kw, "RAW") {
            Command::Raw
        } else if eq_ic(kw, "VAL") {
            Command::Val
        } else if eq_ic(kw, "BTN") {
            Command::Buttons
        } else if eq_ic(kw, "STREAM") {
            Command::Stream(parse_bool(it.next())?)
        } else if eq_ic(kw, "CAL") {
            let sub = it.next().ok_or(ParseError::BadArgument)?;
            if eq_ic(sub, "SHOW") {
                Command::CalShow
            } else if eq_ic(sub, "START") {
                Command::CalStart
            } else if eq_ic(sub, "STOP") {
                Command::CalStop
            } else if eq_ic(sub, "SAVE") {
                Command::CalSave
            } else if eq_ic(sub, "RESET") {
                Command::CalReset
            } else if eq_ic(sub, "SWAP") {
                Command::CalSwap(parse_bool(it.next())?)
            } else if eq_ic(sub, "SET") {
                Command::CalSet {
                    lmin: parse_u16(it.next())?,
                    lmax: parse_u16(it.next())?,
                    rmin: parse_u16(it.next())?,
                    rmax: parse_u16(it.next())?,
                }
            } else {
                return Err(ParseError::BadArgument);
            }
        } else if eq_ic(kw, "INVERT") {
            let knob = match it.next() {
                Some(k) if eq_ic(k, "L") || eq_ic(k, "LEFT") => Knob::Left,
                Some(k) if eq_ic(k, "R") || eq_ic(k, "RIGHT") => Knob::Right,
                _ => return Err(ParseError::BadArgument),
            };
            Command::Invert { knob, on: parse_bool(it.next())? }
        } else if eq_ic(kw, "BOOTSEL") {
            Command::Bootsel
        } else if eq_ic(kw, "REBOOT") {
            Command::Reboot
        } else {
            return Err(ParseError::UnknownCommand);
        };
        if it.next().is_some() {
            return Err(ParseError::BadArgument);
        }
        Ok(cmd)
    }

    /// Button state as bitmasks of [`super::hid::BUTTON_LEFT`]/[`super::hid::BUTTON_RIGHT`],
    /// in logical (post-swap) player order.
    ///
    /// Buttons are optional and switch to GND against an internal pull-up, so
    /// an absent button is indistinguishable from an idle one until pressed:
    /// `seen` marks the buttons that have been pressed at least once since
    /// boot, i.e. are known to be wired.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Buttons {
        pub pressed: u8,
        pub seen: u8,
    }

    impl Buttons {
        /// Parses the `<btn> <seen>` fields of a `BTN` response or `S` line.
        fn parse<'a>(mut it: impl Iterator<Item = &'a str>) -> Option<Self> {
            let mut n = || it.next()?.parse::<u8>().ok();
            Some(Self { pressed: n()?, seen: n()? })
        }

        /// Parses an `OK BTN <btn> <seen>` response.
        pub fn parse_response(line: &str) -> Option<Self> {
            let mut it = line.trim().split_ascii_whitespace();
            if !eq_ic(it.next()?, "OK") || !eq_ic(it.next()?, "BTN") {
                return None;
            }
            Self::parse(it)
        }
    }

    /// A parsed unsolicited stream line `S <l_raw> <r_raw> <l> <r> [<btn> <seen>]`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct StreamSample {
        pub left_raw: u16,
        pub right_raw: u16,
        pub left: u16,
        pub right: u16,
        /// `None` from firmware that predates button support.
        pub buttons: Option<Buttons>,
    }

    impl StreamSample {
        pub fn parse(line: &str) -> Option<Self> {
            let mut it = line.trim().split_ascii_whitespace();
            if !eq_ic(it.next()?, "S") {
                return None;
            }
            let mut n = || it.next()?.parse::<u16>().ok();
            let (left_raw, right_raw, left, right) = (n()?, n()?, n()?, n()?);
            Some(Self { left_raw, right_raw, left, right, buttons: Buttons::parse(it) })
        }
    }

    /// Parsed `OK CAL <lmin> <lmax> <rmin> <rmax> <linv> <rinv> <saved> <swap>` line.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct CalLine {
        pub cal: super::cal::Calibration,
        pub saved: bool,
    }

    impl CalLine {
        pub fn parse(line: &str) -> Option<Self> {
            let mut it = line.trim().split_ascii_whitespace();
            if !eq_ic(it.next()?, "OK") || !eq_ic(it.next()?, "CAL") {
                return None;
            }
            let mut n = || it.next()?.parse::<u16>().ok();
            let (lmin, lmax, rmin, rmax) = (n()?, n()?, n()?, n()?);
            let (linv, rinv, saved) = (n()? != 0, n()? != 0, n()? != 0);
            let swap = match it.next() {
                None => false, // Older firmware has no assignment field.
                Some("0") => false,
                Some("1") => true,
                _ => return None,
            };
            Some(Self {
                cal: super::cal::Calibration {
                    swap,
                    left: super::cal::AxisCal { min: lmin, max: lmax, invert: linv },
                    right: super::cal::AxisCal { min: rmin, max: rmax, invert: rinv },
                },
                saved,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::cal::*;
    use super::hid::*;
    use super::serial::*;

    #[test]
    fn report_roundtrip() {
        let r = PaddleReport::new(123, 4095);
        let b = r.to_bytes();
        assert_eq!(b, [123, 0, 0xff, 0x0f, 0]);
        assert_eq!(PaddleReport::from_bytes(&b), Some(r));
        assert_eq!(PaddleReport::from_bytes(&b[..4]), None);
        // Out-of-range values are clamped on both paths.
        assert_eq!(PaddleReport::new(60000, 0).to_bytes()[..2], [0xff, 0x0f]);
        assert_eq!(PaddleReport::from_bytes(&[0xff, 0xff, 0, 0, 0]).unwrap().x, AXIS_MAX);
        let pressed = PaddleReport::with_buttons(1, 2, BUTTON_LEFT | BUTTON_RIGHT);
        assert_eq!(pressed.to_bytes()[4], 3);
        assert_eq!(PaddleReport::from_bytes(&pressed.to_bytes()), Some(pressed));
    }

    #[test]
    fn calibration_maps_and_clamps() {
        let c = AxisCal { min: 1000, max: 64535, invert: false };
        assert_eq!(c.apply(0), 0);
        assert_eq!(c.apply(1000), 0);
        assert_eq!(c.apply(64535), AXIS_MAX);
        assert_eq!(c.apply(65535), AXIS_MAX);
        let mid = c.apply(32767);
        assert!((2040..=2056).contains(&mid), "mid={mid}");
        let inv = AxisCal { invert: true, ..c };
        assert_eq!(inv.apply(0), AXIS_MAX);
        assert_eq!(inv.apply(65535), 0);
        // Swapped min/max still works.
        let sw = AxisCal { min: 64535, max: 1000, invert: false };
        assert_eq!(sw.apply(0), 0);
        assert_eq!(sw.apply(65535), AXIS_MAX);
        assert!(!AxisCal { min: 100, max: 200, invert: false }.is_sane());
        assert_eq!(AxisCal { min: 5, max: 5, invert: false }.apply(5), 0);
    }

    #[test]
    fn calibration_record_roundtrip() {
        let c = Calibration {
            swap: true,
            left: AxisCal { min: 900, max: 64000, invert: true },
            right: AxisCal { min: 1200, max: 63000, invert: false },
        };
        let b = c.to_bytes();
        assert_eq!(Calibration::from_bytes(&b), Some(c));
        let mut bad = b;
        bad[5] ^= 1;
        assert_eq!(Calibration::from_bytes(&bad), None);
        assert_eq!(Calibration::from_bytes(&[0xff; 16]), None);
        assert_eq!(Calibration::from_bytes(&[0x00; 16]), None);
        assert_eq!(Calibration::from_bytes(&Calibration::default().to_bytes()), Some(Calibration::default()));
    }

    #[test]
    fn legacy_flash_and_cal_response_default_to_unswapped() {
        // This is the original RCC1 layout: only inversion bits were set.
        let mut old = Calibration::default().to_bytes();
        old[12] = 3;
        old[15] = crc8(&old[..15]);
        let cal = Calibration::from_bytes(&old).unwrap();
        assert!(!cal.swap);
        assert!(cal.left.invert && cal.right.invert);
        assert!(!CalLine::parse("OK CAL 1000 64535 1000 64535 0 0 1").unwrap().cal.swap);
        assert!(CalLine::parse("OK CAL 1000 64535 1000 64535 0 0 1 1").unwrap().cal.swap);
    }

    #[test]
    fn assignment_follows_physical_calibration_and_inversion_in_hid() {
        let cal = Calibration {
            left: AxisCal { min: 1000, max: 5000, invert: true },
            right: AxisCal { min: 10000, max: 20000, invert: false },
            swap: true,
        };
        let (left, right) = cal.apply(1000, 15000);
        assert_eq!((left, right), (2048, 4095));
        let bytes = PaddleReport::new(left, right).to_bytes();
        assert_eq!(PaddleReport::from_bytes(&bytes).unwrap(), PaddleReport::new(2048, 4095));
        assert_eq!(parse_command("CAL SWAP on"), Ok(Command::CalSwap(true)));
        assert_eq!(parse_command("CAL SWAP 0"), Ok(Command::CalSwap(false)));
        assert_eq!(parse_command("CAL SWAP yes"), Err(ParseError::BadArgument));
        assert_eq!(parse_command("CAL SWAP 1 extra"), Err(ParseError::BadArgument));
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse_command("INFO"), Ok(Command::Info));
        assert_eq!(parse_command("info\r"), Ok(Command::Info));
        assert_eq!(parse_command("  RAW  "), Ok(Command::Raw));
        assert_eq!(parse_command("VAL"), Ok(Command::Val));
        assert_eq!(parse_command("STREAM ON"), Ok(Command::Stream(true)));
        assert_eq!(parse_command("stream 0"), Ok(Command::Stream(false)));
        assert_eq!(parse_command("STREAM"), Err(ParseError::BadArgument));
        assert_eq!(parse_command("CAL SHOW"), Ok(Command::CalShow));
        assert_eq!(parse_command("CAL START"), Ok(Command::CalStart));
        assert_eq!(parse_command("CAL STOP"), Ok(Command::CalStop));
        assert_eq!(parse_command("CAL SAVE"), Ok(Command::CalSave));
        assert_eq!(parse_command("CAL RESET"), Ok(Command::CalReset));
        assert_eq!(
            parse_command("CAL SET 1 2 3 4"),
            Ok(Command::CalSet { lmin: 1, lmax: 2, rmin: 3, rmax: 4 })
        );
        assert_eq!(parse_command("CAL SET 1 2 3"), Err(ParseError::BadArgument));
        assert_eq!(parse_command("CAL SET 1 2 3 70000"), Err(ParseError::BadArgument));
        assert_eq!(parse_command("CAL BOGUS"), Err(ParseError::BadArgument));
        assert_eq!(parse_command("INVERT L 1"), Ok(Command::Invert { knob: Knob::Left, on: true }));
        assert_eq!(parse_command("invert right off"), Ok(Command::Invert { knob: Knob::Right, on: false }));
        assert_eq!(parse_command("BOOTSEL"), Ok(Command::Bootsel));
        assert_eq!(parse_command("REBOOT"), Ok(Command::Reboot));
        assert_eq!(parse_command("INFO extra"), Err(ParseError::BadArgument));
        assert_eq!(parse_command("FROB"), Err(ParseError::UnknownCommand));
        assert_eq!(parse_command(""), Err(ParseError::Empty));
        assert_eq!(parse_command("\r\n"), Err(ParseError::Empty));
    }

    #[test]
    fn parses_stream_and_cal_lines() {
        assert_eq!(
            StreamSample::parse("S 100 200 6 12\r\n"),
            Some(StreamSample { left_raw: 100, right_raw: 200, left: 6, right: 12, buttons: None })
        );
        assert_eq!(
            StreamSample::parse("S 100 200 6 12 1 3").unwrap().buttons,
            Some(Buttons { pressed: 1, seen: 3 })
        );
        assert_eq!(Buttons::parse_response("OK BTN 2 2\n"), Some(Buttons { pressed: 2, seen: 2 }));
        assert_eq!(Buttons::parse_response("ERR unknown command"), None);
        assert_eq!(parse_command("btn"), Ok(Command::Buttons));
        assert_eq!(StreamSample::parse("OK RAW 1 2"), None);
        let cl = CalLine::parse("OK CAL 1000 64535 1000 64535 0 1 1").unwrap();
        assert!(cl.saved);
        assert!(cl.cal.right.invert);
        assert_eq!(cl.cal.left.min, 1000);
        assert_eq!(CalLine::parse("OK RAW 1 2"), None);
    }
}
