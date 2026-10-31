//! The display window as cells (spec §14.3), including the score digits
//! (spec §9).

use crate::play::objects;
use crate::{
    Console, Phase, BALL_H, BALL_W, BOTTOM_BORDER, DIGIT_H, DIGIT_ROW0, DISPLAY_H, DISPLAY_ROW0, DISPLAY_W,
    DISPLAY_X0, FIELD_H, FIELD_W, GLYPHS, LEFT_TENS_X, LEFT_UNITS_X, NET_W, NET_X, PADDLE_W, RIGHT_TENS_X,
    RIGHT_UNITS_X, TOP_BORDER,
};
use std::ops::Range;

/// Width of one digit grid cell, and of the tens bar, in clocks.
const GRID_W: i32 = 4;
/// Height of one digit grid cell in lines.
const GRID_H: i32 = 8;

/// What one display cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Cell {
    /// Nothing lit.
    Background = 0,
    /// A border line.
    Border = 1,
    /// Any lit object: paddle, wall, net, ball, digit or light-show block.
    Foreground = 2,
}

/// The display window rasterised: [`DISPLAY_W`] x [`DISPLAY_H`] cells,
/// row-major, index = `(row - DISPLAY_ROW0) * DISPLAY_W + (x - DISPLAY_X0)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    cells: Vec<Cell>,
}

impl Default for Frame {
    fn default() -> Frame {
        Frame::new()
    }
}

impl Frame {
    /// A frame with every cell [`Cell::Background`].
    pub fn new() -> Frame {
        Frame { cells: vec![Cell::Background; (DISPLAY_W * DISPLAY_H) as usize] }
    }

    fn index(x: i32, row: i32) -> Option<usize> {
        let (cx, cy) = (x - DISPLAY_X0, row - DISPLAY_ROW0);
        if (0..DISPLAY_W).contains(&cx) && (0..DISPLAY_H).contains(&cy) {
            Some((cy * DISPLAY_W + cx) as usize)
        } else {
            None
        }
    }

    /// The cell at `(x, row)`; background outside the display window.
    pub fn get(&self, x: i32, row: i32) -> Cell {
        Frame::index(x, row).map_or(Cell::Background, |i| self.cells[i])
    }

    /// One row of [`DISPLAY_W`] cells, starting at [`DISPLAY_X0`].
    ///
    /// Panics if `row` is outside `DISPLAY_ROW0..DISPLAY_ROW0 + DISPLAY_H`.
    pub fn row(&self, row: i32) -> &[Cell] {
        let start = Frame::index(DISPLAY_X0, row).expect("row outside the display window");
        &self.cells[start..start + DISPLAY_W as usize]
    }

    /// All cells, row-major.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// One character per cell (`.` background, `:` border, `#` foreground)
    /// and a newline per row; for golden tests.
    pub fn to_ascii(&self) -> String {
        let mut s = String::with_capacity(((DISPLAY_W + 1) * DISPLAY_H) as usize);
        for line in self.cells.chunks(DISPLAY_W as usize) {
            s.extend(line.iter().map(|c| match c {
                Cell::Background => '.',
                Cell::Border => ':',
                Cell::Foreground => '#',
            }));
            s.push('\n');
        }
        s
    }

    /// Lights `(x, row)` as `cell` if it lies in the window, keeping
    /// `Foreground` over `Border`.
    fn put(&mut self, x: i32, row: i32, cell: Cell) {
        if let Some(i) = Frame::index(x, row) {
            if self.cells[i] != Cell::Foreground {
                self.cells[i] = cell;
            }
        }
    }

    /// Lights the cells of `xs` by `rows` as `Foreground`, clipped to the
    /// border box: nothing is ever lit outside it.
    fn light(&mut self, xs: Range<i32>, rows: Range<i32>) {
        for row in rows.start.max(0)..rows.end.min(FIELD_H) {
            for x in xs.start.max(0)..xs.end.min(FIELD_W) {
                self.put(x, row, Cell::Foreground);
            }
        }
    }
}

