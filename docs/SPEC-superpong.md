# Super Pong (Atari model C-140): behavioural specification

This is the authoritative behavioural spec for the `superpong` game. It describes the real 1976 console, as measured frame by frame from a reference video of a working unit, and fills the gaps with the owner's manual and with explicit choices. The core (`game/superpong-core`) implements sections 2 and 4 to 12, and the frontend (`game/superpong`) implements sections 3, 12 and 13. Section 14 is the contract between them.

## 0. How to read this spec

**Data files.** These files in `docs/superpong-data/` are part of this spec and authoritative:

- `colour_model.json`: the anchor colours of the border and foreground gradients, and the luma levels and stripe constants (the background is black).
- `digits.json`: the score glyphs and their positions.

The tables in §3 and §9 copy them. If the two ever disagree, the data file wins.

**Integer notation.**
- All positions and velocities are integers.
- `a..b` is half-open, as in Rust; `a..=b` is inclusive.
- `x.div_euclid(n)` and `x.rem_euclid(n)` are floor division and floor modulo, as in Rust, so they are correct for negative `x`.
- Pseudo-code is Rust-flavoured and normative unless it is marked as an example.

**Determinism.** The console has **no randomness**. Given the same sequence of inputs, the core produces bit-identical frames and events.

## 1. The console and controls

The C-140 has a left-player knob, a right-player knob, a four-position Game Select switch, a Start Game button, an On/Off switch and a speaker in the console.

| C-140 control | Ours |
|---|---|
| Left knob | RetroController knob X (left pot); gamepad fallback |
| Right knob | RetroController knob Y (right pot); gamepad fallback |
| Game Select 1-4 (Catch, Solitaire, Super Pong, Pong) | GAME MODE pause-menu entry |
| Start Game | `Enter` or keypad `Enter` |
| On/Off | starting the program is power-on; `Esc` opens the pause menu (§13.6) |

- **Power-on** shows the light show (§10) with the scores `0 0`.
- **Start** turns the light show into the selected game's playfield at once (§11).
- **Game over:** when a score reaches 15, the light show returns at once and keeps the final scores on screen (§8).

## 2. Raster, coordinates and timing

### 2.1 Units

| Unit | Meaning |
|---|---|
| clock (X) | one horizontal console pixel clock |
| line (row) | one scan line; frames are non-interlaced |
| frame | one console frame, which is one core `tick()` |

- X grows to the right and rows grow downward.
- X = 0 is the left end of the borders.
- Row 0 is the top line of the top border.

### 2.2 The playfield box

| Region | X | Rows |
|---|---|---|
| Border box | 0..176 | 0..224 |
| Top border | 0..176 | 0..16 |
| Bottom border | 0..176 | 208..224 |
| Interior | 0..176 | 16..208 (192 lines) |
| Display window (overscan) | −6..181 (187 columns) | −8..232 (240 lines) |

- **Nothing is ever lit outside rows 0..224 or outside X 0..176.**
  - Every object in §4 lies inside that box.
  - The rest of the display window is background.
- The real borders end at X 0.25 and 176.25. The clone uses whole clocks, 0..176.

### 2.3 Frame rate

| Phase (§14) | Frame rate |
|---|---|
| `Waiting`, `Rally` (a game is shown) | **59.94 Hz** (`FRAME_HZ_PLAY`) |
| `Attract` (light show) | **60.19 Hz** (`FRAME_HZ_ATTRACT`) |

- The core advances exactly one console frame per `tick()`.
- The frontend calls `tick()` at `Console::frame_hz()`, the rate of the *current* phase (§13.5).

### 2.4 Line and frame structure, as far as the clone needs it

| Fact | Value |
|---|---|
| Clocks per line seen by the light-show counter | 228 |
| Lines per frame in attract mode | 262 |
| Lines per frame in play, used only for the hidden ball's vertical wrap | 263 |
| Horizontal wrap of the hidden ball in play | 228 clocks moving right, 227 moving left (`HIDDEN_WRAP_RIGHT`, `HIDDEN_WRAP_LEFT`) |

## 3. Colour

### 3.1 Cell kinds and colour

Every cell of the display window is one of three kinds:

| Kind | What |
|---|---|
| `Background` | everything not lit: **black**, (0, 0, 0), at every X, in every display mode |
| `Border` | border rows inside X 0..176 that no foreground object covers |
| `Foreground` | everything lit: net, paddles, walls, ball, digits, light-show blocks. Foreground covers the border wherever they overlap. |

- **The background is black. Only the drawn elements (the border and everything lit) carry colour.** The video's faint green, blue and maroon wash on the background is a capture/TV artefact, not the game, and is not modelled.
- **Colour depends only on the kind and on X, never on the row.**
- So the left paddle and wall are yellow and the right ones pink, the net and forwards are light blue, a ball changes colour as it crosses, and a digit's colour changes across its own width.
- **The colours are the game's packaging art, as a left-to-right gradient.** The box shows black screens whose drawn objects are coloured by horizontal position: yellow at the far left (left paddle, left Catch wall), green for the left score digits, light blue for the net, lilac and pink for the right score digits, pink for the right paddle and wall; the border bars are blue at the left, sky blue in the middle and violet at the right. Each kind has a few **anchor colours**, clean versions of what the box shows, placed at the clock column X of the element each was read from.
- Between two neighbouring anchors the colour is the straight line in OKLab (Ottosson's perceptual space; the anchors' sRGB values are converted to OKLab, interpolated, and converted back), rounded to 8-bit sRGB. Beyond the first and last anchor it is constant.

  | Kind | Anchors (X → colour) |
  |---|---|
  | `Border` | 0 → `#4666bc` blue, 88 → `#5cb8f0` sky blue, 175 → `#7862c2` violet |
  | `Foreground` | 16 → `#f4ec70` yellow, 48 → `#8edba0` green, 88 → `#98d4f4` light blue, 136 → `#ecc6ec` lilac, 161 → `#f29cc4` pink |

  The foreground anchors sit at the elements they were read from: the left paddle and wall at X 16-19, the left digits at X 24-55, the net at X 88-91, the right digits at X 120-151 and the right paddle and wall at X 160-163. The border's light spot in the box art at the net is the net itself, which the core draws through the border as foreground, so the border is a plain blue to violet gradient with its lightest blue under the net. The light-show blocks, which span the full width in the foreground colour, therefore read as a pastel rainbow from yellow through green, blue and lilac to pink. The anchors and constants are in `docs/superpong-data/colour_model.json`.
- The colours are defined for every X; border cells exist only at X 0..176. The background is black at every X.
- Selected values:

| X | background | border | foreground |
|---|---|---|---|
| 0 | 0, 0, 0 | 70, 102, 188 | 244, 236, 112 |
| 16 | 0, 0, 0 | 74, 117, 198 | 244, 236, 112 |
| 32 | 0, 0, 0 | 78, 132, 207 | 196, 228, 139 |
| 48 | 0, 0, 0 | 82, 146, 216 | 142, 219, 160 |
| 68 | 0, 0, 0 | 87, 165, 228 | 146, 216, 204 |
| 88 | 0, 0, 0 | 92, 184, 240 | 152, 212, 244 |
| 112 | 0, 0, 0 | 106, 161, 227 | 198, 206, 240 |
| 136 | 0, 0, 0 | 114, 137, 215 | 236, 198, 236 |
| 148 | 0, 0, 0 | 117, 125, 208 | 240, 178, 217 |
| 160 | 0, 0, 0 | 119, 113, 202 | 242, 158, 198 |
| 175 | 0, 0, 0 | 120, 98, 194 | 242, 156, 196 |

### 3.2 Display options (frontend)

| Option | Rule | Default |
|---|---|---|
| Colour | RGB from the anchor gradients of §3.1 | **on** |
| Black-and-white (luma) | A constant grey per kind, the console's luma levels as a monochrome set shows them: `Background` → 0 (black), `Border` → 152, `Foreground` → 161, at every X. The luma levels are a property of the video signal, not of the colours, so they do not follow the anchors. Grey = (Y, Y, Y). With stripes on, the stripe offset is added to these grey levels too (clamped to 0..=255), so B/W keeps the carrier texture. | off |
| Display mode | **Pixel**: the colours above as sharp pixels, nothing else. **Classic video**: the picture as a 1970s colour TV showed it, with the stripe texture below (§13.1). | CRT |
| Stripes (carrier texture) | Draw each clock as 4 sub-columns `j = 0..4`. Add `a · cos(2π((j + 0.5)/4 − φ))` to R, G and B **before** the 0..=255 clamp, so a channel that sits below black swallows it. `a` = 94 for foreground and 17 for border (levels of the picture texture), and the peak `φ` = 0.48 for the foreground and 0.42 for the border (the constants `stripe_peak` of the data file; measured as `0.48 + offset/360` in the video). **The background has no stripe (`a` = 0): it stays black in CRT mode.** In B/W the offset is added to the grey level of the cell's kind (see above). | on in classic video, off in pixel mode |

