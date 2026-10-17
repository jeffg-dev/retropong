//! Headless, deterministic simulation of the four games of the Atari
//! **Super Pong** console (model C-140, 1976): Catch, Solitaire, Super Pong
//! and Pong.
//!
//! The behavioural specification lives in `docs/SPEC-superpong.md` at the
//! repository root; this crate implements it and nothing else: no rendering
//! colours, no audio, no input devices. The model is the console's own: one
//! [`Console::tick`] is one video frame, and the core rasterises the whole
//! display window into a [`Frame`] of [`Cell`]s with [`Console::render`].
//! The frontend (`superpong`) feeds knob values and button presses into
//! `tick` at [`Console::frame_hz`], maps cells to colours and plays the
//! [`SoundEvent`]s that `tick` reports.
//!
//! Positions are integer clocks (x) and scan lines (rows), as measured on
//! the real unit. Everything is deterministic: there is no random number
//! generator, and the same inputs always give the same frames.
//!
//! The ball and the games live in `play` (spec §6-§8 and §11.1), the light
//! show in `light_show` (spec §10) and the rasteriser in `raster` (spec §9
//! and §14.3).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod light_show;
mod play;
mod raster;

#[cfg(test)]
mod tests;

use light_show::LightShow;
use play::Ball;
pub use raster::{Cell, Frame};

/// Width of the border box in clocks (x in `0..FIELD_W`).
pub const FIELD_W: i32 = 176;
/// Height of the border box in lines (rows in `0..FIELD_H`).
pub const FIELD_H: i32 = 224;
/// Rows covered by the top border.
pub const TOP_BORDER: std::ops::Range<i32> = 0..16;
/// Rows covered by the bottom border.
pub const BOTTOM_BORDER: std::ops::Range<i32> = 208..224;
/// Leftmost x of the display window (overscan).
pub const DISPLAY_X0: i32 = -6;
/// Width of the display window in columns.
pub const DISPLAY_W: i32 = 187;
/// Top row of the display window (overscan).
pub const DISPLAY_ROW0: i32 = -8;
/// Height of the display window in lines.
pub const DISPLAY_H: i32 = 240;

/// Frame rate while a game is shown (`Waiting` and `Rally`).
pub const FRAME_HZ_PLAY: f64 = 59.94;
/// Frame rate of the light show (`Attract`).
pub const FRAME_HZ_ATTRACT: f64 = 60.19;

/// Left x of the net.
pub const NET_X: i32 = 88;
/// Net thickness; it is dashed 8 lines on, 8 off (`row % 16 < 8`).
pub const NET_W: i32 = 4;
/// Paddle (and wall) thickness.
pub const PADDLE_W: i32 = 4;
/// Paddle height in lines.
pub const PADDLE_H: i32 = 15;
/// Height of a Catch hole in lines.
pub const HOLE_H: i32 = 15;
/// Left x of the left back paddle, Catch wall and Solitaire paddle.
pub const LEFT_BACK_X: i32 = 16;
/// Left x of the left Super Pong Forward paddle.
pub const LEFT_FORWARD_X: i32 = 80;
/// Left x of the right Super Pong Forward paddle.
pub const RIGHT_FORWARD_X: i32 = 96;
/// Left x of the right back paddle, Catch wall and Solitaire back wall.
pub const RIGHT_BACK_X: i32 = 160;
/// Drawn width of the ball in clocks.
pub const BALL_W: i32 = 2;
/// Collision width of the ball: one extra column on the right.
pub const BALL_HIT_W: i32 = 3;
/// Height of the ball in lines.
pub const BALL_H: i32 = 4;

/// Largest paddle top a knob can give.
pub const TOP_MAX: i32 = 209;
/// Number of distinct knob positions.
pub const KNOB_STEPS: i32 = 210;

