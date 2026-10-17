//! The menu's lettering (SPEC §13.6): a block font five font pixels tall,
//! drawn in whole clocks and lines like everything else the console shows.
//!
//! The console itself has no letters, only its score digits (SPEC §9.3):
//! square outlines one grid cell thick, a `1` that is just the right-hand
//! column. The letters follow them: square corners, strokes one font pixel
//! thick, no diagonals beyond single-pixel steps, uppercase only. Digits
//! are the score glyphs cut down from 4 x 8 to 3 x 5. Most glyphs are three
//! pixels wide; `I` is one, and `M`, `N` and `W` are wider so they stay
//! readable.

use std::ops::Range;

use superpong_core::Cell;

use crate::canvas::Canvas;

/// Font pixels per glyph, top to bottom.
pub const GLYPH_ROWS: usize = 5;
/// Blank font pixels between two glyphs.
const GAP: i32 = 1;

/// One glyph, top row first: `#` lit, `.` dark. Every row is as wide as the
/// glyph.
type Glyph = [&'static str; GLYPH_ROWS];

/// The size of one font pixel, in clocks and lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    /// Clocks per font pixel.
    pub w: i32,
    /// Lines per font pixel.
    pub h: i32,
}

/// The glyph of `c`, or `None` for a character the font does not have.
pub fn glyph(c: char) -> Option<Glyph> {
    Some(match c {
        // Digits: the score glyphs (SPEC §9.3) on a 3 x 5 grid.
        '0' => ["###", "#.#", "#.#", "#.#", "###"],
        '1' => ["..#", "..#", "..#", "..#", "..#"],
        '2' => ["###", "..#", "###", "#..", "###"],
        '3' => ["###", "..#", "###", "..#", "###"],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "###", "..#", "###"],
        '6' => ["###", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", "..#", "..#", "..#"],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "###"],
        'A' => ["###", "#.#", "###", "#.#", "#.#"],
        'B' => ["##.", "#.#", "##.", "#.#", "##."],
        'C' => ["###", "#..", "#..", "#..", "###"],
        'D' => ["##.", "#.#", "#.#", "#.#", "##."],
        'E' => ["###", "#..", "##.", "#..", "###"],
        'F' => ["###", "#..", "##.", "#..", "#.."],
        'G' => ["###", "#..", "#.#", "#.#", "###"],
        'H' => ["#.#", "#.#", "###", "#.#", "#.#"],
        'I' => ["#", "#", "#", "#", "#"],
        'J' => ["..#", "..#", "..#", "#.#", "###"],
        'K' => ["#.#", "#.#", "##.", "#.#", "#.#"],
        'L' => ["#..", "#..", "#..", "#..", "###"],
        'M' => ["#...#", "##.##", "#.#.#", "#...#", "#...#"],
        'N' => ["#..#", "##.#", "#.##", "#..#", "#..#"],
        'O' => ["###", "#.#", "#.#", "#.#", "###"],
        'P' => ["###", "#.#", "###", "#..", "#.."],
        'Q' => ["###", "#.#", "#.#", "###", "..#"],
        'R' => ["###", "#.#", "##.", "#.#", "#.#"],
        'S' => ["###", "#..", "###", "..#", "###"],
        'T' => ["###", ".#.", ".#.", ".#.", ".#."],
        'U' => ["#.#", "#.#", "#.#", "#.#", "###"],
        'V' => ["#.#", "#.#", "#.#", "#.#", ".#."],
        'W' => ["#...#", "#...#", "#.#.#", "##.##", "#...#"],
        'X' => ["#.#", "#.#", ".#.", "#.#", "#.#"],
        'Y' => ["#.#", "#.#", ".#.", ".#.", ".#."],
        'Z' => ["###", "..#", ".#.", "#..", "###"],
        '-' => ["...", "...", "###", "...", "..."],
        ' ' => ["..", "..", "..", "..", ".."],
        _ => return None,
    })
}

/// Width of a glyph in font pixels.
fn glyph_w(g: &Glyph) -> i32 {
    g[0].len() as i32
}

/// The glyphs of `text`; characters the font lacks are skipped.
fn glyphs(text: &str) -> impl Iterator<Item = Glyph> + '_ {
    text.chars().filter_map(glyph)
}

/// Width of `text` in clocks at `scale`, from the left edge of its first
/// glyph to the right edge of its last.
pub fn text_width(text: &str, scale: Scale) -> i32 {
    let (w, n) = glyphs(text).fold((0, 0), |(w, n), g| (w + glyph_w(&g), n + 1));
    (w + GAP * (n - 1).max(0)) * scale.w
}

