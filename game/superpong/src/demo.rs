//! The `--demo` autopilot (SPEC §13.4): plays both sides of whichever game
//! is selected, so rallies, points and game overs happen with no controller,
//! and presses Start a few seconds after every game over.
//!
//! The core is deterministic, so a plain "follow the ball" demo falls into
//! one fixed loop (a flat basket every serve, forwards that only ever hit
//! each other, a hole the ball always flies through). Instead every
//! reflection is a decision: where on the paddle the ball is met, which
//! paddle of a Super Pong player takes it, and after how many reflections a
//! rally ends and which side lets it through. Those decisions come from a
//! hash of the score, the reflection count and the console's own tick
//! counter, never from rendered frames, so a given console tick always
//! gets the same decision whatever the display's refresh rate.
//!
//! [`knobs`] is a pure function of the console's [`Status`]; only the Start
//! timer in [`Demo`] carries state.

use superpong_core::{
    knob_for_top, BallState, GameKind, Input, Phase, Side, Status, BALL_HIT_W, LEFT_FORWARD_X, PADDLE_W,
    RIGHT_FORWARD_X, TIER_THRESHOLDS, TOP_MAX, VX_BY_TIER,
};

/// Consecutive light-show ticks before the demo presses Start (about 3 s).
const RESTART_TICKS: u32 = 180;

/// Ticks each aim and rally-length step lasts (about 2 s), so rallies vary
/// over simulated time and not only with the score.
const STEP_TICKS: u64 = 120;
/// Ticks each Solitaire back-wall height lasts (about 3 s).
const WALL_STEP_TICKS: u64 = 180;

/// The back wall's heights (its top row), stepped through in turn: low on
/// the screen, then up to the middle.
const WALL_TOPS: [i32; 6] = [165, 125, 105, 150, 115, 170];

/// Where on a paddle the ball is met, as ball top row minus paddle top. The
/// segment (two rows each, top first, spec §6.4) gives the outgoing vy:
/// -3, -2, -1, +1, +2, +3 for these. The flat middle segments are left out
/// on purpose, so no rally settles into a flat loop.
const AIMS: [i32; 6] = [0, 2, 4, 10, 12, 14];

/// How far from the ball a paddle sits to let it pass, in rows. The paddle
/// is 15 and the ball 4 rows tall, so nothing within 18 rows overlaps.
const MISS_DISTANCE: i32 = 30;

/// A knob left alone, in the middle.
const REST: f32 = 0.5;

/// The autopilot: [`knobs`] plus the timer that restarts a finished game.
#[derive(Debug, Default)]
pub struct Demo {
    /// Consecutive ticks seen in `Attract`.
    attract_ticks: u32,
}

impl Demo {
    pub fn new() -> Self {
        Self::default()
    }

    /// The input for the next tick given the console as it stands: the
    /// knobs from [`knobs`], and Start once the light show has run for
    /// [`RESTART_TICKS`] ticks.
    pub fn input(&mut self, status: &Status) -> Input {
        let (left, right) = knobs(status);
        let mut start = false;
        if status.phase == Phase::Attract {
            self.attract_ticks += 1;
            if self.attract_ticks >= RESTART_TICKS {
                start = true;
                self.attract_ticks = 0;
            }
        } else {
            self.attract_ticks = 0;
        }
        Input { left, right, start, select: None }
    }
}

