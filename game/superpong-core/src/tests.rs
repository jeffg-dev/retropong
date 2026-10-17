//! Integration-level tests for [`Console`], covering the public contract
//! described in `docs/SPEC-superpong.md`.
//!
//! Golden numbers and the fixtures in `testdata/` come from an independent
//! executable mirror of the spec (Python, `tools/superpong-mirror/`, not
//! shipped). Where a test reproduces an event of the reference video, it
//! cites it.

use crate::*;

// ---------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------

/// Input with the knobs at paddle tops `left` and `right`.
fn knobs(left: i32, right: i32) -> Input {
    Input { left: knob_for_top(left), right: knob_for_top(right), ..Input::default() }
}

/// Both knobs at the bottom (top 209), out of the way of a mid-field ball.
fn away() -> Input {
    Input { left: 1.0, right: 1.0, ..Input::default() }
}

/// Ticks once and returns the frame's events.
fn step(c: &mut Console, input: &Input) -> Vec<SoundEvent> {
    let mut events = Vec::new();
    c.tick(input, &mut events);
    events
}

/// A console showing `kind` with the ball visible at `(x, row)`, volleys 0.
fn rally(kind: GameKind, x: i32, row: i32, dirx: i32, vy: i32) -> Console {
    let mut c = Console::new(kind);
    c.debug_set_ball(BallState { x, row, dirx, vy, visible: true }, 0);
    c
}

/// The ball, which must be in play.
fn ball(c: &Console) -> BallState {
    c.status().ball.expect("ball in play")
}

const PADDLE: SoundEvent = SoundEvent::Bounce(BounceKind::Paddle);
const WALL: SoundEvent = SoundEvent::Bounce(BounceKind::Wall);
const BORDER: SoundEvent = SoundEvent::Bounce(BounceKind::Border);

/// Ticks with `input` until a tick reports `event` (at most `max` ticks)
/// and returns how many ticks that took.
fn ticks_until(c: &mut Console, input: &Input, event: SoundEvent, max: u32) -> u32 {
    for n in 1..=max {
        if step(c, input).contains(&event) {
            return n;
        }
    }
    panic!("no {event:?} within {max} ticks");
}

/// A tiny xorshift64 generator, so that tests need no dependencies. The
/// differential trace's generator script uses the same one.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// FNV-1a, for comparing frames by hash.
fn fnv64(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn rendered(c: &Console) -> Frame {
    let mut frame = Frame::new();
    c.render(&mut frame);
    frame
}

// ---------------------------------------------------------------------
// GameKind, Side, knobs
// ---------------------------------------------------------------------

#[test]
fn game_kind_numbers_round_trip() {
    for kind in GameKind::ALL {
        assert_eq!(GameKind::from_number(kind.number()), Some(kind));
    }
    assert_eq!(GameKind::Catch.number(), 1);
    assert_eq!(GameKind::Solitaire.number(), 2);
    assert_eq!(GameKind::SuperPong.number(), 3);
    assert_eq!(GameKind::Pong.number(), 4);
    assert_eq!(GameKind::from_number(0), None);
    assert_eq!(GameKind::from_number(5), None);
}

#[test]
fn game_kind_next_wraps() {
    assert_eq!(GameKind::Catch.next(), GameKind::Solitaire);
    assert_eq!(GameKind::Solitaire.next(), GameKind::SuperPong);
    assert_eq!(GameKind::SuperPong.next(), GameKind::Pong);
    assert_eq!(GameKind::Pong.next(), GameKind::Catch);
}

#[test]
fn side_other_is_involution() {
    assert_eq!(Side::Left.other(), Side::Right);
    assert_eq!(Side::Right.other(), Side::Left);
    assert_eq!(Side::Left.other().other(), Side::Left);
}

#[test]
fn knob_mapping() {
    assert_eq!(knob_to_top(0.0), 0);
    assert_eq!(knob_to_top(0.5), 105);
    assert_eq!(knob_to_top(1.0), 209);
    assert_eq!(knob_to_top(f32::NAN), 105);
    assert_eq!(knob_to_top(-1.0), 0);
    assert_eq!(knob_to_top(2.0), 209);
    assert_eq!(knob_to_top(f32::INFINITY), 209);
    for top in 0..=TOP_MAX {
        assert_eq!(knob_to_top(knob_for_top(top)), top);
    }
    assert_eq!(knob_for_top(-5), knob_for_top(0));
    assert_eq!(knob_for_top(500), knob_for_top(TOP_MAX));
}

#[test]
fn knobs_are_read_every_tick_in_every_phase() {
    let mut c = Console::new(GameKind::Pong);
    step(&mut c, &knobs(3, 200));
    assert_eq!((c.status().left_top, c.status().right_top), (3, 200));
    c.press_start();
    step(&mut c, &knobs(150, 7));
    assert_eq!(c.phase(), Phase::Waiting);
    assert_eq!((c.status().left_top, c.status().right_top), (150, 7));
}

// ---------------------------------------------------------------------
// Power-on, status, frame rate
// ---------------------------------------------------------------------

#[test]
fn power_on_is_the_light_show_with_zero_scores() {
    let c = Console::new(GameKind::Catch);
    let s = c.status();
    assert_eq!(s.kind, GameKind::Catch);
    assert_eq!(s.phase, Phase::Attract);
    assert_eq!((s.left_score, s.right_score), (0, 0));
    assert!(s.digits_visible);
    assert_eq!(s.ball, None);
    assert_eq!((s.left_top, s.right_top), (105, 105));
    assert_eq!(s.frame, 0);
    assert_eq!(c.frame_hz(), FRAME_HZ_ATTRACT);
    assert_eq!((c.ls.h0, c.ls.vy, c.ls.last_wrap), (POWER_ON_H0, POWER_ON_VY, POWER_ON_WRAP_LINE));
}

#[test]
fn status_follows_the_phase() {
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 50, row: 60, dirx: -1, vy: 2, visible: false }, 5);
    let s = c.status();
    assert_eq!(s.phase, Phase::Waiting);
    assert!(s.digits_visible);
    assert_eq!(s.ball, Some(BallState { x: 50, row: 60, dirx: -1, vy: 2, visible: false }));
    assert_eq!(s.volleys, 5);
    c.debug_set_ball(BallState { x: 50, row: 60, dirx: -1, vy: 2, visible: true }, 5);
    let s = c.status();
    assert_eq!(s.phase, Phase::Rally);
    assert!(!s.digits_visible);
    assert!(s.ball.unwrap().visible);
}

#[test]
fn debug_set_ball_normalises_direction_and_speed() {
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 50, row: 60, dirx: 0, vy: 9, visible: true }, 0);
    assert_eq!((ball(&c).dirx, ball(&c).vy), (1, 3));
    c.debug_set_ball(BallState { x: 50, row: 60, dirx: -7, vy: -9, visible: true }, 0);
    assert_eq!((ball(&c).dirx, ball(&c).vy), (-1, -3));
}

#[test]
fn frame_counts_ticks() {
    let mut c = Console::new(GameKind::Pong);
    for _ in 0..7 {
        step(&mut c, &Input::default());
    }
    assert_eq!(c.status().frame, 7);
}

#[test]
fn frame_rate_switches_on_the_tick_of_the_phase_change() {
    let mut c = Console::new(GameKind::Pong);
    assert_eq!(c.frame_hz(), 60.19);
    step(&mut c, &Input { start: true, ..away() });
    assert_eq!(c.frame_hz(), 59.94);
    c.scores = (14, 0);
    c.debug_set_ball(BallState { x: 174, row: 100, dirx: 1, vy: 0, visible: true }, 0);
    assert_eq!(c.frame_hz(), 59.94);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    assert_eq!(c.phase(), Phase::Attract);
    assert_eq!(c.frame_hz(), 60.19);
}

// ---------------------------------------------------------------------
// Speed tiers and volleys (spec §7)
// ---------------------------------------------------------------------