/// Volley counts at which the ball speeds up to the second and third tier.
pub const TIER_THRESHOLDS: [u32; 2] = [4, 8];
/// Horizontal speed in clocks per frame for each speed tier.
pub const VX_BY_TIER: [i32; 3] = [1, 2, 3];
/// Outgoing vertical speed for each of the eight paddle segments, top first.
pub const SEGMENT_VY: [i32; 8] = [-3, -2, -1, 0, 0, 1, 2, 3];
/// Period of the segment pattern on a Solitaire wall, in lines.
pub const WALL_SEGMENT_PERIOD: i32 = 16;
/// Outgoing vertical speed off a Catch wall above the hole top.
pub const CATCH_WALL_VY_ABOVE: i32 = -3;
/// Outgoing vertical speed off a Catch wall below the hole top.
pub const CATCH_WALL_VY_BELOW: i32 = 3;
/// The ball turns around at the top border when `row <= TOP_BOUNCE_ROW` moving up ...
pub const TOP_BOUNCE_ROW: i32 = 15;
/// ... and `row >= TOP_BOUNCE_ROW_MIN`: the light show's top zone (wrap
/// lines `LS_TOP_ZONE_WRAP_MIN..LS_LINES` and `0..=LS_TOP_ZONE_MAX`) in
/// rows. A hidden ball further up passes through vertical blanking.
pub const TOP_BOUNCE_ROW_MIN: i32 = LS_TOP_ZONE_WRAP_MIN - LS_LINES - LS_BALL_ROW_OFFSET;
/// The ball turns around at the bottom border when `row + 3 >= BOTTOM_BOUNCE_LINE` moving down.
pub const BOTTOM_BOUNCE_LINE: i32 = 208;
/// Columns in which borders and the light-show dead zone apply: `0..=LIVE_X_MAX`.
pub const LIVE_X_MAX: i32 = 175;
/// Leftmost x of the hidden ball.
pub const HIDDEN_X_MIN: i32 = -25;
/// Horizontal wrap length of the hidden ball moving right.
pub const HIDDEN_WRAP_RIGHT: i32 = 228;
/// Horizontal wrap length of the hidden ball moving left.
pub const HIDDEN_WRAP_LEFT: i32 = 227;
/// Topmost row of the hidden ball.
pub const ROW_MIN: i32 = -40;
/// Lines per frame, for the hidden ball's vertical wrap.
pub const PLAY_LINES: i32 = 263;
/// Column at which a rightward hidden ball becomes visible.
pub const SERVE_X_RIGHT: i32 = 87;
/// Column at which a leftward hidden ball becomes visible.
pub const SERVE_X_LEFT: i32 = 90;
/// Column of the hidden ball when Start is pressed; the serve follows 202 frames later.
pub const START_X: i32 = 112;
/// Score that wins the game.
pub const WIN_SCORE: u8 = 15;

/// First row of the score digits.
pub const DIGIT_ROW0: i32 = 32;
/// Height of the score digits in lines.
pub const DIGIT_H: i32 = 64;
/// Left x of the left score's units cell.
pub const LEFT_UNITS_X: i32 = 40;
/// Left x of the left score's tens bar.
pub const LEFT_TENS_X: i32 = 24;
/// Left x of the right score's units cell.
pub const RIGHT_UNITS_X: i32 = 136;
/// Left x of the right score's tens bar.
pub const RIGHT_TENS_X: i32 = 120;

/// Digit glyphs on a 4 x 8 grid; bit 3 is the leftmost grid column.
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

/// Clocks per line seen by the light-show counter.
pub const LS_H: i32 = 228;
/// Lines per frame in attract mode.
pub const LS_LINES: i32 = 262;
/// Light-show dead zone: while `h0` is in this range the invisible ball is
/// outside x `0..=LIVE_X_MAX`, and the light show does not bounce.
pub const LS_DEAD_ZONE: std::ops::RangeInclusive<i32> = 172..=223;
/// Light-show V wrap line is in the top zone when `l <= LS_TOP_ZONE_MAX` ...
pub const LS_TOP_ZONE_MAX: i32 = 17;
/// ... or `l >= LS_TOP_ZONE_WRAP_MIN` (the wrapped part of the top zone).
pub const LS_TOP_ZONE_WRAP_MIN: i32 = 258;
/// Light-show V wrap line is in the bottom zone when it lies in this range.
pub const LS_BOTTOM_ZONE: std::ops::RangeInclusive<i32> = 207..=225;
/// Rows of the score band, in which the light show draws no blocks.
pub const LS_SCORE_BAND: std::ops::RangeInclusive<i32> = 32..=95;
/// Offset from the light-show `h0` to the invisible ball's left column.
pub const LS_BALL_X_OFFSET: i32 = 4;
/// Offset from the light-show wrap line to the invisible ball's top row.
pub const LS_BALL_ROW_OFFSET: i32 = 2;
/// At game over the light show starts with `h0 = x - LS_GAME_OVER_X_OFFSET`
/// (measured; not the attract mapping `LS_BALL_X_OFFSET`).
pub const LS_GAME_OVER_X_OFFSET: i32 = 0;
/// At game over the light show's V wrap falls on `row + LS_GAME_OVER_ROW_OFFSET`
/// (measured; not the attract mapping `LS_BALL_ROW_OFFSET`).
pub const LS_GAME_OVER_ROW_OFFSET: i32 = 8;
/// The attract tick sounds when `h0` equals this.
pub const ATTRACT_TICK_H0: i32 = 82;
/// Light-show `h0` at power-on.
pub const POWER_ON_H0: i32 = 92;
/// Light-show V wrap line of the power-on frame.
pub const POWER_ON_WRAP_LINE: i32 = 177;
/// Light-show `vy` at power-on.
pub const POWER_ON_VY: i32 = 1;