**Stripe measurements (reference video, 905 s Pong, 600 s Super Pong, light show 470 s; per-column phase-locked averages over 750 frames, fitted with a one-clock cosine).**

| Kind | Video: amplitude in luma | Video: peak position | Clone CRT output before | after |
|---|---|---|---|---|
| Foreground | 47 levels (43-55 over the light-show blocks; the net, 4 clocks wide, 49 in R) | 0.46-0.48 | 22 | 47 |
| Border | 9.7 (9.3-9.9, equal in R, G and B, at every X) | 0.40-0.43 | 5.8 | 9.7 |
| Background (not used) | 2.8 in luma (0.3 to 5.2 over X); 5 in the brightest channel (R, G or B), 1-2 in channels below black | 0.27-0.31 | n/a | n/a |

- The peak positions 0.48 and 0.42 for the foreground and border are the measured 0.47 and 0.42 (the background's 0.29 measured is not used; in the video they followed the kind's hue offset as `0.48 + offset/360`). The stripe is locked to the clock (its position inside the clock does not move along the line, while the hue does), so it is the chip's carrier leaking into luma, not dot crawl of the decoded chroma.
- The amplitude is **not** proportional to the kind's chroma (82 and 30 give 0.57 and 0.32 stripe per chroma), so the simple per-kind amplitude stays.
- The video's background stripe (small, in the brightest channel of the wash) belongs to the capture's wash, not the game, and is dropped with it: the background stays black. The stripe is still added before the clamp, so a channel of the border or foreground that sits below black swallows part of it.
- The CRT's luma filter passes about 0.57 of the one-clock carrier, and the chroma filter removes the chromatic part of the stripe, so the strengths above are about twice the video's amplitudes. They were fitted so the CRT output measures the video's numbers (table). The strengths need fitting again if `LUMA_SIGMA` in `crt.frag` changes. Pale palette colours clip the carrier in the red and green channels, reducing the output amplitude.