#[test]
fn speed_follows_the_volley_count() {
    for (volleys, speed) in [(0, 1), (3, 1), (4, 2), (7, 2), (8, 3), (100, 3), (u32::MAX, 3)] {
        let mut c = Console::new(GameKind::Pong);
        c.debug_set_ball(BallState { x: 100, row: 100, dirx: 1, vy: 0, visible: true }, volleys);
        step(&mut c, &away());
        assert_eq!(ball(&c).x, 100 + speed, "volleys {volleys}");
        // The hidden ball always moves one clock per frame.
        c.debug_set_ball(BallState { x: 100, row: 100, dirx: -1, vy: 0, visible: false }, volleys);
        step(&mut c, &away());
        assert_eq!(ball(&c).x, 99, "hidden, volleys {volleys}");
    }
}

#[test]
fn new_speed_applies_from_the_frame_after_the_reflection() {
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 157, row: 100, dirx: 1, vy: 0, visible: true }, 3);
    assert_eq!(step(&mut c, &knobs(209, 94)), [PADDLE]);
    assert_eq!((ball(&c).x, c.status().volleys), (158, 4));
    step(&mut c, &knobs(209, 94));
    assert_eq!(ball(&c).x, 156);
}

#[test]
fn volleys_count_paddles_and_walls_but_not_borders() {
    // A border and a paddle in the same frame: only the paddle counts.
    let mut c = rally(GameKind::Pong, 157, 16, 1, -1);
    assert_eq!(step(&mut c, &knobs(209, 10)), [BORDER, PADDLE]);
    assert_eq!(c.status().volleys, 1);
    // Super Pong forward.
    let mut c = rally(GameKind::SuperPong, 84, 100, -1, 0);
    assert_eq!(step(&mut c, &knobs(95, 209)), [PADDLE]);
    assert_eq!(c.status().volleys, 1);
    // Catch wall.
    let mut c = rally(GameKind::Catch, 20, 100, -1, 0);
    assert_eq!(step(&mut c, &knobs(150, 150)), [WALL]);
    assert_eq!(c.status().volleys, 1);
    // Solitaire wall.
    let mut c = rally(GameKind::Solitaire, 157, 100, 1, 0);
    assert_eq!(step(&mut c, &knobs(150, 50)), [WALL]);
    assert_eq!(c.status().volleys, 1);
    // Borders do not count.
    let mut c = rally(GameKind::Pong, 100, 16, 1, -1);
    assert_eq!(step(&mut c, &away()), [BORDER]);
    assert_eq!(c.status().volleys, 0);
}

#[test]
fn volleys_reset_at_exit_and_start_but_not_at_select() {
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 100, row: 100, dirx: 1, vy: 0, visible: true }, 6);
    c.select(GameKind::Catch);
    assert_eq!(c.status().volleys, 6);
    step(&mut c, &Input { select: Some(GameKind::SuperPong), ..away() });
    assert_eq!(c.status().volleys, 6);
    c.press_start();
    assert_eq!(c.status().volleys, 0);
    c.debug_set_ball(BallState { x: 174, row: 100, dirx: 1, vy: 0, visible: true }, 6);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    assert_eq!(c.status().volleys, 0);
}

// ---------------------------------------------------------------------
// Paddles and walls (spec §6.4, §6.5)
// ---------------------------------------------------------------------

/// Puts the ball where, after one move at `volleys`' speed, it is at `x`,
/// row `r`, moving `dirx` with `vy`, and ticks once. Returns the events.
fn meet(c: &mut Console, input: &Input, x: i32, r: i32, dirx: i32, vy: i32, volleys: u32) -> Vec<SoundEvent> {
    let speed = play::tier_speed(volleys);
    let at = BallState { x: x - dirx * speed, row: r - vy, dirx, vy, visible: true };
    c.debug_set_ball(at, volleys);
    let events = step(c, input);
    assert_eq!((ball(c).x, ball(c).row), (x, r));
    events
}

#[test]
fn paddle_segments_ignore_the_incoming_ball() {
    // Paddle at top 100; the ball's top row R = 97..=114 (delta -3..=14).
    let expected = [-3, -3, -3, -3, -3, -2, -2, -1, -1, 0, 0, 0, 0, 1, 1, 2, 2, 3];
    // (game, knobs, turnaround x, direction) for every kind of paddle.
    let paddles = [
        (GameKind::Pong, knobs(100, 209), 19, -1),
        (GameKind::Pong, knobs(209, 100), 158, 1),
        (GameKind::SuperPong, knobs(100, 209), 83, -1),
        (GameKind::SuperPong, knobs(209, 100), 94, 1),
        (GameKind::Solitaire, knobs(100, 209), 19, -1),
    ];
    for (kind, input, x, dirx) in paddles {
        for vy_in in -3..=3 {
            for volleys in [0, 4, 8] {
                let got: Vec<i32> = (97..=114)
                    .map(|r| {
                        let mut c = Console::new(kind);
                        assert_eq!(meet(&mut c, &input, x, r, dirx, vy_in, volleys), [PADDLE]);
                        assert_eq!(ball(&c).dirx, -dirx);
                        ball(&c).vy
                    })
                    .collect();
                assert_eq!(got, expected, "{kind:?} x {x}, vy in {vy_in}, volleys {volleys}");
            }
        }
    }
}

#[test]
fn paddle_row_edges() {
    let input = knobs(209, 100);
    for (r, hit) in [(96, false), (97, true), (114, true), (115, false)] {
        let mut c = Console::new(GameKind::Pong);
        let events = meet(&mut c, &input, 158, r, 1, 0, 0);
        assert_eq!(events.contains(&PADDLE), hit, "R {r}");
    }
}

#[test]
fn a_ball_moving_away_passes_through_from_behind() {
    // Moving right through the left forward (80..84) on its rows.
    let mut c = rally(GameKind::SuperPong, 76, 100, 1, 0);
    for x in 77..=93 {
        assert_eq!(step(&mut c, &knobs(98, 209)), [], "x {x}");
        assert_eq!((ball(&c).x, ball(&c).dirx, ball(&c).vy), (x, 1, 0));
    }
    assert_eq!(c.status().volleys, 0);
    // Moving left through the right forward (96..100) and back paddle.
    let mut c = rally(GameKind::SuperPong, 170, 100, -1, 1);
    for x in (84..=169).rev() {
        assert_eq!(step(&mut c, &knobs(209, 98)), [], "x {x}");
        assert_eq!((ball(&c).x, ball(&c).dirx, ball(&c).vy), (x, -1, 1));
    }
    // ... and through a Catch wall and the Solitaire wall from behind.
    let mut c = rally(GameKind::Catch, 12, 100, 1, 0);
    for _ in 0..12 {
        assert_eq!(step(&mut c, &knobs(0, 0)), []);
    }
    let mut c = rally(GameKind::Solitaire, 166, 100, -1, 0);
    for _ in 0..12 {
        assert_eq!(step(&mut c, &knobs(209, 0)), []);
    }
}

/// Ticks a visible ball from `x0` until it is reflected and returns the
/// x at which that happened.
fn turnaround(kind: GameKind, x0: i32, dirx: i32, volleys: u32) -> i32 {
    let mut c = Console::new(kind);
    c.debug_set_ball(BallState { x: x0, row: 100, dirx, vy: 0, visible: true }, volleys);
    for _ in 0..20 {
        let events = step(&mut c, &knobs(95, 95));
        if !events.is_empty() {
            assert_eq!(events, [PADDLE]);
            return ball(&c).x;
        }
    }
    panic!("no reflection");
}

#[test]
fn turnaround_columns() {
    use GameKind::{Pong, SuperPong};
    // Tier 1.
    assert_eq!(turnaround(Pong, 30, -1, 0), 19);
    assert_eq!(turnaround(Pong, 150, 1, 0), 158);
    assert_eq!(turnaround(SuperPong, 89, -1, 0), 83);
    assert_eq!(turnaround(SuperPong, 89, 1, 0), 94);
    // Tier 2: 21 -> 19; 22 -> 20 -> 18.
    assert_eq!(turnaround(Pong, 21, -1, 4), 19);
    assert_eq!(turnaround(Pong, 22, -1, 4), 18);
    assert_eq!(turnaround(Pong, 156, 1, 4), 158);
    assert_eq!(turnaround(Pong, 157, 1, 4), 159);
    // Tier 3: 19, 18 or 17 on the left; 158, 159 or 160 on the right.
    assert_eq!(turnaround(Pong, 22, -1, 8), 19);
    assert_eq!(turnaround(Pong, 21, -1, 8), 18);
    assert_eq!(turnaround(Pong, 23, -1, 8), 17);
    assert_eq!(turnaround(Pong, 155, 1, 8), 158);
    assert_eq!(turnaround(Pong, 156, 1, 8), 159);
    assert_eq!(turnaround(Pong, 157, 1, 8), 160);
}

