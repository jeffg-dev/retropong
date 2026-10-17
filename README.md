# RetroPong

A faithful clone of the Atari Super PONG Model C-140, released in 1976.
[See the original console](https://www.pong-story.com/atpong2.htm).

Original game sounds, physics, game modes, graphics. Recreated from measurements
of a real console, with all four games: Catch, Solitaire, Super Pong and Pong.

A CRT overlay recreates the scanlines, glow and curved picture of an old television.
Switch to sharp pixels or black-and-white, adjust the volume, and choose each
knob's direction in the game menu. Play on macOS, Linux or in a browser; browser
game settings are remembered between visits.

## Hardware

Two rotary knobs connect to a small USB controller:

- One [Seeed Studio XIAO RP2350](https://www.digikey.com/en/products/detail/seeed-technology-co-ltd/102010550/24814463).
- Two [Bourns 95A1A-B28-B15L 10 kΩ linear potentiometers](https://www.digikey.com/en/products/detail/bourns-inc/95A1A-B28-B15L/2564735).
- Two knobs for ¼-inch shafts, a USB-C data cable, wire and an enclosure.

Each potentiometer connects to 3.3 V and ground, with its wiper connected to
A0 or A1. See the [wiring guide](hardware/README.md) for assembly details.

## Firmware

The firmware reads and smooths the knobs, then registers as a USB game controller.
Calibration and left/right wheel assignment are saved on the controller.

With [Rust installed](https://www.rust-lang.org/tools/install), run `make flash`
from this repository to install or update the firmware. For the first flash,
hold the board's BOOT button while plugging it in.

Plug in the controller and run the full calibration:

```sh
make calibrate
```

Watch the live knob bars and learned ranges as you turn both knobs fully in each
direction. Press Enter when finished, then answer `y` to save.
To exchange which wheel controls the left and right players:

```sh
make swap
```

Each run flips the assignment; run it again to switch back. This setting survives
unplugging and recalibration.

## Play

```sh
make run
```

Or follow the [web app setup](game/superpong-web/README.md).
**Enter** starts a game; **Esc** opens the menu. Turn the knobs to move the paddles.
