//! Attract mode: the light show (spec §10). It is the hidden ball's
//! horizontal and vertical counters running free, displayed as blocks.

use crate::play::Ball;
use crate::{
    SoundEvent, ATTRACT_TICK_H0, FIELD_H, LS_BALL_ROW_OFFSET, LS_BOTTOM_ZONE, LS_DEAD_ZONE,
    LS_GAME_OVER_ROW_OFFSET, LS_GAME_OVER_X_OFFSET, LS_H, LS_LINES, LS_SCORE_BAND, LS_TOP_ZONE_MAX,
    LS_TOP_ZONE_WRAP_MIN, PLAY_LINES, POWER_ON_H0, POWER_ON_VY, POWER_ON_WRAP_LINE, ROW_MIN,
};

/// V counter values from this one up to the wrap are lit (the tall row).
const V_TALL_FROM: i32 = 256;
/// Bit of the V counter that blanks every other 16-line row.
const V_ROW_BIT: i32 = 16;
/// Blocks exist in `0..H_BLOCKS_END` of each H cycle; the rest is the gap.
const H_BLOCKS_END: i32 = 224;
/// Bit of the H counter that blanks every other 8-clock block.
const H_BLOCK_BIT: i32 = 8;

/// The light show's free-running counters (spec §10.2).
#[derive(Clone, Debug)]
pub(crate) struct LightShow {
    /// H-counter phase, `0..LS_H`: blocks start where `(x - h0) mod LS_H == 0`.
    pub(crate) h0: i32,
    /// V counter on the last line of the most recently simulated frame.
    pub(crate) vc: i32,
    /// `-3..=3`, positive down: the vertical step per V cycle.
    pub(crate) vy: i32,
    /// Line of the most recent V wrap, relative to line 0 of the current
    /// frame (negative when it was in an earlier frame).
    pub(crate) last_wrap: i32,
    /// Per-line V pattern of the current frame, rows `0..FIELD_H`.
    pub(crate) rows_lit: [bool; FIELD_H as usize],
}

impl LightShow {
    /// The measured power-on state, with frame 0 already simulated (spec §10.4).
    pub(crate) fn power_on() -> LightShow {
        LightShow::with_wrap_on(POWER_ON_H0, POWER_ON_VY, POWER_ON_WRAP_LINE)
    }

    /// Game over: the light show starts from the ball's column and row,
    /// with its vertical speed and direction (spec §10.6). The offsets are
    /// the measured game-over phase, not the attract mapping used by Start.
    /// The frame is simulated without moving `h0`.
    pub(crate) fn from_ball(ball: &Ball) -> LightShow {
        LightShow::with_wrap_on(
            (ball.x - LS_GAME_OVER_X_OFFSET).rem_euclid(LS_H),
            ball.vy,
            ball.row + LS_GAME_OVER_ROW_OFFSET,
        )
    }

    /// Counters set so that the current frame has its V wrap on
    /// `wrap_line`, and that frame simulated.
    fn with_wrap_on(h0: i32, vy: i32, wrap_line: i32) -> LightShow {
        let period = LS_LINES + vy;
        let mut ls = LightShow {
            h0,
            vc: (period - 1 - wrap_line).rem_euclid(period),
            vy,
            // The wrap one period earlier, before `run_lines` re-bases it.
            last_wrap: wrap_line - period + LS_LINES,
            rows_lit: [false; FIELD_H as usize],
        };
        ls.run_lines();
        ls
    }

    /// One attract frame (spec §10.3): the blocks move one clock right and
    /// the frame's lines are simulated.
    pub(crate) fn attract_frame(&mut self, events: &mut Vec<SoundEvent>) {
        self.h0 = (self.h0 + 1).rem_euclid(LS_H);
        if self.h0 == ATTRACT_TICK_H0 {
            events.push(SoundEvent::AttractTick);
        }
        self.run_lines();
    }

