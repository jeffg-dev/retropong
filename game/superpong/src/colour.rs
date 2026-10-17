//! Colour: maps a core [`Cell`] kind and its X clock to RGB (SPEC §3).
//!
//! The colours come from the game's packaging art, not from the reference
//! video, which carries analogue and capture distortion. The background is
//! black. Only the drawn elements carry colour, and it depends only on the
//! cell kind and on X, never on the row: a handful of anchor colours, placed
//! at the X of the box-art element each was taken from, blended smoothly
//! between neighbours in the OKLab colour space and held constant beyond the
//! end anchors. The constants mirror `docs/superpong-data/colour_model.json`.
//! On top of that sit the display options of SPEC §3.2: black-and-white (a
//! constant grey per kind, the console's luma levels) and the stripe
//! (carrier) texture, which splits each clock into four sub-columns and is
//! added to the signal before the black clip. Neither touches the
//! background, which stays black in every mode. [`Palette`] bakes the
//! gradient and the options together once, so drawing a frame is a plain
//! lookup per pixel.

use superpong_core::{Cell, DISPLAY_W, DISPLAY_X0, FIELD_W};

/// Sub-columns drawn per clock; the stripe option needs four, and they are
/// identical when it is off.
pub const SUBCOLUMNS: usize = 4;

/// Number of palette columns, X `-6..=180` (one per display-window clock).
const COLUMNS: usize = DISPLAY_W as usize;
/// Cell kinds that have a colour.
const KINDS: usize = 3;

type Rgb = [u8; 3];

/// A colour anchor: the clock column it sits at and its sRGB value.
type Anchor = (i32, Rgb);

/// The border bars of the box art: blue at the left end, sky blue where the
/// net crosses them, violet at the right end (SPEC §3.1). The border is a
/// plain gradient; the light spot of the art is the net, which the core draws
/// through the border as foreground.
const BORDER_ANCHORS: [Anchor; 3] =
    [(0, [0x46, 0x66, 0xbc]), (88, [0x5c, 0xb8, 0xf0]), (175, [0x78, 0x62, 0xc2])];

/// The lit objects of the box art, left to right: yellow at the left paddle
/// and wall (X 16-19), green at the left score digits (X 24-55), light blue
/// at the net (X 88-91), lilac at the right digits (X 120-151) and pink at the
/// right paddle and wall (X 160-163).
const FOREGROUND_ANCHORS: [Anchor; 5] = [
    (16, [0xf4, 0xec, 0x70]),
    (48, [0x8e, 0xdb, 0xa0]),
    (88, [0x98, 0xd4, 0xf4]),
    (136, [0xec, 0xc6, 0xec]),
    (161, [0xf2, 0x9c, 0xc4]),
];

/// What a drawn cell kind is made of beyond its colour gradient.
struct Signal {
    /// The kind's luma level: the grey of black-and-white mode. It is a
    /// property of the video signal, not of the colours.
    luma: f32,
    /// Stripe strength in levels of the picture texture (SPEC §3.2). The CRT
    /// filters pass about half of the one-clock carrier, and a channel below
    /// black swallows part of it, so these are chosen for the screen to show
    /// the amplitudes measured in the reference video (border 9.7, foreground
    /// 47 levels). Fit them again if `crt.frag`'s luma filter changes.
    stripe: f32,
    /// Position of the stripe's peak inside the clock, in cycles.
    stripe_peak: f32,
    /// The colour anchors, ascending in X.
    anchors: &'static [Anchor],
}

/// The border signal (SPEC §3.1).
const BORDER: Signal = Signal { luma: 152.0, stripe: 17.0, stripe_peak: 0.42, anchors: &BORDER_ANCHORS };
/// The foreground signal: paddles, ball, net, walls, digits, light-show blocks.
const FOREGROUND: Signal =
    Signal { luma: 161.0, stripe: 94.0, stripe_peak: 0.48, anchors: &FOREGROUND_ANCHORS };

/// Palette position of a cell kind: background, border, foreground.
fn kind_index(cell: Cell) -> usize {
    match cell {
        Cell::Background => 0,
        Cell::Border => 1,
        Cell::Foreground => 2,
    }
}

/// The signal of a cell kind, or `None` for the background, which is black.
fn signal(cell: Cell) -> Option<&'static Signal> {
    match cell {
        Cell::Background => None,
        Cell::Border => Some(&BORDER),
        Cell::Foreground => Some(&FOREGROUND),
    }
}

