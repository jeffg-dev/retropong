//! The ball in play (spec §6-§8): the per-frame update, the paddles and
//! walls of each game, scoring, game over and Start (spec §11.1).

use crate::light_show::LightShow;
use crate::{
    BounceKind, Console, GameKind, Phase, Side, SoundEvent, BALL_H, BALL_HIT_W, BOTTOM_BOUNCE_LINE,
    CATCH_WALL_VY_ABOVE, CATCH_WALL_VY_BELOW, FIELD_H, FIELD_W, HIDDEN_WRAP_LEFT, HIDDEN_WRAP_RIGHT,
    HIDDEN_X_MIN, HOLE_H, LEFT_BACK_X, LEFT_FORWARD_X, LIVE_X_MAX, PADDLE_H, PADDLE_W, PLAY_LINES,
    RIGHT_BACK_X, RIGHT_FORWARD_X, ROW_MIN, SEGMENT_VY, SERVE_X_LEFT, SERVE_X_RIGHT, START_X,
    TIER_THRESHOLDS, TOP_BOUNCE_ROW, TOP_BOUNCE_ROW_MIN, VX_BY_TIER, WALL_SEGMENT_PERIOD, WIN_SCORE,
};

/// The ball as the console tracks it (spec §6.1). Whether it is drawn
/// follows from the phase: only in `Rally`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ball {
    /// Left column; drawn `x..x + BALL_W`, collides `x..x + BALL_HIT_W`.
    pub(crate) x: i32,
    /// Top row; rows `row..row + BALL_H`.
    pub(crate) row: i32,
    /// `+1` moving right, `-1` moving left.
    pub(crate) dirx: i32,
    /// Lines per frame, `-3..=3`, positive down.
    pub(crate) vy: i32,
}

/// How an object is drawn, which ball rows it meets and what vy it sends
/// back (spec §4.2, §6.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// A back or forward paddle: rows `top..top + PADDLE_H`.
    Paddle,
    /// A Catch wall: every row except the hole `top..top + HOLE_H`.
    CatchWall,
    /// The Solitaire back wall: rows `top..FIELD_H`.
    SolitaireWall,
}

/// One paddle or wall of the selected game, at its knob row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Object {
    pub(crate) shape: Shape,
    /// Left x; the object is `PADDLE_W` clocks wide.
    pub(crate) x: i32,
    /// Paddle top, hole top or wall top: the player's knob row.
    pub(crate) top: i32,
    /// The direction of a ball this object reflects: `-1` for the left
    /// player's objects (balls moving left), `+1` for the right player's.
    pub(crate) reflects: i32,
}

impl Object {
    /// Whether the object lights `row` (spec §14.3).
    pub(crate) fn covers(&self, row: i32) -> bool {
        match self.shape {
            Shape::Paddle => (self.top..self.top + PADDLE_H).contains(&row),
            Shape::CatchWall => (0..FIELD_H).contains(&row) && !(self.top..self.top + HOLE_H).contains(&row),
            Shape::SolitaireWall => (self.top..FIELD_H).contains(&row),
        }
    }

    /// Whether a ball with top row `r` (rows `r..r + 4`) meets the object
    /// vertically (spec §6.4).
    fn rows_overlap(&self, r: i32) -> bool {
        let t = self.top;
        match self.shape {
            Shape::Paddle => r < t + PADDLE_H && r + BALL_H > t,
            Shape::CatchWall => (r.max(0)..(r + BALL_H).min(FIELD_H)).any(|row| row < t || row >= t + HOLE_H),
            Shape::SolitaireWall => r + BALL_H > t && r < FIELD_H,
        }
    }

    /// Outgoing vy for a ball with top row `r`. It depends only on where
    /// the ball meets the object, never on how it arrived (spec §6.4).
    fn new_vy(&self, r: i32) -> i32 {
        let t = self.top;
        match self.shape {
            Shape::Paddle => SEGMENT_VY[(r - t).div_euclid(2).clamp(0, 7) as usize],
            Shape::CatchWall => {
                if r < t {
                    CATCH_WALL_VY_ABOVE
                } else {
                    CATCH_WALL_VY_BELOW
                }
            }
            Shape::SolitaireWall => {
                if r < t {
                    SEGMENT_VY[0] // -3, as off a paddle's top segment
                } else {
                    SEGMENT_VY[((r - t).rem_euclid(WALL_SEGMENT_PERIOD) / 2) as usize]
                }
            }
        }
    }

    fn bounce_kind(&self) -> BounceKind {
        match self.shape {
            Shape::Paddle => BounceKind::Paddle,
            Shape::CatchWall | Shape::SolitaireWall => BounceKind::Wall,
        }
    }
}