/// Largest vertical speed of the ball, in lines per frame.
const VY_MAX: i32 = 3;

/// The four games, numbered 1..4 like the Game Select switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GameKind {
    /// Game #1: catch the ball through your hole.
    Catch,
    /// Game #2: one player against the back wall.
    Solitaire,
    /// Game #3: two paddles per player.
    SuperPong,
    /// Game #4: classic Pong.
    Pong,
}

impl GameKind {
    /// All games in switch order.
    pub const ALL: [GameKind; 4] =
        [GameKind::Catch, GameKind::Solitaire, GameKind::SuperPong, GameKind::Pong];

    /// Game Select position, 1..=4.
    pub fn number(self) -> u8 {
        match self {
            GameKind::Catch => 1,
            GameKind::Solitaire => 2,
            GameKind::SuperPong => 3,
            GameKind::Pong => 4,
        }
    }

    /// Inverse of [`GameKind::number`].
    pub fn from_number(n: u8) -> Option<GameKind> {
        GameKind::ALL.get(n.checked_sub(1)? as usize).copied()
    }

    /// Display name as printed on the console.
    pub fn name(self) -> &'static str {
        match self {
            GameKind::Catch => "CATCH",
            GameKind::Solitaire => "SOLITAIRE",
            GameKind::SuperPong => "SUPER PONG",
            GameKind::Pong => "PONG",
        }
    }

    /// Next position on the switch, wrapping 4 -> 1.
    pub fn next(self) -> GameKind {
        GameKind::from_number(self.number() % 4 + 1).unwrap()
    }
}

/// One side of the playfield / one player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// Left player.
    Left,
    /// Right player or, in Solitaire, the back wall.
    Right,
}

impl Side {
    /// The other side.
    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

/// Per-tick input to the console.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// Left knob, `0.0` (top) ..= `1.0` (bottom); NaN reads as 0.5.
    pub left: f32,
    /// Right knob, `0.0` (top) ..= `1.0` (bottom); NaN reads as 0.5.
    pub right: f32,
    /// Start was pressed this tick (an edge, not a level).
    pub start: bool,
    /// The Game Select switch moved to this position this tick.
    pub select: Option<GameKind>,
}

impl Default for Input {
    fn default() -> Input {
        Input { left: 0.5, right: 0.5, start: false, select: None }
    }
}

/// What a ball reflected off, for sound sets that tell reflections apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BounceKind {
    /// A back or forward paddle.
    Paddle,
    /// A Catch wall or the Solitaire back wall.
    Wall,
    /// The top or bottom border.
    Border,
}

/// A sound the frontend should play, reported by [`Console::tick`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SoundEvent {
    /// The visible ball was reflected.
    Bounce(BounceKind),
    /// The ball left the playfield: a point, including the winning one.
    Point,
    /// The hidden ball became visible.
    Serve,
    /// The light show's once-per-cycle tick.
    AttractTick,
}

/// What the console is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Light show; digits on; no ball drawn.
    Attract,
    /// Game shown, ball hidden (after Start or a point); digits on.
    Waiting,
    /// Ball visible; digits off.
    Rally,
}

/// The ball's state as the console tracks it (spec §6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BallState {
    /// Left column.
    pub x: i32,
    /// Top row.
    pub row: i32,
    /// `+1` moving right, `-1` moving left.
    pub dirx: i32,
    /// Lines per frame, `-3..=3`, positive down.
    pub vy: i32,
    /// Whether the ball is drawn and collides.
    pub visible: bool,
}

/// A snapshot of the console for the frontend's HUD and for tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    /// Selected game.
    pub kind: GameKind,
    /// Current phase.
    pub phase: Phase,
    /// Left score (Solitaire: the player).
    pub left_score: u8,
    /// Right score (Solitaire: the game's misses).
    pub right_score: u8,
    /// Whether the digits are drawn (`phase != Rally`).
    pub digits_visible: bool,
    /// The ball; `None` in `Attract`.
    pub ball: Option<BallState>,
    /// Left knob row in use, `0..=209`.
    pub left_top: i32,
    /// Right knob row in use, `0..=209`.
    pub right_top: i32,
    /// Reflections since the ball last entered play.
    pub volleys: u32,
    /// Ticks since power-on.
    pub frame: u64,
}

/// The whole console: selected game, phase, scores, knobs, ball and light show.
#[derive(Clone, Debug)]
pub struct Console {
    kind: GameKind,
    phase: Phase,
    scores: (u8, u8),
    left_top: i32,
    right_top: i32,
    ball: Ball,
    volleys: u32,
    ls: LightShow,
    frame: u64,
}

