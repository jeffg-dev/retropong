//! The picture as cells, owned by the frontend: a copy of the core's
//! [`Frame`] that the menu and the resume countdown draw over (SPEC §13.6).
//!
//! It has exactly the core's raster: [`DISPLAY_W`] x [`DISPLAY_H`] cells of
//! one clock by one line, X from [`DISPLAY_X0`] and rows from
//! [`DISPLAY_ROW0`]. Whatever is drawn here is therefore coloured by the same
//! gradient, made of the same chunky blocks and sent through the same CRT
//! shader as the game itself.

use std::ops::Range;

use superpong_core::{Cell, Frame, DISPLAY_H, DISPLAY_ROW0, DISPLAY_W, DISPLAY_X0};

/// The display window as cells, row-major like [`Frame`].
#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    cells: Vec<Cell>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

impl Canvas {
    /// A canvas with every cell [`Cell::Background`].
    pub fn new() -> Self {
        Self { cells: vec![Cell::Background; (DISPLAY_W * DISPLAY_H) as usize] }
    }

    /// Replaces every cell with the core's rendered `frame`.
    pub fn copy_from(&mut self, frame: &Frame) {
        self.cells.copy_from_slice(frame.cells());
    }

    fn index(x: i32, row: i32) -> Option<usize> {
        let (cx, cy) = (x - DISPLAY_X0, row - DISPLAY_ROW0);
        ((0..DISPLAY_W).contains(&cx) && (0..DISPLAY_H).contains(&cy))
            .then_some((cy * DISPLAY_W + cx) as usize)
    }

    /// The cell at `(x, row)`; background outside the display window.
    #[cfg(test)]
    pub fn get(&self, x: i32, row: i32) -> Cell {
        Self::index(x, row).map_or(Cell::Background, |i| self.cells[i])
    }

    /// Sets the cells of `xs` by `rows` to `cell`, clipped to the display
    /// window. Unlike the core's drawing this overwrites: the menu blanks
    /// what is behind it.
    pub fn fill(&mut self, xs: Range<i32>, rows: Range<i32>, cell: Cell) {
        for row in rows {
            for x in xs.clone() {
                if let Some(i) = Self::index(x, row) {
                    self.cells[i] = cell;
                }
            }
        }
    }

    /// Every cell set to [`Cell::Background`].
    pub fn clear(&mut self) {
        self.cells.fill(Cell::Background);
    }

    /// All cells, row-major, as [`Frame::cells`].
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// One character per cell (`.` background, `:` border, `#` foreground)
    /// for `xs` by `rows`, and a newline per row; for golden tests.
    #[cfg(test)]
    pub fn to_ascii(&self, xs: Range<i32>, rows: Range<i32>) -> String {
        let mut s = String::new();
        for row in rows {
            s.extend(xs.clone().map(|x| match self.get(x, row) {
                Cell::Background => '.',
                Cell::Border => ':',
                Cell::Foreground => '#',
            }));
            s.push('\n');
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use superpong_core::{Console, GameKind};

    #[test]
    fn a_copied_frame_is_the_core_picture() {
        let mut frame = Frame::new();
        Console::new(GameKind::Pong).render(&mut frame);
        let mut canvas = Canvas::new();
        canvas.copy_from(&frame);
        assert_eq!(canvas.cells(), frame.cells());
        for (x, row) in [(0, 0), (89, 0), (89, 100), (-6, -8), (180, 231)] {
            assert_eq!(canvas.get(x, row), frame.get(x, row), "({x}, {row})");
        }
    }

    #[test]
    fn fill_overwrites_and_clips_to_the_window() {
        let mut canvas = Canvas::new();
        canvas.fill(-10..-4, -12..-6, Cell::Foreground);
        assert_eq!(canvas.get(-6, -8), Cell::Foreground);
        assert_eq!(canvas.get(-5, -7), Cell::Foreground);
        assert_eq!(canvas.get(-4, -8), Cell::Background);
        assert_eq!(canvas.get(-7, -8), Cell::Background, "outside the window reads as background");
        canvas.fill(-6..-5, -8..-7, Cell::Border);
        assert_eq!(canvas.get(-6, -8), Cell::Border, "fill overwrites foreground");
        assert_eq!(canvas.cells().iter().filter(|&&c| c != Cell::Background).count(), 4);
        canvas.clear();
        assert!(canvas.cells().iter().all(|&c| c == Cell::Background));
        assert_eq!(canvas.to_ascii(0..3, 0..2), "...\n...\n");
    }
}
