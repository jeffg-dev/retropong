//! Talking to a connected RetroController: enumerating and opening the
//! CDC-ACM serial port (SPEC-controller.md §5) and the HID gamepad
//! interface.

use std::io::{BufRead, BufReader, Write};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use retrocontroller_protocol::{hid::PaddleReport, serial, usb};
use serialport::{SerialPort, SerialPortInfo, SerialPortType};

/// Per-read timeout on the opened port.
pub const OPEN_TIMEOUT: Duration = Duration::from_millis(200);
/// How long [`SerialClient::send`] waits for a response line.
pub const COMMAND_DEADLINE: Duration = Duration::from_secs(2);
/// Poll interval for [`wait_for_serial_port`].
pub const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[cfg(target_os = "linux")]
fn permissions_hint() -> &'static str {
    "\n  On Linux this is often a udev permissions issue -- try `rcctl udev --install`."
}
#[cfg(not(target_os = "linux"))]
fn permissions_hint() -> &'static str {
    ""
}

/// Returns [`permissions_hint`] if `msg` looks like a permission error,
/// otherwise an empty string. On Linux, both `serialport` (sysfs-based
/// enumeration) and hidapi's `linux-native` backend (which reads
/// `/sys/.../report_descriptor`) can enumerate a device whose node the user
/// has no permission to *open* -- so the realistic first-run failure comes
/// from `open()`/`open_device()`, not from "device not found"
/// (`research:host-hid-input` §2). Matching on the message text (rather than
/// a specific error type) covers both the `serialport::Error` and
/// `hidapi::HidError` cases with one check.
fn hint_if_permission_denied(msg: &str) -> &'static str {
    if msg.to_ascii_lowercase().contains("permission denied") {
        permissions_hint()
    } else {
        ""
    }
}

/// Finds the controller's CDC-ACM serial port by USB VID/PID. On macOS,
/// where every USB-CDC device shows up twice (`/dev/cu.*` call-out and
/// `/dev/tty.*` dial-in), prefers the `cu.` device, which doesn't block
/// waiting for carrier detect.
///
/// The "not found" case here never appends [`permissions_hint`]: `serialport`
/// (built with `default-features = false`, per `Cargo.toml`) enumerates
/// Linux ports from `/sys/class/tty`, never opening a device node, so a udev
/// permissions problem cannot be *why* nothing showed up here -- it can only
/// ever surface later, when actually opening the port
/// (`research:host-hid-input` §2, and this file's own `hint_if_permission_denied`
/// doc comment).
pub fn find_serial_port() -> Result<SerialPortInfo> {
    let mut ports: Vec<_> = serialport::available_ports()
        .context("listing serial ports")?
        .into_iter()
        .filter(
            |p| matches!(&p.port_type, SerialPortType::UsbPort(u) if u.vid == usb::VID && u.pid == usb::PID),
        )
        .collect();
    ports.sort_by_key(|p| p.port_name.starts_with("/dev/tty."));
    ports.into_iter().next().ok_or_else(|| {
        anyhow!(
            "no RetroController serial port found (looked for USB {:04x}:{:04x}); \
             is the controller plugged in and out of the boot ROM? (see `rcctl devices`)",
            usb::VID,
            usb::PID,
        )
    })
}

/// Polls every [`POLL_INTERVAL`] until the controller's serial port
/// reappears (e.g. after a flash) or `timeout` elapses.
pub fn wait_for_serial_port(timeout: Duration) -> Option<SerialPortInfo> {
    let start = Instant::now();
    loop {
        if let Ok(p) = find_serial_port() {
            return Some(p);
        }
        if start.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Opens `port_name` at the protocol's baud rate and asserts DTR. The
/// firmware only *sends* while DTR is asserted (so an unattended port never
/// stalls the device), and macOS in particular does not assert it on open
/// by itself (SPEC-controller.md §5 / `research:host-hid-input` §6).
pub fn open_serial(port_name: &str) -> Result<Box<dyn SerialPort>> {
    let mut port = serialport::new(port_name, serial::NOMINAL_BAUD)
        .timeout(OPEN_TIMEOUT)
        .dtr_on_open(true)
        .open()
        .map_err(|e| {
            let hint = hint_if_permission_denied(&e.to_string());
            anyhow!("opening serial port {port_name}: {e}{hint}")
        })?;
    port.write_data_terminal_ready(true).context("asserting DTR")?;
    Ok(port)
}

/// Reads one line out of `reader`, keeping any bytes read but not yet
/// newline-terminated in `partial` across calls (see
/// [`SerialClient::read_line`]'s doc comment for the bug this fixes). Pure
/// text-buffering logic over `BufRead`'s `fill_buf`/`consume`, so it's
/// exercised directly by this module's tests against a scripted, non-serial
/// reader instead of needing a real (or fake trait-object) serial port.
fn read_line_from<R: BufRead>(reader: &mut R, partial: &mut Vec<u8>) -> Result<Option<String>> {
    loop {
        if let Some(pos) = partial.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = partial.drain(..=pos).collect();
            let line = String::from_utf8(raw).context("serial line was not valid UTF-8")?;
            return Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()));
        }

        let buf = match reader.fill_buf() {
            Ok(buf) => buf,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                return Ok(None);
            }
            Err(e) => return Err(e).context("reading from serial port"),
        };
        if buf.is_empty() {
            bail!("serial port closed unexpectedly");
        }
        partial.extend_from_slice(buf);
        let n = buf.len();
        reader.consume(n);
    }
}