    /// Simulates the `LS_LINES` lines of the current frame: the V counter,
    /// its wraps and bounces, and the lit rows.
    pub(crate) fn run_lines(&mut self) {
        self.last_wrap -= LS_LINES;
        for l in 0..LS_LINES {
            self.vc += 1;
            if self.vc >= LS_LINES + self.vy {
                // End of a V cycle: the invisible ball's top or bottom
                // test. A bounce sets the direction, it does not toggle it,
                // and nothing bounces while the ball is outside the box.
                self.vc = 0;
                self.last_wrap = l;
                let live = !LS_DEAD_ZONE.contains(&self.h0);
                let top = self.vy < 0 && (l <= LS_TOP_ZONE_MAX || l >= LS_TOP_ZONE_WRAP_MIN);
                let bottom = self.vy > 0 && LS_BOTTOM_ZONE.contains(&l);
                if live && (top || bottom) {
                    self.vy = -self.vy;
                }
            }
            if l < FIELD_H {
                self.rows_lit[l as usize] = self.vc >= V_TALL_FROM || self.vc & V_ROW_BIT == 0;
            }
        }
    }

    /// Whether the light show lights `(x, row)`, for x in `0..FIELD_W` and
    /// row in `0..FIELD_H`; never in the score band.
    pub(crate) fn block(&self, x: i32, row: i32) -> bool {
        if LS_SCORE_BAND.contains(&row) {
            return false;
        }
        let h = (x - self.h0).rem_euclid(LS_H);
        self.rows_lit[row as usize] && h < H_BLOCKS_END && h & H_BLOCK_BIT == 0
    }

