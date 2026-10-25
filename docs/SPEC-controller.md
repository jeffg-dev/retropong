# RetroController — controller specification

Two rotary potentiometers on a Seeed Studio **XIAO RP2350**, presented over
USB-C as a standard **HID game pad** (two absolute axes) plus a **CDC-ACM
serial port** for the host toolkit. This document specifies the hardware
interface, firmware behaviour, USB descriptors and the serial protocol. The
shared constants live in the `retrocontroller-protocol` crate, which is the
normative source for numbers.

## 1. Hardware

| Part | Detail |
|------|--------|
| MCU board | Seeed Studio XIAO RP2350 (RP2350A, Arm Cortex-M33 ×2 @150 MHz, 520 KB SRAM, 2 MB QSPI flash, USB-C) |
| Pots (×2) | Bourns/Vishay-style **95A1A-B28-B15L**: 10 kΩ, linear taper, conductive plastic, solder lugs, ~300° travel |
| Wiring | Pot 1 (left player): lug 1 → GND, lug 2 (wiper) → **A0**, lug 3 → **3V3**. Pot 2 (right player): lug 1 → GND, lug 2 → **A1**, lug 3 → 3V3. |
| Optional | 100 nF ceramic from each wiper to GND, close to the XIAO, for extra noise immunity. |
| Optional | Two momentary NO buttons, each from its pad to GND: left beside pot 1 → **D2**, right beside pot 2 → **D3**. |

Board pin mapping used by the firmware (see `hardware/README.md` for the full
pinout and photos):

| Function | XIAO pad | RP2350 GPIO | ADC channel |
|----------|----------|-------------|-------------|
| Left knob | A0 / D0 | GPIO26 | ADC0 |
| Right knob | A1 / D1 | GPIO27 | ADC1 |
| Left button (optional) | A2 / D2 | GPIO28 (pull-up, active-low) | — |
| Right button (optional) | D3 | GPIO5 (pull-up, active-low) | — |
| User LED (heartbeat) | on-board | GPIO25 (active-low) | — |
| RGB LED (unused) | on-board WS2812 | data GPIO22, power-enable GPIO23 | — |

10 kΩ across 3.3 V draws 0.33 mA per pot; the ADC sees a source impedance of at
most 2.5 kΩ, well within the RP2350 ADC's recommendation.

**Clockwise = larger value** with the wiring above when looking at the shaft
(lug 1 CCW end). If a knob turns out backwards, flip it in firmware with
`INVERT L|R 1` (persisted), or swap lugs 1 and 3. The firmware's default
direction convention: knob fully counter-clockwise → axis 0 → paddle at the top
of the screen.

## 2. USB device

Composite device, interface association descriptors enabled
(`bDeviceClass 0xEF / 0x02 / 0x01`).