/// sRGB level to linear light.
fn to_linear(c: u8) -> f32 {
    let v = f32::from(c) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light to an sRGB level (0..=255, not rounded).
fn from_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    255.0 * if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// sRGB to OKLab `[L, a, b]` (Ottosson, 2020).
fn oklab(rgb: Rgb) -> [f32; 3] {
    let [r, g, b] = rgb.map(to_linear);
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// OKLab back to sRGB levels (0..=255, not rounded; out-of-gamut channels are
/// clamped).
fn srgb_levels(lab: [f32; 3]) -> [f32; 3] {
    let [big_l, a, b] = lab;
    let l = (big_l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (big_l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (big_l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_93 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_4 * s,
        -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
    .map(from_linear)
}

/// The gradient of an anchor list at clock `x`, in OKLab: linear between the
/// two anchors around `x`, the end anchors' colours beyond them.
fn gradient_lab(anchors: &[Anchor], x: i32) -> [f32; 3] {
    let (first, last) = (anchors[0], anchors[anchors.len() - 1]);
    if x <= first.0 {
        return oklab(first.1);
    }
    if x >= last.0 {
        return oklab(last.1);
    }
    let k = anchors.iter().rposition(|a| a.0 <= x).unwrap_or(0);
    let ((x0, c0), (x1, c1)) = (anchors[k], anchors[k + 1]);
    let t = (x - x0) as f32 / (x1 - x0) as f32;
    let (p, q) = (oklab(c0), oklab(c1));
    [0, 1, 2].map(|i| p[i] + (q[i] - p[i]) * t)
}

/// The colour of a cell of `cell` kind at clock `x` (SPEC §3.1): black for the
/// background; otherwise the kind's anchor gradient, rounded to 8 bits.
#[cfg(test)]
pub fn rgb(cell: Cell, x: i32) -> Rgb {
    unclipped(cell, x).map(level)
}

/// [`rgb`] before the rounding.
fn unclipped(cell: Cell, x: i32) -> [f32; 3] {
    let Some(s) = signal(cell) else { return [0.0; 3] };
    srgb_levels(gradient_lab(s.anchors, x))
}

/// Stripe offset in levels for sub-column `j` of a cell of the given kind:
/// `a * cos(2 pi ((j + 0.5) / 4 - peak))` (SPEC §3.2). The background has no
/// stripe.
pub fn stripe_offset(cell: Cell, j: usize) -> f32 {
    let Some(s) = signal(cell) else { return 0.0 };
    let phase = (j as f32 + 0.5) / SUBCOLUMNS as f32 - s.stripe_peak;
    s.stripe * (std::f32::consts::TAU * phase).cos()
}

/// Rounds and clamps one level to a byte.
fn level(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// The final colour of one sub-column under the display options.
///
/// The stripe offset is added to R, G and B before the clamp, so a channel
/// that sits at black or white swallows part of it. Black-and-white shows
/// the kind's luma alone, as a monochrome set does, plus the same offset. The
/// background is black whatever the options.
fn shade(cell: Cell, x: i32, j: usize, colour: bool, stripes: bool) -> Rgb {
    let Some(s) = signal(cell) else { return [0; 3] };
    let off = if stripes { stripe_offset(cell, j) } else { 0.0 };
    if colour {
        return unclipped(cell, x).map(|c| level(c + off));
    }
    [level(s.luma + off); 3]
}

/// Every (kind, X, sub-column) colour for one combination of display
/// options, built once and rebuilt when COLOR or CRT changes in the menu.
pub struct Palette {
    colour: bool,
    stripes: bool,
    /// Indexed `[(kind * COLUMNS + x - DISPLAY_X0) * SUBCOLUMNS + j]`.
    table: Vec<Rgb>,
}

impl Palette {
    /// Builds the palette for colour (or black-and-white) with or without
    /// the stripe texture.
    pub fn new(colour: bool, stripes: bool) -> Self {
        let mut table = Vec::with_capacity(KINDS * COLUMNS * SUBCOLUMNS);
        for cell in [Cell::Background, Cell::Border, Cell::Foreground] {
            for i in 0..COLUMNS {
                let x = DISPLAY_X0 + i as i32;
                for j in 0..SUBCOLUMNS {
                    table.push(shade(cell, x, j, colour, stripes));
                }
            }
        }
        Self { colour, stripes, table }
    }

    /// Whether this palette was built for the given options.
    pub fn matches(&self, colour: bool, stripes: bool) -> bool {
        self.colour == colour && self.stripes == stripes
    }

    /// The colour of sub-column `j` (0..4) of the cell at clock `x`.
    pub fn get(&self, cell: Cell, x: i32, j: usize) -> Rgb {
        debug_assert!(cell != Cell::Border || (0..FIELD_W).contains(&x), "border colour asked for at X {x}");
        let i = (x - DISPLAY_X0).clamp(0, COLUMNS as i32 - 1) as usize;
        self.table[(kind_index(cell) * COLUMNS + i) * SUBCOLUMNS + j]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `docs/superpong-data/colour_model.json`, the authoritative parameters.
    const MODEL_JSON: &str = include_str!("../../../docs/superpong-data/colour_model.json");

    const KIND_CELLS: [Cell; 3] = [Cell::Background, Cell::Border, Cell::Foreground];

    fn hex(rgb: Rgb) -> String {
        format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
    }

    #[test]
    fn constants_match_the_data_file() {
        let compact: String = MODEL_JSON.chars().filter(|c| !c.is_whitespace()).collect();
        let mut needles = vec!["\"background\":\"black\"".to_string(), "\"space\":\"OKLab\"".to_string()];
        for (name, s) in [("border", &BORDER), ("foreground", &FOREGROUND)] {
            let anchors: Vec<String> =
                s.anchors.iter().map(|(x, c)| format!("[{x},\"{}\"]", hex(*c))).collect();
            needles.push(format!(
                "\"{name}\":{{\"luma\":{},\"stripe\":{},\"stripe_peak\":{},\"anchors\":[{}]}}",
                s.luma,
                s.stripe,
                s.stripe_peak,
                anchors.join(",")
            ));
        }
        for needle in needles {
            assert!(compact.contains(&needle), "colour_model.json has {needle}");
        }
    }

    #[test]
    fn anchors_are_hit_exactly_at_their_x() {
        for (cell, s) in [(Cell::Border, &BORDER), (Cell::Foreground, &FOREGROUND)] {
            for (x, c) in s.anchors {
                assert_eq!(rgb(cell, *x), *c, "{cell:?} anchor at X {x}");
            }
        }
        // The values the box was read as (SPEC §3.1 table).
        assert_eq!(rgb(Cell::Foreground, 16), [0xf4, 0xec, 0x70]);
        assert_eq!(rgb(Cell::Foreground, 88), [0x98, 0xd4, 0xf4]);
        assert_eq!(rgb(Cell::Foreground, 161), [0xf2, 0x9c, 0xc4]);
        assert_eq!(rgb(Cell::Border, 0), [0x46, 0x66, 0xbc]);
        assert_eq!(rgb(Cell::Border, 88), [0x5c, 0xb8, 0xf0]);
        assert_eq!(rgb(Cell::Border, 175), [0x78, 0x62, 0xc2]);
    }

    #[test]
    fn spec_table_entries() {
        // The "selected entries" table of SPEC §3.1 (the background is black).
        for (x, border, foreground) in [
            (0, [70, 102, 188], [244, 236, 112]),
            (32, [78, 132, 207], [196, 228, 139]),
            (68, [87, 165, 228], [146, 216, 204]),
            (112, [106, 161, 227], [198, 206, 240]),
            (148, [117, 125, 208], [240, 178, 217]),
            (175, [120, 98, 194], [242, 156, 196]),
        ] {
            assert_eq!(rgb(Cell::Border, x), border, "border X {x}");
            assert_eq!(rgb(Cell::Foreground, x), foreground, "foreground X {x}");
        }
    }

    #[test]
    fn anchors_are_ascending_and_inside_the_field() {
        for s in [&BORDER, &FOREGROUND] {
            assert!(s.anchors.windows(2).all(|w| w[0].0 < w[1].0));
            assert!(s.anchors.iter().all(|(x, _)| (0..FIELD_W).contains(x)));
        }
    }

    #[test]
    fn constant_beyond_the_end_anchors() {
        for (cell, s) in [(Cell::Border, &BORDER), (Cell::Foreground, &FOREGROUND)] {
            let (first, last) = (s.anchors[0], s.anchors[s.anchors.len() - 1]);
            for x in (DISPLAY_X0..=first.0).chain(last.0..DISPLAY_X0 + DISPLAY_W) {
                let want = if x <= first.0 { first.1 } else { last.1 };
                assert_eq!(rgb(cell, x), want, "{cell:?} X {x}");
            }
        }
    }

    #[test]
    fn the_blend_is_monotone_and_smooth_between_anchors() {
        for s in [&BORDER, &FOREGROUND] {
            for w in s.anchors.windows(2) {
                let (p, q) = (oklab(w[0].1), oklab(w[1].1));
                let mut prev = p;
                for x in w[0].0 + 1..=w[1].0 {
                    let cur = gradient_lab(s.anchors, x);
                    for i in 0..3 {
                        let dir = (q[i] - p[i]).signum();
                        assert!(dir * (cur[i] - prev[i]) >= -1e-5, "OKLab[{i}] not monotone at X {x}");
                    }
                    // No jumps: one clock moves far less than the whole segment.
                    let step: f32 = (0..3).map(|i| (cur[i] - prev[i]).powi(2)).sum::<f32>().sqrt();
                    let span: f32 = (0..3).map(|i| (q[i] - p[i]).powi(2)).sum::<f32>().sqrt();
                    let clocks = (w[1].0 - w[0].0) as f32;
                    assert!((step - span / clocks).abs() < 1e-4, "uneven step at X {x}");
                    prev = cur;
                }
            }
        }
        // Neighbouring clocks differ by a few levels at most.
        for cell in [Cell::Border, Cell::Foreground] {
            for x in 0..FIELD_W - 1 {
                let (a, b) = (rgb(cell, x), rgb(cell, x + 1));
                assert!(a.iter().zip(&b).all(|(p, q)| p.abs_diff(*q) <= 6), "{cell:?} X {x}: {a:?} {b:?}");
            }
        }
    }

    #[test]
    fn colour_depends_on_x_not_row() {
        // Yellow paddle on the left, pink on the right (the box art).
        let left = rgb(Cell::Foreground, 17);
        let right = rgb(Cell::Foreground, 161);
        assert!(left[0] > 200 && left[1] > 200 && left[2] < 160, "left foreground is yellow: {left:?}");
        assert!(right[0] > 220 && right[1] < 180 && right[2] > 150, "right foreground is pink: {right:?}");
        // The left digits are green, the net light blue.
        let digit = rgb(Cell::Foreground, 48);
        assert!(digit[1] > digit[0] && digit[1] > digit[2], "left digit is green: {digit:?}");
        let net = rgb(Cell::Foreground, 89);
        assert!(net[2] > net[1] && net[1] > net[0], "net is light blue: {net:?}");
        assert_eq!(rgb(Cell::Background, 89), [0; 3]);
        // The players are clearly apart.
        let dist: i32 = (0..3).map(|i| (i32::from(left[i]) - i32::from(right[i])).abs()).sum();
        assert!(dist > 150, "players' colours differ by {dist}");
    }

    #[test]
    fn the_border_is_blue_to_violet_and_bluest_in_the_middle() {
        let (l, m, r) = (rgb(Cell::Border, 0), rgb(Cell::Border, 88), rgb(Cell::Border, 175));
        for c in [l, m, r] {
            assert!(c[2] > c[0] && c[2] > c[1], "blue dominates the border: {c:?}");
        }
        assert!(m[1] > l[1] && m[1] > r[1], "the middle is the lightest blue: {l:?} {m:?} {r:?}");
        assert!(r[0] > l[0] && r[0] > m[0], "the right end is violet: {r:?}");
        // The net (foreground) stands out from the border where they cross.
        assert!(rgb(Cell::Foreground, 88)[0] > m[0] + 40);
    }

    #[test]
    fn the_background_is_black_in_every_mode() {
        for colour in [true, false] {
            for stripes in [true, false] {
                let p = Palette::new(colour, stripes);
                for x in DISPLAY_X0..DISPLAY_X0 + DISPLAY_W {
                    for j in 0..SUBCOLUMNS {
                        assert_eq!(
                            p.get(Cell::Background, x, j),
                            [0; 3],
                            "colour {colour} stripes {stripes} X {x} j {j}"
                        );
                    }
                }
            }
        }
        for j in 0..SUBCOLUMNS {
            assert_eq!(stripe_offset(Cell::Background, j), 0.0);
        }
    }

    #[test]
    fn mono_is_a_constant_grey_per_kind() {
        let p = Palette::new(false, false);
        for x in [-6, 0, 88, 175, 180] {
            assert_eq!(p.get(Cell::Background, x, 0), [0; 3]);
            assert_eq!(p.get(Cell::Foreground, x, 3), [161; 3]);
        }
        for x in [0, 88, 175] {
            assert_eq!(p.get(Cell::Border, x, 2), [152; 3]);
        }
    }

    #[test]
    fn the_palette_is_deterministic() {
        for (colour, stripes) in [(true, true), (true, false), (false, true), (false, false)] {
            let (a, b) = (Palette::new(colour, stripes), Palette::new(colour, stripes));
            assert_eq!(a.table, b.table);
        }
    }

    #[test]
    fn oklab_round_trips() {
        for rgb in [[0, 0, 0], [255, 255, 255], [244, 236, 112], [74, 108, 196], [12, 200, 99]] {
            assert_eq!(srgb_levels(oklab(rgb)).map(|c| c.round() as u8), rgb);
        }
        // White is L = 1 with no chroma.
        let [l, a, b] = oklab([255, 255, 255]);
        assert!((l - 1.0).abs() < 1e-3 && a.abs() < 1e-3 && b.abs() < 1e-3);
    }

    #[test]
    fn stripe_offsets() {
        // a cos(2 pi ((j + 0.5) / 4 - peak)) for the foreground, a = 94,
        // peak 0.48.
        let expect = [-57.6, 74.3, 57.6, -74.3];
        for (j, e) in expect.into_iter().enumerate() {
            let off = stripe_offset(Cell::Foreground, j);
            assert!((off - e).abs() < 0.1, "j {j}: {off} vs {e}");
        }
        // The offsets sum to zero over a clock. The peaks are at 0.48 for the
        // foreground and 0.42 for the border; the background has none.
        for cell in KIND_CELLS {
            let sum: f32 = (0..SUBCOLUMNS).map(|j| stripe_offset(cell, j)).sum();
            assert!(sum.abs() < 1e-3);
        }
        let signs = |cell| (0..SUBCOLUMNS).map(|j| stripe_offset(cell, j) > 0.0).collect::<Vec<_>>();
        assert_eq!(signs(Cell::Foreground), [false, true, true, false]);
        assert_eq!(signs(Cell::Border), [false, true, true, false]);
        let ratio = stripe_offset(Cell::Border, 1).abs().max(stripe_offset(Cell::Border, 2).abs())
            / stripe_offset(Cell::Foreground, 1).abs();
        assert!(ratio > 0.1 && ratio < 0.3, "border to foreground {ratio}");
    }

    #[test]
    fn stripes_off_are_flat_and_on_are_not() {
        let flat = Palette::new(true, false);
        let striped = Palette::new(true, true);
        let sub: Vec<_> = (0..SUBCOLUMNS).map(|j| flat.get(Cell::Foreground, 16, j)).collect();
        assert!(sub.iter().all(|c| *c == sub[0]));
        assert_eq!(sub[0], [0xf4, 0xec, 0x70]);
        // The stripe moves every channel that has room, clamped to 0..=255.
        for j in 0..SUBCOLUMNS {
            let c = striped.get(Cell::Foreground, 16, j);
            assert_ne!(c, sub[0], "sub-column {j}");
        }
        assert!(striped.matches(true, true) && !striped.matches(true, false));
    }

    #[test]
    fn the_stripe_is_added_before_the_clip() {
        // The yellow's blue channel (0x70) has room both ways, so it carries
        // the whole stripe; its red (0xf4) is near white, so the positive half
        // is swallowed by the clamp.
        let striped = Palette::new(true, true);
        let blues: Vec<i32> =
            (0..SUBCOLUMNS).map(|j| i32::from(striped.get(Cell::Foreground, 16, j)[2])).collect();
        assert!(blues.iter().max().unwrap() - blues.iter().min().unwrap() > 100, "{blues:?}");
        let reds: Vec<u8> = (0..SUBCOLUMNS).map(|j| striped.get(Cell::Foreground, 16, j)[0]).collect();
        assert!(reds.iter().filter(|r| **r == 255).count() >= 2, "red is clipped at white: {reds:?}");
    }

    #[test]
    fn stripes_apply_to_mono_levels() {
        let striped = Palette::new(false, true);
        // The background stays black.
        assert!((0..SUBCOLUMNS).all(|j| striped.get(Cell::Background, 0, j) == [0; 3]));
        // Border: 152 +- 17 and foreground: 161 +- 74, clamped to 0..=255.
        for (cell, level, a) in [(Cell::Border, 152.0, 17.0), (Cell::Foreground, 161.0, 74.3)] {
            let v: Vec<u8> = (0..SUBCOLUMNS).map(|j| striped.get(cell, 0, j)[0]).collect();
            let (min, max) = (*v.iter().min().unwrap() as f32, *v.iter().max().unwrap() as f32);
            assert!(max > level + 0.9 * a && min < level - 0.9 * a, "{cell:?} {v:?}");
        }
    }
}