/// The paddles and walls of `kind`, in the order of the spec §4.2 table
/// (which is also the collision test order).
pub(crate) fn objects(kind: GameKind, left_top: i32, right_top: i32) -> impl Iterator<Item = Object> {
    let (left_back, right_back, forwards) = match kind {
        GameKind::Catch => (Shape::CatchWall, Shape::CatchWall, false),
        GameKind::Solitaire => (Shape::Paddle, Shape::SolitaireWall, false),
        GameKind::SuperPong => (Shape::Paddle, Shape::Paddle, true),
        GameKind::Pong => (Shape::Paddle, Shape::Paddle, false),
    };
    let left = |shape, x| Object { shape, x, top: left_top, reflects: -1 };
    let right = |shape, x| Object { shape, x, top: right_top, reflects: 1 };
    [
        Some(left(left_back, LEFT_BACK_X)),
        forwards.then(|| left(Shape::Paddle, LEFT_FORWARD_X)),
        forwards.then(|| right(Shape::Paddle, RIGHT_FORWARD_X)),
        Some(right(right_back, RIGHT_BACK_X)),
    ]
    .into_iter()
    .flatten()
}

/// Horizontal speed of the visible ball after `volleys` reflections (spec §7).
pub(crate) fn tier_speed(volleys: u32) -> i32 {
    VX_BY_TIER[TIER_THRESHOLDS.iter().filter(|&&t| volleys >= t).count()]
}

/// Whether borders (and the light show's bounces) act on a ball at `x`.
pub(crate) fn is_live(x: i32) -> bool {
    (0..=LIVE_X_MAX).contains(&x)
}

impl Console {
    /// One frame of `Waiting` or `Rally` (spec §6.3). The order of the
    /// steps is normative.
    pub(crate) fn play_frame(&mut self, events: &mut Vec<SoundEvent>) {
        let was_hidden = self.phase == Phase::Waiting;
        let b = &mut self.ball;

        // 1. Move. A collision is tested at the moved position; the new
        // velocity takes effect on the next frame's move.
        let speed = if was_hidden { 1 } else { tier_speed(self.volleys) };
        b.x += b.dirx * speed;
        b.row += b.vy;

        // 2. The hidden ball wraps through horizontal blanking; every ball
        // is kept in the frame's lines (no effect on a visible ball).
        if was_hidden {
            let wrap = if b.dirx > 0 { HIDDEN_WRAP_RIGHT } else { HIDDEN_WRAP_LEFT };
            b.x = (b.x - HIDDEN_X_MIN).rem_euclid(wrap) + HIDDEN_X_MIN;
        }
        b.row = (b.row - ROW_MIN).rem_euclid(PLAY_LINES) + ROW_MIN;

        // 3. Borders: the first frame the ball's top row reaches row 15 or
        // its bottom row (row + 3) reaches line 208, only while the ball is
        // inside the border box's columns. The top test ends 22 rows up, in
        // vertical blanking, so a hidden ball that comes back into the box
        // higher up keeps going up and wraps. Hidden-ball reversals are
        // silent.
        let top = (TOP_BOUNCE_ROW_MIN..=TOP_BOUNCE_ROW).contains(&b.row);
        if is_live(b.x) && ((b.vy < 0 && top) || (b.vy > 0 && b.row + BALL_H > BOTTOM_BOUNCE_LINE)) {
            b.vy = -b.vy;
            if !was_hidden {
                events.push(SoundEvent::Bounce(BounceKind::Border));
            }
        }

        // 4. The hidden ball: the serve test is all that is left.
        if was_hidden {
            if (b.dirx > 0 && b.x == SERVE_X_RIGHT) || (b.dirx < 0 && b.x == SERVE_X_LEFT) {
                self.phase = Phase::Rally;
                events.push(SoundEvent::Serve);
            }
            return;
        }

        // 5. Paddles and walls: at most one reflection per frame, and only
        // off the object's reflecting side.
        for obj in objects(self.kind, self.left_top, self.right_top) {
            if b.dirx == obj.reflects
                && b.x < obj.x + PADDLE_W
                && b.x + BALL_HIT_W > obj.x
                && obj.rows_overlap(b.row)
            {
                b.dirx = -b.dirx;
                b.vy = obj.new_vy(b.row);
                self.volleys = self.volleys.saturating_add(1);
                events.push(SoundEvent::Bounce(obj.bounce_kind()));
                break;
            }
        }

        // 6. The ball's centre column leaving the border box is a point.
        let centre = b.x + 1;
        let exit = if b.dirx > 0 && centre >= FIELD_W {
            Some(Side::Right)
        } else if b.dirx < 0 && centre <= 0 {
            Some(Side::Left)
        } else {
            None
        };
        if let Some(exit) = exit {
            self.score_point(exit, events);
        }
    }

    /// The ball left through `exit` (spec §6.3 step 6, §8).
    fn score_point(&mut self, exit: Side, events: &mut Vec<SoundEvent>) {
        // In Catch the player on the exit side caught the ball; in every
        // other game the other side scores.
        let scorer = if self.kind == GameKind::Catch { exit } else { exit.other() };
        let score = match scorer {
            Side::Left => &mut self.scores.0,
            Side::Right => &mut self.scores.1,
        };
        *score = (*score + 1).min(WIN_SCORE);
        let won = *score >= WIN_SCORE;
        self.volleys = 0;
        events.push(SoundEvent::Point);
        if won {
            self.enter_attract_from_ball();
        } else {
            // The ball keeps x, row, dirx and vy; it is hidden from this frame.
            self.phase = Phase::Waiting;
        }
    }

