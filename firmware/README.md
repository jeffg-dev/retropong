# RetroController firmware

Firmware for the Seeed Studio **XIAO RP2350**: reads two potentiometers and
presents them to the host as a USB composite device — a HID game pad (two
absolute axes) plus a CDC-ACM serial port for calibration and diagnostics.
This is an implementation of `/docs/SPEC-controller.md`; read that first for
the hardware wiring, USB descriptors, sampling/filtering behaviour, the
serial command protocol, LED patterns and the flashing procedure.

This directory is deliberately **two separate Cargo crates**:

| Crate | Path | What it is |
|---|---|---|
| `retrocontroller-firmware-logic` | `logic/` | `#![no_std]`, pure Rust, no embassy, no hardware access. Oversampling/filtering, output hysteresis, calibration learning, the serial command handler, line framing, and LED pattern selection. Depends only on `retrocontroller-protocol`. Host-testable: `cargo test -p retrocontroller-firmware-logic` from the repo root (it's a root-workspace member). |
| `retrocontroller-firmware` | `firmware/` (this directory) | The embassy-rp application binary: ADC sampling, composite USB (HID + CDC-ACM), flash-backed calibration storage, LED GPIO, and the BOOTSEL/reboot commands. It's its own Cargo workspace (see below), not a root-workspace member. |

Splitting it this way means almost all of the actual *behaviour* — the parts
worth writing unit tests for — runs and is tested on the host in
milliseconds, with no hardware, probe, or QEMU involved. `main.rs` in this
crate is comparatively thin: it wires real peripherals to the logic crate's
types and otherwise just moves bytes and does I/O.

## Why `firmware/` is its own Cargo workspace

`firmware/Cargo.toml` has its own `[workspace]` table so the root workspace's
`exclude = ["firmware"]` keeps it out of `cargo build --workspace` at the
repo root (different target, `no_std`, custom linker script — it doesn't
belong in the same dependency resolution as the host tools). `logic/` is
nested inside this directory but is *not* a member of this inner workspace —
it's excluded here too (see the comment in `firmware/Cargo.toml`) so that
Cargo's workspace search continues up to the *root* `Cargo.toml`, which is
where it's actually listed as a member. Both `firmware/Cargo.toml` and
`firmware/logic/Cargo.toml` use plain literal `version`/`edition`/etc.
instead of `field.workspace = true` inheritance, because `logic/` is built
from two different "current workspace" contexts (root, and `firmware/`) and
inheritance resolves against whichever one issued the build.

## Building

```sh
# The embassy application (from this directory):
cd firmware
cargo build --release

# The logic crate's unit tests (from the repo root):
cd ..
cargo test -p retrocontroller-firmware-logic
```

Toolchain: stable Rust with the `thumbv8m.main-none-eabihf` target (see the
repo's `rust-toolchain.toml`) and, to inspect the resulting ELF without ARM
GNU binutils installed, the `llvm-tools` rustup component (`llvm-size`,
`llvm-readobj`, `llvm-objdump`, `llvm-nm`, under
`$(rustc --print sysroot)/lib/rustlib/<host-triple>/bin/`).

## Flashing

1. Hold the **B** (BOOT) button while plugging in USB (or hold **B** and tap
   **R**/RESET if already plugged in), or send the serial `BOOTSEL` command
   to a running controller (`rcctl flash` does this for you).
2. A mass-storage volume named **`RP2350`** appears.
3. Copy the built `.uf2` onto it (`rcctl uf2`/`rcctl flash` converts the ELF;
   `picotool load -u -v -x -t elf <path>` also works directly against the
   ELF via the `.cargo/config.toml` `runner`). The board reboots into the new
   firmware and the volume disappears.

## Pin map (Seeed XIAO RP2350 / RP2350A)

| Function | XIAO pad | GPIO | Notes |
|---|---|---|---|
| Left knob (wiper) | A0 / D0 | GPIO26 (ADC0) | `Pull::None`; ends to 3V3/GND |
| Right knob (wiper) | A1 / D1 | GPIO27 (ADC1) | `Pull::None`; ends to 3V3/GND |
| User LED | on-board | GPIO25 | **Active-low** (drive low to light) |

(RGB LED, battery-sense ADC, and the other castellated pads exist on the
board but are unused by this firmware.)

## LED meanings (SPEC-controller.md §6)

| Pattern | Meaning |
|---|---|
| Fast blink, 5 Hz | USB not configured yet |
| Slow blink, 1 Hz | Configured, idle (no recent knob motion) |
| Solid on | A HID report with a changed value was sent within the last 100 ms ("knob moving") |
| Blink, 10 Hz | Calibration learning in progress (`CAL START`..`CAL STOP`) |

Priority when several could apply: not-configured > learning > moving > idle
(see `retrocontroller_firmware_logic::led::select_led_state`).

## Serial protocol

CDC-ACM, 115200-8-N-1 (baud is ignored; the host must assert DTR). One
`OK ...`/`ERR ...` line per command; `STREAM ON` additionally starts
unsolicited `S l_raw r_raw l r btn seen` lines at `retrocontroller_protocol::serial::STREAM_HZ`
(50 Hz) until `STREAM OFF` or disconnect. See the table in
`retrocontroller_protocol::serial` (also reproduced in
`/docs/SPEC-controller.md` §5) for every command and its exact response, and
`logic/src/device.rs` for the implementation each response line comes from.

Commands are processed whenever the USB interface is configured, regardless
of DTR; only *sending* a response or a stream line is gated on DTR, and each
CDC write is bounded by a 100ms timeout (`main.rs`'s `CDC_WRITE_TIMEOUT`) so a
host that stops draining the IN endpoint can never stall the firmware. On the
DTR high-to-low *transition* (not on every tick while DTR happens to be low —
that would corrupt a command whose bytes straddle a tick on a host that never
raises DTR at all), and on any other loss of the connection, the firmware
calls `retrocontroller_firmware_logic::device::Device::on_disconnect`: it
stops an unattended `STREAM`, leaves any in-progress `CAL START` learning
session (discarding whatever min/max the learners have accumulated so far),
and resets the line assembler/pending response, so nothing from a vanished
session (killed process, closed terminal, unplug, or a stray `CAL START`
never followed by `CAL STOP`) leaks into whatever host connects next.

## Implementation details

- **HID report descriptor byte encoding.** `usbd_hid`'s `gen_hid_descriptor`
  macro emits X and Y as two separate 16-bit `Input` items (each its own
  `Report Count 1`) rather than one combined `Report Count 2` item as
  SPEC §2.1's illustrative descriptor bytes show. Both encode the identical
  report shape (`x:u16 y:u16 buttons:u8`, axes `0..=4095`); this is a
  byte-level descriptor difference only, not a functional one (verified by
  generating and inspecting the actual descriptor bytes).
- **USB serial number source.** SPEC §2 calls
  for "the RP2350 unique flash ID". `embassy-rp` 0.10.0's
  `Flash::blocking_unique_id`/`blocking_jedec_id` (the QSPI flash chip's
  unique-id command) turned out to only be implemented under that crate's
  `rp2040` feature — there's no such method for `rp235xa` (confirmed by
  trying to compile against it). RP2350 has its own OTP-programmed 64-bit
  chip id instead (`embassy_rp::otp::get_chipid()`, rows 0-3), which this
  firmware uses in its place: still a stable, unique-per-board 64-bit value,
  formatted the same way as 16 uppercase hex digits for both `INFO`'s `uid=`
  field and the USB serial number string.
- **Framing-level "line too long" error text.** `CAL SET`/etc. syntax errors
  use `retrocontroller_protocol::serial::ParseError`'s messages verbatim, but
  a line that exceeds `serial::MAX_LINE` bytes before any newline is a
  framing error the shared protocol crate's `ParseError` enum doesn't cover
  (it only parses already-delimited lines). The logic crate defines its own
  fixed text for this one case: `retrocontroller_firmware_logic::line::ERR_LINE_TOO_LONG`
  = `"ERR line too long"`.
- **Calibration rejection error text.** Likewise, `retrocontroller_protocol`
  has no enum variant for "a learned or explicitly-set calibration span is
  too small to be usable" (that's a runtime/semantic rejection, not a syntax
  error). `retrocontroller_firmware_logic::device::ERR_CALIBRATION` =
  `"ERR calibration"` covers `CAL STOP` (nothing learned, or the
  margin-adjusted learned span is insane) and `CAL SET` (an explicitly
  insane range) alike. Two more runtime-rejection texts of the same kind:
  `retrocontroller_firmware_logic::device::ERR_NOT_LEARNING` = `"ERR not
  learning"` (`CAL STOP` without a preceding `CAL START`), and
  `retrocontroller_firmware_logic::device::ERR_FLASH` = `"ERR flash"`
  (`CAL SAVE`/`CAL RESET`/`INVERT` when the deferred flash erase, write or
  read-back fails). All five `ERR ...` texts specific to calibration and
  framing are listed alongside their commands in the response table in
  `retrocontroller_protocol::serial` (SPEC-controller.md §5).
- **EMA fixed-point dead band.** SPEC §3 specifies `y += (x - y) / 4` exactly
  as written; implemented with plain (truncating) integer division, this has
  an inherent dead band of up to 3 raw counts (out of 65535) once the signal
  settles, rather than converging to the exact target. This is standard for
  a fixed-point EMA with no fractional accumulator and is far below the
  resolution that survives calibration down to the 12-bit output axis; see
  the doc comment on `retrocontroller_firmware_logic::filter::Ema`.
