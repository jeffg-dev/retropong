//! Taps on the picture from a touchscreen, turned into the presses a
//! keyboard player would make. The page reports where on the canvas a tap
//! landed; this finds what the menu draws there.
//!
//! With the menu closed a tap is Enter over the light show and Esc during a
//! match or its countdown, so it pauses a match and never restarts one
//! (`WebGame::advance` decides which). With the menu open, a tap on
//! an entry brings the cursor there and changes the entry: the arrows, then
//! Enter, Left or Right.

use superpong_core::{GameKind, BOTTOM_BORDER, DISPLAY_H, DISPLAY_ROW0, DISPLAY_W, DISPLAY_X0, PADDLE_W};

use crate::audio::VOLUME_MAX;
use crate::menu::screen::{item_row, stop_centre, STOP_PITCH, TEXT_H, VALUE_X, VOLUME_PITCH};
use crate::menu::{Command, Item, Menu};
use crate::settings::Settings;

/// The launch hints on a touchscreen, top line first.
pub const TOUCH_HINTS: [&str; 2] = ["TAP TO START", "HOLD FOR MENU"];

/// The CRT shader's barrel warp (`WARP` in crt.frag), so a tap lands where
/// the curved picture shows it.
const CRT_WARP: (f32, f32) = (0.024, 0.032);

/// The clock and line under a tap at `(x, y)`, fractions of the canvas from
/// its top left; clamped to the display window, `None` if not finite. With
/// `crt` the position is warped as the shader warps the picture.
pub fn raster(x: f32, y: f32, crt: bool) -> Option<(i32, i32)> {
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let (x, y) = if crt { warp(x, y) } else { (x, y) };
    let cell = |f: f32, start: i32, len: i32| start + ((f * len as f32) as i32).clamp(0, len - 1);
    Some((cell(x, DISPLAY_X0, DISPLAY_W), cell(y, DISPLAY_ROW0, DISPLAY_H)))
}

/// crt.frag's `warp`: from a point on the screen to the point of the
/// picture shown there.
fn warp(x: f32, y: f32) -> (f32, f32) {
    let (px, py) = (x * 2.0 - 1.0, y * 2.0 - 1.0);
    let (wx, wy) = (px * (1.0 + py * py * CRT_WARP.0), py * (1.0 + px * px * CRT_WARP.1));
    (wx * 0.5 + 0.5, wy * 0.5 + 0.5)
}

/// The commands a tap at clock `x`, line `row` gives the open menu. None
/// off the entries (the title, the border bars) or with the menu closed.
pub fn menu_commands(x: i32, row: i32, menu: &Menu, settings: &Settings, game: GameKind) -> Vec<Command> {
    let Some(item) = entry_at(row).filter(|_| menu.is_open()) else {
        return Vec::new();
    };
    let mut commands = steps(menu.cursor() as i32, item as i32, Command::Up, Command::Down);
    commands.extend(change(item, x, row, settings, game));
    commands
}

/// The entry at line `row`: from the line above its text to the line above
/// the next entry's, the last down to the bottom border bar.
fn entry_at(row: i32) -> Option<Item> {
    if row >= BOTTOM_BORDER.start {
        return None;
    }
    Item::ALL.into_iter().rev().find(|&item| row >= item_row(item) - 1)
}

/// What a tap at `(x, row)` on `item` does to it, the cursor already there.
fn change(item: Item, x: i32, row: i32, settings: &Settings, game: GameKind) -> Vec<Command> {
    match item {
        // The slider under the game's name: the nearest switch position.
        // The name and the label step on to the next game, as Enter does.
        Item::GameMode if row >= item_row(item) + TEXT_H && x >= VALUE_X - STOP_PITCH / 2 => {
            let to = GameKind::ALL
                .into_iter()
                .min_by_key(|kind| (x - stop_centre(kind.number())).abs())
                .unwrap_or(game);
            steps(game.number().into(), to.number().into(), Command::Left, Command::Right)
        }
        // Block i is level i + 1, each split from the next in the middle of
        // the gap. The top lit block steps down one, so silence is a tap on
        // the first block at level 1. The label only moves the cursor.
        Item::Volume => {
            let half_gap = (VOLUME_PITCH - PADDLE_W + 1) / 2;
            if x < VALUE_X - half_gap {
                return Vec::new();
            }
            let block = ((x - VALUE_X + half_gap) / VOLUME_PITCH).min(i32::from(VOLUME_MAX) - 1);
            let current = i32::from(settings.volume);
            let level = if block + 1 == current && !settings.muted { block } else { block + 1 };
            steps(current, level, Command::Left, Command::Right)
        }
        _ => vec![Command::Activate],
    }
}