#[test]
fn late_hit_by_a_moving_paddle() {
    // The ball slid past the face beside the paddle; the paddle moves onto it.
    let mut c = rally(GameKind::Pong, 18, 50, -1, 0);
    assert_eq!(step(&mut c, &knobs(150, 209)), []);
    assert_eq!(ball(&c).x, 17);
    assert_eq!(step(&mut c, &knobs(45, 209)), [PADDLE]);
    let b = ball(&c);
    assert_eq!((b.x, b.dirx, b.vy), (16, 1, -1)); // delta 5
    assert_eq!(c.status().volleys, 1);
}

#[test]
fn late_hit_by_the_ball_own_vy() {
    // Below the paddle (rows 40..55), climbing into it inside its columns.
    let mut c = rally(GameKind::Pong, 18, 60, -1, -3);
    assert_eq!(step(&mut c, &knobs(40, 209)), []);
    assert_eq!((ball(&c).x, ball(&c).row), (17, 57));
    assert_eq!(step(&mut c, &knobs(40, 209)), [PADDLE]);
    let b = ball(&c);
    assert_eq!((b.x, b.row, b.dirx, b.vy), (16, 54, 1, 3)); // delta 14
}

#[test]
fn catch_wall_reflects_outside_the_hole() {
    for vy_in in -3..=3 {
        for (r, vy) in [(99, Some(-3)), (112, Some(3)), (60, Some(-3)), (150, Some(3))] {
            let mut c = Console::new(GameKind::Catch);
            let events = meet(&mut c, &knobs(100, 209), 19, r, -1, vy_in, 0);
            assert_eq!(events, [WALL], "R {r}, vy in {vy_in}");
            assert_eq!((ball(&c).dirx, Some(ball(&c).vy)), (1, vy));
        }
    }
}

#[test]
fn catch_ball_through_the_hole_scores_for_the_catcher() {
    for r in 100..=111 {
        let mut c = rally(GameKind::Catch, 30, r, -1, 0);
        let mut events = Vec::new();
        for _ in 0..31 {
            c.tick(&knobs(100, 209), &mut events);
        }
        assert_eq!(events, [SoundEvent::Point], "R {r}");
        assert_eq!(c.scores(), (1, 0));
        assert_eq!(c.phase(), Phase::Waiting);
    }
}

#[test]
fn catch_late_hit_when_the_hole_moves() {
    let mut c = rally(GameKind::Catch, 20, 104, -1, 0);
    assert_eq!(step(&mut c, &knobs(100, 209)), []);
    assert_eq!(ball(&c).x, 19);
    assert_eq!(step(&mut c, &knobs(105, 209)), [WALL]);
    let b = ball(&c);
    assert_eq!((b.x, b.dirx, b.vy), (18, 1, -3));
}

#[test]
fn segment_vy_is_antisymmetric() {
    // The reflection angles are symmetric top/bottom (spec 6.4).
    for i in 0..8 {
        assert_eq!(SEGMENT_VY[i], -SEGMENT_VY[7 - i], "segment {i}");
    }
    assert_eq!(SEGMENT_VY[3], 0);
    assert_eq!(SEGMENT_VY[4], 0);
}

#[test]
fn solitaire_wall_segments() {
    // Measured wall hits, wall top 69 (ball_physics.md §4.4: delta -2, 10,
    // 16, 66, 100, 106).
    for (r, vy) in [(67, -3), (79, 1), (85, -3), (135, -2), (169, -1), (175, 1)] {
        let mut c = Console::new(GameKind::Solitaire);
        assert_eq!(meet(&mut c, &knobs(209, 69), 158, r, 1, 0, 0), [WALL], "R {r}");
        assert_eq!(ball(&c).vy, vy, "R {r}");
    }
}

#[test]
fn solitaire_ball_over_the_wall_is_a_basket() {
    let mut c = rally(GameKind::Solitaire, 150, 65, 1, 0);
    let mut events = Vec::new();
    for _ in 0..25 {
        c.tick(&knobs(209, 69), &mut events);
    }
    assert_eq!(events, [SoundEvent::Point]);
    assert_eq!(c.scores(), (1, 0));
}

#[test]
fn solitaire_serves_toward_the_wall_after_a_basket_and_toward_the_player_after_a_miss() {
    // Spec 6.6: the serve direction is the exit direction.
    let mut c = rally(GameKind::Solitaire, 150, 65, 1, 0);
    ticks_until(&mut c, &knobs(209, 69), SoundEvent::Point, 60);
    assert_eq!(c.scores(), (1, 0));
    ticks_until(&mut c, &knobs(209, 69), SoundEvent::Serve, 400);
    assert_eq!(ball(&c).dirx, 1);

    let mut c = rally(GameKind::Solitaire, 30, 65, -1, 0);
    ticks_until(&mut c, &knobs(209, 69), SoundEvent::Point, 60);
    assert_eq!(c.scores(), (0, 1));
    ticks_until(&mut c, &knobs(209, 69), SoundEvent::Serve, 400);
    assert_eq!(ball(&c).dirx, -1);
}

#[test]
fn closed_solitaire_wall_returns_every_ball() {
    for r in 10..=210 {
        let mut c = Console::new(GameKind::Solitaire);
        let events = meet(&mut c, &knobs(209, 0), 158, r, 1, 0, 0);
        assert!(events.contains(&WALL), "R {r}");
        assert_eq!(ball(&c).dirx, -1);
    }
}

#[test]
fn hidden_ball_ignores_paddles() {
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 21, row: 100, dirx: -1, vy: 0, visible: false }, 0);
    for x in (10..=20).rev() {
        assert_eq!(step(&mut c, &knobs(98, 98)), []);
        assert_eq!((ball(&c).x, ball(&c).dirx), (x, -1));
    }
}

#[test]
fn border_and_paddle_in_the_same_frame() {
    // Spec §6.5 corner case: the border flips vy, then the paddle sets it.
    let mut c = rally(GameKind::SuperPong, 93, 15, 1, -1);
    assert_eq!(step(&mut c, &knobs(209, 14)), [BORDER, PADDLE]);
    let b = ball(&c);
    assert_eq!((b.x, b.row, b.dirx, b.vy), (94, 14, -1, -3));
    assert_eq!(step(&mut c, &knobs(209, 14)), [BORDER]);
    let b = ball(&c);
    assert_eq!((b.x, b.row, b.vy), (93, 11, 3));
}

// ---------------------------------------------------------------------
// Borders and the hidden ball (spec §6.3, §6.6)
// ---------------------------------------------------------------------

/// Ticks a ball until its first border bounce and returns the row at it.
fn border_turnaround(row: i32, vy: i32) -> i32 {
    let mut c = rally(GameKind::Pong, 60, row, 1, vy);
    for _ in 0..10 {
        let events = step(&mut c, &away());
        if !events.is_empty() {
            assert_eq!(events, [BORDER]);
            assert_eq!(ball(&c).vy, -vy, "the magnitude is kept");
            assert_eq!(c.status().volleys, 0);
            return ball(&c).row;
        }
    }
    panic!("no border bounce");
}

#[test]
fn border_turnaround_rows() {
    // Top: row <= 15 moving up.
    let mut c = rally(GameKind::Pong, 60, 18, 1, -1);
    let rows: Vec<(i32, Vec<SoundEvent>)> = (0..4)
        .map(|_| {
            let events = step(&mut c, &away());
            (ball(&c).row, events)
        })
        .collect();
    assert_eq!(rows, [(17, vec![]), (16, vec![]), (15, vec![BORDER]), (16, vec![])]);
    assert_eq!(border_turnaround(18, -3), 15);
    assert_eq!(border_turnaround(17, -3), 14);
    assert_eq!(border_turnaround(19, -3), 13);
    // Bottom: row + 3 >= 208 moving down.
    assert_eq!(border_turnaround(200, 1) + 3, 208);
    assert_eq!(border_turnaround(202, 3) + 3, 208);
    assert_eq!(border_turnaround(203, 3) + 3, 209);
    assert_eq!(border_turnaround(201, 3) + 3, 210);
}

