# Development guide

Keep the root README concise and human-facing. Put implementation notes here or
in the linked specifications rather than expanding the product overview.

## Repository map

- `game/superpong-core/`: dependency-free deterministic simulation of all four
  C-140 games. Owns rules, physics, scoring, sounds-as-events and cell raster.
- `game/superpong/`: native Macroquad frontend (`retropong`), HID/gamepad input,
  synthesized audio, palette, CRT shader, settings, menu and frame scheduler.
- `game/superpong-web/`: static HTML/JavaScript host and Rust WASM frontend. Reuses
  the native menu, font, settings, canvas, palette and scheduler via path modules.
- `protocol/`: shared HID layout, USB identity, serial commands and flash record.
- `firmware/logic/`: host-testable `no_std` filtering/calibration/serial behavior.
- `firmware/`: Embassy RP2350 hardware integration, separate Cargo workspace.
- `tools/rcctl/`: firmware build/flash, UF2 conversion, monitoring and calibration.
- `tools/superpong-mirror/`: independent Python simulation and differential fixtures.

## Build and verification

The root Makefile is the human-facing entry point: `make run` builds/launches
the native game, `make calibrate` runs rcctl's interactive learn-and-save flow,
and `make flash` installs the firmware target before building/flashing. Keep
flashing separate from calibration so recalibration does not reflash a device.
`make swap` reads the current wheel assignment, flips it, and persists it. Bare `make` lists commands.

```sh
rustup target add thumbv8m.main-none-eabihf wasm32-unknown-unknown
cargo test --workspace
cargo build -p superpong --release
(cd firmware && cargo build --release)
sh game/superpong-web/build.sh
node --test game/superpong-web/tests/*.test.mjs
python3 -m http.server 8080 --bind 127.0.0.1 --directory game/superpong-web/public
```

Build firmware from its directory so its `.cargo/config.toml` linker and target
options apply.
The web build exports native WAV sounds, copies the CRT shader and compiles WASM;
its generated assets are ignored by Git. Deploy the contents of `public/` after
building to static HTTPS hosting. Localhost is also a secure context for input.

On Linux, native build dependencies include `pkg-config`, `libudev-dev`,
`libasound2-dev`, `libx11-dev`, `libxi-dev`, `libgl1-mesa-dev`, and
`libxkbcommon-dev`. Runtime uses X11/GL, ALSA and udev. macOS needs Xcode command
line tools. Windows has not been verified.

For Linux HID access, build rcctl, run `sudo target/debug/rcctl udev --install`,
then unplug/replug. The HID backend is workspace-shared: do not enable competing
hidapi Linux backends. macOS uses shared device opens.

## Game behavior and rendering

The core advances at its phase-specific frame rate (59.94/60.19 Hz), on the
native 187-clock by 240-line display window. Physics is integer and deterministic.
Keep gameplay rules in the core; frontends consume its raster and sound events.
Scores, hidden-ball serves, eight-segment paddles and the power-on light show
come from the measured C-140 behavior. Tests include independent mirror fixtures.

Colours reproduce the packaging art, with a black background and OKLab gradients:
yellow/green/blue/lilac/pink foreground, blue/violet borders. Mono uses the
console's luma levels. Four subcolumns per clock provide the carrier texture.
The CRT shader applies composite colour blur, scanlines, phosphor mask, glow and
curvature; pixel mode bypasses it. Preserve 4:3 output and device-pixel sizing.

Audio is synthesized from the chip line counter and modeled speaker response,
not sampled from a recording. Playback is a single channel; newer events replace
older ones. Volume gain is the square of level/10. Export WAVs with
`retropong --export-sounds <dir>`.

The menu and resume countdown are drawn on the same raster and use the same
palette and shader. Enter starts/restarts or activates a menu item, Escape toggles
the menu, and arrows navigate. Paddle keyboard controls are intentionally absent.
Startup hints disappear after the first start. Countdown keeps the simulation
frozen but previews live knob positions. Never let the Enter that resumes also
restart the match. The controller's buttons are plain gamepad buttons (HID
Buttons 1/2: South/East); the games give them menu meaning. South opens the
menu/activates (like Enter in the menu, never a Start); East steps down and
wraps. Both games map them through `Menu::button_command`.

The web connection screen is static: only a large native RETROPONG banner and
CONNECT CONTROLLER / & TURN KNOB, with no scores, paddles or animated background.
The connection screen also has a prominent GitHub source link, hidden during play.
Do not add other page copy, DOM game controls, fullscreen requests or keyboard locks.
The user manages browser fullscreen. Gamepad input is automatic when exposed;
clicking the canvas offers WebHID when needed. Browser settings are local to the
site, validated on load, and storage failure must not stop play.

Wheel assignment belongs to the firmware, not the game menu. Per-player
direction preferences belong to the game. With the documented wiring, default
is CW up; disabling game inversion makes CCW up. Firmware inversion is an additional physical
wiring correction, so reversed wiring/firmware inversion changes this convention.

## Controller and persistence

The XIAO samples two 10 kΩ linear pots with oversampling/filtering and sends a
5-byte HID report (X and Y little-endian u16, then buttons), axes 0..4095, at
1 kHz. CDC serial provides diagnostics and calibration. The two momentary
buttons (D2/D3, active-low, internal pull-up) are optional: unwired pads read
released, and a button counts as present only once pressed. They are debounced
in firmware, set HID bits 0/1 in logical order (following wheel swap), and
appear in `BTN` and appended stream fields that older hosts ignore.

Calibration remains attached to physical A0/A1 inputs. Apply each physical
axis's range/inversion before routing the calibrated values to logical left/right.
`rcctl cal swap` toggles and persists that routing; optional `on|off` sets it explicitly. Recalibration must preserve routing;
reset returns to defaults. Preserve compatibility with existing flash records and
serial responses. Report persistent-write success only after flash verification;
on swap-write failure leave the live wheel assignment unchanged.

The VID:PID `1209:0001` is the pid.codes private test pair, not a production USB
allocation. Firmware flash/calibration commands affect attached hardware; building
and host tests do not exercise real flash writes. Distinguish build/test results
from hardware validation in reports.

## Visual QA

```sh
target/release/retropong --game 3 --auto-start --demo --frames 3000 --screenshot shot.png
target/release/retropong --game 3 --auto-start --demo --display crt --frames 900 --screenshot crt.png
```

On Linux without a display, prefix with `LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -a`.
Headless runs advance fixed time for deterministic captures. Use `--menu-at`,
`--menu-item`, `--resume-at` for menu/countdown captures. The menu golden raster
is regenerated with `UPDATE_GOLDEN=1 cargo test -p superpong`; inspect the diff.

## Detailed references

- [Architecture](docs/ARCHITECTURE.md)
- [Controller specification](docs/SPEC-controller.md)
- [Game specification](docs/SPEC-superpong.md) and [data](docs/superpong-data/README.md)
- [Hardware wiring](hardware/README.md)
- [Firmware](firmware/README.md) and [rcctl](tools/rcctl/README.md)
- [Native frontend](game/superpong/README.md) and [web frontend](game/superpong-web/README.md)