/// A line-oriented client for the text protocol in SPEC-controller.md §5.
pub struct SerialClient {
    reader: BufReader<Box<dyn SerialPort>>,
    /// Bytes of a line read so far that haven't seen a trailing `\n` yet,
    /// kept across [`SerialClient::read_line`] calls -- see that method's doc
    /// comment for why this can't just be a local variable.
    partial: Vec<u8>,
}

impl SerialClient {
    pub fn open(port_name: &str) -> Result<Self> {
        Ok(Self { reader: BufReader::new(open_serial(port_name)?), partial: Vec::new() })
    }

    /// Writes one command line (the trailing `\n` is added).
    pub fn write_line(&mut self, cmd: &str) -> Result<()> {
        let port = self.reader.get_mut();
        port.write_all(cmd.as_bytes()).context("writing command")?;
        port.write_all(b"\n").context("writing command newline")?;
        port.flush().context("flushing command")?;
        Ok(())
    }

    /// Reads one line if one is immediately available (within the port's
    /// read timeout), or `None` if nothing arrived in that window; the
    /// caller decides whether/how long to keep polling.
    ///
    /// Delegates to [`read_line_from`], keeping already-read bytes in
    /// `self.partial` across calls that hit a mid-line timeout. Before this
    /// fix, `read_line` called `BufReader::read_line` directly into a
    /// `String` local to that one call: on a `TimedOut`/`WouldBlock` error,
    /// `std`'s default `read_until` had already moved any bytes read so far
    /// for that call out of the `BufReader`'s own internal buffer and into
    /// that local `String`, which was then simply dropped along with the
    /// `Err` branch -- silently losing them. The `OPEN_TIMEOUT` is only
    /// 200ms and a ~100-byte `OK INFO ...` response line spans two 64-byte
    /// USB packets, so any stall between them (host scheduling under load, a
    /// flash erase around `CAL SAVE`/`CAL RESET` stalling the firmware)
    /// triggered this: the fragment already read would be discarded here,
    /// and the remainder would then get returned as if it were the *whole*
    /// response to whatever command `send()` issued next, which reliably
    /// isn't a well-formed `OK .../ERR ...` line.
    pub fn read_line(&mut self) -> Result<Option<String>> {
        read_line_from(&mut self.reader, &mut self.partial)
    }

    /// Sends one command and returns its `OK ...`/`ERR ...` response line,
    /// silently skipping any unsolicited `S ...` stream samples that arrive
    /// first (e.g. if `STREAM ON` is already active), with a
    /// [`COMMAND_DEADLINE`] overall timeout.
    pub fn send(&mut self, cmd: &str) -> Result<String> {
        self.write_line(cmd)?;
        let deadline = Instant::now() + COMMAND_DEADLINE;
        loop {
            if Instant::now() >= deadline {
                bail!("timed out waiting for a response to `{cmd}`");
            }
            match self.read_line()? {
                None => continue,
                Some(line) => {
                    if line.is_empty() {
                        continue;
                    }
                    if serial::StreamSample::parse(&line).is_some() {
                        continue; // unsolicited STREAM sample
                    }
                    return Ok(line);
                }
            }
        }
    }
}

/// Finds our HID device, preferring the Generic Desktop (0x01) / Game Pad
/// (0x05) usage the firmware declares, and falling back to VID/PID alone in
/// case the platform/backend doesn't expose usage info (the libusb hidapi
/// backends do not; `research:host-hid-input` §1).
pub fn find_hid_device_info(api: &hidapi::HidApi) -> Option<&hidapi::DeviceInfo> {
    api.device_list()
        .find(|d| {
            d.vendor_id() == usb::VID
                && d.product_id() == usb::PID
                && d.usage_page() == 0x01
                && d.usage() == 0x05
        })
        .or_else(|| api.device_list().find(|d| d.vendor_id() == usb::VID && d.product_id() == usb::PID))
}

/// Opens the controller's HID gamepad interface.
///
/// As with [`find_serial_port`], the "not found" error here never appends
/// [`permissions_hint`]: the `linux-native-basic-udev` hidapi backend this
/// crate is built with enumerates purely from sysfs/uevent properties
/// (`HID_ID`, `HID_NAME`, `HID_UNIQ`, `report_descriptor`), never opening
/// `/dev/hidrawN` (`research:host-hid-input` §1), so a udev permissions
/// problem cannot explain nothing being found here -- only opening a device
/// that *was* found can hit that.
pub fn open_hid() -> Result<hidapi::HidDevice> {
    let api = hidapi::HidApi::new().context("initializing hidapi")?;
    let info = find_hid_device_info(&api).ok_or_else(|| {
        anyhow!(
            "no RetroController HID device found (looked for USB {:04x}:{:04x}); \
             is the controller plugged in? (see `rcctl devices`)",
            usb::VID,
            usb::PID,
        )
    })?;
    info.open_device(&api).map_err(|e| {
        let hint = hint_if_permission_denied(&e.to_string());
        anyhow!("opening HID device: {e}{hint}")
    })
}