    /// The invisible ball's top row in play terms, for Start (spec §11.1).
    pub(crate) fn ball_row(&self) -> i32 {
        (self.last_wrap - LS_BALL_ROW_OFFSET - ROW_MIN).rem_euclid(PLAY_LINES) + ROW_MIN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::is_live;
    use crate::{FIELD_W, LS_BALL_X_OFFSET};

    #[test]
    fn power_on_state() {
        let ls = LightShow::power_on();
        assert_eq!((ls.h0, ls.vy, ls.last_wrap), (92, 1, 177));
        // vc = 263 - 1 - 177 = 85 before frame 0; at the end of the frame
        // it is 84, the lines after the wrap on line 177.
        assert_eq!(ls.vc, 261 - 177);
    }

    #[test]
    fn dead_zone_is_the_ball_outside_the_box() {
        for h0 in 0..LS_H {
            let x = (h0 + LS_BALL_X_OFFSET).rem_euclid(LS_H);
            assert_eq!(!is_live(x), LS_DEAD_ZONE.contains(&h0), "h0 {h0}");
        }
    }

    #[test]
    fn blocks_are_8_on_8_off_with_a_12_clock_gap() {
        let mut ls = LightShow::power_on();
        ls.h0 = 0;
        ls.rows_lit = [true; FIELD_H as usize];
        let row: String = (0..LS_H).map(|x| if ls.block(x, 0) { '#' } else { '.' }).collect();
        let expected = "########........".repeat(13) + "########" + &".".repeat(12);
        assert_eq!(row, expected);
        // The pattern follows h0 and repeats every LS_H clocks: with h0 200
        // the cycle starts at x -28, and the first whole block in the box
        // is h 32..40 at x 4..12.
        ls.h0 = 200;
        assert!(!ls.block(3, 0) && ls.block(4, 0) && ls.block(11, 0) && !ls.block(12, 0));
    }

    #[test]
    fn no_blocks_in_the_score_band() {
        let mut ls = LightShow::power_on();
        ls.rows_lit = [true; FIELD_H as usize];
        for row in 0..FIELD_H {
            let any = (0..FIELD_W).any(|x| ls.block(x, row));
            assert_eq!(any, !(32..=95).contains(&row), "row {row}");
        }
    }

    /// Lit runs of rows in this frame: (first row, length).
    fn runs(ls: &LightShow) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        let mut start = None;
        for row in 0..=FIELD_H {
            let lit = row < FIELD_H && ls.rows_lit[row as usize];
            match (lit, start) {
                (true, None) => start = Some(row),
                (false, Some(s)) => {
                    out.push((s, row - s));
                    start = None;
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn rows_are_16_on_16_off_and_the_wrap_row_is_22_plus_vy() {
        for vy in -3..=3 {
            // Wrap on line 100, away from both bounce zones.
            let ls = LightShow::with_wrap_on(0, vy, 100);
            assert_eq!(ls.last_wrap, 100);
            let r = runs(&ls);
            let tall = r.iter().find(|&&(s, n)| s <= 100 && 100 < s + n).unwrap();
            assert_eq!(*tall, (100 - (6 + vy), 22 + vy), "vy {vy}");
            for &(s, n) in &r {
                if (s, n) != *tall && s > 0 && s + n < FIELD_H {
                    assert_eq!(n, 16, "vy {vy}, run at {s}");
                }
            }
        }
    }

    #[test]
    fn period_is_262_plus_vy_without_pause_after_a_bounce() {
        // Bottom bounce at line 210 with vy +2: the next wrap is 262 - 2
        // lines later, i.e. on line 208 of the next frame.
        let mut ls = LightShow::with_wrap_on(0, 2, 210);
        assert_eq!(ls.vy, -2);
        ls.run_lines();
        assert_eq!((ls.last_wrap, ls.vy), (208, -2));
        // Without a bounce the wrap moves by vy lines per frame.
        let mut ls = LightShow::with_wrap_on(0, 3, 100);
        ls.run_lines();
        assert_eq!(ls.last_wrap, 103);
    }

    #[test]
    fn bounce_sets_the_direction_in_its_zones_only() {
        // Top zone: l <= 17 or l >= 258 while moving up.
        for (l, flips) in [(17, true), (18, false), (257, false), (258, true), (0, true)] {
            let ls = LightShow::with_wrap_on(0, -1, l);
            assert_eq!(ls.vy == 1, flips, "top, line {l}");
        }
        // Bottom zone: 207..=225 while moving down.
        for (l, flips) in [(206, false), (207, true), (225, true), (226, false)] {
            let ls = LightShow::with_wrap_on(0, 1, l);
            assert_eq!(ls.vy == -1, flips, "bottom, line {l}");
        }
        // Moving down in the top zone (or up in the bottom one) keeps vy.
        assert_eq!(LightShow::with_wrap_on(0, 1, 10).vy, 1);
        assert_eq!(LightShow::with_wrap_on(0, -1, 210).vy, -1);
        // Nothing bounces in the dead zone (h0 172..=223).
        assert_eq!(LightShow::with_wrap_on(172, 1, 210).vy, 1);
        assert_eq!(LightShow::with_wrap_on(223, -1, 10).vy, -1);
        assert_eq!(LightShow::with_wrap_on(171, 1, 210).vy, -1);
        assert_eq!(LightShow::with_wrap_on(224, -1, 10).vy, 1);
    }

    #[test]
    fn a_frame_can_hold_two_wraps_or_none() {
        // With vy -3 the period is 259 lines: a wrap on line 1 is followed
        // by one on line 260 of the same frame.
        let mut ls = LightShow::with_wrap_on(0, -3, 50);
        ls.vc = 259 - 2; // next wrap on line 1
        ls.vy = -3;
        ls.h0 = 200; // dead zone, so neither wrap bounces
        ls.run_lines();
        assert_eq!(ls.last_wrap, 260);
        // With vy +3 (period 265) a wrap on line 261 leaves the next frame
        // without one.
        let mut ls = LightShow::with_wrap_on(200, 3, 261);
        assert_eq!(ls.last_wrap, 261);
        ls.run_lines();
        assert_eq!(ls.last_wrap, 261 - LS_LINES);
        ls.run_lines();
        assert_eq!(ls.last_wrap, 2);
    }

    #[test]
    fn ball_row_is_the_wrap_line_minus_two_in_play_lines() {
        let mut ls = LightShow::power_on();
        ls.last_wrap = 179;
        assert_eq!(ls.ball_row(), 177);
        ls.last_wrap = -20; // in the previous frame
        assert_eq!(ls.ball_row(), -22);
        ls.last_wrap = 230;
        assert_eq!(ls.ball_row(), 228 - PLAY_LINES);
    }
}
