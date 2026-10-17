//! Drawing: the console picture and raster overlays (SPEC §3, §13).
//!
//! The core rasterises the display window into a [`Frame`] of cell kinds,
//! which is copied to a [`Canvas`] for the menu or the resume countdown to
//! draw over (SPEC §13.6). This module colours it ([`colour::Palette`]), uploads it as one RGBA image
//! with four sub-columns per clock, and draws that texture into the largest
//! 4:3 rectangle that fits the window: with nearest filtering in pixel
//! mode, or through the [`Crt`] shader, stripe texture on, in classic-video
//! mode.
//! It decides nothing about what is lit.

use macroquad::prelude::*;
use superpong_core::{Cell, Console, Frame, DISPLAY_H, DISPLAY_W, DISPLAY_X0};

use crate::canvas::Canvas;
use crate::colour::{Palette, SUBCOLUMNS};
use crate::crt::Crt;
use crate::menu::screen::Overlay;
use crate::settings::{DisplayMode, Settings};

/// Width of the picture image: four sub-columns per clock.
pub const PICTURE_W: usize = DISPLAY_W as usize * SUBCOLUMNS;
/// Height of the picture image: one pixel row per line.
pub const PICTURE_H: usize = DISPLAY_H as usize;

/// Default window size, also used by windowed QA runs.
pub const DEFAULT_WINDOW: (i32, i32) = (960, 720);

/// A whole-line picture must be at least this percentage of the largest
/// picture that fits, or the largest is used instead.
const WHOLE_LINES_PERCENT: i32 = 90;

/// Centred 4:3 picture in device pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
pub fn layout(screen_w: i32, screen_h: i32) -> Layout {
    let full = screen_h.min(screen_w * 3 / 4).max(1);
    let whole = full / DISPLAY_H * DISPLAY_H;
    let h = if whole > 0 && whole * 100 >= full * WHOLE_LINES_PERCENT { whole } else { full };
    let w = h * 4 / 3;
    Layout { x: (screen_w - w) / 2, y: (screen_h - h) / 2, w, h }
}

/// The mode actually drawn: classic video needs its shader, so without one
/// the picture falls back to pixels.
pub fn shown_mode(wanted: DisplayMode, crt_available: bool) -> DisplayMode {
    if crt_available {
        wanted
    } else {
        DisplayMode::Pixel
    }
}

/// Whether the palette's stripe texture is on: it is part of classic video
/// (the one-clock carrier ripple a TV shows, geometry.md §3.5) and absent
/// from the clean pixel picture.
pub fn stripes(shown: DisplayMode) -> bool {
    shown == DisplayMode::Crt
}

/// Fills `pixels` (RGBA, [`PICTURE_W`] x [`PICTURE_H`]) from the display
/// window's `cells` (row-major, as [`Frame::cells`] and [`Canvas::cells`]):
/// each cell becomes four sub-columns coloured by kind and X.
pub fn fill_image(cells: &[Cell], palette: &Palette, pixels: &mut [u8]) {
    assert_eq!(pixels.len(), PICTURE_W * PICTURE_H * 4, "picture buffer size");
    assert_eq!(cells.len(), DISPLAY_W as usize * PICTURE_H, "display window size");
    let (px, _) = pixels.as_chunks_mut::<4>();
    for (r, row) in cells.chunks(DISPLAY_W as usize).enumerate() {
        let out = &mut px[r * PICTURE_W..(r + 1) * PICTURE_W];
        for (c, &cell) in row.iter().enumerate() {
            for j in 0..SUBCOLUMNS {
                let [red, green, blue] = palette.get(cell, DISPLAY_X0 + c as i32, j);
                out[c * SUBCOLUMNS + j] = [red, green, blue, 255];
            }
        }
    }
}

/// Owns the frame buffer and the canvas drawn over it, the picture image
/// and its texture, the palette for the current display options, and the
/// CRT shader (or why it failed).
pub struct Renderer {
    frame: Frame,
    canvas: Canvas,
    image: Image,
    texture: Texture2D,
    palette: Palette,
    crt: Result<Crt, String>,
}

