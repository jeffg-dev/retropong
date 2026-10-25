# `rcctl` -- RetroController host toolkit

Builds and flashes the RetroController paddle firmware, and talks to a
connected controller over its USB HID gamepad interface and its CDC-ACM
serial protocol. See `docs/SPEC-controller.md` §7 ("Flashing") and §8
("Host toolkit") for the normative spec this implements.

> **pid.codes test PID.** RetroController identifies itself as USB
> `1209:0001`, the [pid.codes](https://pid.codes/1209/0001/) *test* PID.
> That ID is shared by every hobbyist project using it for private testing
> and **must not** be used on anything sold, redistributed or manufactured.
> If you build a real batch of these, get your own VID/PID
> ([pid.codes/howto](https://pid.codes/howto/) or a purchased VID) and
> update `protocol::usb::{VID, PID}`.

## Install / build

From the repo root:

```sh
cargo build -p rcctl --release
# binary at target/release/rcctl
```

or just `cargo run -p rcctl -- <command>` during development.

## Commands

### `rcctl build`

Runs `cargo build --release --target thumbv8m.main-none-eabihf` from
*inside* the firmware directory (the firmware is its own Cargo workspace,
excluded from the root one -- see the comment in `firmware/Cargo.toml` for
why), so Cargo picks up `firmware/.cargo/config.toml` regardless of where
`rcctl` itself was invoked from, and prints the resulting ELF path. Warns and
prints the `rustup target add` command if the `thumbv8m.main-none-eabihf`
target isn't installed.

```
$ rcctl build
Built /path/to/retropong/firmware/target/thumbv8m.main-none-eabihf/release/retrocontroller-firmware
```

The firmware directory is auto-detected by walking up from the current
directory (and from `rcctl`'s own source tree) looking for
`firmware/Cargo.toml`; override with `--firmware-dir <path>` if needed.

### `rcctl uf2 <elf> [-o out.uf2] [--family FAMILY] [--abs-block]`

Converts a linked ELF32 firmware image into an RP2350-flashable `.uf2`,
entirely in Rust (no `elf2uf2-rs` -- the crates.io release only knows the
RP2040 family; see the notes below). Content is taken from each `PT_LOAD`
segment's file bytes, placed at its **load address** (`p_paddr`), not its
virtual address, so a `.data` section's initializer is captured at its
flash location even though it *runs* from RAM.

```
$ rcctl uf2 firmware/target/thumbv8m.main-none-eabihf/release/retrocontroller-firmware -o firmware.uf2
Wrote firmware.uf2 (36352 bytes, 71 blocks, family rp2350-arm-s)
```

`--family` selects the family ID stamped on every block: `absolute`, `data`,
`rp2350-arm-s` (default), `rp2350-arm-ns`. `rp2040` and `rp2350-riscv` are
deliberately not offered -- see `uf2::Family`'s doc comment for why -- so
`rcctl uf2-info` still names those two ids for diagnostic purposes if it
finds them in a foreign UF2 (see below), but they aren't `--family` values.
Passing `--family rp2350-arm-ns` prints a warning: on a board with no
partition table (this one) the bootrom silently drops every block of that
family, so the copy "succeeds" but nothing is actually written. `--abs-block`
prepends the picotool-compatible RP2350-E10 "absolute block" workaround,
which is only needed for A2-silicon boards booting a UF2 that has a
partition table -- this board doesn't have one, so the default (off) is
correct here; the flag exists for parity with `picotool uf2 convert
--abs-block`.

### `rcctl uf2-info <file>`

Prints a summary of an existing `.uf2`: block count, address range and
family/families seen.

```
$ rcctl uf2-info firmware.uf2
firmware.uf2: 36352 bytes, 71 block(s)
  71 data block(s)
  address range: 0x10000000..0x10004700 (18176 bytes)
  family: rp2350-arm-s (0xe48bff59) x71
```

### `rcctl flash [--elf PATH] [--no-build] [--no-bootsel] [--volume PATH] [--uf2-out PATH] [--abs-block] [--firmware-dir PATH]`

The full flashing flow:

1. Build the firmware (unless `--elf` or `--no-build` is given).
2. Convert it to UF2.
3. If `--volume` was given, skip steps 3-4 entirely (no `BOOTSEL` is sent,
   and no wait happens) -- go straight to writing the UF2 there. Otherwise,
   if a controller is present on serial and `--no-bootsel` wasn't given,
   send `BOOTSEL` so it reboots into the ROM bootloader without touching any
   buttons; if that isn't possible, print the manual instructions ("hold B,
   tap R, release B") and wait for it.
4. Wait for the `RP2350` bootloader volume (10s if we just rebooted it
   ourselves, 60s if waiting on the button combo) -- skipped when `--volume`
   was given (see step 3). Since `--volume` never sends `BOOTSEL`, point it
   at a volume the device is *already* sitting in (e.g. already in BOOTSEL,
   or `--uf2-out` plus a manual copy) -- pointing it at a mounted directory
   while the device is still running firmware fails with "does not look like
   an RP2350 bootloader volume (no INFO_UF2.TXT)".
5. Write `firmware.uf2` onto it with a plain `File::create` +
   `write_all` + `sync_all` (no extended attributes, which `cp`/Finder can
   choke on -- see notes below). I/O errors *after* every byte has been
   written are treated as success, since the device reboots about 500ms
   after the last block and can drop the connection mid-`sync`.
6. Wait up to 5s for the volume to disappear, then up to 5s for the
   controller's serial port to reappear (printing its `INFO` line). If
   neither happens, `rcctl flash` exits with an error (a rejected/partial
   UF2 download looks exactly like this) -- except when `--volume` pointed
   at a volume that did not look like an RP2350, which only warns.

```
$ rcctl flash
Firmware ELF: .../firmware/target/thumbv8m.main-none-eabihf/release/retrocontroller-firmware
Converted to UF2: 36352 bytes, 71 block(s)
Sent BOOTSEL to /dev/ttyACM0 (OK BOOTSEL); waiting for the bootloader volume...
Bootloader volume: /media/jeff/RP2350
Wrote /media/jeff/RP2350/firmware.uf2
Volume disappeared -- device is rebooting.
Controller back on /dev/ttyACM0.
OK INFO name=RetroController fw=0.1.0 proto=1 board=xiao-rp2350 uid=... axes=2 max=4095
```

### `rcctl devices`

Lists RetroController serial ports, HID devices (with product/serial
strings) and any `RP2350`/UF2 bootloader volumes currently visible,
printing the pid.codes warning first. Never fails just because nothing is
plugged in -- it prints "(none found)" for each category.

### `rcctl info`

Sends `INFO` over serial and prints the response line (name, firmware
version, protocol version, board, UID, axis count/range).

### `rcctl monitor [--raw]`

Turns on the serial `STREAM` (`STREAM ON`, 50 Hz `S l_raw r_raw l r btn seen`
lines) and redraws a two-bar live view in place with `\r` (no TUI library
needed). Each bar is followed by its player's button: `[X]` pressed, `[ ]`
released (pressed at least once since boot, so known to be wired), `[?]` not
pressed yet -- possibly not wired, which is fine. Firmware without button
support shows no button cells. Ctrl-C stops it and sends `STREAM OFF` on the way out. `--raw`
prints the protocol lines verbatim instead of bars.

### `rcctl test`

Reads HID reports directly -- i.e. exactly what a game sees, bypassing the
serial protocol entirely -- and shows the same two-bar live view, draining
the HID queue each tick to always show the latest sample.

### `rcctl calibrate`

Interactive wizard: `CAL START` and live streaming show two physical input
bars (A0/A1), their raw readings, and expanding observed minimum/maximum/span.
Sweep both knobs slowly to each physical stop, in both directions, a few times,
then press Enter. The ADC percentage describes observed raw span, not how
completely you swept the mechanical range. Assignment swapping does not change
these physical labels. Terminal feedback redraws at up to 10 Hz; redirected
output and terminals narrower than 78 columns use plain snapshots once a second.
The wizard stops streaming/learning, shows the learned ranges, then asks y/n
before `CAL SAVE`. Ctrl-C, EOF, serial errors, and missing samples cancel without
saving and attempt to stop streaming and learning. Save failures are reported.

### `rcctl cal show|reset|set <lmin> <lmax> <rmin> <rmax>|invert <L|R> <0|1>`

Direct calibration commands, sent as-is over serial (`CAL SHOW`,
`CAL RESET`, `CAL SET ...`, `INVERT ...`); `show`/`set`/`reset`/`invert`
all also pretty-print the resulting `CAL` line.

### `rcctl cal swap [on|off]`

Run `rcctl cal swap` (or `make swap`) to read and flip the current assignment.
To set it explicitly: `rcctl cal swap on`
exchanges them; `rcctl cal swap off` restores the wiring order. `1`/`0` are
also accepted. The assignment is saved with calibration and survives reboot
and firmware updates that preserve the calibration sector. `cal show` displays
`swapped: true/false`. Calibration ranges and `cal invert L|R` always refer to
the physical ADC inputs; assignment happens after calibration/inversion and
affects HID, `VAL`, and calibrated stream values. Raw values retain wiring order.
Recalibration and saving retain assignment; `cal reset` clears it along with
ranges and inversion. A failed flash write returns an error and keeps the
previous live assignment (the saved record may need to be written again).
Older firmware rejects `CAL SWAP`; update firmware before using this command.

### `rcctl bootsel` / `rcctl reboot`

Send `BOOTSEL` (reboot into the USB/UF2 bootloader) or `REBOOT` (normal
reset) and print the response.

### `rcctl udev [--install]`

Prints the Linux udev rules this toolkit (and any HID/joystick library)
needs. With `--install`, writes them to
`/etc/udev/rules.d/70-retrocontroller.rules` (must sort before
`73-seat-late.rules` for `TAG+="uaccess"` to actually apply -- hence `70-`,
not `99-`) and runs `udevadm control --reload-rules && udevadm trigger`.
On anything other than Linux this just prints a note and exits 0.

## Linux permissions

There's no default udev rule for `hidraw` devices, so `/dev/hidrawN` is
`root:root 0600` by default -- hidapi needs a rule. The joystick/evdev node
is usually already accessible via systemd's `ID_INPUT_JOYSTICK` +
`uaccess` rule, and `/dev/ttyACM0` is usually `root:dialout`, but the
toolkit's rules cover all three (plus tagging the CDC-ACM interface so
ModemManager leaves it alone) for robustness across distros:

```sh
sudo rcctl udev --install
# then replug the controller
```

Without `uaccess`/ACL support at all, add yourself to the relevant groups
instead: `sudo usermod -aG input,dialout $USER` and log back in. (The rules
use `GROUP="input"`, not the Debian/Ubuntu-specific `plugdev`, precisely so
this fallback works on Fedora/Arch/openSUSE too -- `input` is created by
systemd's own default rules on every systemd distro.)

Every command that talks to hardware fails with a clear message (listing
what it looked for, e.g. the USB VID:PID) when nothing is connected at all --
enumeration itself never opens a device node, so a udev permissions problem
can't be why nothing was *found*. The `rcctl udev --install` hint is only
appended when a device *was* found but opening it failed with "permission
denied" (the actual symptom of a missing udev rule); a plain not-found error
doesn't mention it.

## UF2 notes

* 512-byte blocks, 256-byte payload (`payload_size` must be exactly 256 for
  the RP2350 bootrom to accept a block at all).
* Flags `0x00002000` (`UF2_FLAG_FAMILY_ID_PRESENT`) on every data block; no
  `NOT_MAIN_FLASH` flag, since we only ever produce flash images.
* Family `0xe48bff59` (`rp2350-arm-s`) by default -- one of the three
  families (along with `absolute` and `rp2350-riscv`) the bootrom accepts on
  a board with no partition table. `rp2350-riscv` isn't offered by
  `--family` since this tool only ever converts Arm (`EM_ARM`) ELF images;
  `rp2350-arm-ns` and `data` need a partition table to be routed anywhere and
  are silently ignored (not rejected -- the copy still "succeeds") on a bare
  board like this one, which is why `--family rp2350-arm-ns` prints a
  warning (`--family data` currently does not).
* `target_addr` comes from each `PT_LOAD` segment's **load address**
  (`p_paddr`), not its virtual address (`p_vaddr`) -- this is what makes
  `.data`'s initializer land in flash instead of being (incorrectly) aimed
  at its RAM address. Content must fall inside `0x10000000..0x12000000`;
  anything else (e.g. a link that leaves `.data`'s LMA in SRAM) is a hard
  error, since this tool only produces flash-targeting UF2s.
* No 4K-sector zero-padding and no partition table are emitted -- neither
  is required on RP2350 (unlike RP2040, the bootrom does not use
  `block_no` for erase-sector math, and there's no partition table on this
  board).
* `--abs-block` adds the picotool-style RP2350-E10 workaround block
  (target `0x10ffff00` end-of-flash address, `0xef`-filled payload,
  tagged so the bootrom ignores it for transfer-completion purposes) for
  parity with `picotool uf2 convert --abs-block`. It's *not* needed here:
  that erratum only bites A2-silicon bootroms when a partition table is
  present, and this board has neither confirmed A2 silicon nor a partition
  table.
* Do **not** use the crates.io `elf2uf2-rs` (2.2.0) for RP2350 -- it
  hardcodes the RP2040 family ID and RP2040 address ranges; a UF2 it
  produces for this board is silently rejected by the bootrom (RP2040 isn't
  in the accepted-family set without a partition table). `rcctl uf2-info`
  recognizes the `rp2040` and `rp2350-riscv` family ids (even though they
  aren't `--family` values) specifically so it can name this mistake instead
  of just printing an unhelpful "unknown family".
* The bootloader volume is detected purely by the presence of
  `INFO_UF2.TXT` (its "RP2350"/"Board-ID" text is a hint, not proof, since
  it's OTP white-labelable) under `/media/$USER/*`, `/run/media/$USER/*`, or
  any FAT (`vfat`/`msdos`/`fat`) mount point listed in `/proc/mounts` (whose
  octal escapes, e.g. `\040` for a space, are decoded); on macOS, only
  `msdos`/`exfat` entries under `/Volumes` are checked (deliberately, so a
  hung network share can't stall detection).