/// `less` or `more` as many times as it takes from `from` to `to`.
fn steps(from: i32, to: i32, less: Command, more: Command) -> Vec<Command> {
    vec![if to < from { less } else { more }; from.abs_diff(to) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;
    use crate::menu::font::glyph;
    use crate::menu::screen::{draw_menu, MenuView, TITLE_TEXT};
    use superpong_core::{Cell, FIELD_W, TOP_BORDER};

    fn open_at(item: Item) -> Menu {
        let mut menu = Menu::new();
        menu.open_at(item);
        menu
    }

    fn view(game: GameKind, volume: u8) -> MenuView {
        MenuView {
            cursor: Item::Resume,
            game,
            color: true,
            crt: true,
            volume,
            muted: false,
            invert_left: true,
            invert_right: true,
        }
    }

    /// The lit cells of the open menu's picture in `rows`, at or right of `x0`.
    fn lit(canvas: &Canvas, x0: i32, rows: std::ops::Range<i32>) -> Vec<(i32, i32)> {
        rows.flat_map(|row| (x0..FIELD_W).map(move |x| (x, row)))
            .filter(|&(x, row)| canvas.get(x, row) == Cell::Foreground)
            .collect()
    }

    #[test]
    fn every_line_of_an_entry_and_its_cursor_belongs_to_that_entry() {
        for item in Item::ALL {
            for row in item_row(item)..item_row(item) + TEXT_H {
                assert_eq!(entry_at(row), Some(item), "{item:?} line {row}");
            }
        }
        // The Game Mode slider and its numbers sit under the name.
        let mut canvas = Canvas::new();
        draw_menu(&mut canvas, &view(GameKind::SuperPong, 8));
        for (_, row) in lit(&canvas, VALUE_X, item_row(Item::GameMode)..item_row(Item::Color) - 1) {
            assert_eq!(entry_at(row), Some(Item::GameMode), "line {row}");
        }
        // Nothing is an entry in the border bars or the title.
        let title_rows = TOP_BORDER.start..item_row(Item::Resume) - 1;
        assert!(title_rows.clone().any(|row| !lit(&canvas, 0, row..row + 1).is_empty()), "{TITLE_TEXT}");
        for row in title_rows.chain(BOTTOM_BORDER) {
            assert_eq!(entry_at(row), None, "line {row}");
        }
    }

    #[test]
    fn a_tap_brings_the_cursor_to_the_entry_then_changes_it() {
        let settings = Settings::default();
        let tap = |cursor, item: Item| {
            menu_commands(20, item_row(item) + 7, &open_at(cursor), &settings, GameKind::SuperPong)
        };
        assert_eq!(tap(Item::Resume, Item::Resume), [Command::Activate]);
        assert_eq!(tap(Item::Resume, Item::Color), [Command::Down, Command::Down, Command::Activate]);
        assert_eq!(
            tap(Item::Exit, Item::Crt),
            [Command::Up, Command::Up, Command::Up, Command::Up, Command::Activate]
        );
        assert_eq!(tap(Item::Crt, Item::Volume), [Command::Down], "the volume label only moves the cursor");
        assert_eq!(tap(Item::Crt, Item::GameMode)[2..], [Command::Activate], "the name steps on");
        // A closed menu, the title and the bars take no commands.
        assert!(menu_commands(20, 50, &Menu::new(), &settings, GameKind::Pong).is_empty());
        assert!(menu_commands(88, 25, &open_at(Item::Color), &settings, GameKind::Pong).is_empty());
        assert!(menu_commands(88, 215, &open_at(Item::Color), &settings, GameKind::Pong).is_empty());
    }

    /// Plays `commands` on a menu and returns the game it selects.
    fn slide(from: GameKind, commands: &[Command]) -> GameKind {
        let mut console = superpong_core::Console::new(from);
        let (mut menu, mut settings) = (open_at(Item::GameMode), Settings::default());
        for &command in commands {
            if let crate::menu::Action::Select(kind) = menu.command(command, &mut settings, &console.status())
            {
                console = superpong_core::Console::new(kind);
            }
        }
        console.kind()
    }

    #[test]
    fn a_tap_on_the_slider_selects_the_position_under_it() {
        for to in GameKind::ALL {
            let mut canvas = Canvas::new();
            draw_menu(&mut canvas, &view(to, 8));
            // The knob and the number of the selected position.
            let row = item_row(Item::GameMode);
            let cells = lit(&canvas, VALUE_X, row + TEXT_H..item_row(Item::Color) - 1);
            let at = cells.iter().filter(|&&(x, _)| (x - stop_centre(to.number())).abs() < STOP_PITCH / 2);
            for &(x, row) in at {
                for from in GameKind::ALL {
                    let commands =
                        menu_commands(x, row, &open_at(Item::GameMode), &Settings::default(), from);
                    assert_eq!(slide(from, &commands), to, "{from:?} to {to:?} at ({x}, {row})");
                }
            }
        }
    }

    #[test]
    fn a_tap_on_a_volume_block_sets_that_level_and_the_top_block_steps_down() {
        let row = item_row(Item::Volume) + TEXT_H - 1;
        let level_after = |from: u8, x: i32| {
            let mut settings = Settings { volume: from, ..Settings::default() };
            let mut menu = open_at(Item::Volume);
            let status = superpong_core::Console::new(GameKind::Pong).status();
            for command in menu_commands(x, row, &menu.clone(), &settings, GameKind::Pong) {
                menu.command(command, &mut settings, &status);
            }
            settings.volume
        };
        let mut canvas = Canvas::new();
        draw_menu(&mut canvas, &view(GameKind::Pong, 0));
        // Unlit, every block still draws its floor on the bottom line.
        let floors = lit(&canvas, VALUE_X, row..row + 1);
        assert_eq!(floors.len(), usize::from(VOLUME_MAX) * PADDLE_W as usize);
        for (i, &(x, _)) in floors.iter().enumerate() {
            let level = (i / PADDLE_W as usize + 1) as u8;
            for from in (0..=VOLUME_MAX).filter(|&from| from != level) {
                assert_eq!(level_after(from, x), level, "from {from} at x {x}");
            }
            assert_eq!(level_after(level, x), level - 1, "the top block at x {x}");
        }
        // The gaps split between neighbours; past the last block is the top.
        assert_eq!(level_after(5, VALUE_X - 3), 5, "the label");
        assert_eq!(level_after(5, VALUE_X - 2), 1);
        assert_eq!(level_after(5, VALUE_X + PADDLE_W), 1);
        assert_eq!(level_after(5, VALUE_X + PADDLE_W + 1), 2);
        assert_eq!(level_after(5, FIELD_W - 1), VOLUME_MAX);
    }

    #[test]
    fn taps_are_placed_on_the_curved_crt_picture() {
        assert!(include_str!("../../superpong/src/shaders/crt.frag")
            .contains("const vec2 WARP = vec2(0.024, 0.032);"));
        let centre = (DISPLAY_X0 + DISPLAY_W / 2, DISPLAY_ROW0 + DISPLAY_H / 2);
        assert_eq!(raster(0.5, 0.5, false), Some(centre));
        assert_eq!(raster(0.5, 0.5, true), Some(centre));
        assert_eq!(raster(0.0, 0.0, false), Some((DISPLAY_X0, DISPLAY_ROW0)));
        assert_eq!(raster(1.0, 1.0, false), Some((DISPLAY_X0 + DISPLAY_W - 1, DISPLAY_ROW0 + DISPLAY_H - 1)));
        // Towards a corner the curved picture shows lines further out.
        let (flat, curved) = (raster(0.1, 0.9, false).unwrap(), raster(0.1, 0.9, true).unwrap());
        assert!(curved.0 < flat.0 && curved.1 > flat.1, "{flat:?} {curved:?}");
        assert_eq!(raster(f32::NAN, 0.5, true), None);
        assert_eq!(raster(0.5, f32::INFINITY, false), None);
    }

    #[test]
    fn touch_hints_are_in_the_menu_font() {
        assert!(TOUCH_HINTS.iter().flat_map(|hint| hint.chars()).all(|c| glyph(c).is_some()));
    }
}