/// Drains every report currently queued on `dev` and returns the most
/// recent one, or `None` if nothing was queued. hidraw/IOHIDFamily queues
/// fill up quickly relative to a UI redraw rate, so callers should call
/// this once per frame/tick rather than a single blocking `read`
/// (`research:host-hid-input` §1).
pub fn read_latest_report(dev: &hidapi::HidDevice) -> Result<Option<PaddleReport>> {
    let mut buf = [0u8; 64];
    let mut latest = None;
    loop {
        match dev.read_timeout(&mut buf, 0) {
            Ok(0) => break,
            Ok(n) => {
                if let Some(r) = PaddleReport::from_bytes(&buf[..n]) {
                    latest = Some(r);
                }
            }
            Err(e) => bail!("reading HID report: {e}"),
        }
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    /// One scripted `Read::read` call: either a chunk of bytes (as one real
    /// `read()` on the underlying port would return) or a deterministic
    /// `TimedOut` error, standing in for the port's OS-level read timeout
    /// elapsing mid-line.
    enum Chunk {
        Data(&'static [u8]),
        Timeout,
    }

    struct ScriptedPort(VecDeque<Chunk>);

    impl std::io::Read for ScriptedPort {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.0.pop_front() {
                None => Ok(0),
                Some(Chunk::Timeout) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out")),
                Some(Chunk::Data(d)) => {
                    let n = d.len().min(buf.len());
                    buf[..n].copy_from_slice(&d[..n]);
                    Ok(n)
                }
            }
        }
    }

    /// Reproduces the bug report's exact scenario: an `OK INFO ...` response
    /// line arrives split across a read timeout that lands between two USB
    /// packets. Before this fix, the first `read_line` call (the one that
    /// hits the timeout) discarded `"OK INFO name=Retro"` entirely instead of
    /// keeping it for the next call.
    #[test]
    fn read_line_keeps_partial_line_across_a_mid_line_timeout() {
        let port = ScriptedPort(VecDeque::from([
            Chunk::Data(b"OK INFO name=Retro"),
            Chunk::Timeout,
            Chunk::Data(b"Controller fw=0.1.0\n"),
        ]));
        let mut reader = BufReader::new(port);
        let mut partial = Vec::new();

        assert_eq!(read_line_from(&mut reader, &mut partial).unwrap(), None);
        assert_eq!(
            read_line_from(&mut reader, &mut partial).unwrap(),
            Some("OK INFO name=RetroController fw=0.1.0".to_string())
        );
    }

    #[test]
    fn read_line_returns_none_repeatedly_while_nothing_arrives() {
        let port = ScriptedPort(VecDeque::from([Chunk::Timeout, Chunk::Timeout]));
        let mut reader = BufReader::new(port);
        let mut partial = Vec::new();
        assert_eq!(read_line_from(&mut reader, &mut partial).unwrap(), None);
        assert_eq!(read_line_from(&mut reader, &mut partial).unwrap(), None);
    }

    #[test]
    fn read_line_handles_a_line_delivered_whole() {
        let port = ScriptedPort(VecDeque::from([Chunk::Data(b"OK BOOTSEL\r\n")]));
        let mut reader = BufReader::new(port);
        let mut partial = Vec::new();
        assert_eq!(read_line_from(&mut reader, &mut partial).unwrap(), Some("OK BOOTSEL".to_string()));
    }

    /// Two complete lines can arrive in a single underlying read (e.g. an
    /// unsolicited `STREAM` sample followed immediately by a command
    /// response); the second must come back on the *next* call with no
    /// further I/O needed.
    #[test]
    fn read_line_handles_two_lines_delivered_in_one_read() {
        let port = ScriptedPort(VecDeque::from([Chunk::Data(b"S 1 2 3 4\nOK BOOTSEL\n")]));
        let mut reader = BufReader::new(port);
        let mut partial = Vec::new();
        assert_eq!(read_line_from(&mut reader, &mut partial).unwrap(), Some("S 1 2 3 4".to_string()));
        assert_eq!(read_line_from(&mut reader, &mut partial).unwrap(), Some("OK BOOTSEL".to_string()));
    }

    #[test]
    fn read_line_errors_on_unexpected_close() {
        let port = ScriptedPort(VecDeque::from([Chunk::Data(b"")]));
        let mut reader = BufReader::new(port);
        let mut partial = Vec::new();
        let err = read_line_from(&mut reader, &mut partial).unwrap_err();
        assert!(err.to_string().contains("closed unexpectedly"), "unexpected error: {err}");
    }
}