| Field | Value |
|-------|-------|
| VID:PID | `0x1209:0x0001` (pid.codes test PID: fine for a private build, must not be used on anything sold or redistributed; `rcctl devices` and `tools/rcctl/README.md` say so) |
| Manufacturer | `RetroController` |
| Product | `RetroController Paddles` |
| Serial number | RP2350 OTP chip id (64-bit), 16 uppercase hex digits (same value as INFO's uid=) |
| Interface 0 | HID, boot-protocol none, IN endpoint 8 bytes, **1 ms** poll |
| Interface 1+2 | CDC-ACM (comm + data), 64-byte bulk endpoints |
| Power | bus powered, 100 mA |

### 2.1 HID report descriptor

```
Usage Page (Generic Desktop)      05 01
Usage (Game Pad)                  09 05
Collection (Application)          A1 01
  Usage (Pointer)                 09 01
  Collection (Physical)           A1 00
    Usage (X)                     09 30
    Usage (Y)                     09 31
    Logical Minimum (0)           15 00
    Logical Maximum (4095)        26 FF 0F
    Report Size (16)              75 10
    Report Count (2)              95 02
    Input (Data,Var,Abs)          81 02
  End Collection                  C0
  Usage Page (Button)             05 09
  Usage Minimum (1)               19 01
  Usage Maximum (8)               29 08
  Logical Minimum (0)             15 00
  Logical Maximum (1)             25 01
  Report Size (1)                 75 01
  Report Count (8)                95 08
  Input (Data,Var,Abs)            81 02
End Collection                    C0
```

(the firmware's generated descriptor encodes X and Y as two `Report Count (1)`
items rather than the one combined `Report Count (2)` item shown above; same
report layout)

Input report (no report ID), 5 bytes little-endian: `x:u16 y:u16 buttons:u8`.
`x` = left knob, `y` = right knob, `0..=4095`. Button bit 0 (HID Button 1) is
the left player's button and bit 1 (Button 2) the right's, 1 = pressed; bits
2–7 are always 0. The device sends a report every 1 ms when a value changed
(a button edge is sent on the next 1 ms tick) and at least every 50 ms
regardless (keep-alive so hosts see a live device).

Buttons are optional. They switch to GND against the internal pull-up, so a
pad with no button wired reads exactly like an idle button and its bit just
stays 0. The firmware samples both at 1 kHz and accepts a level after 5
agreeing samples (5 ms debounce). A pad must read released for that long
before any press counts, so a pad held low from boot (button held while
plugging in, or tied to GND) reports released. A button is *seen* once it has
been pressed since boot; that is the only way to tell a wired button from an
absent one. Buttons follow the wheel assignment: with `swap` set, the D3
button reports as the left player's, like the A1 knob.

## 3. Sampling and filtering

* ADC: 12-bit, 3.3 V reference (ADC_AVDD), round-robin on channels 0 and 1.
* Each axis is sampled at ~8 kHz and **oversampled 16×** (sum of 16 samples
  → 16-bit raw value `0..=65535`), giving a new raw value every 2 ms per axis
  and ~2 extra bits of resolution.
* An exponential moving average (`y += (x − y) / 4`) removes residual noise
  without noticeable lag (< 6 ms).
* Calibration (`retrocontroller_protocol::cal::AxisCal::apply`) maps raw
  `[min, max]` → `0..=4095` with optional inversion. Default calibration trims
  1000 raw counts (~1.5 %) from each rail so the axis reaches its limits even
  if the pot does not quite reach 0 Ω / 10 kΩ.
* `CAL STOP` adopts `[learned_min + 200, learned_max − 200]` (raw counts) from
  the just-finished `CAL START` sweep, not the raw learned extremes, so the
  end stops stay reachable; the trimmed span must still be ≥ 1024 or the
  command answers `ERR calibration`.
* Output hysteresis: the reported HID value changes only when the new value
  differs from the last reported one by ≥ 2 counts, which kills 1-LSB jitter
  while keeping 1-count moves possible when the knob is actually turning
  (implementation: report when |new − last| ≥ 2, or when the calibrated value
  has held exactly one reading continuously for 50 ms and that reading
  differs from the last reported one; a 1-count wobble restarts the 50 ms
  timer).

## 4. Calibration persistence

The 16-byte record `retrocontroller_protocol::cal::Calibration::to_bytes()` is
stored in the **second-to-last 4 KB sector of flash** (flash offset
`FLASH_SIZE − 8192` = `0x1FE000`, address `0x101FE000`), well clear of the
program image. The *last* sector is deliberately left alone: UF2 files produced
by picotool/pico-sdk with `--abs-block` carry an RP2350-E10 workaround block
aimed at `0x10FFFF00`, which on a 2 MB part wraps onto the last physical
sector and would erase anything stored there on A2 silicon. On boot the
firmware reads the record; an invalid record (erased flash, bad CRC, insane
span) means defaults. `CAL SAVE`, `CAL RESET`, `CAL SWAP` and `INVERT` erase and rewrite
the sector.
Flash programming runs with interrupts masked for a few ms; the HID stream
may skip a report or two, so it is only ever done on an explicit command.

The existing `RCC1` record stores physical-axis inversion in flag bits 0/1
and logical wheel assignment in bit 2 (`swap`). Legacy records have bit 2
clear and decode in wiring order. `CAL SWAP ON|OFF` (also `1|0`) persists the
candidate record and commits the live assignment only after verified flash
success; failure returns `ERR flash` and retains the previous live assignment.
A failed erase/write can invalidate flash, so the saved flag is cleared.
The assignment swaps the two calibrated/inverted physical values before HID,
`VAL`, and stream output. Raw stream/`RAW` values and calibration/inversion
parameters always follow physical ADC wiring. `CAL START`/`STOP`, `CAL SET`,
`INVERT`, and `CAL SAVE` retain assignment; `CAL RESET` clears it.
`OK CAL` appends a `swap` (0/1) field after `saved`; hosts accept older responses
without the field as unswapped. Existing hosts can ignore the appended field.

## 5. Serial protocol

Text lines over CDC-ACM, see the table in `protocol/src/lib.rs` (module
`serial`). Highlights:

* Host must assert **DTR** (`serialport`'s `dtr_on_open(true)` /
  `write_data_terminal_ready(true)`; macOS does not do it automatically).
  The firmware processes commands whenever the interface is configured but
  only *sends* (responses and stream lines) while DTR is asserted, so an
  unattended port can never stall the device. Baud rate is ignored.
* Every command yields exactly one `OK …` or `ERR …` line. `STREAM ON` adds
  `S l_raw r_raw l r btn seen` lines at 50 Hz until `STREAM OFF` or the port
  closes. `btn`/`seen` are the button bitmasks (pressed now / pressed since
  boot); older firmware omits them and hosts accept lines without them.
* `BTN` → `OK BTN <btn> <seen>`; older firmware answers `ERR unknown command`.
* `INFO` → `OK INFO name=RetroController fw=<semver> proto=1 board=xiao-rp2350 uid=<16 hex> axes=2 max=4095`
* `BOOTSEL` → replies `OK BOOTSEL`, flushes, then reboots into the ROM USB
  bootloader (`RP2350` mass-storage volume) so the toolkit can flash without
  touching the button.

## 6. LED behaviour

| State | User LED |
|-------|----------|
| USB not configured | fast blink 5 Hz |
| Configured, idle | slow blink 1 Hz |
| Knob moving | solid on for 100 ms after each HID report that carried a changed value (keep-alive reports do not count); learning (10 Hz) takes priority |
| Calibration learning (`CAL START`) | 10 Hz blink |

## 7. Flashing

1. Hold the **B** (BOOT) button on the XIAO while plugging in USB (or, while
   powered, hold **B**, tap **R** (RESET), release **B**), **or** run
   `rcctl flash`, which sends `BOOTSEL` to a connected controller so no
   buttons are needed.
2. A volume named **`RP2350`** appears (`/Volumes/RP2350` on macOS,
   `/media/$USER/RP2350` or `/run/media/$USER/RP2350` on Linux). It contains
   `INFO_UF2.TXT` and `INDEX.HTM`.
3. Copy the `.uf2` file to it. The board reboots into the new firmware and the
   volume disappears.

UF2 details (`rcctl uf2`): 512-byte blocks, 256-byte payload, flags
`0x00002000` (family ID present), family `0xE48BFF59` (RP2350 Arm Secure),
`num_blocks` = number of 256-byte pages, target addresses taken from the ELF
program headers' physical (load) addresses (`p_paddr`) in the
`0x10000000..0x12000000` flash window (so `.data` is captured at its flash
LMA, not its RAM VMA); the image contains the `IMAGE_DEF` block in its first
4 KB as required by the RP2350 boot ROM, which is emitted by embassy-rp. No
partition table and no RP2350-E10 "absolute" block are emitted by default
(`--abs-block` adds the picotool-compatible one for A2 silicon with a
partition table, which this board does not have). Note the crates.io release
of `elf2uf2-rs` (2.2.0) only knows the RP2040 family and must not be used.

The bootloader volume is detected by the presence of `INFO_UF2.TXT` (whose
text contains `RP2350`, a hint only — a volume without it is still used,
with a warning), with the label `RP2350` as a hint; candidates are
`/media/$USER/*` and `/run/media/$USER/*` plus any FAT (`vfat`/`msdos`/`fat`)
mount point in `/proc/mounts` on Linux, and only `msdos`/`exfat` entries
under `/Volumes` on macOS (so a hung network share can't stall detection).
The file is written with a plain
`File::create` + `write_all` + `sync_all` (no extended attributes, which
Finder/`cp` may fail on) and I/O errors *after* the last byte are ignored
because the device reboots about 500 ms after the final block.

## 8. Host toolkit (`rcctl`)

| Command | Purpose |
|---------|---------|
| `rcctl build [--firmware-dir PATH]` | `cargo build` the firmware for `thumbv8m.main-none-eabihf` (always release; `--release` is accepted for parity but has no effect) |
| `rcctl uf2 <elf> [-o out.uf2] [--family F] [--abs-block]` | Convert an ELF to a RP2350 UF2 |
| `rcctl uf2-info <file>` | Print a summary of an existing `.uf2` (block count, address range, family) |
| `rcctl flash [--elf PATH] [--no-build] [--no-bootsel] [--volume PATH] [--uf2-out PATH] [--abs-block] [--firmware-dir PATH]` | Build (unless told not to), convert, ask the device to reboot to BOOTSEL, wait for the volume, copy, wait for the device to come back |
| `rcctl devices` | List RetroController serial ports and HID devices, and any `RP2350` bootloader volumes |
| `rcctl info` | Print the `INFO` line |
| `rcctl monitor [--raw]` | Live bar graph of both knobs and buttons (serial `STREAM`) |
| `rcctl test` | Read HID reports directly (what games see), including buttons |
| `rcctl calibrate` | Interactive: `CAL START`, ask user to sweep both knobs, `CAL STOP`, show, confirm, `CAL SAVE` |
| `rcctl cal show/reset/set/invert` | Direct calibration commands |
| `rcctl bootsel` / `rcctl reboot` | Reboot commands |
| `rcctl udev [--install]` | Print (or install to `/etc/udev/rules.d/70-retrocontroller.rules` — it must sort before `73-seat-late.rules` for `uaccess` to work) Linux udev rules for hidraw, evdev, the serial port and the bootloader |