impl Console {
    /// A console at power-on: `Attract`, scores 0-0, the light show in its
    /// measured power-on state (spec §10.4) and both knobs centred. The
    /// power-on frame is already simulated, so it can be rendered before
    /// the first tick.
    pub fn new(kind: GameKind) -> Console {
        let mid = knob_to_top(0.5);
        Console {
            kind,
            phase: Phase::Attract,
            scores: (0, 0),
            left_top: mid,
            right_top: mid,
            // Hidden until Start places it (spec §11.1).
            ball: Ball { x: START_X, row: 0, dirx: 1, vy: 0 },
            volleys: 0,
            ls: LightShow::power_on(),
            frame: 0,
        }
    }

    /// Advances the console by exactly one frame (spec §11.3), pushing the
    /// sounds it makes onto `events` in order.
    pub fn tick(&mut self, input: &Input, events: &mut Vec<SoundEvent>) {
        self.left_top = knob_to_top(input.left);
        self.right_top = knob_to_top(input.right);
        if let Some(kind) = input.select {
            self.select(kind);
        }
        if input.start {
            self.press_start();
        }
        match self.phase {
            Phase::Attract => self.ls.attract_frame(events),
            Phase::Waiting | Phase::Rally => self.play_frame(events),
        }
        self.frame += 1;
    }

    /// Frame rate of the current phase: the frontend ticks at this rate.
    pub fn frame_hz(&self) -> f64 {
        match self.phase {
            Phase::Attract => FRAME_HZ_ATTRACT,
            Phase::Waiting | Phase::Rally => FRAME_HZ_PLAY,
        }
    }

    /// Rasterises the console into `frame`, overwriting every cell (spec §14.3).
    pub fn render(&self, frame: &mut Frame) {
        raster::draw(self, frame);
    }

    /// A snapshot of the console.
    pub fn status(&self) -> Status {
        Status {
            kind: self.kind,
            phase: self.phase,
            left_score: self.scores.0,
            right_score: self.scores.1,
            digits_visible: self.phase != Phase::Rally,
            ball: (self.phase != Phase::Attract).then_some(BallState {
                x: self.ball.x,
                row: self.ball.row,
                dirx: self.ball.dirx,
                vy: self.ball.vy,
                visible: self.phase == Phase::Rally,
            }),
            left_top: self.left_top,
            right_top: self.right_top,
            volleys: self.volleys,
            frame: self.frame,
        }
    }

    /// The selected game.
    pub fn kind(&self) -> GameKind {
        self.kind
    }

    /// The current phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// `(left, right)` scores.
    pub fn scores(&self) -> (u8, u8) {
        self.scores
    }

    /// Moves the Game Select switch; same as [`Input::select`] but immediate.
    ///
    /// Only the playfield changes (spec §11.2): the ball, `volleys`, phase
    /// and scores carry over, and in `Attract` only the objects drawn over
    /// the light show change.
    pub fn select(&mut self, kind: GameKind) {
        self.kind = kind;
    }

    /// Presses Start; same as [`Input::start`] but immediate.
    ///
    /// Restarts the selected game in any phase (spec §11.1): scores 0-0 and
    /// the hidden ball on its way to a rightward serve 202 frames later.
    pub fn press_start(&mut self) {
        self.start_game();
    }

    /// Test hook: sets the ball and volley count, entering `Rally` if the
    /// ball is visible and `Waiting` otherwise. `dirx` is reduced to its
    /// sign (0 counts as right) and `vy` is clamped to `-3..=3`.
    #[doc(hidden)]
    pub fn debug_set_ball(&mut self, ball: BallState, volleys: u32) {
        self.ball = Ball {
            x: ball.x,
            row: ball.row,
            dirx: if ball.dirx < 0 { -1 } else { 1 },
            vy: ball.vy.clamp(-VY_MAX, VY_MAX),
        };
        self.volleys = volleys;
        self.phase = if ball.visible { Phase::Rally } else { Phase::Waiting };
    }
}

fn sanitize(k: f32) -> f32 {
    if k.is_nan() {
        0.5
    } else {
        k.clamp(0.0, 1.0)
    }
}

/// Maps a knob position (`0.0` = top) to a paddle top row, `0..=`[`TOP_MAX`].
pub fn knob_to_top(k: f32) -> i32 {
    TOP_MAX.min((sanitize(k) * KNOB_STEPS as f32).floor() as i32)
}

/// Exact inverse of [`knob_to_top`], for demos and tests.
pub fn knob_for_top(top: i32) -> f32 {
    (top.clamp(0, TOP_MAX) as f32 + 0.5) / KNOB_STEPS as f32
}