#[test]
fn hidden_ball_outside_the_box_passes_through_blanking() {
    // Rightward, from x 176 through the wrap at 202 -> -25 to x -1.
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 176, row: 200, dirx: 1, vy: 3, visible: false }, 0);
    for k in 1..=51i32 {
        assert_eq!(step(&mut c, &away()), [], "tick {k}");
        let b = ball(&c);
        let x = if k <= 26 { 176 + k } else { -25 + (k - 27) };
        let row = (200 + 3 * k + 40).rem_euclid(263) - 40;
        assert_eq!((b.x, b.row, b.vy), (x, row, 3), "tick {k}");
        assert!((-40..223).contains(&b.row));
    }
    // Leftward, from x -1 through the wrap at -25 -> 201 to x 176.
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: -1, row: 0, dirx: -1, vy: -2, visible: false }, 0);
    for k in 1..=50i32 {
        assert_eq!(step(&mut c, &away()), [], "tick {k}");
        let b = ball(&c);
        let x = if k <= 24 { -1 - k } else { 201 - (k - 25) };
        let row = (-2 * k + 40).rem_euclid(263) - 40;
        assert_eq!((b.x, b.row, b.vy), (x, row, -2), "tick {k}");
    }
}

#[test]
fn hidden_ball_top_border_ends_in_blanking() {
    // The top test covers rows -6..=15, the light show's top zone. A hidden
    // ball coming back into the box higher up keeps going up and wraps.
    assert_eq!(TOP_BOUNCE_ROW_MIN, -6);
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: -1, row: -5, dirx: 1, vy: -1, visible: false }, 0);
    step(&mut c, &away());
    assert_eq!((ball(&c).x, ball(&c).row, ball(&c).vy), (0, -6, 1));
    c.debug_set_ball(BallState { x: -1, row: -6, dirx: 1, vy: -1, visible: false }, 0);
    step(&mut c, &away());
    assert_eq!((ball(&c).x, ball(&c).row, ball(&c).vy), (0, -7, -1));
    for _ in 0..33 {
        step(&mut c, &away());
    }
    assert_eq!((ball(&c).x, ball(&c).row, ball(&c).vy), (33, -40, -1));
    step(&mut c, &away());
    assert_eq!((ball(&c).x, ball(&c).row, ball(&c).vy), (34, 222, -1));
}

#[test]
fn hidden_ball_border_reversals_are_silent() {
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 50, row: 17, dirx: 1, vy: -3, visible: false }, 0);
    assert_eq!(step(&mut c, &away()), []);
    assert_eq!((ball(&c).row, ball(&c).vy), (14, 3));
    c.debug_set_ball(BallState { x: 0, row: 204, dirx: -1, vy: 1, visible: false }, 0);
    assert_eq!(step(&mut c, &away()), []);
    // x -1 is outside the box: no reversal.
    assert_eq!((ball(&c).x, ball(&c).row, ball(&c).vy), (-1, 205, 1));
    c.debug_set_ball(BallState { x: 1, row: 204, dirx: -1, vy: 1, visible: false }, 0);
    assert_eq!(step(&mut c, &away()), []);
    assert_eq!((ball(&c).x, ball(&c).row, ball(&c).vy), (0, 205, -1));
}

#[test]
fn hidden_wrap_is_228_moving_right_and_227_moving_left() {
    // From one clock past the serve column, a whole cycle less one frame.
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: SERVE_X_RIGHT + 1, row: 100, dirx: 1, vy: 0, visible: false }, 0);
    assert_eq!(ticks_until(&mut c, &away(), SoundEvent::Serve, 400), 227);
    assert_eq!(ball(&c).x, 87);
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: SERVE_X_LEFT - 1, row: 100, dirx: -1, vy: 0, visible: false }, 0);
    assert_eq!(ticks_until(&mut c, &away(), SoundEvent::Serve, 400), 226);
    assert_eq!(ball(&c).x, 90);
}

#[test]
fn serve_at_the_net_column_for_each_direction() {
    for (dirx, serve_x) in [(1, 87), (-1, 90)] {
        let mut c = Console::new(GameKind::Pong);
        let x0 = serve_x - dirx;
        c.debug_set_ball(BallState { x: x0, row: 120, dirx, vy: 1, visible: false }, 0);
        let frame = rendered(&c);
        assert_eq!(frame.get(LEFT_UNITS_X, DIGIT_ROW0), Cell::Foreground, "digits on while hidden");
        assert_eq!(step(&mut c, &away()), [SoundEvent::Serve]);
        let s = c.status();
        assert_eq!(s.phase, Phase::Rally);
        assert!(!s.digits_visible);
        assert_eq!(s.ball, Some(BallState { x: serve_x, row: 121, dirx, vy: 1, visible: true }));
        let frame = rendered(&c);
        for row in 121..125 {
            assert_eq!(frame.get(serve_x, row), Cell::Foreground);
            assert_eq!(frame.get(serve_x - 1, row), Cell::Background);
        }
        assert_eq!(frame.get(LEFT_UNITS_X, DIGIT_ROW0), Cell::Background, "digits off");
    }
    // A hidden ball crossing the other direction's serve column does not serve.
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 89, row: 120, dirx: 1, vy: 0, visible: false }, 0);
    assert_eq!(step(&mut c, &away()), []);
    c.debug_set_ball(BallState { x: 88, row: 120, dirx: -1, vy: 0, visible: false }, 0);
    assert_eq!(step(&mut c, &away()), []);
    assert_eq!(c.phase(), Phase::Waiting);
}

// ---------------------------------------------------------------------
// Exits, scoring, game over (spec §6.3 step 6, §8, §10.6)
// ---------------------------------------------------------------------

#[test]
fn exit_when_the_centre_leaves_the_box() {
    // Moving right: x 174 is still in play, x 175 exits.
    let mut c = rally(GameKind::Pong, 173, 100, 1, 0);
    assert_eq!(step(&mut c, &away()), []);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    let s = c.status();
    assert_eq!(s.ball, Some(BallState { x: 175, row: 100, dirx: 1, vy: 0, visible: false }));
    assert_eq!(s.phase, Phase::Waiting);
    assert!(s.digits_visible);
    // Moving left: x 0 is still in play, x -1 exits.
    let mut c = rally(GameKind::Pong, 1, 100, -1, 0);
    assert_eq!(step(&mut c, &away()), []);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    assert_eq!(ball(&c).x, -1);
    // At tier 3 the exit lands further out and the display is shorter.
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 173, row: 100, dirx: 1, vy: 0, visible: true }, 8);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    assert_eq!(ball(&c).x, 176);
    assert_eq!(ticks_until(&mut c, &away(), SoundEvent::Serve, 200), 139);
}

#[test]
fn who_scores_in_each_game() {
    for kind in GameKind::ALL {
        let mut c = rally(kind, 0, 100, -1, 0);
        assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
        let left_exit = c.scores();
        let mut c = rally(kind, 174, 100, 1, 0);
        // Keep the Catch hole and the Solitaire wall out of the way.
        assert_eq!(step(&mut c, &knobs(209, 209)), [SoundEvent::Point]);
        let right_exit = c.scores();
        if kind == GameKind::Catch {
            assert_eq!((left_exit, right_exit), ((1, 0), (0, 1)), "{kind:?}");
        } else {
            assert_eq!((left_exit, right_exit), ((0, 1), (1, 0)), "{kind:?}");
        }
    }
}

#[test]
fn fifteen_wins_and_returns_to_the_light_show_in_the_same_frame() {
    for (kind, scores, x, dirx, final_scores) in [
        (GameKind::Pong, (14, 9), 174, 1, (15, 9)),
        (GameKind::SuperPong, (3, 14), 0, -1, (3, 15)),
        (GameKind::Catch, (14, 0), 0, -1, (15, 0)),
        // Solitaire's misses counter wins too.
        (GameKind::Solitaire, (2, 14), 0, -1, (2, 15)),
    ] {
        let mut c = rally(kind, x, 100, dirx, 0);
        c.scores = scores;
        assert_eq!(step(&mut c, &away()), [SoundEvent::Point], "{kind:?}");
        let s = c.status();
        assert_eq!(s.phase, Phase::Attract);
        assert_eq!((s.left_score, s.right_score), final_scores);
        assert_eq!(s.ball, None);
        assert!(s.digits_visible);
        // The light show runs on, the scores stay, and nothing is served.
        for _ in 0..1000 {
            let events = step(&mut c, &away());
            assert!(events.iter().all(|&e| e == SoundEvent::AttractTick));
        }
        assert_eq!(c.scores(), final_scores);
    }
}