Not emulated :
- the content-dependent level shifts of 4-15 levels ;
- the chroma bleed at object edges (classic video blurs chroma like a generic composite set, §13.1, but is not fitted to the video's bleed);
- the 3-4 level vertical drift .
- the video's own colours: its capture desaturation, hue drift and the other residuals of the old colour fit , now that the colours come from the packaging art.

## 4. Objects and geometry

### 4.1 Common objects

| Object | X | Rows | Drawn |
|---|---|---|---|
| Top border | 0..176 | 0..16 | `Border` |
| Bottom border | 0..176 | 208..224 | `Border` |
| Net | 88..92 | rows 0..224 with `row % 16 < 8` (8 on, 8 off; it runs through both borders) | `Foreground` |
| Ball | drawn columns X..X+2; collision columns X..X+3 (one extra column on the right) | R..R+4 | `Foreground`, only in phase `Rally` |

- The net has no effect on the ball.
- (X, R) is the ball's **left column and top row**.

### 4.2 Per-game objects

A paddle is 4 clocks × **15 lines**.  The left paddle on this unit sometimes measures 14 lines, and the right one 16 lines. These are analogue quirks and are **not** reproduced.

`top_L` and `top_R` are the knob rows (§5).

| Game | Object | X | Rows | Reflects a ball moving | Kind |
|---|---|---|---|---|---|
| 1 Catch | left wall | 16..20 | 0..224 except the hole `top_L..top_L+15` | left (dirx −1) | Wall |
| 1 Catch | right wall | 160..164 | 0..224 except the hole `top_R..top_R+15` | right (dirx +1) | Wall |
| 2 Solitaire | player paddle | 16..20 | `top_L..top_L+15` | left | Paddle |
| 2 Solitaire | back wall | 160..164 | `top_R..224` (the right paddle extended down to row 223) | right | Wall |
| 3 Super Pong | left back | 16..20 | `top_L..top_L+15` | left | Paddle |
| 3 Super Pong | left forward | 80..84 | `top_L..top_L+15` | left | Paddle |
| 3 Super Pong | right forward | 96..100 | `top_R..top_R+15` | right | Paddle |
| 3 Super Pong | right back | 160..164 | `top_R..top_R+15` | right | Paddle |
| 4 Pong | left back | 16..20 | `top_L..top_L+15` | left | Paddle |
| 4 Pong | right back | 160..164 | `top_R..top_R+15` | right | Paddle |

- Every object is drawn as `Foreground` wherever it lies, including over the borders.
- Every object is drawn in every phase, including the light show, at its live knob position.
- **Order in the object list** (used for collision tests) is the order of the table rows above. Two objects can never both overlap the ball in the same frame, so the order has no effect. It is fixed for determinism. **(choice)**

## 5. Knobs

```
fn sanitize(k: f32) -> f32 { if k.is_nan() { 0.5 } else { k.clamp(0.0, 1.0) } }
fn knob_to_top(k: f32) -> i32 { min(TOP_MAX, floor(sanitize(k) * 210.0) as i32) }   // TOP_MAX = 209
fn knob_for_top(top: i32) -> f32 { (top.clamp(0, TOP_MAX) as f32 + 0.5) / 210.0 }   // exact inverse; for demo/tests
```

- Knob 0.0 is the top. The frontend applies any user inversion before handing values to the core.
- The one mapping gives:
  - every paddle top (backs and forwards share their player's top);
  - the Catch hole tops;
  - the Solitaire wall top (right knob).

- The range is 0..=209, so a paddle can sit fully inside either border but never leaves the window. The Solitaire wall can close the opening completely: any top ≤ 16 covers the whole interior.
- Knobs are read on every tick, in every phase.
- **Not reproduced.** On the filmed unit the knob range reached past the window: an object whose top went above row 0 disappeared, including the whole Solitaire wall (796.96-797.93 s). That depends on the unit's pot range.

## 6. Ball physics (phases `Waiting` and `Rally`)

### 6.1 State

```
x: i32          // left column (drawn x..x+2, collides x..x+3)
row: i32        // top row (rows row..row+4)
dirx: i32       // +1 = moving right, -1 = moving left
vy: i32         // lines per frame, -3..=3, + = down
volleys: u32    // reflections since the ball last entered play (§7)
phase           // Waiting = ball hidden, digits shown; Rally = ball visible, digits off
```

### 6.2 Speeds

- `vx = dirx * speed`. Speed is 1 while hidden, and otherwise `tier(volleys)` (§7), which is 1, 2 or 3 clocks per frame.
- `vy` is one of 0, ±1, ±2, ±3 lines per frame, with the same set at every tier. Angles narrow only because vx grows.

### 6.3 Per-frame update (normative)

```
fn play_frame(&mut self, events: &mut Vec<SoundEvent>) {
    let was_hidden = self.phase == Waiting;

    // 1. move
    let speed = if was_hidden { 1 } else { tier(self.volleys) };
    self.x += self.dirx * speed;
    self.row += self.vy;

    // 2. wrap (hidden ball only) and vertical normalisation (always; no effect on a visible ball)
    if was_hidden {
        let wrap = if self.dirx > 0 { HIDDEN_WRAP_RIGHT } else { HIDDEN_WRAP_LEFT };      // 228 / 227 (§6.9)
        self.x = (self.x - HIDDEN_X_MIN).rem_euclid(wrap) + HIDDEN_X_MIN;                 // x in -25..=202 / -25..=201
    }
    self.row = (self.row - ROW_MIN).rem_euclid(PLAY_LINES) + ROW_MIN;                              // row in -40..223

    // 3. borders: only while the ball is horizontally inside the border box
    if 0 <= self.x && self.x <= 175 {
        if (self.vy < 0 && -6 <= self.row && self.row <= 15) || (self.vy > 0 && self.row + 3 >= 208) {
            self.vy = -self.vy;
            if !was_hidden { events.push(Bounce(Border)); }
        }
    }

    // 4. hidden ball: the serve test is the only remaining step
    if was_hidden {
        if (self.dirx > 0 && self.x == SERVE_X_RIGHT) || (self.dirx < 0 && self.x == SERVE_X_LEFT) {
            self.phase = Rally;                       // ball drawn from this frame; digits off
            events.push(Serve);
        }
        return;
    }

    // 5. paddles and walls: at most one reflection per frame
    for obj in self.objects() {                       // §4.2, in table order
        if self.dirx == obj.reflect_dir
            && self.x < obj.x + 4 && self.x + 3 > obj.x
            && obj.rows_overlap(self.row)
        {
            self.dirx = -self.dirx;
            self.volleys += 1;                        // saturating
            self.vy = obj.new_vy(self.row);
            events.push(Bounce(obj.kind));            // Paddle or Wall
            break;
        }
    }

    // 6. exit = point
    if (self.dirx > 0 && self.x + 1 >= 176) || (self.dirx < 0 && self.x + 1 <= 0) {
        let exit = if self.dirx > 0 { Right } else { Left };
        let scorer = if self.kind == Catch { exit } else { exit.other() };
        self.score[scorer] += 1;
        self.volleys = 0;
        events.push(Point);
        if self.score[scorer] >= WIN_SCORE { self.enter_attract_from_ball(); }  // §10.6
        else { self.phase = Waiting; }                // ball hidden from this frame; digits on
    }
}

fn tier(v: u32) -> i32 { if v <= 3 { 1 } else if v <= 7 { 2 } else { 3 } }
```

Basis for each step:

| Step | Rule |
|---|---|
| 1 | Move first, then test. A collision is detected at the moved position, the ball is drawn there (it can overlap the object), and the new velocity takes effect on the next frame's move. |
| 2 | Hidden-ball horizontal wrap of 228 clocks moving right and 227 clocks moving left; rows kept in −40..223 with period 263 |
| 3 | Borders reverse vy (magnitude kept) in the first frame the ball overlaps a border by at least one line, for both the visible and the hidden ball, but only while 0 ≤ x ≤ 175. A hidden ball outside that range passes through the vertical blanking. The top test covers rows −6..=15 only (`TOP_BOUNCE_ROW_MIN` = −6): a hidden ball that comes back into the box above row −6 moving up keeps going up through the blanking and wraps to the bottom. Hidden-ball reversals are silent. |
| 4 | Serve when the hidden ball reaches the net column |
| 5 | Direction-gated per-frame coincidence test against the collision columns |
| 6 | Exit when the ball's centre (x + 1) reaches a border-box end |

**The top limit of step 3.** −6..=15 is the light show's top zone (wrap lines 258..=261 and 0..=17, §10.3) in rows, with row = wrap line − 2 (§10.2), so the hidden ball and the light show turn around at the same places.

### 6.4 Object rows and outgoing vy

| Object | `rows_overlap(R)` (ball rows R..R+4) | `new_vy(R)` |
|---|---|---|
| Paddle (back or forward), top `t` | `R < t + 15 && R + 4 > t` | `SEGMENT_VY[clamp((R - t).div_euclid(2), 0, 7)]` with `SEGMENT_VY = [-3, -2, -1, 0, 0, 1, 2, 3]` |
| Catch wall, hole top `h` | some row r in `max(R,0)..min(R+4,224)` with `r < h \|\| r >= h + 15` | `if R < h { -3 } else { 3 }` |
| Solitaire wall, top `t` | `R + 4 > t && R < 224` | `if R < t { -3 } else { SEGMENT_VY[(R - t).rem_euclid(16) / 2] }` |

- **Incoming direction is irrelevant.** The outgoing vy depends only on where the ball's top row is relative to the object, never on the incoming vy, the tier or the side.
- **Paddle segments.** Δ = R − t runs from −3 to 14 while the ball overlaps:

| Δ | −3..=1 | 2..=3 | 4..=5 | 6..=7 | 8..=9 | 10..=11 | 12..=13 | 14 |
|---|---|---|---|---|---|---|---|---|
| vy | −3 | −2 | −1 | 0 | 0 | +1 | +2 | +3 |

- **Angles are symmetric top/bottom.** The manual's steeper bottom arrow in the "8th + Volley" diagram is a print artefact, and the user decided the reflection angles are symmetric. `SEGMENT_VY` is antisymmetric (`SEGMENT_VY[i] == -SEGMENT_VY[7 - i]`). The segment is counted from the ball's top row (as measured), so more ball positions clamp to the top segment (Δ −3..=1) than to the bottom one (Δ 14). That is an effect of the 4-line ball overlapping the paddle's ends, not an asymmetry in the angles.
- **Late hits.** The test runs on every frame in which the ball's collision columns overlap the object's columns and the direction matches. So:
  - a ball that slid past the face beside a paddle is hit when the paddle, or the ball's own vy, brings their rows together;
  - a paddle moved onto a ball inside its columns hits it.

- **From behind.** A ball moving away from an object's reflecting side passes through it with x, vy and volleys unchanged. This applies to every paddle and wall.
- **Catch holes.** A ball whose four rows are all inside the hole passes the wall. A ball still inside the wall's columns that comes to overlap a wall row (because the hole moved, or because of its own vy) is reflected.

### 6.5 Where the ball turns around

These follow from 6.3 and are listed for tests.

| Object | Turnaround x at tier 1 / 2 / 3 |
|---|---|
| Left back paddle, left Catch wall (columns 16..20) | 19 / 19 or 18 / 19, 18 or 17 |
| Right back paddle, right Catch wall, Solitaire wall (columns 160..164) | 158 / 158 or 159 / 158, 159 or 160 |
| Left forward (80..84) | 83 / … |
| Right forward (96..100) | 94 / … |

- **Borders.** The turnaround row (the row at which vy flips) is 15, 14 or 13 at the top, and `row + 3` = 208, 209 or 210 at the bottom, depending on phase. With |vy| = 1 it is always 15 and 208.
- **Corner case.** A border flip and a paddle or wall reflection can occur in the same frame. Step 3 runs first, so the object's `new_vy` wins, and the border may flip the ball again on the next frame.
  - A visible ball's row therefore stays within 10..=210 once it has turned around, except a flat ball served after a game over and Start, which can be up to 6 rows lower (§10.6). A ball served moving up from below the bottom border (step 3's top limit) can start at any row up to 222.
  - This was not observed.

### 6.6 The hidden ball and the serve

There is no serve timer and no re-serve.

- **At the exit frame:**
  - the point registers and the Point sound plays;
  - the digits are drawn and the ball is hidden (phase `Waiting`);
  - `volleys` resets to 0;
  - x, row, dirx and vy are kept.
- **While hidden:**
  - the ball moves 1 clock per frame in its exit direction and keeps its vy;
  - it wraps horizontally, reflects off the borders only while 0 ≤ x ≤ 175 (the top border only at rows −6..=15), and wraps vertically through blanking otherwise (§6.3 steps 2-3);
  - paddles and walls ignore it. It always meets them from behind anyway.
- **The serve:**
  - It happens in the frame the hidden ball reaches `SERVE_X_RIGHT` = 87 moving right, or `SERVE_X_LEFT` = 90 moving left.
  - In that frame the ball becomes visible at that x (phase `Rally`), the digits disappear and the Serve sound plays.
  - The ball is first visible at the net: x = 87 (centre 88) when moving right, x = 90 (centre 91) when moving left.
- **The serve direction is the exit direction in every game:**
  - Pong and Super Pong: toward the player who missed.
  - Catch: toward the player who caught.
  - Solitaire: toward the wall after a basket, toward the player after a miss.
  -  The manual's "served again automatically towards you" for Solitaire is not what the console does.
- **Display durations** follow from the geometry:

| Exit | Exit x at tier 1 | Frames with the digits shown (exit frame to the frame before the serve) | At 59.94 Hz | Measured |
|---|---|---|---|---|
| Right | 175 | **140** | 2.336 s | 2.327 ± 0.003 s |
| Left | −1 | **136** | 2.269 s | 2.263 ± 0.005 s |

- Exits at tier 2 or 3 can land 1-2 clocks further out, which shortens the display by 1-2 frames. This matches the measured independence from exit speed.

### 6.7 Worked example

This is an example, not normative. In Pong, the ball leaves on the right at tier 1:

| Frame | Event |
|---|---|
| E | x = 175. Point registers: Point sound, digits on, ball hidden. |
| E + 1 … E + 27 | x = 176 … 202 (horizontal blanking; no border test) |
| E + 28 | x wraps from 203 to −25 (rightward wrap 228) |
| E + 53 | x = 0; border tests resume |
| E + 140 | x = 87. Serve: ball visible, digits off, Serve sound. |

### 6.8 Hidden-phase vertical behaviour

- Flat exits (vy = 0) serve at the exit row.
- Two Solitaire baskets with vy = 0 came back with vy = +1 (537.50 s, 558.06 s). This is unexplained and is **not** modelled.

### 6.9 The hidden wrap is direction-dependent

The normative values are `HIDDEN_WRAP_RIGHT` = 228 with `SERVE_X_RIGHT` = 87, and `HIDDEN_WRAP_LEFT` = 227 with `SERVE_X_LEFT` = 90.

The score display lasts 140 frames after a right exit and 136 after a left
exit. Start serves on tick S + 202 with `START_X` = 112.

## 7. Volleys and speed tiers

- `volleys` counts the reflections off paddles (back and forward), Catch walls and the Solitaire wall since the ball last entered play.
  - Border bounces do not count. Neither does passing an object from behind.
  - The manual counts only "good hits"; the console also counts wall bounces.
- **Reset:** to 0 at every exit (§6.3 step 6) and at Start (§11.1).
- **Game Select does not reset it.**

| volleys | Tier | Speed (clocks/frame) |
|---|---|---|
| 0..=3 | 1 | 1 |
| 4..=7 | 2 | 2 |
| 8 or more | 3 | 3 |

- The new speed applies from the frame after the 4th and 8th reflection, i.e. on its outgoing leg.
- Tiers apply in **all four games**, including Catch.

## 8. Scoring and game end

| Game | Ball leaves the left edge | Ball leaves the right edge |
|---|---|---|
| 1 Catch | **left** player +1 (caught it) | **right** player +1 |
| 2 Solitaire | right score +1 (the game's "misses") | left score +1 (the player's basket) |
| 3 Super Pong | right player +1 | left player +1 |
| 4 Pong | right player +1 | left player +1 |

- **First to 15 wins**, on either side, in every game. That includes Solitaire's misses counter.
- **Game end.** The point that makes 15 plays the Point sound and switches to the light show **in the same frame**.
  - There is no hold, no serve and no end screen.
  - The final scores stay on screen.
  - The real switch happened mid-frame, at the ball's line; the clone switches at the frame boundary.
- Scores are `u8` and never exceed 15. They reset to 0-0 only at Start.
- **Not reproduced.** The filmed unit's spontaneous score resets.

## 9. Score display and digit glyphs

### 9.1 When the digits are shown

| Phase | Digits |
|---|---|
| `Attract` | always shown: `0 0` after power-on, the final score after a game over |
| `Waiting` (ball hidden) | shown: after Start, and from the exit frame up to the frame before the serve |
| `Rally` | hidden |

### 9.2 Layout

All digits occupy rows 32..96 (64 lines).

| Side | Units cell X | Tens bar X (values 10..=15) |
|---|---|---|
| Left score | 40..56 | 24..28 |
| Right score | 136..152 | 120..124 |

- The units cell is 16 clocks × 64 lines, on a grid of 4 columns × 8 rows. Each grid cell is 4 clocks × 8 lines.
- The units cell shows `value % 10`.
- For values 10..=15 a single tens bar, 4 clocks × 64 lines, is added 16 clocks left of the units cell.

### 9.3 Glyphs

Grid row 0 is the top, and column 0 is the left of each string. Authoritative copy: `docs/superpong-data/digits.json`.

```
   0      1      2      3      4      5      6      7      8      9
 ####   ...#   ####   ####   #..#   ####   ####   ####   ####   ####
 #..#   ...#   ...#   ...#   #..#   #...   #...   ...#   #..#   #..#
 #..#   ...#   ...#   ...#   #..#   #...   #...   ...#   #..#   #..#
 #..#   ...#   ####   ####   ####   ####   ####   ...#   ####   ####
 #..#   ...#   #...   ...#   ...#   ...#   #..#   ...#   #..#   ...#
 #..#   ...#   #...   ...#   ...#   ...#   #..#   ...#   #..#   ...#
 #..#   ...#   #...   ...#   ...#   ...#   #..#   ...#   #..#   ...#
 ####   ...#   ####   ####   ...#   ####   ####   ...#   ####   ####
```

The same glyphs as Rust masks, where bit 3 is the leftmost grid column:

```rust
pub const GLYPHS: [[u8; 8]; 10] = [
    [0b1111, 0b1001, 0b1001, 0b1001, 0b1001, 0b1001, 0b1001, 0b1111], // 0
    [0b0001, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001], // 1
    [0b1111, 0b0001, 0b0001, 0b1111, 0b1000, 0b1000, 0b1000, 0b1111], // 2
    [0b1111, 0b0001, 0b0001, 0b1111, 0b0001, 0b0001, 0b0001, 0b1111], // 3
    [0b1001, 0b1001, 0b1001, 0b1111, 0b0001, 0b0001, 0b0001, 0b0001], // 4
    [0b1111, 0b1000, 0b1000, 0b1111, 0b0001, 0b0001, 0b0001, 0b1111], // 5
    [0b1111, 0b1000, 0b1000, 0b1111, 0b1001, 0b1001, 0b1001, 0b1111], // 6
    [0b1111, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001, 0b0001], // 7
    [0b1111, 0b1001, 0b1001, 0b1111, 0b1001, 0b1001, 0b1001, 0b1111], // 8
    [0b1111, 0b1001, 0b1001, 0b1111, 0b0001, 0b0001, 0b0001, 0b1111], // 9
];
```

Grid cell (gr, gc) of a glyph is lit when `GLYPHS[d][gr] & (0b1000 >> gc) != 0`. It covers X `units_x + 4*gc .. units_x + 4*gc + 4` and rows `32 + 8*gr .. 40 + 8*gr`.

Notes:
- The middle bar is above centre: the upper hole is 16 lines and the lower hole 24.
- 4 is open at the top, 6 and 9 have tails, and 7 has no tail.
- Right-side 11-15 were never seen. They are built from the observed parts.

## 10. Attract mode: the light show

### 10.1 What is on screen

1. Background.
2. Borders (`Border`).
3. `Foreground`, OR'ed together:
   - the light-show **blocks** (§10.3) everywhere in X 0..176 and rows 0..224, except rows **32..=95** (the score band);
   - the net;
   - the **selected game's** walls or paddles at the live knob positions (§4.2);
   - the scores (§9).
4. **No ball is drawn.**

Game Select changes only which game's objects are drawn.  The light show for games 2 and 3 was never seen; they show the same objects as in play.

### 10.2 State

```
h0: i32            // 0..228. H-counter phase: blocks start where (X - h0) mod 228 == 0
vc: i32            // V counter value on the last line (261) of the most recently simulated frame
vy: i32            // -3..=3, + = down; the vertical step per V cycle
last_wrap: i32     // line of the most recent V wrap, relative to line 0 of the current frame (can be negative)
rows_lit: [bool; 224]  // per-line V pattern of the current frame (output of run_lines)
```

The light show is the hidden ball's counters running free and displayed as blocks.  The invisible attract ball has left column `X = h0 + 4` (mod 228) and top row `R = (wrap line) − 2` (`LS_BALL_X_OFFSET`, `LS_BALL_ROW_OFFSET`). Start uses this mapping (§11.1). A game over does not: it starts the light show in a measured phase of its own (§10.6).   The first Start in the video agrees: from the light show calibrated at power-on, row = wrap − 2 gives the measured first serve to within a line (§6.3). L's H ≈ 0-1  came from the single game-over event, which §10.6 now models directly.

### 10.3 Per-frame update and pattern (normative)

```
const LS_H: i32 = 228;      // H-counter states per line
const LS_LINES: i32 = 262;  // lines per attract frame

fn attract_frame(&mut self, events: &mut Vec<SoundEvent>) {
    self.h0 = (self.h0 + 1).rem_euclid(LS_H);                       // +1 clock per frame, always rightward
    if self.h0 == ATTRACT_TICK_H0 { events.push(AttractTick); }     // §10.5
    self.run_lines();
}

fn run_lines(&mut self) {                                            // simulate the 262 lines of this frame
    self.last_wrap -= LS_LINES;                                      // re-base to this frame
    for l in 0..LS_LINES {
        self.vc += 1;
        if self.vc >= LS_LINES + self.vy {                           // end of a V cycle: wrap on line l
            self.vc = 0;
            self.last_wrap = l;
            let live = !(172 <= self.h0 && self.h0 <= 223);          // dead zone: invisible ball outside X 0..=175
            if live && self.vy < 0 && (l <= 17 || l >= 258) { self.vy = -self.vy; }
            else if live && self.vy > 0 && (207 <= l && l <= 225) { self.vy = -self.vy; }
        }
        if l < 224 { self.rows_lit[l as usize] = self.vc >= 256 || (self.vc & 16) == 0; }
    }
}

// block test used by render (§14.3), for X in 0..176 and row in 0..224:
fn block(&self, x: i32, row: i32) -> bool {
    if (32..=95).contains(&row) { return false; }                    // score band
    let h = (x - self.h0).rem_euclid(LS_H);
    self.rows_lit[row as usize] && h < 224 && (h & 8) == 0
}
```

Basis:

| Rule |
|---|
| Blocks 8 clocks lit and 8 dark (period 16), with one 12-clock gap per 228-clock line |
| Rows 16 lit and 16 dark; the row containing a wrap is 16 + 6 + vy lines tall (`V ≥ 256` lit) |
| The cycle period is 262 + vy, with **no pause** after a bounce |
| A bounce **sets** the direction, it does not toggle it: up at `l ≤ 17`, down at `l ≥ 207` |
| Bounces are ignored while `h0` is in 172..=223 |
| The upper limits `l ≥ 258` (top zone) and `l ≤ 225` (bottom zone) inside vertical blanking |
| A frame can contain 0 or 2 wraps when the wrap crosses the frame boundary; the loop handles this |
| Blocks are blanked on rows 32..=95 |
| `h0` moves +1 per frame and never reverses; vy is ±1 at power-on and inherited from the last ball after a game over |

### 10.4 Power-on state

`h0 = 92`, `vy = +1`, and the first frame has its V wrap on line 177. In code: `vc = 263 − 1 − 177 = 85` before `run_lines()` runs for frame 0. Frame 0 is the first console frame of the video, at 463.730 s.

- `Console::new` runs `run_lines()` once, without incrementing `h0`, so the power-on frame (frame 0) can be rendered before the first tick.
- The scores are 0-0.
- **Check:** with this state the model's first bounces fall at frames 30 (bottom), 220 (top), 410 (bottom) and 600 (top), the bottom bounce due at frame 790 is skipped (dead zone), and the next is at frame 1053 (bottom). At 60.19 Hz from frame 0 (463.730 s) these are 464.23, 467.39, 470.54, 473.70, (476.86 skipped) and 481.22 s. The video shows bounces at 464.21, 467.38, 470.55, 473.72, a skip at 476.86-477.18 and a bounce at 481.21 s.

### 10.5 Attract sound

- An `AttractTick` event fires in the frame `h0` becomes `ATTRACT_TICK_H0` = **82**, i.e. once every 228 frames (3.788 s).
- It marks the invisible ball crossing the net (X = 86).

### 10.6 Game over: from the play ball to the light show

`enter_attract_from_ball()` runs in the frame of the 15th point, after the score update:

```
fn enter_attract_from_ball(&mut self) {
    self.phase = Attract;
    self.ls.h0 = (self.x - LS_GAME_OVER_X_OFFSET).rem_euclid(228);   // h0 = x
    self.ls.vy = self.vy;                         // keeps its vertical speed and direction
    let v = self.row + LS_GAME_OVER_ROW_OFFSET;   // this frame's wrap line: row + 8, 18..=218 at any exit
    let p = 262 + self.ls.vy;
    self.ls.vc = (p - 1 - v).rem_euclid(p);      // so this frame's wrap falls on line v
    self.ls.run_lines();                          // this frame is drawn as light show; h0 is not incremented
}
```

`self.ls` is the light-show state of §10.2. The functions in §10.3 are its methods; `attract_frame` also receives the event list.

- `LS_GAME_OVER_X_OFFSET` = **0** and `LS_GAME_OVER_ROW_OFFSET` = **8**. This is not the attract mapping of §10.2 (x − 4, row + 2), which Start still uses: after the 15th point the real light show is 4 clocks further right and its wrap 6 lines lower than that mapping gives.
- The bounce test inside `run_lines` applies in this frame as in any other, and with the wrap at row + 8 it can fire:
  - a left exit (h0 225..=227, live) moving down at rows 199..=204 puts the wrap in the bottom zone (207..=212), and the light show starts moving up;
  - a right exit starts in the dead zone (h0 175..=177), so the light show does not bounce for its first 47-49 frames;
  - the top zone is never reached (v ≥ 18).
- Start after a game over reads the light show with the attract mapping (§11.1). A flat ball (vy 0, so the light show never moves vertically) therefore comes back 6 rows lower than its exit row, up to row 216, inside the bottom border.
- The frame of the 15th point is drawn as the light show, with the final scores and the score band blanked.

## 11. Start and Game Select

### 11.1 Start

Start does the same thing in every phase.

```
fn start_game(&mut self) {
    if self.phase == Attract {
        // keep the invisible ball's vertical state
        self.row = (self.ls.last_wrap - 2 - ROW_MIN).rem_euclid(PLAY_LINES) + ROW_MIN;
        self.vy = self.ls.vy;
    }                                     // from Waiting or Rally: row and vy are kept as they are
    self.score = [0, 0];
    self.x = START_X;                     // 112
    self.dirx = +1;
    self.volleys = 0;
    self.phase = Waiting;                 // playfield and "0 0" shown from this frame; ball hidden
}
```

- `START_X = 112` is chosen so that the ball, after moving in the Start tick and then one clock per frame through the rightward 228 wrap, reaches x = 87 on **tick S + 202** (S = the tick that applied Start).
  - The playfield and `0 0` are shown on frames S … S + 201 (202 frames, 3.370 s).
  - The ball and the Serve sound appear on frame S + 202.
  - The first serve is always rightward and at tier 1.
- The real console keeps the hidden ball's horizontal phase through Start ; the clone resets x.
- A Start pressed in the same tick as a Select applies after the Select.

### 11.2 Game Select

The switch is read every tick.

| Phase | Effect of a Select change |
|---|---|
| `Attract` | only changes which game's objects are drawn over the light show |
| `Waiting`, `Rally` | the playfield switches **in this tick** (draw and collisions). Ball position, velocity, `volleys`, phase and scores all carry over. |

- Selecting the position already selected does nothing.
- **Not reproduced:**
  - the 1-7 transitional frames in which the real switch passes through a Pong-like layout  ;
  - the score resets that coincided with switching on this unit  .

### 11.3 Tick order

```
fn tick(&mut self, input: &Input, events: &mut Vec<SoundEvent>) {
    self.top_l = knob_to_top(input.left);
    self.top_r = knob_to_top(input.right);
    if let Some(k) = input.select { self.kind = k; }
    if input.start { self.start_game(); }
    match self.phase {
        Attract => self.attract_frame(events),
        Waiting | Rally => self.play_frame(events),
    }
    self.frame += 1;
}
```

## 12. Sounds

### 12.1 Events (core)

| Event | When |
|---|---|
| `Bounce(BounceKind::Paddle)` | visible ball reflected by a back or forward paddle |
| `Bounce(BounceKind::Wall)` | visible ball reflected by a Catch wall or the Solitaire wall |
| `Bounce(BounceKind::Border)` | visible ball reflected by the top or bottom border (hidden-ball reversals are silent) |
| `Point` | the exit frame, including the 15th point |
| `Serve` | the frame the hidden ball becomes visible, including after Start |
| `AttractTick` | attract mode, `h0 == 82` (§10.5) |

- The core pushes events in the order they occur within the frame; a frame can produce two, e.g. a border bounce and then a paddle hit.
- The C-140 uses one sound for every reflection, so the frontend plays the same Bounce for every `BounceKind`.

### 12.2 Playback rules (frontend)

- **One channel.** Starting a sound stops the one playing. If several events arrive in one frame, they are played in order, so the last one is heard.
- Consecutive-frame bounces simply retrigger, which matches the measured continuous 2-frame tone.
- Sounds start at the tick that produced them.
- `--mute` starts muted; the menu volume can silence or unmute playback.

### 12.3 The four sounds: synthesised

- The frontend synthesises the four sounds at startup (`audio.rs`). There are no sound files.
- Event mapping: every `Bounce(_)` plays the Bounce; `Point`, `Serve` and `AttractTick` play their own sound.
- `--export-sounds <dir>` writes the four sounds as WAV files and exits.
- The model has two halves, both fitted to the video: the chip's sound pin (the drive), and one speaker model shared by all four sounds. **No sound has a gain of its own;** their relative levels come from the speaker model.

**Drive.** The sound pin is a logic square wave. It is a bit of the vertical line counter, switched on for whole frames. Line rate `LINE_HZ` = 3,595,295 Hz / 228 = 15,768.8 Hz (§2.3).

| Sound | Lines per half-period | Pitch | Drive length | Video  |
|---|---|---|---|---|
| Serve | 8 | `LINE_HZ`/16 = 985.6 Hz | 1 frame of 263 lines = 16.68 ms | 979.7 Hz square, 1 frame, then about 1 frame of ringing |
| Bounce | 16 | `LINE_HZ`/32 = 492.8 Hz | 1 frame, 16.68 ms | 487.6 Hz square, 1 frame |
| Point | 32 | `LINE_HZ`/64 = 246.4 Hz | 1 frame, 16.68 ms | 246.4 Hz square, 1 frame |
| AttractTick | 32 | 246.4 Hz | 5 frames of 262 lines = 83.08 ms | 245.6 Hz per frame, 83.7 ms, phase restarts every frame |

- The pin turns on at line 0 of the first frame and is on during the first half-period. The counter restarts every frame, so the 5-frame AttractTick has one 38-line on-half at each frame boundary (262 = 4 × 64 + 6).
- Levels are 0 (off: the rest state) and 1.
- The video's Bounce reads 487.6 Hz, not 492.8 Hz, because the speaker's ~455 Hz resonance pulls a one-frame tone's apparent pitch down. The synthesised Bounce reads 489.2 Hz with the same estimator, against 488.4 Hz in the video's balanced average.
- Likewise the Serve reads 981 Hz, not 985.6 Hz: the speaker's ~1 kHz mode sits just above the drive, under an anti-resonance, and pulls the driven frame's apparent pitch down. The synthesised Serve reads 980.9 Hz, the video 980.9 Hz. The Bounce's harmonics confirm the line rate within 0.1 %.

**Speaker.** Three analog sections in series, then a fixed output gain.

| Section | Parameters | Fitted from |
|---|---|---|
| Cone resonance: 2nd-order high-pass | 455 Hz, Q 5.3 | the joint fit; the Bounce's ringing after the drive (452-455 Hz) and the levels re Bounce |
| Mode under an anti-resonance: `(s² + s·ωz/Qz + ωz²) / (s² + s·ωp/Qp + ωp²)` | ωp = 2π·997 Hz, Qp 48; ωz = 2π·1008 Hz, Qz 250 | the Serve's pitch reading (981 Hz), its ringing (997-999 Hz) and its envelope; only the Serve has energy near 1 kHz |
| 2nd-order resonant low-pass: `ωl² / (s² + s·ωl/Ql + ωl²)` | ωl = 2π·2.4 kHz, Ql 2.1 | the harmonic levels of Bounce, Point and Serve |
| Output gain | 0.24 of full scale per unit drive (the Bounce peaks at −0.9 dBFS) |  |

**Rendering.** Mono, 16-bit, 44.1 kHz (quad-snd's mixing rate).
- The drive is area-sampled exactly at 16 × 44.1 kHz.
- The sections are bilinear-transformed biquads at that rate, in `f64`. Each output sample is the mean of its 16 sub-samples.
- Each sound starts one sample before the drive and lasts the drive plus 80 ms. The last 10 ms are a raised-cosine fade to exactly 0; the ringing there is already at least 40 dB down (the Serve; 57-71 dB the others).
- The output is bit-for-bit deterministic, and identical sample for sample to the Python reference used for the fit.

**Result against the clean video instances.** All levels are relative to the Bounce, with one common gain. The video has two recording segments whose 246/492 Hz balance differs by about 5 dB; the targets weight the two equally.

| Sound | RMS during the drive, video / synth | Duration at −6 dB, video / synth | Odd harmonics below 5 kHz, RMS difference | Pitch read, video / synth |
|---|---|---|---|---|
| Bounce | 0 / 0 dB | 16.4 / 16.6 ms | 4.7 dB | 488.4 / 489.2 Hz |
| Point | −11.7 / −13.5 dB (segments: −9.3, −15.3) | 19.2 / 22.9 ms | 2.9 dB | 246.5 / 246.5 Hz |
| Serve | −9.3 / −8.9 dB (ringing in frame 2: −5.1 / −5.1 dB re frame 1, at 999 / 997 Hz) | 29.4 / 25.2 ms | 5.6 dB | 980.9 / 980.9 Hz |
| AttractTick | −13.6 / −13.7 dB (segments: −10.8, −16.2) | 85.2 / 88.8 ms | not fitted (its frame restarts) | |

- **AttractTick level.** The tick uses the Point tone and the same speaker response.
- The harmonic differences are mostly the room's comb-like ripple, which a low-order model cannot follow.

## 13. Display and frontend

### 13.1 Picture

- The frontend asks the core to rasterise the display window (§14.3) into 187 × 240 cells. It maps each (kind, X) to RGB (§3.1), applies the options (§3.2) and shows the result as an image.
- **Aspect.** Emulate a correctly adjusted TV. One clock is 1.711 times as wide as one line is tall, so the 176 × 224 border box is about 4:3 and the 187 × 240 window is exactly 4:3.
  - The picture is therefore always drawn into a **4:3 rectangle**.
- **Size.** The picture is the largest 4:3 rectangle that fits the whole window. If the largest such rectangle whose height is a whole multiple of 240 device pixels is at least 90 % of that height, use it, so all lines are equally tall; otherwise use the full size with nearest filtering (lines may then differ by one device pixel).
- **Recommended implementation** (choice):
  - Build an RGBA image of `187·4 × 240` pixels: 4 sub-columns per clock, which the stripe texture needs, and which are identical when it is off.
  - Upload it to a texture with `FilterMode::Nearest`.
  - Draw it centred, at the size given above: directly in pixel mode, through the CRT shader in classic-video mode.
- **Display modes** (CRT menu entry, flag `--display <pixel|crt>`). Both use the same 4:3 rectangle and the same layout rule. **(choice)**
  - **Pixel**: the image with nearest filtering and no effects, as a modern monitor shows an image.
  - **Classic video (CRT, default)**: the image, with the stripe texture of §3.2 on, drawn through one fragment-shader pass (`crt.rs`, `shaders/crt.frag`). The shader builds on Timothy Lottes' public-domain CRT shader (libretro `glsl-shaders`, `crt-lottes`). It applies, in this order:
    - **Composite filtering** on the video signal (YIQ before gamma, per line): Gaussian blur of standard deviation 0.6 sub-column for luma, which keeps the one-clock stripes, and 3.5 sub-columns for chroma, so colour bleeds about a clock past object edges.
    - **Scanlines**: each of the 240 lines is a Gaussian beam across its height, standard deviation 0.25 line when black to 0.40 line at full white, per channel, normalised so each line keeps its average level. Bright lines bloom wider; dark lines show gaps.
    - **Glow**: 10 % of a Gaussian blur of radius 1.3 line heights is added.
    - **Phosphor mask**: vertical R, G, B stripes of one device pixel each, unlit stripes at 70 %, rescaled to keep the average level.
    - **Curvature and tube face**: barrel warp `p *= (1 + 0.024 y², 1 + 0.032 x²)` in −1..1 picture coordinates; outside a rounded rectangle (corner radius 0.035 picture heights) is black; a vignette `(16 x y (1−x)(1−y))^0.12`.
    - Scanlines and mask fade out between 3 and 1.5 device pixels per line, where they would only make moire.
    - Beams, glow and mask work in linear light (the 8-bit levels decoded with gamma 2.2 and re-encoded), so the average levels of §3.1 are kept apart from the glow, vignette and clipping.
  - Not emulated: phosphor persistence (P22 phosphors fade within about a millisecond, so nothing visible is left a frame later; the video's two-frame blend is the camera's, §15), interlace, and the video's own stripe and level artefacts beyond §3.2.
  - If the shader cannot be built (a GL compile error, or a non-OpenGL backend), the picture stays in pixel mode, the reason is printed to stderr. The game never stops for it.
- **Default display:** fullscreen, CRT on. `--fullscreen false` selects a 960 × 720 window. No HUD space is reserved.

### 13.2 First-launch hints

There is no HUD. At first launch, `PRESS ENTER TO START` and `PRESS ESC FOR MENU`
are drawn using the game's block font and CRT path. Hints disappear after the
first start and never return between rounds. Opening the menu temporarily hides
them; closing it before starting shows them again.

### 13.3 Keys

| Key | Action |
|---|---|
| `Enter`, keypad `Enter` | Start/restart; activate while the menu is open |
| `Esc` | Open/close the pause menu |
| Arrow keys | Navigate/change settings only while the menu is open |

Other keyboard shortcuts and keyboard paddle controls are removed. Gamepad
menu controls remain: D-pad navigates, South activates, East closes, Start toggles.
Knobs remain centred if no controller is connected.

### 13.4 CLI flags

| Flag | Effect |
|---|---|
| `--game <1-4>` | initial Game Select position (default 3) |
| `--fullscreen [true\|false]` (default true), `--mute`, `--invert-left [true\|false]`, `--invert-right [true\|false]`, `--wayland` | Knob inversion defaults to true (CW UP); false selects CCW UP |
| `--volume <0-10>` | master volume at start (default 8, §13.6) |
| `--menu-at <frame>`, `--menu-item <entry>`, `--resume-at <frame>` | Hidden (headless testing): open the menu on that rendered frame with the cursor on `entry` (`resume`, `game-mode`, `color`, `crt`, `volume`, `left-direction`, `right-direction`, `exit`), and press `Esc` on another (§13.6) |
| `--display <pixel\|crt>` | start in this display mode (default `crt`, §13.1) |
| `--screenshot <path> --frames <n>`, `--auto-start`, `--demo`, `--knobs <l> <r>` | hidden screenshot/frame flags are internal QA tooling; no screenshot UI or shortcut |
| `--mono` | start in black-and-white |
| `--export-sounds <dir>` | write the four synthesised sounds as WAV files into `dir`, then exit (§12.3) |

- `--demo`: the autopilot plays both sides with per-side offsets that change on simulated time, steps the Solitaire wall between low and middle heights, and presses Start a few seconds after each game over, so headless runs show varied rallies in all four games.
- `--auto-start` presses Start on rendered frame 2. So `--frames 600` at 60 fps shows a rally: the serve comes about 3.4 s after Start.

### 13.5 Tick scheduling

```
accumulator += frame_time.min(0.25);
let mut first = true;
loop {
    let dt = 1.0 / console.frame_hz();        // re-read every tick: the phase may have changed
    if accumulator < dt { break; }
    console.tick(&Input { left, right, start: first && pending_start, select: if first { pending_select.take() } else { None } }, &mut events);
    if first { pending_start = false; }
    first = false;
    accumulator -= dt;
}
// then: play events (§12.2), console.render(&mut frame), draw picture and raster overlay
```

- Start and Select presses are buffered until the next tick, so none is lost in a rendered frame that runs no tick.
- `render` can be called any number of times; it does not change the state.
- While the menu is open or its countdown runs, the scheduler is not run at all (§13.6): no ticks, and the accumulator keeps whatever it held.

### 13.6 RetroPong: pause menu, resume countdown, volume

The program is called **RetroPong**: window title, `--help`, the menu's title and the binary `retropong` (the crates keep their `superpong` names, and game #3 is still SUPER PONG). The C-140 had no pause, no menu and no volume control; everything in this section is .

**Look.** The menu and the countdown are drawn in the console's own raster: the frontend copies the core's frame (§14.3) to a canvas of the same 187 × 240 cells and draws over it in the same cell kinds, so they get the same gradient colours (§3.1), black-and-white, stripe texture and CRT shader as the game. The menu draws nothing outside that picture; there is no HUD.
- **Lettering**: a block font 5 font pixels tall, uppercase only, with square corners and strokes one font pixel thick, like the score glyphs (§9.3). Most glyphs are 3 pixels wide, `I` is 1 and `M`, `N` and `W` are wider. Digits are the score glyphs cut down to 3 × 5 (a `1` is just the right-hand column). A font pixel is 2 clocks × 3 lines for the entries and 3 clocks × 4 lines for the title.
- **Menu screen**: the two border bars, with the net's light spots, stay as the console draws them, and the playfield between them is blanked. Inside it: the title `RETROPONG` centred at the top, then eight entries, labels left, values right:

| Entry | Value | Left / Right | Enter, pad South |
|---|---|---|---|
| `RESUME` | | | closes the menu |
| `GAME MODE` | the selected game's name; below it the Select Game switch: a track (border cells, 3 lines) over the numbers `1 2 3 4`, with a paddle-sized knob (4 clocks × 15 lines) on the selected position, centred on the ink of its number (the `1` is only its right-hand column) | slides the switch one position, stopping at 1 and 4 | steps it on, 4 wraps to 1 |
| `COLOR` | `ON` / `OFF` | toggles colour ↔ black-and-white | toggles |
| `CRT` | `ON` / `OFF`: whether classic video is on screen, so `OFF` while its shader is unavailable | toggles classic video ↔ pixel  | toggles |
| `VOLUME` | ten paddle-wide blocks, the first *volume* lit to full height, the rest only their bottom font pixel; `MUTE` when started muted | volume −1 / +1 within 0-10, unmuting, with a Bounce beep at the new level | |
| `LEFT` | `CCW UP` / `CW UP` | toggle left knob direction independently | toggles |
| `RIGHT` | `CCW UP` / `CW UP` | toggle right knob direction independently | toggles |
| `EXIT GAME` | | | quits the program |

- **Knob direction**: `CW UP` (default) means clockwise moves the paddle up; `CCW UP` means counterclockwise moves it up. Each entry changes only that knob’s inversion; physical controller assignments remain fixed.
- The **cursor** is a paddle (4 clocks × 15 lines), as tall as the entry's letters and left of its label: the player's paddle points at the entry. `Up`/`Down` move it and wrap at the top and bottom. It is on `RESUME` each time the menu opens.
- **Margins**: the cursor is at X 4, the labels at X 11 and the values at X 94, so the widest value (`SUPER PONG`, ending at X 172) is as far from the right edge as the cursor is from the left.
- A **Game Mode** change moves the console's Game Select switch at once, as `Console::select` (§11.2); a Select key press still buffered from before the pause is dropped.

**Pause.** `Esc` (or a pad's Start) opens the menu at any time, over a game or over the light show. While it is open the console is not ticked at all (the light show stops too), the sound playing is cut off, and the `--demo` autopilot does nothing. Arrow keys only navigate the menu. `Esc`, Start, a pad's East or `RESUME` closes it.

**Start and Select while paused.** The console is paused while the menu is open and during the countdown, and then `Enter` are never a console Start (§13.3): in the menu they activate, in the countdown they do nothing, since a Start kept for the first tick after the pause would restart the match at 0-0. A Start still buffered when the menu opens (pressed on the same frame as `Esc`) is dropped, and so is `--auto-start` while paused. A game mode is selected through the menu.

**Countdown.** Closing the menu while a game is in progress (`Waiting` or `Rally`) does not restart the console at once:
- a **3, 2, 1, GO** countdown runs first, 0.5 s per step, timed on rendered-frame time (`--screenshot` runs use the fixed 1/60 s per frame, so a given frame always shows the same number);
- numbers use oversized score glyphs (32 clocks × 96 lines), and GO uses large block letters, centred in a clear panel. The outer field remains visible; central forward paddles can be covered while counting. The ball is redrawn over the panel so it stays visible;
- each number comes with one Bounce sound at the master volume;
- the console stays frozen, but the knobs are live: the picture is the frame the next tick would draw with the current knobs, made on a copy of the console that is then thrown away. The first real tick after the countdown draws that same frame, and the preview stays on screen until a real tick has run (a rendered frame can run none), so the paddles never jump back for a frame;
- `Esc` (or Start) during the countdown opens the menu again, and closing it starts the countdown from 3 again;
- the console ticks again from the rendered frame on which 2 s have passed.

Closing the menu over the light show (`Attract`) resumes at once, with no countdown.

**Volume.** A master volume of 0-10 (default 8; `--volume`) scales every sound's playback gain by `(volume / 10)²`, so each step is a roughly even change in loudness (5 is −12 dB, 1 is −40 dB). 0 is silent. `--mute` starts muted; changing the volume unmutes. The synthesised sounds themselves (§12.3) do not change.

## 14. Core API contract (crate `superpong-core`)

The core decides everything that is drawn: which cells are lit, clipping, blanking, the light-show blocks and the digits. The frontend never re-implements a rule. It only maps cells to colours, applies display options, plays sounds and reads input. The crate stays `std`, dependency-free, `#![forbid(unsafe_code)]` and fully unit-tested.

### 14.1 Types

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameKind { Catch, Solitaire, SuperPong, Pong }   // Game Select 1..=4; keep number(), from_number(), name(), next(), ALL
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side { Left, Right }                              // keep other()

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    pub left: f32,                  // knob 0.0 (top) ..= 1.0 (bottom); NaN = 0.5
    pub right: f32,
    pub start: bool,                // pressed this tick (edge)
    pub select: Option<GameKind>,   // switch moved to this position this tick
}
impl Default for Input { /* left: 0.5, right: 0.5, start: false, select: None */ }

#[derive(Clone, Debug)]
pub struct Console { /* private: kind, phase, scores, knob tops, ball state (§6.1), light-show state (§10.2), frame */ }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BounceKind { Paddle, Wall, Border }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundEvent { Bounce(BounceKind), Point, Serve, AttractTick }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    Attract,   // light show; digits on; no ball drawn
    Waiting,   // game shown, ball hidden (after Start or a point); digits on
    Rally,     // ball visible; digits off
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Cell { Background = 0, Border = 1, Foreground = 2 }

/// The display window rasterised: DISPLAY_W x DISPLAY_H cells, row-major,
/// index = (row - DISPLAY_ROW0) * DISPLAY_W + (x - DISPLAY_X0).
#[derive(Clone, Debug, PartialEq)]
pub struct Frame { cells: Vec<Cell> }
impl Frame {
    pub fn new() -> Frame;                         // all Background
    pub fn get(&self, x: i32, row: i32) -> Cell;   // Background outside the window
    pub fn row(&self, row: i32) -> &[Cell];        // DISPLAY_W cells; panics outside DISPLAY_ROW0..DISPLAY_ROW0+DISPLAY_H
    pub fn cells(&self) -> &[Cell];
    pub fn to_ascii(&self) -> String;              // '.', ':', '#' per cell, '\n' per row; for golden tests
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BallState { pub x: i32, pub row: i32, pub dirx: i32, pub vy: i32, pub visible: bool }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    pub kind: GameKind,
    pub phase: Phase,
    pub left_score: u8,          // Solitaire: the player
    pub right_score: u8,         // Solitaire: the game's misses
    pub digits_visible: bool,    // phase != Rally
    pub ball: Option<BallState>, // None in Attract
    pub left_top: i32,           // knob rows actually in use (0..=209)
    pub right_top: i32,
    pub volleys: u32,
    pub frame: u64,              // ticks since power-on
}
```

### 14.2 Console

```rust
impl Console {
    pub fn new(kind: GameKind) -> Console;      // power-on: Attract, scores 0-0, light show §10.4, knobs at 0.5
    pub fn tick(&mut self, input: &Input, events: &mut Vec<SoundEvent>);   // exactly one console frame (§11.3)
    pub fn frame_hz(&self) -> f64;              // FRAME_HZ_ATTRACT in Attract, else FRAME_HZ_PLAY
    pub fn render(&self, frame: &mut Frame);    // §14.3; pure, overwrites every cell
    pub fn status(&self) -> Status;
    pub fn kind(&self) -> GameKind;
    pub fn phase(&self) -> Phase;
    pub fn scores(&self) -> (u8, u8);
    pub fn select(&mut self, kind: GameKind);   // same as Input::select, applied immediately
    pub fn press_start(&mut self);              // same as Input::start, applied immediately
    #[doc(hidden)] pub fn debug_set_ball(&mut self, ball: BallState, volleys: u32);   // tests: sets the play state (Rally if visible, else Waiting)
}
pub fn knob_to_top(k: f32) -> i32;
pub fn knob_for_top(top: i32) -> f32;
```

### 14.3 `render` (normative)

```
fill every cell with Background
for row in 0..16 and 208..224, x in 0..176: Border
fg(x, row) = if 0 <= row < 224 && 0 <= x < 176 { Foreground }       // only these cells can ever be lit
net: x in 88..92, row in 0..224 with row % 16 < 8
objects of self.kind at top_l / top_r (§4.2), every phase:
    paddle: x..x+4 by top..top+15
    Catch wall: x..x+4 by (0..224 minus hole..hole+15)
    Solitaire wall: 160..164 by top..224
if phase == Rally: ball x..x+2 by row..row+4
if phase != Rally: digits (§9) for both scores
if phase == Attract: every (x, row) with block(x, row) (§10.3)
```

Everything lit is OR'ed into `Foreground`, so the drawing order does not matter.

### 14.4 Constants (all `pub const` in `superpong-core`)

| Name | Value |
|---|---|
| `FIELD_W`, `FIELD_H` | 176, 224 |
| `TOP_BORDER` / `BOTTOM_BORDER` (rows) | 0..16 / 208..224 |
| `DISPLAY_X0`, `DISPLAY_W` | −6, 187 |
| `DISPLAY_ROW0`, `DISPLAY_H` | −8, 240 |
| `FRAME_HZ_PLAY`, `FRAME_HZ_ATTRACT` | 59.94, 60.19 |
| `NET_X`, `NET_W`, net dash | 88, 4, `row % 16 < 8` |
| `PADDLE_W`, `PADDLE_H`, `HOLE_H` | 4, 15, 15 |
| `LEFT_BACK_X`, `LEFT_FORWARD_X`, `RIGHT_FORWARD_X`, `RIGHT_BACK_X` | 16, 80, 96, 160 |
| `BALL_W`, `BALL_HIT_W`, `BALL_H` | 2, 3, 4 |
| `TOP_MAX`, `KNOB_STEPS` | 209, 210 |
| tier thresholds; `VX_BY_TIER` | 4, 8; [1, 2, 3] |
| `SEGMENT_VY`; wall segment period | [−3, −2, −1, 0, 0, 1, 2, 3]; 16 |
| `CATCH_WALL_VY` | −3 above the hole top, +3 otherwise |
| top bounce / bottom bounce | `−6 ≤ row ≤ 15` moving up (`TOP_BOUNCE_ROW_MIN`, `TOP_BOUNCE_ROW`) / `row + 3 ≥ 208` moving down |
| live columns (borders, light-show dead zone) | x in 0..=175 |
| exit | centre `x + 1 ≥ 176` moving right / `≤ 0` moving left |
| `HIDDEN_X_MIN`, `HIDDEN_WRAP_RIGHT`, `HIDDEN_WRAP_LEFT` | −25, 228, 227 |
| `ROW_MIN`, `PLAY_LINES` | −40, 263 |
| `SERVE_X_RIGHT`, `SERVE_X_LEFT` | 87, 90 |
| `START_X` | 112 (serve on tick S + 202) |
| `WIN_SCORE` | 15 |
| `DIGIT_ROW0`, `DIGIT_H` | 32, 64 |
| `LEFT_UNITS_X`, `LEFT_TENS_X`, `RIGHT_UNITS_X`, `RIGHT_TENS_X` | 40, 24, 136, 120 |
| `GLYPHS` | §9.3 |
| `LS_H`, `LS_LINES` | 228, 262 |
| light-show dead zone (`h0`) | 172..=223 |
| light-show top / bottom zones (wrap line) | `l ≤ 17 \|\| l ≥ 258` / `207 ≤ l ≤ 225` |
| score band (no blocks) | rows 32..=95 |
| `LS_BALL_X_OFFSET`, `LS_BALL_ROW_OFFSET` | 4, 2 |
| `LS_GAME_OVER_X_OFFSET`, `LS_GAME_OVER_ROW_OFFSET` | 0, 8 |
| `ATTRACT_TICK_H0` | 82 |
| `POWER_ON_H0`, `POWER_ON_WRAP_LINE`, `POWER_ON_VY` | 92, 177, +1 |

## 15. Not reproduced, and open questions

**Unit-specific faults, not reproduced:**
- left paddle 14 lines and right paddle 16 lines ;
- paddle jitter of ±1 line ;
- spontaneous score resets ;
- objects vanishing past the knob range (§5);
- Pong-layout frames during Game Select (§11.2).

**Capture or analogue artefacts, not reproduced:**
- the two-frame blend and capture tear;
- content-dependent levels;
- the dark green, blue and maroon wash on the background (the game's background is black, §3.1);
- chroma bleed at object edges (CRT mode applies a generic composite-style chroma blur, not fitted to the video, §3.2);
- the 1/4-clock border offset.

**Open questions, where the spec makes a choice:**
- why the hidden wrap differs by direction (§6.9; encoded as measured);
- the light-show zones inside vertical blanking (§10.3), which the hidden ball's top border shares (§6.3, n = 1);
- the attract-ball offsets (§10.2), and why the game-over phase differs from them (§10.6, n = 1);
- Start's effect on the hidden ball's horizontal phase (§11.1);
- the Solitaire flat-basket vy change (§6.8);
- the volley count across Game Select (§7);
- the segment boundary on the left paddle, ±1 line ;
- light-show content for games 2 and 3 (§10.1);
- a corner hit in the same frame as a border flip (§6.5).
