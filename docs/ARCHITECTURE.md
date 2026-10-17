# Architecture

```
retropong/
├── protocol/            retrocontroller-protocol  – no_std crate shared by all three programs:
│                        USB IDs, HID report layout, serial text protocol, calibration math
├── firmware/            XIAO RP2350 firmware (Rust, embassy). Separate Cargo project
│                        because it targets thumbv8m.main-none-eabihf with its own linker script.
├── tools/rcctl/         Host toolkit CLI: build firmware, ELF→UF2, flash, monitor, calibrate, test
├── game/superpong-core/ Deterministic headless simulation and rasteriser of the four C-140 games (+ tests)
├── game/superpong/      RetroPong (binary `retropong`): macroquad frontend with colour, sound,
│                        input (HID / gamepad / keyboard) and the pause menu
├── tools/superpong-mirror/ Python mirror of the core spec; generates the core's test fixtures
├── hardware/            Wiring, BOM, pinout
└── docs/                Specs (this file, SPEC-superpong.md, SPEC-controller.md);
                         docs/superpong-data/ holds the measured data (colour model, digits)
```

## Data flow

```
 pot L ─┐                       ┌──────────────── USB ────────────────┐
        ├─ ADC ─ oversample ─ filter ─ calibrate ─┤ HID gamepad (X,Y, 1 kHz)  ├─▶ retropong (hidapi) ─▶ superpong-core
 pot R ─┘        (firmware, XIAO RP2350)          │ CDC-ACM text protocol     ├─▶ rcctl monitor / calibrate / BOOTSEL
                                                  └──────────────────────────┘
```

* The **firmware** owns sampling and calibration and publishes clean `0..4095`
  axis values so every consumer (our game, any gamepad-aware program, a
  browser Gamepad API page) sees the same thing.
* The **serial protocol** is for tooling only (identification, raw values,
  calibration, reboot-to-bootloader). The game never needs it.
* The **game** reads HID reports directly with `hidapi`, matched on VID/PID, so
  it does not depend on OS gamepad mapping databases. If the controller is not
  present it falls back to any gamepad via `gilrs`, then to the keyboard.

## Super Pong core and RetroPong frontend

RetroPong is a clone of the Atari Super Pong C-140, modelled on measurements of a real C-140, not on a generic Pong.

* **Native raster, console frame rate.** `superpong-core` works in the console's
  own units: chip clocks across, scan lines down. One `tick` is one video frame
  at the console's rate (59.94 Hz, or 60.19 Hz in some phases; the core reports
  `frame_hz()`, and the frontend re-reads it every tick). Positions and
  velocities are integers per frame; there are no floats in the physics and no
  swept collisions.
* **No randomness.** The serve, the hidden ball, the light show and game-over
  are deterministic functions of the state and inputs, so a given input
  sequence always produces the same frames. That is what makes the headless
  screenshots reproducible and the fixtures checkable.
* **The core rasterises.** `render` fills a frame of 187 x 240 *cells*, each a
  *kind* (border, paddle, ball, net, digit, light-show block, ...), including
  clipping, blanking and the score digits. It does not change the state and may
  be called any number of times. The frontend never re-implements a rule.
* **The frontend maps cells to pixels.** Colour is a function of the cell's kind
  and its column: the background is black, and the drawn elements carry a
  left-to-right anchor gradient per kind taken from the packaging art, blended
  in OKLab (`docs/superpong-data/colour_model.json`; `colour.rs`). It also applies
  the display options (black-and-white; pixel or CRT mode, the latter a single
  shader pass in `crt.rs`), draws the picture in a 4:3 rectangle and the HUD
  strip below it.
* **The menu is drawn on the same raster.** `Esc` opens a pause menu (`src/menu/`:
  state machine in `mod.rs`, drawing in `screen.rs`, block font in `font.rs`),
  followed on resume by a 3-2-1 countdown. `canvas.rs` copies the core's frame to a
  canvas of the same 187 x 240 cells and the menu and countdown are drawn on it in
  the same cell kinds, so they get the same colours, black-and-white mode and CRT
  shader as the game. `routing.rs` decides which keys mean what (menu or console),
  and `scheduler.rs` turns frame time into console ticks. While the menu is open
  or the countdown runs, the console is not ticked at all. Spec: `SPEC-superpong.md` section 13.6.
* **Sounds are events.** The core only emits events (bounce kinds, point, serve,
  attract tick); the frontend plays four sounds, synthesised at startup from a model of the
  chip and its speaker (`audio.rs`), on one channel.
* **Spec, data and mirror.** `docs/SPEC-superpong.md` is normative, with its
  measured tables in `docs/superpong-data/`. `tools/superpong-mirror/` is an
  independent Python implementation of the spec that regenerates the fixtures
  the core's tests replay, so the Rust and the spec cannot drift apart silently.

## Building

* Host workspace: `cargo build --release` at the root builds `rcctl`,
  `retropong` (package `superpong`) and the libraries. Tests: `cargo test`.
* Firmware: `cd firmware && cargo build --release` (needs
  `rustup target add thumbv8m.main-none-eabihf`; `rust-toolchain.toml`
  requests it automatically), or `cargo run -p rcctl -- build`.
* Flash: `cargo run -p rcctl -- flash` builds the firmware (release, thumbv8m,
  from inside `firmware/` -- skip with `--no-build` or `--elf`), converts the
  ELF to UF2, and copies it to the mounted `RP2350` bootloader volume (asking
  the running firmware to reboot into the bootloader first if it is
  connected).

## Design decisions

* **Rust everywhere.** One language across firmware, tooling and game; the
  `protocol` crate is compiled into all three so the report layout and the
  command grammar cannot drift.
* **HID gamepad rather than a custom USB class** so the controller works with
  anything, and no drivers are needed on macOS or Linux.
* **Composite HID + CDC-ACM** (with interface association descriptors) so the
  toolkit gets a plain serial port without a custom driver.
* **Simulation separated from presentation.** `superpong-core` has no
  dependencies and is tested headlessly; the frontend is thin.
* **Integer physics at the console's frame rate**, as measured from a real
  C-140, rather than a finer tick with swept collisions: it is what makes the
  rally, the hidden-ball serve and the light show match the original frame by
  frame.
* **macroquad** for the frontend: tiny dependency surface, works on macOS and
  Linux (X11/XWayland) out of the box, software-friendly for CI screenshots.
