//! The menu and the countdown as cells (SPEC §13.6), drawn on the
//! [`Canvas`] in the console's own raster and vocabulary.
//!
//! The menu blanks the playfield between the two border bars, which stay as
//! the console draws them, and is lit only as foreground (letters, the
//! cursor, knobs, volume blocks) and border (the slider's track), so the
//! palette colours it with the same left-to-right gradient as the game. The
//! cursor is a paddle as tall as the entry's text, the Game Mode slider is
//! the console's Select Game switch (a track over the numbers 1 to 4 with a
//! paddle-sized knob), and the volume is a row of paddle-wide blocks. The
//! countdown uses oversized score digits and GO in a clear central panel.
//!
//! All positions are clocks (X) and lines (rows) of the display window.

use superpong_core::{
    Cell, GameKind, BALL_H, BALL_W, BOTTOM_BORDER, FIELD_H, FIELD_W, GLYPHS, NET_W, NET_X, PADDLE_H,
    PADDLE_W, TOP_BORDER,
};

use super::font::{centred, draw_text, Scale, GLYPH_ROWS};
use super::Item;
use crate::canvas::Canvas;

/// Font pixel of the entries: 2 clocks by 3 lines, about square on a TV
/// (a clock is 1.71 lines wide, SPEC §13.1).
const TEXT: Scale = Scale { w: 2, h: 3 };
/// Font pixel of the title: 3 clocks by 4 lines.
const TITLE: Scale = Scale { w: 3, h: 4 };
/// Height of a line of entry text.
const TEXT_H: i32 = GLYPH_ROWS as i32 * TEXT.h;

/// The title, and the top row of its letters.
pub const TITLE_TEXT: &str = "RETROPONG";
const TITLE_ROW: i32 = 17;

/// Left X of the cursor paddle, of the labels and of the values. The widest
/// value (`SUPER PONG`) ends as far from the right edge as the cursor is
/// from the left one.
const CURSOR_X: i32 = 4;
const LABEL_X: i32 = 11;
const VALUE_X: i32 = 94;

/// Top row of each entry's text, in [`Item::ALL`] order. Game Mode takes
/// three lines of the menu: the game's name, the slider and its numbers.
const ITEM_ROWS: [i32; 8] = [39, 55, 107, 124, 141, 158, 175, 192];

/// The Game Mode slider, in lines below the top of its entry: the knob (a
/// paddle) and the track it rides on, then the numbers under it, as on the
/// console.
const KNOB_ROWS: (i32, i32) = (18, 18 + PADDLE_H);
const TRACK_ROWS: (i32, i32) = (24, 27);
const NUMBER_ROW: i32 = 36;
/// Clocks between the switch positions' numbers.
const STOP_PITCH: i32 = 24;

/// Volume blocks: paddle-wide, this many clocks apart. An unlit block is
/// only its bottom font pixel, a floor marking the step.
const VOLUME_PITCH: i32 = 7;

/// Large countdown score cells: twice the normal digit width and 1.5x its height.
const COUNT_CELL: (i32, i32) = (8, 12);

/// What the menu shows: the cursor and the settings it edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuView {
    pub cursor: Item,
    pub game: GameKind,
    pub color: bool,
    pub crt: bool,
    /// `0..=VOLUME_MAX`.
    pub volume: u8,
    pub muted: bool,
    pub invert_left: bool,
    pub invert_right: bool,
}

/// What is drawn over the console's picture this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// Nothing: the game as the console drew it.
    None,
    Launch,
    Menu(MenuView),
    /// The resume countdown.
    Countdown(Countdown),
}

/// The resume countdown's picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Countdown {
    /// The number shown, 3, 2 or 1; zero means GO.
    pub number: u8,
    /// The selected game, retained with the countdown view.
    pub game: GameKind,
    /// Left column and top row of the ball if it is on the screen: drawn
    /// again over the digit and the gap in the net, so it is never hidden.
    pub ball: Option<(i32, i32)>,
}