#[test]
fn scores_never_pass_fifteen() {
    // Even if a test hook puts a ball in play after the game is over.
    let mut c = rally(GameKind::Pong, 174, 100, 1, 0);
    c.scores = (15, 3);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    assert_eq!(c.scores(), (15, 3));
    assert_eq!(c.phase(), Phase::Attract);
}

// ---------------------------------------------------------------------
// Start and Game Select (spec §11)
// ---------------------------------------------------------------------

#[test]
fn start_from_the_light_show() {
    let mut c = Console::new(GameKind::SuperPong);
    for _ in 0..5 {
        step(&mut c, &Input::default());
    }
    let (wrap, vy) = (c.ls.last_wrap, c.ls.vy);
    let events = step(&mut c, &Input { start: true, ..Input::default() });
    assert_eq!(events, []);
    let s = c.status();
    assert_eq!(s.phase, Phase::Waiting);
    assert!(s.digits_visible);
    assert_eq!((s.left_score, s.right_score, s.volleys), (0, 0, 0));
    // The invisible ball keeps its row (wrap line - 2) and vy, and has
    // moved once in the Start tick.
    let b = s.ball.unwrap();
    assert_eq!((b.x, b.dirx, b.vy, b.visible), (START_X + 1, 1, vy, false));
    assert_eq!(b.row, wrap - 2 + vy);
}

#[test]
fn start_mid_rally_restarts_but_keeps_the_ball_rows() {
    let mut c = rally(GameKind::Pong, 120, 77, -1, -2);
    c.scores = (7, 3);
    c.volleys = 9;
    step(&mut c, &Input { start: true, ..away() });
    let s = c.status();
    assert_eq!((s.left_score, s.right_score, s.volleys, s.phase), (0, 0, 0, Phase::Waiting));
    assert_eq!(s.ball, Some(BallState { x: 113, row: 75, dirx: 1, vy: -2, visible: false }));
    // press_start is the same, applied at once.
    let mut d = rally(GameKind::Pong, 120, 77, -1, -2);
    d.scores = (7, 3);
    d.press_start();
    step(&mut d, &away());
    assert_eq!(d.status(), c.status());
}

#[test]
fn start_while_waiting_keeps_the_hidden_ball_rows() {
    // Only Start from Attract takes the row and vy from the light show.
    let mut c = Console::new(GameKind::Pong);
    c.debug_set_ball(BallState { x: 30, row: 150, dirx: -1, vy: 2, visible: false }, 0);
    step(&mut c, &Input { start: true, ..away() });
    let s = c.status();
    assert_eq!((s.left_score, s.right_score, s.phase), (0, 0, Phase::Waiting));
    assert_eq!(s.ball, Some(BallState { x: 113, row: 152, dirx: 1, vy: 2, visible: false }));
}

#[test]
fn start_then_serve_after_202_frames() {
    // verify_game_flow.md #7: 3.371 s from Start to the first serve.
    for (start_tick, row, vy) in [(1, 32, -1), (6, 27, -1)] {
        let mut c = Console::new(GameKind::Pong);
        for _ in 1..start_tick {
            step(&mut c, &Input::default());
        }
        assert_eq!(step(&mut c, &Input { start: true, ..Input::default() }), []);
        for k in 1..=201 {
            assert_eq!(step(&mut c, &Input::default()), [], "S + {k}");
            assert_eq!(c.phase(), Phase::Waiting);
        }
        assert_eq!(step(&mut c, &Input::default()), [SoundEvent::Serve]);
        let b = ball(&c);
        assert_eq!((b.x, b.row, b.dirx, b.vy, b.visible), (87, row, 1, vy, true));
        assert_eq!(c.status().volleys, 0);
    }
}

#[test]
fn first_start_in_the_video() {
    // game_flow.md §6.1: Start at 483.016 s (the first play frame), about
    // frame 1160 after power-on at 60.19 Hz; the light show then moves up.
    // ball_tracks.csv rally 1: the first serve is at x 87, row 159, moving
    // up (x + row = 246 over the next second). The hidden ball, which
    // comes back into the box at row -17 after the blanking, must not
    // turn around there.
    let mut c = Console::new(GameKind::Catch);
    for _ in 1..1160 {
        step(&mut c, &Input::default());
    }
    assert_eq!(c.ls.vy, -1);
    step(&mut c, &Input { start: true, ..Input::default() });
    assert_eq!(ticks_until(&mut c, &Input::default(), SoundEvent::Serve, 300), 202);
    let b = ball(&c);
    assert_eq!((b.x, b.row, b.dirx, b.vy), (87, 159, 1, -1));
}

#[test]
fn select_mid_rally_switches_the_playfield_only() {
    let mut c = rally(GameKind::Pong, 20, 100, -1, 0);
    c.scores = (4, 2);
    c.volleys = 2;
    let before = c.status();
    c.select(GameKind::Catch);
    assert_eq!(c.status(), Status { kind: GameKind::Catch, ..before });
    // The playfield switches in this tick, for collisions too: Pong's
    // paddle at top 98 would reflect the ball, Catch's hole lets it pass.
    let mut pong = rally(GameKind::Pong, 20, 100, -1, 0);
    let mut catch = pong.clone();
    assert_eq!(step(&mut pong, &knobs(98, 98)), [PADDLE]);
    let switch = Input { select: Some(GameKind::Catch), ..knobs(98, 98) };
    assert_eq!(step(&mut catch, &switch), []);
    assert_eq!(catch.kind(), GameKind::Catch);
    assert_eq!((ball(&catch).x, ball(&catch).dirx), (19, -1));
    // The drawn objects switch too.
    let frame = rendered(&catch);
    assert_eq!(frame.get(LEFT_BACK_X, 50), Cell::Foreground, "Catch wall above the hole");
    assert_eq!(frame.get(LEFT_FORWARD_X, 100), Cell::Background);
}

#[test]
fn select_in_attract_only_changes_the_objects() {
    let mut c = Console::new(GameKind::Pong);
    let mut d = c.clone();
    step(&mut c, &Input { select: Some(GameKind::SuperPong), ..Input::default() });
    step(&mut d, &Input::default());
    assert_eq!(c.kind(), GameKind::SuperPong);
    assert_eq!(c.status(), Status { kind: GameKind::SuperPong, ..d.status() });
    assert_eq!((c.ls.h0, c.ls.vc, c.ls.vy, c.ls.last_wrap), (d.ls.h0, d.ls.vc, d.ls.vy, d.ls.last_wrap));
    let (fc, fd) = (rendered(&c), rendered(&d));
    assert_eq!(fc.get(LEFT_FORWARD_X, 110), Cell::Foreground);
    // Outside the forwards' columns the frames are the same.
    for row in DISPLAY_ROW0..DISPLAY_ROW0 + DISPLAY_H {
        for x in (DISPLAY_X0..DISPLAY_X0 + DISPLAY_W)
            .filter(|&x| !(80..84).contains(&x) && !(96..100).contains(&x))
        {
            assert_eq!(fc.get(x, row), fd.get(x, row), "({x}, {row})");
        }
    }
}

#[test]
fn selecting_the_current_game_does_nothing() {
    let mut c = rally(GameKind::Solitaire, 60, 70, 1, 2);
    let mut d = c.clone();
    let mut events = (Vec::new(), Vec::new());
    for _ in 0..300 {
        c.tick(&Input { select: Some(GameKind::Solitaire), ..knobs(60, 60) }, &mut events.0);
        d.tick(&knobs(60, 60), &mut events.1);
    }
    assert_eq!(events.0, events.1);
    assert_eq!(c.status(), d.status());
}