/// Rasterises `console` into `frame` (spec §14.3). Everything lit is OR'ed
/// into `Foreground`, so the drawing order does not matter.
pub(crate) fn draw(console: &Console, frame: &mut Frame) {
    frame.cells.fill(Cell::Background);
    for row in TOP_BORDER.chain(BOTTOM_BORDER) {
        for x in 0..FIELD_W {
            frame.put(x, row, Cell::Border);
        }
    }
    for row in (0..FIELD_H).filter(|r| r % 16 < 8) {
        frame.light(NET_X..NET_X + NET_W, row..row + 1);
    }
    // The selected game's paddles and walls, in every phase.
    for obj in objects(console.kind, console.left_top, console.right_top) {
        for row in (0..FIELD_H).filter(|&r| obj.covers(r)) {
            frame.light(obj.x..obj.x + PADDLE_W, row..row + 1);
        }
    }
    if console.phase == Phase::Rally {
        let b = &console.ball;
        frame.light(b.x..b.x + BALL_W, b.row..b.row + BALL_H);
    } else {
        draw_score(frame, console.scores.0, LEFT_UNITS_X, LEFT_TENS_X);
        draw_score(frame, console.scores.1, RIGHT_UNITS_X, RIGHT_TENS_X);
    }
    if console.phase == Phase::Attract {
        for row in 0..FIELD_H {
            for x in (0..FIELD_W).filter(|&x| console.ls.block(x, row)) {
                frame.put(x, row, Cell::Foreground);
            }
        }
    }
}