    /// Game over: the light show takes over the invisible ball's counters
    /// in this same frame, which is drawn as light show (spec §10.6).
    fn enter_attract_from_ball(&mut self) {
        self.phase = Phase::Attract;
        self.ls = LightShow::from_ball(&self.ball);
    }

    /// Start, in any phase (spec §11.1).
    pub(crate) fn start_game(&mut self) {
        if self.phase == Phase::Attract {
            // The invisible ball keeps its vertical state.
            self.ball.row = self.ls.ball_row();
            self.ball.vy = self.ls.vy;
        }
        self.scores = (0, 0);
        self.ball.x = START_X;
        self.ball.dirx = 1;
        self.volleys = 0;
        self.phase = Phase::Waiting;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paddle(top: i32) -> Object {
        Object { shape: Shape::Paddle, x: RIGHT_BACK_X, top, reflects: 1 }
    }

    #[test]
    fn tier_speeds() {
        let speeds: Vec<i32> = (0..=12).map(tier_speed).collect();
        assert_eq!(speeds, [1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 3]);
        assert_eq!(tier_speed(u32::MAX), 3);
    }

    #[test]
    fn paddle_segments_by_delta() {
        // Delta = R - top runs from -3 to 14 while the ball overlaps.
        let p = paddle(100);
        let vys: Vec<i32> = (97..=114).map(|r| p.new_vy(r)).collect();
        assert_eq!(vys, [-3, -3, -3, -3, -3, -2, -2, -1, -1, 0, 0, 0, 0, 1, 1, 2, 2, 3]);
    }

    #[test]
    fn paddle_rows_edges() {
        let p = paddle(100);
        assert!(!p.rows_overlap(96)); // top - 4
        assert!(p.rows_overlap(97)); // top - 3
        assert!(p.rows_overlap(114)); // top + 14
        assert!(!p.rows_overlap(115)); // top + 15
    }

    #[test]
    fn catch_wall_vy_and_rows() {
        let w = Object { shape: Shape::CatchWall, x: LEFT_BACK_X, top: 100, reflects: -1 };
        assert!(w.rows_overlap(99));
        assert_eq!(w.new_vy(99), -3);
        for r in 100..=111 {
            assert!(!w.rows_overlap(r), "R {r} is inside the hole");
        }
        assert!(w.rows_overlap(112)); // rows 112..116 meet row 115
        assert_eq!(w.new_vy(112), 3);
        // A hole at the very bottom: the rows below the box are not wall.
        let w = Object { top: 209, ..w };
        assert!(!w.rows_overlap(220));
        assert!(w.rows_overlap(205));
    }

    #[test]
    fn solitaire_wall_vy() {
        // Measured wall hits (ball_physics.md §4.4), wall top 69.
        let w = Object { shape: Shape::SolitaireWall, x: RIGHT_BACK_X, top: 69, reflects: 1 };
        for (r, vy) in [(67, -3), (79, 1), (85, -3), (135, -2), (169, -1), (175, 1)] {
            assert!(w.rows_overlap(r));
            assert_eq!(w.new_vy(r), vy, "R {r}");
        }
        assert!(!w.rows_overlap(65)); // R + 4 <= top: over the wall
        assert!(w.rows_overlap(66));
    }

    #[test]
    fn rows_overlap_is_coincidence_with_drawn_rows() {
        // The spec's row tests are exactly "some ball row is a lit object row".
        for shape in [Shape::Paddle, Shape::CatchWall, Shape::SolitaireWall] {
            for top in 0..=209 {
                let obj = Object { shape, x: LEFT_BACK_X, top, reflects: -1 };
                for r in -45..230 {
                    let drawn = (r..r + BALL_H).any(|row| obj.covers(row));
                    assert_eq!(obj.rows_overlap(r), drawn, "{shape:?} top {top} R {r}");
                }
            }
        }
    }

    #[test]
    fn object_lists() {
        let xs = |kind| objects(kind, 1, 2).map(|o| (o.x, o.top, o.reflects)).collect::<Vec<_>>();
        assert_eq!(xs(GameKind::Catch), [(16, 1, -1), (160, 2, 1)]);
        assert_eq!(xs(GameKind::Solitaire), [(16, 1, -1), (160, 2, 1)]);
        assert_eq!(xs(GameKind::SuperPong), [(16, 1, -1), (80, 1, -1), (96, 2, 1), (160, 2, 1)]);
        assert_eq!(xs(GameKind::Pong), [(16, 1, -1), (160, 2, 1)]);
        let shapes = |kind| objects(kind, 1, 2).map(|o| o.shape).collect::<Vec<_>>();
        assert_eq!(shapes(GameKind::Catch), [Shape::CatchWall, Shape::CatchWall]);
        assert_eq!(shapes(GameKind::Solitaire), [Shape::Paddle, Shape::SolitaireWall]);
    }
}