#[test]
fn start_and_select_in_the_same_tick() {
    let mut c = Console::new(GameKind::Pong);
    step(&mut c, &Input { start: true, select: Some(GameKind::Catch), ..Input::default() });
    assert_eq!((c.kind(), c.phase()), (GameKind::Catch, Phase::Waiting));
}

// ---------------------------------------------------------------------
// Regression scenarios from the reference video
// ---------------------------------------------------------------------

#[test]
fn display_durations_after_an_exit() {
    // verify_game_flow.md #1: 2.327 s after a right exit, 2.263 s after a
    // left exit; the ball comes back at the net in the exit direction.
    for (x, dirx, frames, serve_x) in [(170, 1, 140, 87), (4, -1, 136, 90)] {
        let mut c = rally(GameKind::Pong, x, 100, dirx, 0);
        assert_eq!(ticks_until(&mut c, &away(), SoundEvent::Point, 10), 5);
        let mut digits = 1; // the exit frame
        loop {
            let events = step(&mut c, &away());
            if events.contains(&SoundEvent::Serve) {
                break;
            }
            assert!(c.status().digits_visible);
            digits += 1;
        }
        assert_eq!(digits, frames, "dirx {dirx}");
        assert_eq!(ball(&c), BallState { x: serve_x, row: 100, dirx, vy: 0, visible: true });
    }
}

#[test]
fn hidden_phase_serve_rows() {
    // Exit legs of measured rallies replayed through the hidden phase
    // (verify_game_flow.md §6, ball_physics.md §6; spec §6.9): with the
    // 227 / 228 wraps every |vy| = 1 serve leg matches the video exactly.
    // (rally, set-up x, row, dirx, vy; ticks to the exit; exit x, row;
    // serve x, row, vy)
    let cases = [
        (4, (3, 132, -1, -1), 4, (-1, 128), (90, 38, 1)),
        (5, (2, 126, -1, 1), 3, (-1, 129), (90, 145, -1)),
        (7, (2, 178, -1, 1), 3, (-1, 181), (90, 54, 1)), // wraps vertically
        (9, (3, 139, -1, -1), 4, (-1, 135), (90, 31, 1)),
        (22, (3, 98, -1, -1), 4, (-1, 94), (90, 72, 1)),
        (46, (3, 83, -1, 0), 4, (-1, 83), (90, 83, 0)),
        (2, (172, 184, 1, 1), 3, (175, 187), (87, 64, 1)),
        (33, (172, 143, 1, -1), 3, (175, 140), (87, 30, 1)),
        (37, (173, 176, 1, -1), 2, (175, 174), (87, 34, -1)),
        (38, (173, 82, 1, 1), 2, (175, 84), (87, 186, -1)),
        (39, (173, 100, 1, -1), 2, (175, 98), (87, 72, 1)),
        (50, (172, 118, 1, 0), 3, (175, 118), (87, 118, 0)),
        (54, (172, 60, 1, 1), 3, (175, 63), (87, 203, 1)),
    ];
    for (rally_id, (x, row, dirx, vy), exit_ticks, exit, serve) in cases {
        let mut c = rally(GameKind::Pong, x, row, dirx, vy);
        assert_eq!(ticks_until(&mut c, &away(), SoundEvent::Point, 10), exit_ticks, "rally {rally_id}");
        assert_eq!((ball(&c).x, ball(&c).row), exit, "rally {rally_id}");
        let frames = if dirx > 0 { 140 } else { 136 };
        assert_eq!(ticks_until(&mut c, &away(), SoundEvent::Serve, 300), frames, "rally {rally_id}");
        let b = ball(&c);
        assert_eq!((b.x, b.row, b.vy), serve, "rally {rally_id}");
    }
}

#[test]
fn pong_rally_43() {
    // 869.97-879.38 s (paddle_hits.csv rally 43): four returns, the speed-up
    // after the 4th and a miss 78 frames after the last hit.
    let mut c = rally(GameKind::Pong, 95, 96, 1, -3);
    let (mut left, mut right) = (89, 126);
    let mut log = Vec::new();
    for tick in 1..=600 {
        for e in step(&mut c, &knobs(left, right)) {
            let b = c.status().ball.unwrap();
            match e {
                PADDLE if b.dirx < 0 => right = 48,
                PADDLE => left = 56,
                SoundEvent::Point => {}
                _ => continue,
            }
            log.push((tick, e, b.x, b.row, b.vy, c.status().volleys));
        }
        if c.phase() == Phase::Waiting {
            break;
        }
    }
    assert_eq!(
        log,
        [
            (63, PADDLE, 158, 123, -3, 1),             // 871.079 s, delta -3
            (202, PADDLE, 19, 90, -3, 2),              // 873.464 s, delta 1
            (341, PADDLE, 158, 57, 0, 3),              // 875.783 s, delta 9
            (480, PADDLE, 19, 57, -3, 4),              // 878.069 s, delta 1 -> tier 2
            (558, SoundEvent::Point, 175, 207, -3, 0), // 879.38 s
        ]
    );
    assert_eq!(c.scores(), (1, 0));
}

#[test]
fn catch_rally_24() {
    // 704.74-713.94 s: the first five wall bounces, the fifth at tier 2
    // turning around at x 18.
    let mut c = rally(GameKind::Catch, 84, 90, -1, 3);
    let (left, right) = ([116, 144, 132], [186, 144, 60]);
    let (mut li, mut ri) = (0, 0);
    let mut log = Vec::new();
    for tick in 1..=600 {
        let input = knobs(left[li.min(2)], right[ri.min(2)]);
        for e in step(&mut c, &input) {
            if e == WALL {
                let b = ball(&c);
                if b.dirx > 0 {
                    li += 1
                } else {
                    ri += 1
                }
                log.push((tick, b.x, b.row, b.vy, c.status().volleys));
            }
        }
        if log.len() == 5 {
            break;
        }
    }
    assert_eq!(
        log,
        [
            (65, 19, 129, 3, 1),    // 705.846 s
            (204, 158, 162, -3, 2), // 708.132 s
            (343, 19, 129, -3, 3),  // 710.451 s
            (482, 158, 96, -3, 4),  // 712.770 s
            (552, 18, 144, 3, 5),   // 713.938 s
        ]
    );
}

#[test]
fn power_on_light_show_bounces() {
    // light_show_and_digits.md §1.5: bounces at 464.21, 467.38, 470.55 and
    // 473.72 s, a skip at 476.86-477.18 s (dead zone), then 481.21 s. The
    // power-on frame is the first console frame of the video, at 463.730 s,
    // and the light show runs at 60.19 Hz.
    let mut c = Console::new(GameKind::Catch);
    let mut flips = Vec::new();
    for tick in 1..=1100 {
        let vy = c.ls.vy;
        step(&mut c, &Input::default());
        if tick == 790 {
            // A bottom wrap with h0 in the dead zone: no bounce.
            assert_eq!((c.ls.last_wrap, c.ls.h0, c.ls.vy), (207, 198, 1));
        }
        if c.ls.vy != vy {
            flips.push((tick, c.ls.last_wrap, c.ls.h0, c.ls.vy));
        }
    }
    assert_eq!(
        flips,
        [(30, 207, 122, -1), (220, 17, 84, 1), (410, 207, 46, -1), (600, 17, 8, 1), (1053, 207, 5, -1)]
    );
    let video = [464.21, 467.38, 470.55, 473.72, 481.21];
    for (&(tick, ..), t) in flips.iter().zip(video) {
        let model = 463.730 + f64::from(tick) / FRAME_HZ_ATTRACT;
        assert!((model - t).abs() < 0.034, "tick {tick}: {model:.2} s vs {t} s");
    }
    // Bottom to bottom without a skip: 380 frames (verify_light_show_digits.md #6).
    assert_eq!(flips[2].0 - flips[0].0, 380);
}

#[test]
fn attract_tick_every_228_frames() {
    // verify_game_flow.md #4: one tick sound every 3.7883 s.
    let mut c = Console::new(GameKind::Pong);
    let mut ticks = Vec::new();
    for tick in 1..=1000 {
        let events = step(&mut c, &Input::default());
        if !events.is_empty() {
            assert_eq!(events, [SoundEvent::AttractTick]);
            assert_eq!(c.ls.h0, ATTRACT_TICK_H0);
            ticks.push(tick);
        }
    }
    assert_eq!(ticks, [218, 446, 674, 902]);
    assert!((228.0 / FRAME_HZ_ATTRACT - 3.7883).abs() < 0.001);
}