/// Draws `text` as foreground with its top-left corner at `(x, row)`;
/// returns the X just past its last glyph.
pub fn draw_text(canvas: &mut Canvas, text: &str, x: i32, row: i32, scale: Scale) -> i32 {
    let mut left = x;
    for g in glyphs(text) {
        for (gr, line) in (0..).zip(g) {
            for (gc, _) in (0..).zip(line.chars()).filter(|&(_, c)| c == '#') {
                let (px, pr) = (left + gc * scale.w, row + gr * scale.h);
                canvas.fill(px..px + scale.w, pr..pr + scale.h, Cell::Foreground);
            }
        }
        left += (glyph_w(&g) + GAP) * scale.w;
    }
    left - GAP * scale.w
}

/// The clocks `text` covers when centred on clock `centre`.
pub fn centred(text: &str, centre: i32, scale: Scale) -> Range<i32> {
    let w = text_width(text, scale);
    let x = centre - w / 2;
    x..x + w
}

#[cfg(test)]
mod tests {
    use super::*;
    use superpong_core::GLYPHS;

    const UNIT: Scale = Scale { w: 1, h: 1 };

    #[test]
    fn every_glyph_is_five_rows_of_one_width_in_hash_and_dot() {
        for c in ('0'..='9').chain('A'..='Z').chain([' ', '-']) {
            let g = glyph(c).unwrap_or_else(|| panic!("no glyph for {c:?}"));
            let w = g[0].len();
            assert!((1..=5).contains(&w), "{c:?}");
            for row in g {
                assert_eq!(row.len(), w, "{c:?}: {g:?}");
                assert!(row.chars().all(|ch| ch == '#' || ch == '.'), "{c:?}: {g:?}");
            }
            let lit = g.iter().any(|row| row.contains('#'));
            assert_eq!(lit, c != ' ', "{c:?}");
        }
    }

    #[test]
    fn lowercase_and_symbols_are_not_in_the_font() {
        for c in ['a', 'z', '!', '/', '.'] {
            assert_eq!(glyph(c), None, "{c:?}");
        }
    }

    #[test]
    fn glyphs_are_distinct_except_the_look_alikes() {
        // O and 0, S and 5 are the same block shapes, as on most block fonts.
        let chars: Vec<char> = ('0'..='9').chain('A'..='Z').collect();
        for (i, &a) in chars.iter().enumerate() {
            for &b in &chars[i + 1..] {
                let same = glyph(a) == glyph(b);
                assert_eq!(same, matches!((a, b), ('0', 'O') | ('5', 'S')), "{a} and {b}");
            }
        }
    }

    #[test]
    fn digits_follow_the_score_glyphs() {
        // A score glyph's columns 0 and 3 and rows 0, 3 and 7 are the 3 x 5
        // digit's columns 0 and 2 and rows 0, 2 and 4: the outline is the same.
        for d in 0..10u8 {
            let small = glyph(char::from(b'0' + d)).unwrap();
            let big = GLYPHS[usize::from(d)];
            for (sr, br) in [(0, 0), (2, 3), (4, 7)] {
                for (sc, bc) in [(0, 0), (2, 3)] {
                    let lit_small = small[sr].as_bytes()[sc] == b'#';
                    let lit_big = big[br] & (0b1000 >> bc) != 0;
                    assert_eq!(lit_small, lit_big, "digit {d}, row {sr}, column {sc}");
                }
            }
        }
    }

    #[test]
    fn widths_count_glyphs_and_gaps() {
        assert_eq!(text_width("", UNIT), 0);
        assert_eq!(text_width("A", UNIT), 3);
        assert_eq!(text_width("AI", UNIT), 3 + 1 + 1);
        assert_eq!(text_width("ON", Scale { w: 2, h: 3 }), (3 + 1 + 4) * 2);
        // The longest value the menu shows fits its value column.
        assert_eq!(text_width("SUPER PONG", Scale { w: 2, h: 3 }), 78);
        assert_eq!(centred("A", 10, UNIT), 9..12);
    }

    #[test]
    fn text_is_drawn_as_foreground_blocks_at_the_scale() {
        let mut canvas = Canvas::new();
        let end = draw_text(&mut canvas, "LI", 10, 20, Scale { w: 2, h: 3 });
        assert_eq!(end, 10 + text_width("LI", Scale { w: 2, h: 3 }));
        let art = canvas.to_ascii(10..20, 20..35);
        let want = [
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "##......##",
            "######..##",
            "######..##",
            "######..##",
        ];
        assert_eq!(art, want.map(|l| format!("{l}\n")).concat());
        assert_eq!(canvas.get(9, 20), Cell::Background);
        assert_eq!(canvas.get(10, 35), Cell::Background);
    }
}