/// One score: the units glyph and, for 10..=15, the tens bar (spec §9.2).
fn draw_score(frame: &mut Frame, value: u8, units_x: i32, tens_x: i32) {
    if value >= 10 {
        frame.light(tens_x..tens_x + GRID_W, DIGIT_ROW0..DIGIT_ROW0 + DIGIT_H);
    }
    for (gr, bits) in (0..).zip(GLYPHS[usize::from(value % 10)]) {
        for gc in (0..4).filter(|gc| bits & (0b1000 >> gc) != 0) {
            let x = units_x + GRID_W * gc;
            let row = DIGIT_ROW0 + GRID_H * gr;
            frame.light(x..x + GRID_W, row..row + GRID_H);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `docs/superpong-data/digits.json`, the authoritative glyph data.
    const DIGITS_JSON: &str = include_str!("../../../docs/superpong-data/digits.json");

    #[test]
    fn glyphs_match_the_data_file() {
        // Without whitespace each glyph reads `"4":["#..#",...,"...#"]`,
        // however the file's arrays are laid out.
        let compact: String = DIGITS_JSON.chars().filter(|c| !c.is_whitespace()).collect();
        for (d, glyph) in GLYPHS.iter().enumerate() {
            let key = format!("\"{d}\":[");
            let start =
                compact.find(&key).unwrap_or_else(|| panic!("digit {d} is in digits.json")) + key.len();
            let list = &compact[start..start + compact[start..].find(']').expect("the glyph's list closes")];
            let rows: Vec<&str> = list.split(',').map(|row| row.trim_matches('"')).collect();
            assert_eq!(rows.len(), 8, "digit {d}: {list}");
            for (gr, row) in rows.iter().enumerate() {
                let mask = row.chars().fold(0u8, |m, c| m << 1 | u8::from(c == '#'));
                assert_eq!(glyph[gr], mask, "digit {d}, grid row {gr}");
            }
        }
    }

    #[test]
    fn digit_positions_match_the_data_file() {
        let compact: String = DIGITS_JSON.chars().filter(|c| !c.is_whitespace()).collect();
        for needle in [
            format!("\"rows\":[{},{}]", DIGIT_ROW0, DIGIT_ROW0 + DIGIT_H),
            format!(
                "\"left\":{{\"units_x\":[{},{}],\"tens_x\":[{},{}]}}",
                LEFT_UNITS_X,
                LEFT_UNITS_X + 4 * GRID_W,
                LEFT_TENS_X,
                LEFT_TENS_X + GRID_W
            ),
            format!(
                "\"right\":{{\"units_x\":[{},{}],\"tens_x\":[{},{}]}}",
                RIGHT_UNITS_X,
                RIGHT_UNITS_X + 4 * GRID_W,
                RIGHT_TENS_X,
                RIGHT_TENS_X + GRID_W
            ),
            "\"x_offset_from_units_cell\":-16".to_string(),
        ] {
            assert!(compact.contains(&needle), "digits.json has {needle}");
        }
        assert_eq!(LEFT_UNITS_X - LEFT_TENS_X, 16);
        assert_eq!(RIGHT_UNITS_X - RIGHT_TENS_X, 16);
    }

    /// The units glyph of `value` as 8 strings of 4 characters, read back
    /// from a frame at grid resolution.
    fn read_glyph(frame: &Frame, units_x: i32) -> Vec<String> {
        (0..8)
            .map(|gr| {
                (0..4)
                    .map(|gc| {
                        let (x, row) = (units_x + GRID_W * gc, DIGIT_ROW0 + GRID_H * gr);
                        let cells: Vec<Cell> = (row..row + GRID_H)
                            .flat_map(|r| (x..x + GRID_W).map(move |x| (x, r)))
                            .map(|(x, r)| frame.get(x, r))
                            .collect();
                        let lit = cells[0] == Cell::Foreground;
                        assert!(cells.iter().all(|&c| c == cells[0]), "grid cell is uniform");
                        if lit {
                            '#'
                        } else {
                            '.'
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn digits_are_drawn_on_their_grid() {
        for value in 0..=15u8 {
            let mut frame = Frame::new();
            draw_score(&mut frame, value, RIGHT_UNITS_X, RIGHT_TENS_X);
            let glyph = read_glyph(&frame, RIGHT_UNITS_X);
            for (gr, row) in glyph.iter().enumerate() {
                let mask = row.chars().fold(0u8, |m, c| m << 1 | u8::from(c == '#'));
                assert_eq!(mask, GLYPHS[usize::from(value % 10)][gr], "value {value}");
            }
            let tens = (DIGIT_ROW0..DIGIT_ROW0 + DIGIT_H).all(|row| {
                (RIGHT_TENS_X..RIGHT_TENS_X + GRID_W).all(|x| frame.get(x, row) == Cell::Foreground)
            });
            assert_eq!(tens, value >= 10, "tens bar for {value}");
            // Nothing outside the units cell and the tens bar.
            let lit = frame.cells().iter().filter(|&&c| c == Cell::Foreground).count();
            let glyph_cells = glyph.concat().matches('#').count() as i32 * GRID_W * GRID_H;
            let tens_cells = if value >= 10 { GRID_W * DIGIT_H } else { 0 };
            assert_eq!(lit as i32, glyph_cells + tens_cells, "value {value}");
        }
    }

    #[test]
    fn frame_indexing() {
        let mut frame = Frame::new();
        frame.put(DISPLAY_X0, DISPLAY_ROW0, Cell::Border);
        frame.put(-7, 0, Cell::Foreground); // outside: ignored
        frame.light(-10..2, -10..1); // clipped to the box: (0, 0) and (1, 0)
        assert_eq!(frame.cells()[0], Cell::Border);
        assert_eq!(frame.get(DISPLAY_X0, DISPLAY_ROW0), Cell::Border);
        assert_eq!(frame.get(-7, 0), Cell::Background);
        assert_eq!(frame.get(0, 0), Cell::Foreground);
        assert_eq!(frame.get(1, 0), Cell::Foreground);
        assert_eq!(frame.get(2, 0), Cell::Background);
        assert_eq!(frame.get(-1, 0), Cell::Background);
        assert_eq!(frame.row(0)[(0 - DISPLAY_X0) as usize], Cell::Foreground);
        assert_eq!(frame.row(DISPLAY_ROW0 + DISPLAY_H - 1).len(), DISPLAY_W as usize);
        // Foreground is never overwritten by a border.
        frame.put(0, 0, Cell::Border);
        assert_eq!(frame.get(0, 0), Cell::Foreground);
        let ascii = frame.to_ascii();
        assert_eq!(ascii.lines().count(), DISPLAY_H as usize);
        assert!(ascii.lines().all(|l| l.len() == DISPLAY_W as usize));
        assert!(ascii.starts_with(':'));
    }

    #[test]
    #[should_panic(expected = "row outside the display window")]
    fn frame_row_panics_outside_the_window() {
        Frame::new().row(DISPLAY_ROW0 + DISPLAY_H);
    }
}