impl Overlay {
    /// Draws the overlay onto `canvas`, which holds the console's picture.
    pub fn draw(&self, canvas: &mut Canvas) {
        match *self {
            Overlay::None => {}
            Overlay::Launch => {
                // Match the menu's chunky lettering, with a single clear panel
                // so the moving light show cannot break up the two hints.
                canvas.fill(8..FIELD_W - 8, 112..175, Cell::Background);
                for (text, row) in [("PRESS ENTER TO START", 120), ("PRESS ESC FOR MENU", 152)] {
                    let span = centred(text, FIELD_W / 2, TEXT);
                    draw_text(canvas, text, span.start, row, TEXT);
                }
            }
            Overlay::Menu(view) => draw_menu(canvas, &view),
            Overlay::Countdown(countdown) => draw_countdown(canvas, &countdown),
        }
    }
}

/// Top row of `item`'s text.
fn item_row(item: Item) -> i32 {
    ITEM_ROWS[item as usize]
}

/// Left X of the digit cell of switch position `number` (1..=4).
fn stop_x(number: u8) -> i32 {
    VALUE_X + STOP_PITCH * (i32::from(number) - 1)
}

/// Centre X of the number's ink, which the knob sits over. The digit cell is
/// 3 font pixels wide, but a `1` is only its right-hand column.
fn stop_centre(number: u8) -> i32 {
    stop_x(number) + if number == 1 { TEXT.w * 5 / 2 } else { TEXT.w * 3 / 2 }
}

/// The menu screen: the border bars, the title and the seven entries.
pub fn draw_menu(canvas: &mut Canvas, view: &MenuView) {
    canvas.clear();
    // The border bars, with the net's light spots in them, as the console
    // draws them (SPEC §4.1).
    for row in TOP_BORDER.chain(BOTTOM_BORDER) {
        canvas.fill(0..FIELD_W, row..row + 1, Cell::Border);
        if row.rem_euclid(16) < 8 {
            canvas.fill(NET_X..NET_X + NET_W, row..row + 1, Cell::Foreground);
        }
    }

    let title = centred(TITLE_TEXT, FIELD_W / 2, TITLE);
    draw_text(canvas, TITLE_TEXT, title.start, TITLE_ROW, TITLE);

    for item in Item::ALL {
        draw_text(canvas, item.label(), LABEL_X, item_row(item), TEXT);
    }
    let on_off = |on: bool| if on { "ON" } else { "OFF" };
    draw_text(canvas, on_off(view.color), VALUE_X, item_row(Item::Color), TEXT);
    draw_text(canvas, on_off(view.crt), VALUE_X, item_row(Item::Crt), TEXT);
    let direction = |inverted: bool| if inverted { "CW UP" } else { "CCW UP" };
    draw_text(canvas, direction(view.invert_left), VALUE_X, item_row(Item::LeftDirection), TEXT);
    draw_text(canvas, direction(view.invert_right), VALUE_X, item_row(Item::RightDirection), TEXT);
    draw_game_mode(canvas, view.game);
    draw_volume(canvas, view.volume, view.muted);

    // The player's paddle points at the entry: as tall as its text.
    let top = item_row(view.cursor);
    canvas.fill(CURSOR_X..CURSOR_X + PADDLE_W, top..top + TEXT_H, Cell::Foreground);
}

/// The selected game's name, and under it the Select Game switch: a track
/// in the border's colours, the knob on the selected position, the
/// numbers 1 to 4 below.
fn draw_game_mode(canvas: &mut Canvas, game: GameKind) {
    let row = item_row(Item::GameMode);
    draw_text(canvas, game.name(), VALUE_X, row, TEXT);
    let track = stop_x(1)..stop_x(4) + TEXT.w * 3;
    canvas.fill(track, row + TRACK_ROWS.0..row + TRACK_ROWS.1, Cell::Border);
    let knob = stop_centre(game.number()) - PADDLE_W / 2;
    canvas.fill(knob..knob + PADDLE_W, row + KNOB_ROWS.0..row + KNOB_ROWS.1, Cell::Foreground);
    for kind in GameKind::ALL {
        let digit = char::from(b'0' + kind.number());
        draw_text(canvas, &digit.to_string(), stop_x(kind.number()), row + NUMBER_ROW, TEXT);
    }
}