#[test]
fn the_wrap_row_is_22_plus_vy_lines_tall() {
    // light_show_and_digits.md §1.2, in every attract frame where the tall
    // row is wholly inside the window.
    let mut c = Console::new(GameKind::Pong);
    let mut checked = 0;
    for _ in 0..2000 {
        let vy = c.ls.vy;
        step(&mut c, &Input::default());
        let l = c.ls.last_wrap;
        if l - (6 + vy) < 1 || l + 16 >= FIELD_H {
            continue;
        }
        let lit = |row: i32| c.ls.rows_lit[row as usize];
        let (first, last) = (l - (6 + vy), l + 15);
        assert!((first..=last).all(lit) && !lit(first - 1) && !lit(last + 1), "wrap at {l}, vy {vy}");
        checked += 1;
    }
    assert!(checked > 1000);
}

#[test]
fn game_over_starts_the_light_show_from_the_ball() {
    // timeline.md segment 14: the frame after the 15th point is light show,
    // continuing the invisible ball's vy, in the phase measured at
    // 1015.948 s (spec §10.6).
    let mut c = rally(GameKind::Pong, 172, 84, 1, 1);
    c.scores = (14, 4);
    assert_eq!(ticks_until(&mut c, &away(), SoundEvent::Point, 5), 3);
    assert_eq!(c.phase(), Phase::Attract);
    assert_eq!(c.scores(), (15, 4));
    // x 175, row 87: h0 = 175 and this frame's wrap on row + 8.
    assert_eq!((c.ls.h0, c.ls.last_wrap, c.ls.vy), (175, 95, 1));
    for k in 1..=4 {
        step(&mut c, &away());
        assert_eq!((c.ls.h0, c.ls.last_wrap), (175 + k, 95 + k));
    }
    // Start reads the light show with the attract mapping (row = wrap - 2).
    step(&mut c, &Input { start: true, ..away() });
    let b = ball(&c);
    assert_eq!((b.x, b.row, b.vy), (113, 98, 1));
}

#[test]
fn game_over_light_show_bounces() {
    // The video's only game over: the 15th point is x 174 -> 175, rows
    // 84 -> 86, vy +2 (ball_tracks.csv rally 58), and frame G + 1 is the
    // first console frame at 1015.948 s. light_show_and_digits.md §1.5:
    // bounces at 1016.87 s (bottom, h0 3) and 1018.47 s (top, h0 99).
    let mut c = rally(GameKind::Pong, 174, 84, 1, 2);
    c.scores = (14, 4);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    assert_eq!((c.phase(), c.ls.h0, c.ls.last_wrap), (Phase::Attract, 175, 94));
    let mut flips = Vec::new();
    for k in 1..=200 {
        let vy = c.ls.vy;
        step(&mut c, &away());
        if c.ls.vy != vy {
            flips.push((k, c.ls.h0, c.ls.vy));
        }
    }
    assert_eq!(flips, [(57, 4, -2), (153, 100, 2)]);
    for (&(k, ..), t) in flips.iter().zip([1016.87, 1018.47]) {
        let model = 1015.948 + f64::from(k - 1) / FRAME_HZ_ATTRACT;
        assert!((model - t).abs() < 0.034, "G + {k}: {model:.3} s vs {t} s");
    }
}

#[test]
fn game_over_frame_can_bounce_the_light_show() {
    // The wrap line is row + 8, so a left exit moving down at rows
    // 199..=204 puts the first wrap in the bottom zone, with h0 227 live.
    let mut c = rally(GameKind::Pong, 0, 199, -1, 1);
    c.scores = (4, 14);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    assert_eq!((c.phase(), c.ls.h0, c.ls.last_wrap, c.ls.vy), (Phase::Attract, 227, 208, -1));
    // A right exit starts in the dead zone (h0 175): no bounce.
    let mut c = rally(GameKind::Pong, 174, 199, 1, 1);
    c.scores = (14, 4);
    assert_eq!(step(&mut c, &away()), [SoundEvent::Point]);
    assert_eq!((c.ls.h0, c.ls.last_wrap, c.ls.vy), (175, 208, 1));
}

// ---------------------------------------------------------------------
// Golden frames (Frame::to_ascii), from the spec mirror
// ---------------------------------------------------------------------

fn assert_frame(c: &Console, golden: &str, name: &str) {
    let ascii = rendered(c).to_ascii();
    for (i, (got, want)) in ascii.lines().zip(golden.lines()).enumerate() {
        assert_eq!(got, want, "{name}: row {} differs", i as i32 + DISPLAY_ROW0);
    }
    assert_eq!(ascii, golden, "{name}");
}

#[test]
fn golden_power_on_frame() {
    let c = Console::new(GameKind::Catch);
    assert_frame(&c, include_str!("../testdata/power_on_catch.txt"), "power-on");
}

#[test]
fn golden_light_show_frame() {
    // img/ls_compare_power_on_t470.00.png: 376 frames after power-on.
    let mut c = Console::new(GameKind::Catch);
    for _ in 0..376 {
        step(&mut c, &Input::default());
    }
    assert_eq!((c.ls.h0, c.ls.last_wrap), (12, 173));
    assert_frame(&c, include_str!("../testdata/attract_catch_t376.txt"), "light show t376");
}

#[test]
fn golden_super_pong_rally_frame() {
    let mut c = Console::new(GameKind::SuperPong);
    step(&mut c, &knobs(40, 150));
    c.debug_set_ball(BallState { x: 61, row: 101, dirx: 1, vy: 1, visible: true }, 0);
    assert_frame(&c, include_str!("../testdata/superpong_rally.txt"), "Super Pong rally");
}

#[test]
fn golden_solitaire_rally_frame() {
    let mut c = Console::new(GameKind::Solitaire);
    step(&mut c, &knobs(120, 69));
    c.debug_set_ball(BallState { x: 100, row: 50, dirx: -1, vy: -2, visible: true }, 0);
    assert_frame(&c, include_str!("../testdata/solitaire_rally.txt"), "Solitaire rally");
}

#[test]
fn golden_pong_waiting_frame_10_5() {
    let mut c = Console::new(GameKind::Pong);
    for (n, x, dirx) in [(10, 174, 1), (5, 0, -1)] {
        for _ in 0..n {
            c.debug_set_ball(BallState { x, row: 100, dirx, vy: 0, visible: true }, 0);
            assert_eq!(step(&mut c, &knobs(20, 180)), [SoundEvent::Point]);
        }
    }
    assert_eq!((c.scores(), c.phase()), ((10, 5), Phase::Waiting));
    assert_frame(&c, include_str!("../testdata/pong_waiting_10_5.txt"), "Pong 10-5");
}

#[test]
fn golden_game_over_frame() {
    let mut c = Console::new(GameKind::Pong);
    for (n, x, dirx) in [(14, 174, 1), (4, 0, -1)] {
        for _ in 0..n {
            c.debug_set_ball(BallState { x, row: 100, dirx, vy: 0, visible: true }, 0);
            step(&mut c, &away());
        }
    }
    c.debug_set_ball(BallState { x: 172, row: 84, dirx: 1, vy: 1, visible: true }, 0);
    assert_eq!(ticks_until(&mut c, &away(), SoundEvent::Point, 5), 3);
    assert_frame(&c, include_str!("../testdata/game_over_15_4.txt"), "game over 15-4");
}

// ---------------------------------------------------------------------
// Determinism, invariants, differential trace
// ---------------------------------------------------------------------

/// A scripted player for long runs: knobs that track the ball with an
/// offset that changes now and then (so rallies end), occasional Start and
/// Game Select. It matches the generator of `testdata/trace.txt`.
struct Script {
    rng: Rng,
    off: (i32, i32),
    tops: (i32, i32),
}

impl Script {
    fn new(seed: u64) -> Script {
        Script { rng: Rng(seed), off: (0, 0), tops: (105, 105) }
    }

