# Hardware

## Bill of materials

| Qty | Part | Notes |
|-----|------|-------|
| 1 | Seeed Studio XIAO RP2350 | USB-C, RP2350A, 2 MB flash |
| 2 | Potentiometer 95A1A-B28-B15L | 10 kΩ, linear taper (B), conductive plastic, solder lugs, panel mount, 1/4" shaft |
| 2 | Knobs to fit the 1/4" shaft | Big paddle-style knobs feel best |
| 1 | USB-C cable | data-capable |
| 2 | 100 nF ceramic capacitor (optional) | wiper → GND, noise filter |
| 2 | Momentary push button, SPST normally-open (NO) | panel mount; 12–16 mm metal or 24/30 mm arcade buttons both work |
| — | Enclosure, hook-up wire, heat shrink | |

## Wiring

```
                      XIAO RP2350 (top view, USB-C at top)
                     ┌───────────────────────────┐
   Left pot wiper ───┤ A0/D0 (GPIO26)   5V       ├
   Right pot wiper ──┤ A1/D1 (GPIO27)   GND      ├──── both pots lug 1 (CCW end) + both buttons
   Left button ──────┤ A2/D2 (GPIO28)   3V3      ├──── both pots lug 3 (CW end)
   Right button ─────┤ D3    (GPIO5)    D10/MOSI ├
                     ┤ D4/SDA(GPIO6)    D9/MISO  ├
                     ┤ D5/SCL(GPIO7)    D8/SCK   ├
                     ┤ D6/TX (GPIO0)    D7/RX    ├
                     └───────────────────────────┘

     Schematic (3V3, GND, A0, A1, D2 and D3 are the XIAO pads above):

     3V3 ──────┬─────────────────────────────────┐
               │                                 │
             lug 3                             lug 3
   LEFT POT   ┌┴┐                    RIGHT POT  ┌┴┐
   10k lin    │ │                    10k lin    │ │
              │ │<── lug 2 ──┬── A0             │ │<── lug 2 ──┬── A1
              │ │            │                  │ │            │
              └┬┘          ──┴── C1             └┬┘          ──┴── C2
             lug 1         ──┬── 100 nF        lug 1         ──┬── 100 nF
               │             │                   │             │
     GND ──────┴─────────────┴───────────────────┴─────────────┴──────────┐
                                                                          │
                       _|_   SW1 (left button)                            │
     D2  ────────────o     o──────────────────────────────────────────────┤
                                                                          │
                       _|_   SW2 (right button)                           │
     D3  ────────────o     o──────────────────────────────────────────────┘
```

* C1/C2 (optional, 100 nF ceramic, no polarity) solder straight across each
  pot's lug 1 and lug 2, so they sit right at the pot. They filter wiper
  noise; the controller works without them.
* Keep wiper leads short (< 20 cm) or use shielded wire; the ADC input is high
  impedance and picks up hum.
* Both pots share GND and 3V3. Never connect a pot to 5V: the RP2350 ADC and
  GPIO are 3.3 V only.
* Linear taper is essential ("B" in the part number). A logarithmic (audio)
  pot would make the paddle move non-linearly.
* Buttons switch to GND and rely on the RP2350's internal pull-up, so they
  read **active-low** (pressed = 0) and need no external resistor. Don't wire
  them to 3V3 with a pull-down instead: RP2350 erratum E9 makes weak
  pull-downs unreliable. Contact bounce is left to the firmware to filter.
* If you use illuminated arcade buttons, wire only the switch terminals; leave
  the LED terminals unconnected.

### Direction

With lug 1 on GND and lug 3 on 3V3, turning the shaft clockwise increases the
voltage on the wiper. RetroPong defaults to **CW UP**; choose **CCW UP** in the
game menu to reverse a player's direction. Firmware inversion (`rcctl cal invert
L 1` or `R 1`) corrects physically reversed wiring and is saved on the controller.

To exchange which physical wheel is assigned to the left and right players, run
`cargo run -p rcctl -- cal swap on`. Use `off` to restore A0 = left and A1 = right.
The assignment is saved in the controller and survives recalibration. Calibration
and firmware inversion always refer to physical A0/A1; the game menu adjusts each
logical player’s direction independently.

## Pinout reference (XIAO RP2350)

| Pad | GPIO | Also |
|-----|------|------|
| D0 / A0 | GPIO26 | ADC0 — **left knob** |
| D1 / A1 | GPIO27 | ADC1 — **right knob** |
| D2 / A2 | GPIO28 | ADC2 — **left button** |
| D3 | GPIO5 | **right button** |
| D4 | GPIO6 | I²C SDA |
| D5 | GPIO7 | I²C SCL |
| D6 | GPIO0 | UART TX |
| D7 | GPIO1 | UART RX |
| D8 | GPIO2 | SPI SCK |
| D9 | GPIO4 | SPI MISO |
| D10 | GPIO3 | SPI MOSI |
| User LED | GPIO25 | active-low |
| RGB LED | GPIO22 data / GPIO23 power | WS2812, unused |
| BOOT button | — | hold while connecting USB for the bootloader |
| RESET button | — | |

(Confirm against the Seeed wiki for your board revision:
<https://wiki.seeedstudio.com/getting-started-xiao-rp2350/>.)

## Assembly notes

1. Drill two 9–10 mm holes for the pot bushings, two holes sized for the
   buttons (one beside each knob), plus a slot for the XIAO's USB-C connector.
2. Solder three wires per pot and two per button; solder the far ends to the
   XIAO pads (or to header pins if you want the board removable). The pots and
   buttons all share the one GND pad. If you're fitting C1/C2, solder each
   across its pot's lug 1 and lug 2 along with the wires.
3. Mount, plug in, run `cargo run -p rcctl -- flash`, then
   `cargo run -p rcctl -- monitor` to see both knobs move, and
   `cargo run -p rcctl -- calibrate` once to learn the end stops. Press each
   button in `monitor`: its `[?]` turns into `[X]`. The buttons are optional;
   leaving them out just leaves their HID bits at 0.