/// Ten blocks, the first `volume` lit; `MUTE` when started with `--mute`.
fn draw_volume(canvas: &mut Canvas, volume: u8, muted: bool) {
    let row = item_row(Item::Volume);
    if muted {
        draw_text(canvas, "MUTE", VALUE_X, row, TEXT);
        return;
    }
    for i in 0..crate::audio::VOLUME_MAX {
        let x = VALUE_X + VOLUME_PITCH * i32::from(i);
        let top = if i < volume { row } else { row + TEXT_H - TEXT.h };
        canvas.fill(x..x + PADDLE_W, top..row + TEXT_H, Cell::Foreground);
    }
}

/// A large centred countdown over a clear panel. Zero is the final GO step.
/// Outer paddles and borders remain visible; the ball is restored over the panel.
pub fn draw_countdown(canvas: &mut Canvas, countdown: &Countdown) {
    canvas.fill(56..124, 56..168, Cell::Background);
    if countdown.number == 0 {
        let scale = Scale { w: 8, h: 16 };
        let span = centred("GO", FIELD_W / 2, scale);
        draw_text(canvas, "GO", span.start, FIELD_H / 2 - 40, scale);
    } else {
        let (cw, ch) = COUNT_CELL;
        let (x0, row0) = (FIELD_W / 2 - 2 * cw, FIELD_H / 2 - 4 * ch);
        for (gr, bits) in (0..).zip(GLYPHS[usize::from(countdown.number % 10)]) {
            for gc in (0..4).filter(|gc| bits & (0b1000 >> gc) != 0) {
                let (x, row) = (x0 + cw * gc, row0 + ch * gr);
                canvas.fill(x..x + cw, row..row + ch, Cell::Foreground);
            }
        }
    }
    if let Some((x, row)) = countdown.ball {
        canvas.fill(x..x + BALL_W, row..row + BALL_H, Cell::Foreground);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::VOLUME_MAX;
    use superpong_core::{Console, Frame, Phase, DISPLAY_H, DISPLAY_ROW0, DISPLAY_W, DISPLAY_X0};

    /// `menu_golden.txt`: the whole display window of [`golden_view`]'s
    /// menu. Regenerate with `UPDATE_GOLDEN=1 cargo test -p superpong`
    /// after a deliberate change, and look at the diff.
    const GOLDEN: &str = include_str!("menu_golden.txt");

    fn golden_view() -> MenuView {
        MenuView {
            cursor: Item::GameMode,
            game: GameKind::SuperPong,
            color: true,
            crt: false,
            volume: 8,
            muted: false,
            invert_left: false,
            invert_right: true,
        }
    }

    fn menu(view: MenuView) -> Canvas {
        let mut canvas = Canvas::new();
        draw_menu(&mut canvas, &view);
        canvas
    }

    fn whole(canvas: &Canvas) -> String {
        canvas.to_ascii(DISPLAY_X0..DISPLAY_X0 + DISPLAY_W, DISPLAY_ROW0..DISPLAY_ROW0 + DISPLAY_H)
    }

    #[test]
    fn the_menu_matches_its_golden_picture() {
        let ascii = whole(&menu(golden_view()));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/menu/menu_golden.txt");
            std::fs::write(path, &ascii).unwrap();
        }
        assert_eq!(ascii, GOLDEN);
    }

    #[test]
    fn the_menu_replaces_whatever_was_behind_it() {
        // Over a light show or over a game, the menu is the same picture.
        let mut frame = Frame::new();
        let mut canvas = Canvas::new();
        for console in [Console::new(GameKind::Pong), {
            let mut c = Console::new(GameKind::Catch);
            c.press_start();
            c
        }] {
            console.render(&mut frame);
            canvas.copy_from(&frame);
            draw_menu(&mut canvas, &golden_view());
            assert_eq!(canvas, menu(golden_view()), "{:?}", console.phase());
        }
    }

    #[test]
    fn everything_lit_is_inside_the_border_box() {
        let canvas = menu(golden_view());
        for row in DISPLAY_ROW0..DISPLAY_ROW0 + DISPLAY_H {
            for x in DISPLAY_X0..DISPLAY_X0 + DISPLAY_W {
                let inside = (0..FIELD_W).contains(&x) && (0..FIELD_H).contains(&row);
                assert!(inside || canvas.get(x, row) == Cell::Background, "({x}, {row})");
            }
        }
        // The bars are border, the playfield between them is not.
        assert_eq!(canvas.get(0, 0), Cell::Border);
        assert_eq!(canvas.get(FIELD_W - 1, BOTTOM_BORDER.end - 1), Cell::Border);
        assert_eq!(canvas.get(NET_X, 0), Cell::Foreground, "the net's light spot");
        let border_in_field = (TOP_BORDER.end..BOTTOM_BORDER.start)
            .flat_map(|row| (0..FIELD_W).map(move |x| (x, row)))
            .filter(|&(x, row)| canvas.get(x, row) == Cell::Border)
            .count();
        let track = (stop_x(4) + TEXT.w * 3 - stop_x(1)) * (TRACK_ROWS.1 - TRACK_ROWS.0);
        assert_eq!(
            border_in_field as i32,
            track - PADDLE_W * (TRACK_ROWS.1 - TRACK_ROWS.0),
            "only the track"
        );
    }

    #[test]
    fn the_cursor_is_a_paddle_as_tall_as_the_entry_under_it() {
        for item in Item::ALL {
            let canvas = menu(MenuView { cursor: item, ..golden_view() });
            let lit: Vec<i32> = (TOP_BORDER.end..BOTTOM_BORDER.start)
                .filter(|&row| canvas.get(CURSOR_X, row) == Cell::Foreground)
                .collect();
            let top = item_row(item);
            assert_eq!(lit, (top..top + TEXT_H).collect::<Vec<_>>(), "{item:?}");
            assert_eq!(TEXT_H, PADDLE_H, "the letters are as tall as a paddle");
            for row in top..top + TEXT_H {
                assert_eq!(canvas.get(CURSOR_X + PADDLE_W - 1, row), Cell::Foreground);
                assert_eq!(canvas.get(CURSOR_X + PADDLE_W, row), Cell::Background);
            }
        }
    }

    /// The lit X on the knob's top row, which only the knob reaches.
    fn knob_x(canvas: &Canvas) -> Vec<i32> {
        let row = item_row(Item::GameMode) + KNOB_ROWS.0;
        (0..FIELD_W).filter(|&x| canvas.get(x, row) == Cell::Foreground).collect()
    }

    #[test]
    fn the_knob_sits_over_the_selected_number() {
        for kind in GameKind::ALL {
            let canvas = menu(MenuView { game: kind, ..golden_view() });
            // The knob is centred on the ink of the number under it: the
            // lit columns of the digit on its top row, whose first and last
            // are the same distance inside the knob's ends.
            let number_row = item_row(Item::GameMode) + NUMBER_ROW;
            let ink: Vec<i32> = (stop_x(kind.number())..stop_x(kind.number()) + TEXT.w * 3)
                .filter(|&x| {
                    (number_row..number_row + TEXT_H).any(|row| canvas.get(x, row) == Cell::Foreground)
                })
                .collect();
            let knob = knob_x(&canvas);
            assert_eq!(knob.len() as i32, PADDLE_W, "{kind:?}");
            let (ink_mid, knob_mid) = (ink[0] + ink[ink.len() - 1], knob[0] + knob[knob.len() - 1]);
            assert_eq!(ink_mid, knob_mid, "{kind:?}: ink {ink:?}, knob {knob:?}");
        }
        // The `1` is only its right-hand column, and the knob is over that.
        let one = menu(MenuView { game: GameKind::Catch, ..golden_view() });
        assert_eq!(knob_x(&one), (stop_x(1) + 3..stop_x(1) + 7).collect::<Vec<_>>());
    }

    #[test]
    fn volume_lights_one_block_per_step() {
        let row = item_row(Item::Volume);
        for volume in 0..=VOLUME_MAX {
            let canvas = menu(MenuView { volume, ..golden_view() });
            let tall = (0..VOLUME_MAX)
                .filter(|&i| canvas.get(VALUE_X + VOLUME_PITCH * i32::from(i), row) == Cell::Foreground)
                .count();
            assert_eq!(tall, usize::from(volume));
            // Every step keeps its floor, lit or not.
            for i in 0..VOLUME_MAX {
                let x = VALUE_X + VOLUME_PITCH * i32::from(i);
                assert_eq!(canvas.get(x, row + TEXT_H - 1), Cell::Foreground);
                assert_eq!(canvas.get(x + PADDLE_W, row + TEXT_H - 1), Cell::Background);
            }
        }
        // Muted: the word instead of the blocks.
        let muted = menu(MenuView { muted: true, ..golden_view() });
        let mut want = menu(MenuView { volume: 0, ..golden_view() });
        want.fill(VALUE_X..FIELD_W, row..row + TEXT_H, Cell::Background);
        draw_text(&mut want, "MUTE", VALUE_X, row, TEXT);
        assert_eq!(muted, want);
    }

    #[test]
    fn toggles_read_on_and_off() {
        let on = menu(golden_view());
        let off = menu(MenuView { color: false, crt: true, ..golden_view() });
        let word = |canvas: &Canvas, item: Item| {
            let row = item_row(item);
            canvas.to_ascii(VALUE_X..VALUE_X + 24, row..row + TEXT_H)
        };
        let draw_word = |text: &str| {
            let mut c = Canvas::new();
            draw_text(&mut c, text, VALUE_X, item_row(Item::Color), TEXT);
            c.to_ascii(VALUE_X..VALUE_X + 24, item_row(Item::Color)..item_row(Item::Color) + TEXT_H)
        };
        assert_eq!(word(&on, Item::Color), draw_word("ON"));
        assert_eq!(word(&on, Item::Crt), draw_word("OFF"));
        assert_eq!(word(&off, Item::Color), draw_word("OFF"));
        assert_eq!(word(&off, Item::Crt), draw_word("ON"));
    }

    #[test]
    fn direction_values_show_the_turn_that_moves_each_paddle_up() {
        for (left, right) in [(false, false), (true, false), (false, true), (true, true)] {
            let canvas = menu(MenuView { invert_left: left, invert_right: right, ..golden_view() });
            for (item, inverted) in [(Item::LeftDirection, left), (Item::RightDirection, right)] {
                let row = item_row(item);
                let mut expected = Canvas::new();
                draw_text(&mut expected, if inverted { "CW UP" } else { "CCW UP" }, VALUE_X, row, TEXT);
                assert_eq!(
                    canvas.to_ascii(VALUE_X..FIELD_W, row..row + TEXT_H),
                    expected.to_ascii(VALUE_X..FIELD_W, row..row + TEXT_H),
                    "{item:?}, inverted={inverted}"
                );
            }
        }
    }

    #[test]
    fn the_layout_fits_between_the_bars_without_overlaps() {
        // Each entry's text, and the slider, keep at least a line clear of
        // the next thing down and of the bars.
        let title_bottom = TITLE_ROW + GLYPH_ROWS as i32 * TITLE.h;
        assert!(TITLE_ROW > TOP_BORDER.end && title_bottom < item_row(Item::Resume));
        let game = item_row(Item::GameMode);
        assert!(game + TEXT_H < game + KNOB_ROWS.0);
        assert!(game + KNOB_ROWS.1 < game + NUMBER_ROW);
        assert!(game + NUMBER_ROW + TEXT_H < item_row(Item::Color));
        for pair in ITEM_ROWS.windows(2) {
            assert!(pair[0] + TEXT_H < pair[1], "{pair:?}");
        }
        assert!(item_row(Item::Exit) + TEXT_H < BOTTOM_BORDER.start);
        // The widest values fit the value column.
        let widest = GameKind::ALL.map(|k| super::super::font::text_width(k.name(), TEXT)).into_iter().max();
        assert!(VALUE_X + widest.unwrap() <= FIELD_W);
        let volume_end = VALUE_X + VOLUME_PITCH * i32::from(VOLUME_MAX - 1) + PADDLE_W;
        assert!(volume_end <= FIELD_W);
        // Labels end before the values start.
        let label_w = Item::ALL.map(|i| super::super::font::text_width(i.label(), TEXT)).into_iter().max();
        assert!(LABEL_X + label_w.unwrap() < VALUE_X);
        // The cursor is as far from the left edge as the widest value is
        // from the right one (give or take a clock).
        let right_margin = FIELD_W - (VALUE_X + widest.unwrap());
        assert!((right_margin - CURSOR_X).abs() <= 1, "{right_margin} against {CURSOR_X}");
        const { assert!(CURSOR_X + PADDLE_W < LABEL_X) };
    }

    /// The playfield of a game in progress, as the countdown sees it.
    fn playfield(kind: GameKind) -> Canvas {
        let mut console = Console::new(kind);
        console.press_start();
        assert_eq!(console.phase(), Phase::Waiting);
        picture(&console)
    }

    fn picture(console: &Console) -> Canvas {
        let mut frame = Frame::new();
        console.render(&mut frame);
        let mut canvas = Canvas::new();
        canvas.copy_from(&frame);
        canvas
    }

    fn countdown(number: u8, game: GameKind) -> Countdown {
        Countdown { number, game, ball: None }
    }

    #[test]
    fn countdown_digits_and_go_are_large_centred_and_preserve_the_outer_field() {
        for game in GameKind::ALL {
            for n in 0..=3 {
                let before = playfield(game);
                let mut canvas = before.clone();
                draw_countdown(&mut canvas, &countdown(n, game));
                let lit = (56..168)
                    .flat_map(|row| (56..124).map(move |x| (x, row)))
                    .filter(|&(x, row)| canvas.get(x, row) == Cell::Foreground)
                    .count();
                assert!(lit >= 768, "large lettering for {n}: {lit}");
                for row in 0..FIELD_H {
                    for x in 0..FIELD_W {
                        if !(56..124).contains(&x) || !(56..168).contains(&row) {
                            assert_eq!(canvas.get(x, row), before.get(x, row));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn launch_panel_clears_the_score_digits() {
        let console = Console::new(GameKind::Pong);
        let before = picture(&console);
        let mut canvas = before.clone();
        Overlay::Launch.draw(&mut canvas);
        for row in superpong_core::DIGIT_ROW0..superpong_core::DIGIT_ROW0 + superpong_core::DIGIT_H {
            for x in 0..FIELD_W {
                assert_eq!(canvas.get(x, row), before.get(x, row));
            }
        }
    }

    #[test]
    fn the_countdown_never_hides_the_ball() {
        // A ball anywhere near the middle, over the digit's strokes, in the
        // gap in the net or beside them, is still lit.
        for game in GameKind::ALL {
            for (x, row) in [(88, 112), (90, 80), (82, 100), (96, 140), (94, 64), (87, 150), (89, 84)] {
                for n in 1..=3 {
                    let mut canvas = playfield(game);
                    let hidden = canvas.clone();
                    draw_countdown(&mut canvas, &Countdown { ball: Some((x, row)), ..countdown(n, game) });
                    let mut without = hidden;
                    draw_countdown(&mut without, &countdown(n, game));
                    for r in row..row + BALL_H {
                        for c in x..x + BALL_W {
                            assert_eq!(canvas.get(c, r), Cell::Foreground, "{game:?} {n} ball ({x}, {row})");
                        }
                    }
                    // Nothing else differs from the countdown without it.
                    for r in 0..FIELD_H {
                        for c in 0..FIELD_W {
                            let in_ball = (x..x + BALL_W).contains(&c) && (row..row + BALL_H).contains(&r);
                            if !in_ball {
                                assert_eq!(canvas.get(c, r), without.get(c, r), "{game:?} ({c}, {r})");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_frozen_rally_ball_stays_visible_through_the_countdown() {
        // The real thing: a Rally console, ball drawn by the core, with the
        // countdown over it.
        let mut console = Console::new(GameKind::Pong);
        console.press_start();
        let mut events = Vec::new();
        while console.phase() != Phase::Rally {
            console.tick(&superpong_core::Input::default(), &mut events);
        }
        let ball = console.status().ball.expect("a ball in the rally");
        let mut canvas = picture(&console);
        assert_eq!(canvas.get(ball.x, ball.row), Cell::Foreground);
        draw_countdown(
            &mut canvas,
            &Countdown { ball: Some((ball.x, ball.row)), ..countdown(2, GameKind::Pong) },
        );
        assert_eq!(canvas.get(ball.x, ball.row), Cell::Foreground);
    }

    #[test]
    fn no_overlay_leaves_the_picture_alone() {
        let before = playfield(GameKind::Pong);
        let mut canvas = before.clone();
        Overlay::None.draw(&mut canvas);
        assert_eq!(canvas, before);
        let two = countdown(2, GameKind::Pong);
        Overlay::Countdown(two).draw(&mut canvas);
        let mut want = before.clone();
        draw_countdown(&mut want, &two);
        assert_eq!(canvas, want);
    }
}