    fn input(&mut self, c: &Console) -> Input {
        let (r1, r2, r3) = (self.rng.next(), self.rng.next(), self.rng.next());
        if r1 % 50 == 0 {
            self.off.0 = (r2 % 41) as i32 - 20;
        }
        if (r1 >> 16) % 50 == 0 {
            self.off.1 = ((r2 >> 16) % 41) as i32 - 20;
        }
        let start = (c.phase() == Phase::Attract && (r3 >> 16) % 300 == 0) || r3 % 25000 == 0;
        let select =
            ((r3 >> 32) % 2500 == 0).then(|| GameKind::from_number(((r3 >> 48) % 4) as u8 + 1).unwrap());
        if c.phase() == Phase::Rally {
            let row = ball(c).row;
            self.tops = (row - 6 + self.off.0, row - 6 + self.off.1);
        }
        Input { left: knob_for_top(self.tops.0), right: knob_for_top(self.tops.1), start, select }
    }
}

#[test]
fn differential_trace_against_the_spec_mirror() {
    // `testdata/trace.txt` was written by the spec mirror running the same
    // script: every event, a status line every 500 ticks and a frame hash
    // every 2000 ticks.
    let text = include_str!("../testdata/trace.txt");
    let mut runs: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        if line.starts_with("run ") {
            runs.push((line.to_string(), Vec::new()));
        } else {
            runs.last_mut().expect("a run header").1.push(line);
        }
    }
    assert_eq!(runs.len(), 3);
    for (header, expected) in runs {
        let mut words = header.split(' ').skip(1);
        let kind = GameKind::from_number(words.next().unwrap().parse().unwrap()).unwrap();
        let seed = u64::from_str_radix(words.next().unwrap(), 16).unwrap();
        let ticks: u64 = words.next().unwrap().parse().unwrap();
        let mut c = Console::new(kind);
        let mut script = Script::new(seed);
        let mut got = Vec::new();
        for t in 1..=ticks {
            let input = script.input(&c);
            for e in step(&mut c, &input) {
                got.push(format!("{t} {e:?}"));
            }
            if t % 500 == 0 {
                let s = c.status();
                let b = s.ball.map_or("-".to_string(), |b| format!("{} {} {} {}", b.x, b.row, b.dirx, b.vy));
                got.push(format!(
                    "{t} S {:?} {} {} {} {} {} {} {b}",
                    s.phase,
                    s.kind.number(),
                    s.left_score,
                    s.right_score,
                    s.volleys,
                    s.left_top,
                    s.right_top
                ));
            }
            if t % 2000 == 0 {
                got.push(format!("{t} F {:016x}", fnv64(&rendered(&c).to_ascii())));
            }
        }
        for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
            assert_eq!(g, e, "{header}: line {i}");
        }
        assert_eq!(got.len(), expected.len(), "{header}");
    }
}

#[test]
fn determinism() {
    let run = || {
        let mut c = Console::new(GameKind::SuperPong);
        let mut script = Script::new(0xD1CE_5EED_0000_0001);
        let mut events = Vec::new();
        let mut hashes = Vec::new();
        for t in 1..=100_000 {
            let input = script.input(&c);
            c.tick(&input, &mut events);
            if t % 1000 == 0 {
                hashes.push(fnv64(&rendered(&c).to_ascii()));
            }
        }
        (events, hashes, c.status())
    };
    let (a, b) = (run(), run());
    assert!(a.0.len() > 1000);
    assert_eq!(a, b);
}

/// Checks the invariants of one rendered frame (spec §2.2, §10.1).
fn check_frame(c: &Console, frame: &Frame) {
    for row in DISPLAY_ROW0..DISPLAY_ROW0 + DISPLAY_H {
        for x in DISPLAY_X0..DISPLAY_X0 + DISPLAY_W {
            let in_box = (0..FIELD_W).contains(&x) && (0..FIELD_H).contains(&row);
            match frame.get(x, row) {
                Cell::Background => {}
                Cell::Border => assert!(
                    in_box && (TOP_BORDER.contains(&row) || BOTTOM_BORDER.contains(&row)),
                    "border at ({x}, {row})"
                ),
                Cell::Foreground => assert!(in_box, "foreground at ({x}, {row})"),
            }
        }
    }
    if c.phase == Phase::Attract {
        // No blocks in the score band: there the light show looks exactly
        // like the same console waiting to serve.
        let mut waiting = c.clone();
        waiting.phase = Phase::Waiting;
        let plain = rendered(&waiting);
        for row in LS_SCORE_BAND {
            assert_eq!(frame.row(row), plain.row(row), "score band row {row}");
        }
    }
}

#[test]
fn invariants_over_a_long_random_run() {
    let mut c = Console::new(GameKind::Pong);
    let mut script = Script::new(0x2545_F491_4F6C_DD1D);
    let mut rng = Rng(0x0BAD_C0FF_EE15_600D);
    let (mut points, mut game_overs, mut serves, mut doubles, mut ticks) = (0, 0, 0, 0, 0);
    let mut frame = Frame::new();
    for t in 0..1_000_000u32 {
        let mut input = script.input(&c);
        // Now and then the knobs jump anywhere, the paddles land on the
        // ball and the switch moves mid-rally.
        if rng.next().is_multiple_of(64) {
            input.left = (rng.next() % 1000) as f32 / 999.0;
            input.right = (rng.next() % 1000) as f32 / 999.0;
        }
        if rng.next().is_multiple_of(4000) {
            input.select = GameKind::from_number((rng.next() % 4) as u8 + 1);
        }
        let before = c.status();
        let events = step(&mut c, &input);
        let s = c.status();
        assert_eq!(s.frame, before.frame + 1);
        assert!(s.left_score <= WIN_SCORE && s.right_score <= WIN_SCORE);
        assert_eq!(s.digits_visible, s.phase != Phase::Rally);
        assert_eq!(s.ball.is_none(), s.phase == Phase::Attract);
        if let Some(b) = s.ball {
            assert!(b.dirx == 1 || b.dirx == -1);
            assert!((-3..=3).contains(&b.vy));
            if b.visible {
                assert!((0..=174).contains(&b.x), "visible x {b:?}");
                // 10..=210 in a rally, but a ball can be served moving up
                // from below the bottom border (spec §6.3 step 3), and a
                // flat ball after a game over and Start comes back up to
                // 6 rows lower (spec §10.6).
                assert!((10..ROW_MIN + PLAY_LINES).contains(&b.row), "visible row {b:?}");
            } else {
                assert!((HIDDEN_X_MIN..=202).contains(&b.x), "hidden x {b:?}");
                assert!((ROW_MIN..223).contains(&b.row), "hidden row {b:?}");
            }
        }
        // Each event only on its transition.
        let played = before.phase == Phase::Rally && !input.start;
        for &e in &events {
            match e {
                SoundEvent::Bounce(_) => assert!(played),
                SoundEvent::Point => {
                    assert!(played && s.phase != Phase::Rally);
                    points += 1;
                    game_overs += usize::from(s.phase == Phase::Attract);
                }
                SoundEvent::Serve => {
                    assert!(before.phase == Phase::Waiting && !input.start);
                    assert_eq!(s.phase, Phase::Rally);
                    serves += 1;
                }
                SoundEvent::AttractTick => {
                    assert!(before.phase == Phase::Attract && s.phase == Phase::Attract && !input.start);
                    ticks += 1;
                }
            }
        }
        if events.contains(&SoundEvent::Point) {
            assert!(!events.contains(&SoundEvent::Serve));
        }
        let bounces = events.iter().filter(|e| matches!(e, SoundEvent::Bounce(_))).count();
        assert!(bounces <= 2);
        if bounces == 2 {
            assert_eq!(events[0], BORDER);
            doubles += 1;
        }
        if s.phase != before.phase && !input.start {
            let expected = match (before.phase, s.phase) {
                (Phase::Rally, Phase::Waiting | Phase::Attract) => SoundEvent::Point,
                (Phase::Waiting, Phase::Rally) => SoundEvent::Serve,
                other => panic!("transition {other:?}"),
            };
            assert!(events.contains(&expected));
        }
        if t % 997 == 0 {
            c.render(&mut frame);
            check_frame(&c, &frame);
        }
    }
    // The run reached every kind of event.
    assert!(
        points > 1000 && game_overs > 5 && serves > 1000 && ticks > 10 && doubles > 5,
        "points {points}, game overs {game_overs}, serves {serves}, ticks {ticks}, doubles {doubles}"
    );
}