/// SplitMix64's finalizer: a cheap, well-spread hash of `n`.
fn mix(n: u64) -> u64 {
    let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A hash of the things a decision may depend on, in `salt`'s own stream.
fn pick(salt: u64, parts: &[u64]) -> u64 {
    parts.iter().fold(mix(salt), |h, &p| mix(h ^ p))
}

/// The Solitaire back wall's top row at console tick `tick`.
fn wall_rest(tick: u64) -> i32 {
    WALL_TOPS[(tick / WALL_STEP_TICKS % WALL_TOPS.len() as u64) as usize]
}

/// The paddle top at which the ball (top row `row`) misses it by a wide
/// margin, on whichever side has more room.
fn away(row: i32) -> i32 {
    if row < 112 {
        row + MISS_DISTANCE
    } else {
        row - MISS_DISTANCE
    }
}

/// Reflections after which a rally of this game is let through, which
/// changes with the score and every 30 s of console time. Catch walls only
/// ever send the ball up or down at 3, so its rallies are kept short.
fn rally_length(kind: GameKind, status: &Status) -> u32 {
    let (min, span) = match kind {
        GameKind::Catch => (1, 6),
        GameKind::Solitaire => (2, 12),
        GameKind::SuperPong | GameKind::Pong => (3, 12),
    };
    let rally = status.left_score as u64 + status.right_score as u64;
    let h = pick(kind.number() as u64, &[rally, status.frame / (STEP_TICKS * 15)]);
    min + (h % span) as u32
}

/// The knob positions `(left, right)` for the next tick (see the module
/// docs). `status.frame` is the console's tick count, the only clock used.
///
/// With the ball hidden both knobs rest in the middle (Solitaire's back
/// wall at its stepped height). Otherwise the player the ball is flying
/// towards receives it and its paddles are placed for the *next* tick,
/// where the ball will be after this tick's move, so the hit lands exactly
/// where [`AIMS`] says. The other player's paddles just follow the ball.
pub fn knobs(status: &Status) -> (f32, f32) {
    let kind = status.kind;
    let wall = wall_rest(status.frame);
    let Some(ball) = status.ball.filter(|b| b.visible) else {
        let right = if kind == GameKind::Solitaire { knob_for_top(wall) } else { REST };
        return (REST, right);
    };

    let turn = Turn::new(status, ball);
    let follow = turn.row - 6;
    let (left, right) = match turn.receiver {
        Side::Left => (turn.paddle_top(kind), if kind == GameKind::Solitaire { wall } else { follow }),
        Side::Right if kind == GameKind::Solitaire => (follow, turn.wall_top(wall)),
        Side::Right => (follow, turn.paddle_top(kind)),
    };
    (knob_for_top(left.clamp(0, TOP_MAX)), knob_for_top(right.clamp(0, TOP_MAX)))
}

/// What the next tick will see, and what the receiving player has decided.
struct Turn {
    /// The player the ball is flying towards.
    receiver: Side,
    /// The ball's left column and top row after the next move.
    x: i32,
    row: i32,
    /// The rally ends on this reflection: the receiver lets the ball by.
    ends: bool,
    /// Where on the paddle to meet the ball, from [`AIMS`].
    aim: i32,
    /// Super Pong: the receiver's forward paddle takes the ball rather than
    /// letting it by to the back paddle.
    forward: bool,
}

impl Turn {
    fn new(status: &Status, ball: BallState) -> Turn {
        let speed = VX_BY_TIER[TIER_THRESHOLDS.iter().filter(|&&t| status.volleys >= t).count()];
        let rally = status.left_score as u64 + status.right_score as u64;
        let step = status.frame / STEP_TICKS;
        let h = pick(status.kind.number() as u64, &[rally, status.volleys as u64, step]);
        Turn {
            receiver: if ball.dirx < 0 { Side::Left } else { Side::Right },
            x: ball.x + ball.dirx * speed,
            row: ball.row + ball.vy,
            ends: status.volleys >= rally_length(status.kind, status),
            aim: AIMS[(h % AIMS.len() as u64) as usize],
            forward: (h >> 32).is_multiple_of(2),
        }
    }

    /// The receiver's paddle top. A Catch wall's "paddle" is its hole.
    fn paddle_top(&self, kind: GameKind) -> i32 {
        match kind {
            GameKind::Catch => self.hole_top(),
            GameKind::Solitaire | GameKind::Pong => self.back_paddle_top(),
            GameKind::SuperPong => {
                // The forward paddle is in the ball's way for a few ticks;
                // a player that wants its back paddle holds clear of the
                // ball until the ball is past.
                let x = match self.receiver {
                    Side::Left => LEFT_FORWARD_X,
                    Side::Right => RIGHT_FORWARD_X,
                };
                let in_the_way = (x - BALL_HIT_W + 1..x + PADDLE_W).contains(&self.x);
                if in_the_way && !self.forward {
                    away(self.row)
                } else {
                    self.back_paddle_top()
                }
            }
        }
    }

    /// Meet the ball at the aimed spot, or miss it if the rally ends here.
    fn back_paddle_top(&self) -> i32 {
        if self.ends {
            away(self.row)
        } else {
            self.row - self.aim
        }
    }

    /// A Catch hole: round the ball when the rally ends (a top 2 to 9 rows
    /// above the ball's keeps all four ball rows inside the 15-row hole),
    /// else clear of it on one side so the wall sends the ball back: up
    /// from a hole below it, down from one above.
    fn hole_top(&self) -> i32 {
        if self.ends {
            self.row - 2 - self.aim / 2
        } else if self.aim < 6 {
            self.row + 8
        } else {
            self.row - 25
        }
    }

    /// Solitaire's back wall: stays at its stepped height `wall`, or rises
    /// to meet a ball that would clear it, with the aim deciding where on
    /// the wall's segments the ball lands. When the rally ends, the wall
    /// drops out of the way and the ball is a basket.
    fn wall_top(&self, wall: i32) -> i32 {
        if self.ends {
            TOP_MAX
        } else {
            wall.min(self.row - self.aim)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::Scheduler;
    use std::collections::BTreeSet;
    use superpong_core::{knob_to_top, BounceKind, Console, SoundEvent, FIELD_W};

    fn status(kind: GameKind, ball: Option<BallState>, frame: u64) -> Status {
        Status {
            kind,
            phase: if ball.is_some() { Phase::Rally } else { Phase::Attract },
            left_score: 0,
            right_score: 0,
            digits_visible: ball.is_none(),
            ball,
            left_top: 0,
            right_top: 0,
            volleys: 0,
            frame,
        }
    }

    fn ball(x: i32, row: i32, dirx: i32, vy: i32) -> Option<BallState> {
        Some(BallState { x, row, dirx, vy, visible: true })
    }

    fn tops(status: &Status) -> (i32, i32) {
        let (l, r) = knobs(status);
        (knob_to_top(l), knob_to_top(r))
    }

    #[test]
    fn with_the_ball_hidden_both_knobs_rest() {
        assert_eq!(knobs(&status(GameKind::Pong, None, 5)), (0.5, 0.5));
        let hidden = Some(BallState { visible: false, ..ball(100, 100, 1, 0).unwrap() });
        assert_eq!(knobs(&status(GameKind::Catch, hidden, 5)), (0.5, 0.5));
    }

    #[test]
    fn solitaire_wall_steps_between_low_and_middle_heights_over_ticks() {
        let at = |tick| tops(&status(GameKind::Solitaire, None, tick)).1;
        let seen: Vec<i32> = (0..WALL_TOPS.len() as u64).map(|i| at(i * WALL_STEP_TICKS)).collect();
        assert_eq!(seen, WALL_TOPS);
        // Constant within a step, then it moves, and wraps round.
        assert_eq!(at(WALL_STEP_TICKS - 1), WALL_TOPS[0]);
        assert_ne!(at(WALL_STEP_TICKS), at(WALL_STEP_TICKS - 1));
        assert_eq!(at(WALL_STEP_TICKS * WALL_TOPS.len() as u64), WALL_TOPS[0]);
        // Low on the screen to the middle of it.
        assert!(WALL_TOPS.iter().all(|&t| (100..=170).contains(&t)));
        // The left paddle rests in the middle.
        assert_eq!(tops(&status(GameKind::Solitaire, None, 0)).0, knob_to_top(0.5));
    }

    #[test]
    fn the_receiver_meets_the_ball_where_the_aim_says() {
        // A Pong ball flying left: the left paddle is placed so that, after
        // the ball's move to row 100 + vy, ball row - paddle top is an aim.
        for vy in -3..=3 {
            let s = status(GameKind::Pong, ball(60, 100, -1, vy), 0);
            let aim = 100 + vy - tops(&s).0;
            assert!(AIMS.contains(&aim), "vy {vy}: aim {aim}");
        }
    }

    #[test]
    fn the_receiver_expects_the_ball_where_its_speed_tier_takes_it() {
        // Flying right from x 91: the next position is 92, 93 or 94 at
        // tiers 1, 2 and 3 (volleys 0, 4 and 8).
        let next_x = |volleys| {
            let s = Status { volleys, ..status(GameKind::SuperPong, ball(91, 100, 1, 0), 0) };
            Turn::new(&s, s.ball.unwrap()).x
        };
        assert_eq!((next_x(0), next_x(4), next_x(8)), (92, 93, 94));
    }

    #[test]
    fn the_player_the_ball_is_leaving_just_follows_it() {
        let s = status(GameKind::Pong, ball(60, 100, -1, 0), 0);
        assert_eq!(tops(&s).1, 100 - 6);
        let s = status(GameKind::Pong, ball(60, 100, 1, 2), 0);
        assert_eq!(tops(&s).0, 100 + 2 - 6);
    }

    #[test]
    fn aims_change_with_console_ticks_and_not_within_a_step() {
        let at = |frame| tops(&status(GameKind::Pong, ball(60, 100, -1, 0), frame)).0;
        let aims: BTreeSet<i32> = (0..AIMS.len() as u64 * 8).map(|i| at(i * STEP_TICKS)).collect();
        assert!(aims.len() >= 4, "{aims:?}");
        for step in 0..8 {
            assert_eq!(at(step * STEP_TICKS), at((step + 1) * STEP_TICKS - 1), "step {step}");
        }
    }

    #[test]
    fn the_rally_ends_with_the_receiver_missing() {
        let mut s = status(GameKind::Pong, ball(60, 100, -1, 0), 0);
        s.volleys = 100;
        let left = tops(&s).0;
        // No ball row in 100..104 overlaps a paddle at rows `left..left + 15`.
        assert!(left >= 104 || left + 15 <= 100, "{left}");
    }

    #[test]
    fn catch_hole_holds_the_ball_when_the_rally_ends_and_is_beside_it_otherwise() {
        let mut s = status(GameKind::Catch, ball(60, 100, -1, 0), 0);
        s.volleys = 100;
        for frame in (0..AIMS.len() as u64 * 4).map(|i| i * STEP_TICKS) {
            s.frame = frame;
            let hole = tops(&s).0;
            assert!(hole <= 100 && 100 + 3 < hole + 15, "frame {frame}: hole {hole} holds rows 100..=103");
        }
        s.volleys = 0;
        for frame in (0..AIMS.len() as u64 * 4).map(|i| i * STEP_TICKS) {
            s.frame = frame;
            let hole = tops(&s).0;
            assert!(hole > 103 || hole + 15 <= 100, "frame {frame}: hole {hole} must not hold the ball");
        }
    }

    #[test]
    fn solitaire_wall_drops_away_when_the_rally_ends_else_stays_in_reach() {
        let mut s = status(GameKind::Solitaire, ball(100, 60, 1, 0), 0);
        // A ball above the wall's stepped height is met by raising the wall.
        let wall = tops(&s).1;
        assert!(wall < 60 + 4 && wall < wall_rest(0), "{wall}");
        // One below it meets the wall where it rests.
        s.ball = ball(100, 190, 1, 0);
        assert_eq!(tops(&s).1, wall_rest(0));
        // At the end of the rally the wall is out of the way.
        s.volleys = 100;
        assert_eq!(tops(&s).1, TOP_MAX);
        // When the ball is on its way to the paddle the wall rests.
        s.ball = ball(100, 60, -1, 0);
        assert_eq!(tops(&s).1, wall_rest(0));
    }

    #[test]
    fn a_back_paddle_player_holds_clear_of_the_forward_paddle_until_the_ball_is_past() {
        // Left's forward paddle is at x 80; a ball at x 84 flying left is
        // at 83 next tick (volleys 0, tier 1), in the zone.
        let (mut back, mut forward) = (None, None);
        for step in 0..64 {
            let s = at_frame(step * STEP_TICKS);
            let (in_zone, before, past) = (tops(&s), tops(&at_x(&s, 100)), tops(&at_x(&s, 70)));
            let wants_forward = Turn::new(&s, s.ball.unwrap()).forward;
            let clear = |top: i32| top >= 104 || top + 15 <= 100;
            if wants_forward {
                assert!(!clear(in_zone.0) && !clear(before.0) && !clear(past.0), "step {step}");
                forward = Some(step);
            } else {
                assert!(clear(in_zone.0), "step {step}: in the zone");
                assert!(!clear(before.0) && !clear(past.0), "step {step}: outside it");
                back = Some(step);
            }
        }
        assert!(back.is_some() && forward.is_some());
    }

    fn at_frame(frame: u64) -> Status {
        status(GameKind::SuperPong, ball(84, 100, -1, 0), frame)
    }

    fn at_x(s: &Status, x: i32) -> Status {
        Status { ball: ball(x, 100, -1, 0), ..*s }
    }

    #[test]
    fn start_is_pressed_three_seconds_after_the_light_show_begins() {
        let attract = status(GameKind::Pong, None, 0);
        let mut demo = Demo::new();
        for tick in 1..RESTART_TICKS {
            assert!(!demo.input(&attract).start, "tick {tick}");
        }
        assert!(demo.input(&attract).start);
        // Then the timer starts over.
        assert!(!demo.input(&attract).start);
        // Leaving the light show resets it.
        let play = status(GameKind::Pong, ball(60, 100, 1, 0), 0);
        let mut demo = Demo::new();
        for _ in 0..RESTART_TICKS - 1 {
            demo.input(&attract);
        }
        assert!(!demo.input(&play).start);
        for _ in 0..RESTART_TICKS - 1 {
            assert!(!demo.input(&attract).start);
        }
        assert!(demo.input(&attract).start);
    }

    /// What the demo showed during a simulated run of one game.
    #[derive(Debug, Default)]
    struct Stats {
        /// Points scored by the left and right side.
        points: [u32; 2],
        /// Games the demo started after a game over.
        restarts: u32,
        /// Reflections off back paddles, forward paddles, walls and borders.
        back: u32,
        forward: u32,
        walls: u32,
        borders: u32,
        /// Reflections in each rally, up to its point.
        rallies: Vec<u32>,
        /// Outgoing vy after paddle and wall reflections.
        vys: BTreeSet<i32>,
        /// Baskets (points for the left side) with a flat / non-flat ball.
        flat_baskets: u32,
        angled_baskets: u32,
    }

    impl Stats {
        fn tier3_rallies(&self) -> usize {
            self.rallies.iter().filter(|&&r| r >= TIER_THRESHOLDS[1]).count()
        }
    }

    /// Ten simulated minutes: 60 ticks a second, as `--demo` at 60 fps.
    const TEN_MINUTES: u32 = 10 * 60 * 60;

    /// Plays `ticks` ticks of `kind` with the demo (and a Start on the
    /// first tick, like `--auto-start`) and collects the [`Stats`].
    fn simulate(kind: GameKind, ticks: u32) -> Stats {
        let mut console = Console::new(kind);
        let mut demo = Demo::new();
        let mut events = Vec::new();
        let mut stats = Stats::default();
        let (mut reflections, mut scores) = (0, (0, 0));
        for tick in 0..ticks {
            let before = console.status();
            let mut input = demo.input(&before);
            input.start |= tick == 0;
            stats.restarts += (input.start && before.phase == Phase::Attract && tick > 0) as u32;
            events.clear();
            console.tick(&input, &mut events);
            let ball = console.status().ball;
            for event in &events {
                match event {
                    SoundEvent::Bounce(BounceKind::Paddle) => {
                        // Back paddles are in the outer quarters of the field.
                        let x = ball.map_or(0, |b| b.x);
                        if (FIELD_W / 4..FIELD_W * 3 / 4).contains(&x) {
                            stats.forward += 1;
                        } else {
                            stats.back += 1;
                        }
                    }
                    SoundEvent::Bounce(BounceKind::Wall) => stats.walls += 1,
                    SoundEvent::Bounce(BounceKind::Border) => stats.borders += 1,
                    SoundEvent::Point => {
                        let now = console.scores();
                        stats.points[0] += (now.0 > scores.0) as u32;
                        stats.points[1] += (now.1 > scores.1) as u32;
                        if now.0 > scores.0 {
                            let flat = ball.is_some_and(|b| b.vy == 0);
                            stats.flat_baskets += flat as u32;
                            stats.angled_baskets += !flat as u32;
                        }
                        stats.rallies.push(reflections);
                        reflections = 0;
                    }
                    _ => {}
                }
                if let SoundEvent::Bounce(BounceKind::Paddle | BounceKind::Wall) = event {
                    reflections += 1;
                    stats.vys.extend(ball.map(|b| b.vy));
                }
            }
            // A new game starts from 0-0; compare later points against that.
            scores = console.scores();
        }
        stats
    }

    fn nonzero(vys: &BTreeSet<i32>) -> usize {
        vys.iter().filter(|&&v| v != 0).count()
    }

    #[test]
    fn pong_rallies_vary_and_reach_tier_3() {
        let s = simulate(GameKind::Pong, TEN_MINUTES);
        assert!(s.tier3_rallies() >= 8 && s.rallies.len() >= 20, "{s:?}");
        assert!(s.points[0] >= 5 && s.points[1] >= 5, "{s:?}");
        assert!(nonzero(&s.vys) >= 5, "{s:?}");
        assert!(s.restarts >= 1, "{s:?}");
    }

    #[test]
    fn super_pong_uses_both_paddles_and_reaches_tier_3() {
        let s = simulate(GameKind::SuperPong, TEN_MINUTES);
        assert!(s.back >= 50 && s.forward >= 50, "{s:?}");
        assert!(s.tier3_rallies() >= 8, "{s:?}");
        assert!(s.points[0] >= 5 && s.points[1] >= 5, "{s:?}");
        assert!(nonzero(&s.vys) >= 5, "{s:?}");
        assert!(s.restarts >= 1, "{s:?}");
    }

    #[test]
    fn catch_shows_wall_bounces_as_well_as_catches() {
        let s = simulate(GameKind::Catch, TEN_MINUTES);
        assert!(s.walls >= 50, "{s:?}");
        assert!(s.points[0] >= 10 && s.points[1] >= 10, "{s:?}");
        assert!(s.rallies.iter().any(|&r| r >= 3), "{s:?}");
        assert!(s.restarts >= 1, "{s:?}");
    }

    #[test]
    fn solitaire_shows_paddle_hits_wall_bounces_and_angled_baskets() {
        let s = simulate(GameKind::Solitaire, TEN_MINUTES);
        assert!(s.back >= 50 && s.walls >= 50, "{s:?}");
        assert!(s.points[0] >= 5 && s.points[1] >= 5, "{s:?}");
        assert!(s.angled_baskets >= 4 * s.flat_baskets.max(1), "{s:?}");
        assert!(nonzero(&s.vys) >= 5, "{s:?}");
        assert!(s.restarts >= 1, "{s:?}");
    }

    /// Frame-rate independence: the demo's play depends on console ticks
    /// only, so a 30, 60 and 144 Hz display all see the same game.
    #[test]
    fn play_does_not_depend_on_the_display_frame_rate() {
        let play = |fps: f64| {
            let mut console = Console::new(GameKind::SuperPong);
            let (mut scheduler, mut demo, mut events) = (Scheduler::new(), Demo::new(), Vec::new());
            scheduler.press_start();
            let mut seen = Vec::new();
            for _ in 0..(90.0 * fps) as u32 {
                scheduler.run(&mut console, 1.0 / fps, &mut events, |status| {
                    seen.push((
                        status.frame,
                        status.ball,
                        status.volleys,
                        status.left_score,
                        status.right_score,
                    ));
                    demo.input(status)
                });
            }
            seen
        };
        let reference = play(60.0);
        assert!(reference.len() > 5000);
        for fps in [30.0, 144.0] {
            let seen = play(fps);
            let common = seen.len().min(reference.len()) - 1;
            assert_eq!(seen[..common], reference[..common], "{fps} fps");
        }
    }
}