impl Renderer {
    pub fn new(settings: &Settings) -> Self {
        let image = Image {
            bytes: vec![0; PICTURE_W * PICTURE_H * 4],
            width: PICTURE_W as u16,
            height: PICTURE_H as u16,
        };
        let texture = Texture2D::from_image(&image);
        texture.set_filter(FilterMode::Nearest);
        let crt = Crt::load();
        if let Err(e) = &crt {
            eprintln!("retropong: CRT shader unavailable, classic video falls back to pixels: {e}");
        }
        let stripes = stripes(shown_mode(settings.display, crt.is_ok()));
        Self {
            frame: Frame::new(),
            canvas: Canvas::new(),
            image,
            texture,
            palette: Palette::new(settings.color, stripes),
            crt,
        }
    }

    /// The mode on screen for `settings`: classic video only if its shader
    /// loaded (see [`shown_mode`]).
    pub fn shown_mode(&self, settings: &Settings) -> DisplayMode {
        shown_mode(settings.display, self.crt.is_ok())
    }

    /// Draws one full frame: the console picture with `overlay` over it, in
    /// the current display mode.
    pub fn draw(&mut self, console: &Console, overlay: &Overlay, settings: &Settings) {
        let shown = self.shown_mode(settings);
        if !self.palette.matches(settings.color, stripes(shown)) {
            self.palette = Palette::new(settings.color, stripes(shown));
        }
        console.render(&mut self.frame);
        self.canvas.copy_from(&self.frame);
        overlay.draw(&mut self.canvas);
        fill_image(self.canvas.cells(), &self.palette, &mut self.image.bytes);
        self.texture.update(&self.image);

        clear_background(BLACK);

        // Layout is worked out in device pixels, so lines get whole pixel
        // heights, and converted to macroquad's logical units to draw.
        let dpi = screen_dpi_scale();
        let lay = layout((screen_width() * dpi).round() as i32, (screen_height() * dpi).round() as i32);
        let logical = |v: i32| v as f32 / dpi;
        let (x, y, w, h) = (logical(lay.x), logical(lay.y), logical(lay.w), logical(lay.h));
        match (&self.crt, shown) {
            (Ok(crt), DisplayMode::Crt) => crt.draw(&self.texture, (x, y, w, h), (lay.w, lay.h)),
            _ => draw_texture_ex(
                &self.texture,
                x,
                y,
                WHITE,
                DrawTextureParams { dest_size: Some(vec2(w, h)), ..Default::default() },
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colour::rgb;
    use superpong_core::DISPLAY_ROW0;

    #[test]
    fn picture_fills_the_window_with_centred_letterboxing() {
        assert_eq!(layout(960, 720), Layout { x: 0, y: 0, w: 960, h: 720 });
        assert_eq!(layout(1920, 1080), Layout { x: 240, y: 0, w: 1440, h: 1080 });
        assert_eq!(layout(1280, 800), Layout { x: 160, y: 40, w: 960, h: 720 });
        assert_eq!(layout(4000, 801).h, 801);
    }

    #[test]
    fn layout_preserves_aspect_and_whole_lines_across_display_sizes() {
        for (sw, sh) in [
            (960, 720),
            (1920, 1080),
            (1280, 720),
            (1366, 768),
            (1280, 800),
            (2560, 1440),
            (1920, 1440),
            (200, 180),
            (160, 600),
        ] {
            let l = layout(sw, sh);
            assert!(l.x >= 0 && l.x + l.w <= sw);
            assert!(l.y >= 0 && l.y + l.h <= sh);
            assert_eq!((l.x, l.y), ((sw - l.w) / 2, (sh - l.h) / 2));
            assert!((l.w * 3 - l.h * 4).abs() < 3);
            let largest = sh.min(sw * 3 / 4);
            assert!(l.h <= largest && l.h * 10 >= largest * 9);
            assert_eq!(l.h % DISPLAY_H == 0, largest / DISPLAY_H * DISPLAY_H * 10 >= largest * 9);
        }
        assert_eq!(layout(4000, 800).h, 720);
        assert_eq!(layout(4000, 801).h, 801);
        assert_eq!(layout(4000, 240).h, 240);
        assert_eq!(layout(4000, 239).h, 239);
        assert_eq!(layout(0, 0).h, 1);
    }

    #[test]
    fn classic_video_falls_back_to_pixels_without_its_shader() {
        assert_eq!(shown_mode(DisplayMode::Crt, true), DisplayMode::Crt);
        assert_eq!(shown_mode(DisplayMode::Crt, false), DisplayMode::Pixel);
        assert_eq!(shown_mode(DisplayMode::Pixel, true), DisplayMode::Pixel);
        assert_eq!(shown_mode(DisplayMode::Pixel, false), DisplayMode::Pixel);
    }

    #[test]
    fn the_stripe_texture_belongs_to_classic_video() {
        assert!(stripes(DisplayMode::Crt));
        assert!(!stripes(DisplayMode::Pixel));
    }

    /// A frame with a border row, the net and background, from the core.
    fn sample_frame() -> Frame {
        let mut frame = Frame::new();
        Console::new(superpong_core::GameKind::Pong).render(&mut frame);
        frame
    }

    /// The four sub-column pixels of the cell at (`x`, `row`).
    fn clock(pixels: &[u8], x: i32, row: i32) -> Vec<[u8; 4]> {
        let at = ((row - DISPLAY_ROW0) as usize * PICTURE_W + (x - DISPLAY_X0) as usize * SUBCOLUMNS) * 4;
        pixels[at..at + SUBCOLUMNS * 4].as_chunks::<4>().0.to_vec()
    }

    #[test]
    fn image_colours_cells_by_kind_and_x() {
        let frame = sample_frame();
        let palette = Palette::new(true, false);
        let mut pixels = vec![0u8; PICTURE_W * PICTURE_H * 4];
        fill_image(frame.cells(), &palette, &mut pixels);
        assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));

        let [r, g, b] = rgb(Cell::Background, DISPLAY_X0);
        assert_eq!(clock(&pixels, DISPLAY_X0, DISPLAY_ROW0), vec![[r, g, b, 255]; SUBCOLUMNS]);
        // Border at the top-left of the border box (the net starts at X 88).
        let [r, g, b] = rgb(Cell::Border, 0);
        assert_eq!(clock(&pixels, 0, 0), vec![[r, g, b, 255]; SUBCOLUMNS]);
        // Foreground (the net, row 0) wins over the border.
        let [r, g, b] = rgb(Cell::Foreground, 89);
        assert_eq!(clock(&pixels, 89, 0), vec![[r, g, b, 255]; SUBCOLUMNS]);
        // Outside the border box and below the playfield is background.
        let [r, g, b] = rgb(Cell::Background, 177);
        assert_eq!(clock(&pixels, 177, 100), vec![[r, g, b, 255]; SUBCOLUMNS]);
    }

    #[test]
    fn stripes_vary_the_subcolumns_of_a_clock() {
        let frame = sample_frame();
        let palette = Palette::new(true, true);
        let mut pixels = vec![0u8; PICTURE_W * PICTURE_H * 4];
        fill_image(frame.cells(), &palette, &mut pixels);
        let net = clock(&pixels, 89, 0);
        assert!(net.iter().any(|p| *p != net[0]), "{net:?}");
        let expect: Vec<[u8; 4]> = (0..SUBCOLUMNS)
            .map(|j| {
                let [r, g, b] = palette.get(Cell::Foreground, 89, j);
                [r, g, b, 255]
            })
            .collect();
        assert_eq!(net, expect);
    }

    #[test]
    fn mono_image_is_grey() {
        let frame = sample_frame();
        let palette = Palette::new(false, false);
        let mut pixels = vec![0u8; PICTURE_W * PICTURE_H * 4];
        fill_image(frame.cells(), &palette, &mut pixels);
        assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
        assert_eq!(clock(&pixels, 0, 0)[0], [152, 152, 152, 255]);
    }
}
