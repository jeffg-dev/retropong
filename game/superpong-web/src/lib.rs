//! WebAssembly bridge using the native simulation, menu, font and scheduler.
//! `init(1..=4)` powers on in attract mode. `advance(dt,left,right,commands)`
//! processes one rendered frame: commands are toggle=1, back=2, up=4, down=8,
//! left=16, right=32, activate=64, and the gamepad South/East buttons
//! select=128, next=256 (mapped by the menu). Returns the last native speaker event:
//! bounce=1, point=2, serve=4, attract=8, or zero for silence/no new event.
//! `tap(x,y)` (fractions of the canvas) is handled by the next `advance`;
//! `set_touch_hints(1)` words the launch hints for a touchscreen.
//! `render()` returns opaque RGBA in exported memory; reacquire memory.buffer
//! after calls that may grow memory. Pixels remain valid until init/render.

use std::cell::RefCell;
use superpong_core::{Console, Frame, GameKind, Input, Phase, SoundEvent, DISPLAY_H, DISPLAY_W, DISPLAY_X0};

#[path = "../../superpong/src/canvas.rs"]
mod canvas;
#[path = "../../superpong/src/colour.rs"]
mod colour;
#[path = "../../superpong/src/menu/mod.rs"]
mod menu;
#[path = "../../superpong/src/scheduler.rs"]
#[allow(dead_code)]
mod scheduler;
#[path = "../../superpong/src/settings.rs"]
mod settings;
mod touch;

// The native settings/menu only need the volume scale, not its audio device.
// This is the same scale/gain as superpong/src/audio.rs.
mod audio {
    pub const VOLUME_MAX: u8 = 10;
    pub const DEFAULT_VOLUME: u8 = 8;
    pub fn volume_gain(level: u8) -> f32 {
        let f = f32::from(level.min(VOLUME_MAX)) / f32::from(VOLUME_MAX);
        f * f
    }
}

use canvas::Canvas;
use menu::screen::{draw_launch, Countdown, MenuView, Overlay};
use menu::{Action, Command, Menu, MenuButton};
use scheduler::Scheduler;
use settings::{DisplayMode, Settings};

struct WebGame {
    console: Console,
    frame: Frame,
    canvas: Canvas,
    palette: colour::Palette,
    rgba: Vec<u8>,
    events: Vec<SoundEvent>,
    menu: Menu,
    scheduler: Scheduler,
    settings: Settings,
    first_launch: bool,
    preview_held: bool,
    knobs: (f32, f32),
    exit: bool,
    /// Clock and line of a tap waiting for the next `advance`.
    tap: Option<(i32, i32)>,
    touch_hints: bool,
}

fn game_kind(number: u32) -> GameKind {
    u8::try_from(number).ok().and_then(GameKind::from_number).unwrap_or(GameKind::SuperPong)
}

impl WebGame {
    fn new(game: u32) -> Self {
        Self {
            console: Console::new(game_kind(game)),
            frame: Frame::new(),
            canvas: Canvas::new(),
            palette: colour::Palette::new(true, true),
            rgba: vec![0; width() as usize * height() as usize * 4],
            events: Vec::with_capacity(4),
            menu: Menu::new(),
            scheduler: Scheduler::new(),
            settings: Settings::default(),
            first_launch: true,
            preview_held: false,
            knobs: (0.5, 0.5),
            exit: false,
            tap: None,
            touch_hints: false,
        }
    }

    fn pause(&mut self) {
        self.tap = None;
        if !self.menu.is_open() {
            self.menu.open_at(menu::Item::Resume);
            self.scheduler.pause(&mut self.console);
            self.preview_held = false;
        }
    }

    /// Applies one menu command; `true` when it exits the game.
    fn menu_command(&mut self, command: Command, sound: &mut u32) -> bool {
        match self.menu.command(command, &mut self.settings, &self.console.status()) {
            Action::None => {}
            Action::Select(kind) => self.scheduler.select_now(&mut self.console, kind),
            Action::Volume => *sound = 1,
            Action::Exit => {
                self.exit = true;
                return true;
            }
        }
        false
    }

    // Same ordering as native main: sample pause state, buffer Start, handle
    // menu actions, advance countdown, run ticks, choose live paddle preview.
    fn advance(&mut self, dt: f32, left: f32, right: f32, mut commands: u32) -> u32 {
        let dt = if dt.is_finite() { scheduler::frame_dt(false, dt.max(0.0)) } else { 0.0 };
        let was_open = self.menu.is_open();
        let paused = !self.menu.runs_console();
        let tap = self.tap.take();
        // A tap on the closed menu is Enter over the light show and Esc
        // otherwise: it pauses a match and never restarts one.
        if tap.is_some() && !was_open {
            commands |= if !paused && self.console.phase() == Phase::Attract { 64 } else { 1 };
        }
        if !paused && commands & 64 != 0 {
            self.scheduler.press_start();
        }
        self.knobs = self.settings.apply(left, right);
        let mut sound = 0;
        for (bit, command) in [
            (1, Command::Toggle),
            (2, Command::Back),
            (4, Command::Up),
            (8, Command::Down),
            (16, Command::Left),
            (32, Command::Right),
            (64, Command::Activate),
        ] {
            if commands & bit == 0 || (bit > 2 && !was_open) {
                continue;
            }
            if self.menu_command(command, &mut sound) {
                return 0;
            }
        }
        // Mapped with the menu as each press arrives, as native does.
        for (bit, button) in [(128, MenuButton::Select), (256, MenuButton::Next)] {
            let Some(command) = (commands & bit != 0).then(|| self.menu.button_command(button)).flatten()
            else {
                continue;
            };
            if self.menu_command(command, &mut sound) {
                return 0;
            }
        }
        // A tap on the open menu, found from the cursor the keys left.
        if let Some((x, row)) = tap.filter(|_| was_open) {
            let game = self.console.kind();
            for command in touch::menu_commands(x, row, &self.menu, &self.settings, game) {
                if self.menu_command(command, &mut sound) {
                    return 0;
                }
            }
        }
        if !paused && !self.menu.runs_console() {
            self.scheduler.pause(&mut self.console);
        }
        if self.menu.advance(dt).is_some() {
            sound = 1;
        }
        let mut ticks = 0;
        if self.menu.runs_console() {
            let (left, right) = self.knobs;
            ticks = self.scheduler.run(&mut self.console, f64::from(dt), &mut self.events, |_| Input {
                left,
                right,
                ..Input::default()
            });
            if let Some(event) = self.events.last() {
                sound = sound_bit(event);
            }
        }
        if self.menu.countdown().is_some() {
            self.preview_held = true;
        } else if self.menu.is_open() || ticks > 0 {
            self.preview_held = false;
        }
        if self.console.phase() != Phase::Attract {
            self.first_launch = false;
        }
        sound
    }

    fn render(&mut self) -> *const u8 {
        let colour = self.settings.color;
        let crt = self.settings.display == DisplayMode::Crt;
        if !self.palette.matches(colour, crt) {
            self.palette = colour::Palette::new(colour, crt);
        }
        // Native routing::preview: next tick on a disposable copy, including
        // the ball, held until the first real tick after the countdown.
        let mut shown = self.console.clone();
        if self.preview_held {
            shown.tick(
                &Input { left: self.knobs.0, right: self.knobs.1, ..Input::default() },
                &mut Vec::new(),
            );
        }
        shown.render(&mut self.frame);
        self.canvas.copy_from(&self.frame);
        let overlay = match self.menu.countdown() {
            Some(number) => Overlay::Countdown(Countdown {
                number,
                game: self.console.kind(),
                ball: shown.status().ball.filter(|b| b.visible).map(|b| (b.x, b.row)),
            }),
            None if self.menu.is_open() => Overlay::Menu(MenuView {
                cursor: self.menu.cursor(),
                game: self.console.kind(),
                color: colour,
                crt,
                volume: self.settings.volume,
                muted: self.settings.muted,
                invert_left: self.settings.invert_left,
                invert_right: self.settings.invert_right,
            }),
            None if self.first_launch => Overlay::Launch,
            None => Overlay::None,
        };
        match overlay {
            // The native hints in their place, worded for a touchscreen.
            Overlay::Launch if self.touch_hints => draw_launch(&mut self.canvas, touch::TOUCH_HINTS),
            _ => overlay.draw(&mut self.canvas),
        }
        self.encode_rgba()
    }

    #[cfg(test)]
    fn settings_bits(&self) -> (bool, DisplayMode, u8, bool, bool, GameKind) {
        let s = &self.settings;
        (s.color, s.display, s.volume, s.invert_left, s.invert_right, self.console.kind())
    }

    fn encode_rgba(&mut self) -> *const u8 {
        for (i, cell) in self.canvas.cells().iter().enumerate() {
            let x = DISPLAY_X0 + (i % DISPLAY_W as usize) as i32;
            for j in 0..colour::SUBCOLUMNS {
                let pixel = (i * colour::SUBCOLUMNS + j) * 4;
                self.rgba[pixel..pixel + 3].copy_from_slice(&self.palette.get(*cell, x, j));
                self.rgba[pixel + 3] = 255;
            }
        }
        self.rgba.as_ptr()
    }
}

fn sound_bit(event: &SoundEvent) -> u32 {
    match event {
        SoundEvent::Bounce(_) => 1,
        SoundEvent::Point => 2,
        SoundEvent::Serve => 4,
        SoundEvent::AttractTick => 8,
    }
}

// The splash owns pixels only: no simulation, animation or match state.
struct Splash {
    rgba: Vec<u8>,
}
impl Splash {
    fn new() -> Self {
        let mut canvas = Canvas::new();
        splash_overlay(&mut canvas);
        let palette = colour::Palette::new(true, true);
        let mut rgba = Vec::with_capacity(width() as usize * height() as usize * 4);
        for (i, cell) in canvas.cells().iter().enumerate() {
            let x = DISPLAY_X0 + (i % DISPLAY_W as usize) as i32;
            for j in 0..colour::SUBCOLUMNS {
                rgba.extend_from_slice(&palette.get(*cell, x, j));
                rgba.push(255);
            }
        }
        Self { rgba }
    }
}
thread_local! {
    static GAME: RefCell<WebGame> = RefCell::new(WebGame::new(3));
    static SPLASH: RefCell<Splash> = RefCell::new(Splash::new());
}

const SPLASH_PROMPT: [&str; 2] = ["CONNECT CONTROLLER", "& TURN KNOB"];

fn splash_overlay(canvas: &mut Canvas) {
    use menu::font::{centred, draw_text, glyph, Scale};
    use superpong_core::{Cell, FIELD_W};
    let title = Scale { w: 4, h: 6 };
    canvas.fill(8..FIELD_W - 8, 40..72, Cell::Background);
    draw_text(canvas, "RETROPONG", centred("RETROPONG", FIELD_W / 2, title).start, 40, title);
    canvas.fill(DISPLAY_X0..DISPLAY_X0 + DISPLAY_W, 112..175, Cell::Background);
    let scale = Scale { w: 2, h: 3 };
    for (text, row) in SPLASH_PROMPT.into_iter().zip([120, 152]) {
        // Only ampersand extends the native font; every other glyph is the
        // exact desktop shape, scale and spacing.
        let glyphs: Vec<_> =
            text.chars()
                .filter_map(|c| {
                    if c == '&' {
                        Some([".##..", "#..#.", ".##..", "#..#.", ".##.#"])
                    } else {
                        glyph(c)
                    }
                })
                .collect();
        let width = (glyphs.iter().map(|g| g[0].len() as i32 + 1).sum::<i32>() - 1) * scale.w;
        let mut x = FIELD_W / 2 - width / 2;
        for g in glyphs {
            for (r, line) in g.into_iter().enumerate() {
                for (col, pixel) in line.chars().enumerate() {
                    if pixel == '#' {
                        let (px, py) = (x + col as i32 * scale.w, row + r as i32 * scale.h);
                        canvas.fill(px..px + scale.w, py..py + scale.h, Cell::Foreground);
                    }
                }
            }
            x += (g[0].len() as i32 + 1) * scale.w;
        }
    }
}

/// Return the immutable connection-screen raster, independent of match state.
#[no_mangle]
pub extern "C" fn render_splash(_dt: f32) -> *const u8 {
    SPLASH.with(|state| state.borrow().rgba.as_ptr())
}

// Compact, validated ABI for the browser's versioned settings document.
#[no_mangle]
pub extern "C" fn settings_snapshot() -> u32 {
    GAME.with(|s| {
        let g = s.borrow();
        u32::from(g.console.kind().number())
            | (u32::from(g.settings.volume) << 3)
            | ((g.settings.color as u32) << 7)
            | (((g.settings.display == DisplayMode::Crt) as u32) << 8)
            | ((g.settings.invert_left as u32) << 9)
            | ((g.settings.invert_right as u32) << 10)
    })
}
#[no_mangle]
pub extern "C" fn restore_settings(value: u32) -> u32 {
    let game = value & 7;
    let volume = (value >> 3) & 15;
    if !(1..=4).contains(&game) || volume > 10 || value & !2047 != 0 {
        return 0;
    }
    GAME.with(|s| {
        let mut g = s.borrow_mut();
        let kind = game_kind(game);
        if g.console.kind() != kind {
            let WebGame { scheduler, console, .. } = &mut *g;
            scheduler.select_now(console, kind);
        }
        g.settings.volume = volume as u8;
        g.settings.color = value & 128 != 0;
        g.settings.display = if value & 256 != 0 { DisplayMode::Crt } else { DisplayMode::Pixel };
        g.settings.invert_left = value & 512 != 0;
        g.settings.invert_right = value & 1024 != 0;
    });
    1
}

#[no_mangle]
pub extern "C" fn init(game: u32) {
    // The hints follow the browser's input, not the match.
    GAME.with(|s| {
        let mut g = s.borrow_mut();
        *g = WebGame { touch_hints: g.touch_hints, ..WebGame::new(game) };
    });
}
/// A tap at `(x, y)`, fractions of the canvas from its top left, for the
/// next `advance`. Non-finite positions are ignored.
#[no_mangle]
pub extern "C" fn tap(x: f32, y: f32) {
    GAME.with(|s| {
        let mut g = s.borrow_mut();
        let crt = g.settings.display == DisplayMode::Crt;
        if let Some(at) = touch::raster(x, y, crt) {
            g.tap = Some(at);
        }
    });
}
#[no_mangle]
pub extern "C" fn set_touch_hints(on: u32) {
    GAME.with(|s| s.borrow_mut().touch_hints = on != 0);
}
#[no_mangle]
pub extern "C" fn advance(dt: f32, left: f32, right: f32, commands: u32) -> u32 {
    GAME.with(|s| s.borrow_mut().advance(dt, left, right, commands))
}
#[no_mangle]
pub extern "C" fn render() -> *const u8 {
    GAME.with(|s| s.borrow_mut().render())
}
#[no_mangle]
pub extern "C" fn pause() {
    GAME.with(|s| s.borrow_mut().pause());
}
#[no_mangle]
pub extern "C" fn width() -> u32 {
    DISPLAY_W as u32 * colour::SUBCOLUMNS as u32
}
#[no_mangle]
pub extern "C" fn height() -> u32 {
    DISPLAY_H as u32
}
#[no_mangle]
pub extern "C" fn frame_hz() -> f64 {
    GAME.with(|s| s.borrow().console.frame_hz())
}
#[no_mangle]
pub extern "C" fn crt_enabled() -> u32 {
    GAME.with(|s| (s.borrow().settings.display == DisplayMode::Crt) as u32)
}
#[no_mangle]
pub extern "C" fn volume() -> u32 {
    GAME.with(|s| u32::from(s.borrow().settings.volume))
}
#[no_mangle]
pub extern "C" fn gain() -> f32 {
    GAME.with(|s| s.borrow().settings.gain())
}
#[no_mangle]
pub extern "C" fn menu_open() -> u32 {
    GAME.with(|s| s.borrow().menu.is_open() as u32)
}
#[no_mangle]
pub extern "C" fn runs_console() -> u32 {
    GAME.with(|s| s.borrow().menu.runs_console() as u32)
}
#[no_mangle]
pub extern "C" fn exit_requested() -> u32 {
    GAME.with(|s| s.borrow().exit as u32)
}
#[no_mangle]
pub extern "C" fn phase() -> u32 {
    GAME.with(|s| match s.borrow().console.phase() {
        Phase::Attract => 0,
        Phase::Waiting => 1,
        Phase::Rally => 2,
    })
}
#[no_mangle]
pub extern "C" fn left_top() -> i32 {
    GAME.with(|s| s.borrow().console.status().left_top)
}
#[no_mangle]
pub extern "C" fn right_top() -> i32 {
    GAME.with(|s| s.borrow().console.status().right_top)
}
#[no_mangle]
pub extern "C" fn frame_number() -> f64 {
    GAME.with(|s| s.borrow().console.status().frame as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splash_uses_native_title_and_only_connection_prompt() {
        let mut expected = Canvas::new();
        let mut actual = Canvas::new();
        splash_overlay(&mut actual);
        let scale = menu::font::Scale { w: 4, h: 6 };
        menu::font::draw_text(
            &mut expected,
            "RETROPONG",
            menu::font::centred("RETROPONG", 88, scale).start,
            40,
            scale,
        );
        assert_eq!(actual.to_ascii(0..176, 40..72), expected.to_ascii(0..176, 40..72));
        assert_eq!(SPLASH_PROMPT, ["CONNECT CONTROLLER", "& TURN KNOB"]);
        // No native launch instructions or extra foreground text remains.
        assert!(actual.to_ascii(0..176, 72..112).chars().all(|c| c == '.' || c == '\n'));
        let mut native = Canvas::new();
        Overlay::Launch.draw(&mut native);
        assert_ne!(actual.to_ascii(0..176, 112..175), native.to_ascii(0..176, 112..175));
    }

    #[test]
    fn static_splash_preserves_match_and_menu_state() {
        init(3);
        advance(0.1, 0.1, 0.9, 64);
        pause();
        let before = GAME.with(|s| (s.borrow().console.status(), s.borrow().menu));
        render();
        let before_rgba = GAME.with(|s| s.borrow().rgba.clone());
        render_splash(0.0);
        let first = SPLASH.with(|s| s.borrow().rgba.clone());
        for _ in 0..60 {
            assert!(!render_splash(1.0 / 60.0).is_null());
        }
        assert_eq!(first, SPLASH.with(|s| s.borrow().rgba.clone()));
        assert_eq!(before, GAME.with(|s| (s.borrow().console.status(), s.borrow().menu)));
        render();
        assert_eq!(before_rgba, GAME.with(|s| s.borrow().rgba.clone()));
    }

    #[test]
    fn scheduler_and_core_match_native_over_all_games() {
        for game in GameKind::ALL {
            let mut web = WebGame::new(game.number().into());
            let mut native = Console::new(game);
            let mut scheduler = Scheduler::new();
            let mut events = Vec::new();
            let mut observed = 0;
            for i in 0..2400 {
                let (left, right) = ((i % 211) as f32 / 210., ((i * 7) % 211) as f32 / 210.);
                let start = i == 30 || i == 1700;
                if start {
                    scheduler.press_start();
                }
                let (native_left, native_right) = Settings::default().apply(left, right);
                scheduler.run(&mut native, f64::from(1.0f32 / 60.0), &mut events, |_| Input {
                    left: native_left,
                    right: native_right,
                    ..Input::default()
                });
                let sound = web.advance(1.0 / 60.0, left, right, if start { 64 } else { 0 });
                observed |= sound;
                assert_eq!(sound, events.last().map_or(0, sound_bit));
                assert_eq!(web.console.status(), native.status());
            }
            assert_ne!(observed & 4, 0);
        }
    }

    #[test]
    fn actual_web_menu_is_native_golden_and_native_rgba() {
        let mut web = WebGame::new(3);
        web.pause();
        web.render();
        // The same golden used by the desktop screen tests.
        let mut expected = Canvas::new();
        menu::screen::draw_menu(
            &mut expected,
            &MenuView {
                cursor: menu::Item::Resume,
                game: GameKind::SuperPong,
                color: true,
                crt: true,
                volume: 8,
                muted: false,
                invert_left: true,
                invert_right: true,
            },
        );
        assert_eq!(web.canvas, expected);
        for color in [false, true] {
            for display in [DisplayMode::Crt, DisplayMode::Pixel] {
                web.settings.color = color;
                web.settings.display = display;
                web.render();
                let palette = colour::Palette::new(color, display == DisplayMode::Crt);
                for (i, cell) in web.canvas.cells().iter().enumerate() {
                    for j in 0..colour::SUBCOLUMNS {
                        let p = (i * colour::SUBCOLUMNS + j) * 4;
                        let x = DISPLAY_X0 + (i % DISPLAY_W as usize) as i32;
                        assert_eq!(&web.rgba[p..p + 3], &palette.get(*cell, x, j));
                        assert_eq!(web.rgba[p + 3], 255);
                    }
                }
            }
        }
    }

    #[test]
    fn launch_menu_resume_countdown_never_restarts_game() {
        let mut web = WebGame::new(3);
        assert_eq!(web.console.phase(), Phase::Attract);
        web.pause();
        web.pause();
        assert!(web.menu.is_open());
        web.advance(0.02, 0.5, 0.5, 64);
        assert!(web.menu.runs_console());
        assert!(web.first_launch);
        assert_eq!(web.console.phase(), Phase::Attract);
        web.advance(0.02, 0.5, 0.5, 64);
        assert_eq!(web.console.phase(), Phase::Waiting);
        assert!(!web.first_launch);
        for _ in 0..220 {
            web.advance(1.0 / 60.0, 0.5, 0.5, 0);
        }
        web.advance(0.02, 0.5, 0.5, 1);
        let paused = web.console.status();
        web.advance(0.02, 0.0, 1.0, 64);
        assert_eq!(web.menu.countdown(), Some(3));
        for _ in 0..100 {
            web.advance(1.0 / 60.0, 0.0, 1.0, 64);
        }
        assert_eq!(web.console.status(), paused);
        web.render();
        let mut preview = web.console.clone();
        let (left, right) = Settings::default().apply(0.0, 1.0);
        preview.tick(&Input { left, right, ..Input::default() }, &mut Vec::new());
        let mut frame = Frame::new();
        preview.render(&mut frame);
        assert_eq!(web.frame, frame);
        while !web.menu.runs_console() {
            web.advance(1.0 / 60.0, 0.0, 1.0, 0);
        }
        assert!(web.console.status().frame > paused.frame);
        assert!(!web.first_launch);
    }

    #[test]
    fn native_menu_switch_settings_and_exit_are_exposed() {
        init(3);
        assert_eq!((phase(), volume(), crt_enabled()), (0, 8, 1));
        pause();
        advance(0.0, 0.5, 0.5, 8); // game mode
        advance(0.0, 0.5, 0.5, 32);
        GAME.with(|s| assert_eq!(s.borrow().console.kind(), GameKind::Pong));
        advance(0.0, 0.5, 0.5, 8); // color
        advance(0.0, 0.5, 0.5, 8); // CRT
        advance(0.0, 0.5, 0.5, 64);
        assert_eq!(crt_enabled(), 0);
        advance(0.0, 0.5, 0.5, 8); // volume
        assert_eq!(advance(0.0, 0.5, 0.5, 16), 1);
        assert_eq!(volume(), 7);
        advance(0.0, 0.5, 0.5, 8); // left direction
        advance(0.0, 0.5, 0.5, 64);
        advance(0.0, 0.5, 0.5, 8); // right direction
        advance(0.0, 0.5, 0.5, 8); // exit
        advance(0.0, 0.5, 0.5, 64);
        assert_eq!(exit_requested(), 1);
    }

    /// A tap on the middle of the screen, or on `item`'s line, then a frame.
    fn tap_at(web: &mut WebGame, at: Option<menu::Item>, dt: f32) {
        web.tap = Some(at.map_or((88, 120), |item| (20, menu::screen::item_row(item) + 7)));
        web.advance(dt, 0.5, 0.5, 0);
    }

    #[test]
    fn a_tap_starts_over_the_light_show_and_pauses_a_match_without_restarting_it() {
        use menu::Item;
        let mut web = WebGame::new(3);
        tap_at(&mut web, None, 0.02);
        assert_eq!(web.console.phase(), Phase::Waiting, "a tap starts, as Enter does");
        while web.console.phase() != Phase::Rally {
            web.advance(1.0 / 60.0, 0.5, 0.5, 0);
        }
        let rally = web.console.status();
        tap_at(&mut web, None, 1.0 / 60.0);
        assert!(web.menu.is_open(), "a tap during a match opens the menu");
        assert_eq!(web.console.status(), rally, "and pauses it at once");
        for _ in 0..30 {
            tap_at(&mut web, Some(Item::Color), 1.0 / 60.0);
        }
        assert_eq!(web.menu.cursor(), Item::Color);
        assert!(web.settings.color, "thirty taps toggle back to colour");
        tap_at(&mut web, Some(Item::Resume), 0.0);
        assert_eq!(web.menu.countdown(), Some(3));
        tap_at(&mut web, None, 0.2);
        assert!(web.menu.is_open(), "a tap during the countdown opens the menu again");
        tap_at(&mut web, Some(Item::Resume), 0.0);
        while !web.menu.runs_console() {
            web.advance(1.0 / 60.0, 0.5, 0.5, 0);
        }
        assert_eq!(web.console.status(), rally, "nothing ran during the pause");
        for _ in 0..3 {
            web.advance(1.0 / 60.0, 0.5, 0.5, 0);
        }
        let after = web.console.status();
        assert!(after.frame > rally.frame);
        assert_eq!(
            (after.phase, after.left_score, after.right_score),
            (Phase::Rally, 0, 0),
            "the same rally"
        );
    }

    #[test]
    fn a_tap_on_the_title_or_off_the_picture_changes_nothing() {
        let mut web = WebGame::new(3);
        web.pause();
        let before = (web.menu, web.settings_bits());
        for at in [(88, 25), (88, 215), (-6, -8)] {
            web.tap = Some(at);
            web.advance(0.0, 0.5, 0.5, 0);
            assert_eq!((web.menu, web.settings_bits()), before, "{at:?}");
        }
        // Non-finite positions from the page are not taps at all.
        init(3);
        tap(f32::NAN, 0.5);
        advance(0.02, 0.5, 0.5, 0);
        assert_eq!(phase(), 0);
    }

    #[test]
    fn exported_taps_land_where_the_crt_curve_shows_the_menu() {
        // Near the bottom left corner, the line above Exit Game on the flat
        // picture shows Exit Game itself on the curved one.
        let row = menu::screen::item_row(menu::Item::Exit) - 2;
        let y = (row - superpong_core::DISPLAY_ROW0) as f32 / DISPLAY_H as f32 + 0.001;
        for crt in [false, true] {
            init(3);
            GAME.with(|s| {
                s.borrow_mut().settings.display = if crt { DisplayMode::Crt } else { DisplayMode::Pixel }
            });
            pause();
            tap(0.02, y);
            advance(0.0, 0.5, 0.5, 0);
            let toggled = GAME.with(|s| s.borrow().settings.invert_right) != Settings::default().invert_right;
            assert_eq!((exit_requested() == 1, toggled), (crt, !crt), "exit or RIGHT, crt {crt}");
        }
    }

    #[test]
    fn touch_hints_replace_the_keys_in_the_native_launch_panel() {
        let mut web = WebGame::new(3);
        web.render();
        let keys = web.canvas.clone();
        web.touch_hints = true;
        web.render();
        let mut expected = keys.clone();
        menu::screen::draw_launch(&mut expected, touch::TOUCH_HINTS);
        assert_eq!(web.canvas, expected);
        assert_ne!(web.canvas, keys);
        // A replay powers on again with the same hints.
        set_touch_hints(1);
        init(2);
        GAME.with(|s| assert!(s.borrow().touch_hints));
        set_touch_hints(0);
        init(2);
        GAME.with(|s| assert!(!s.borrow().touch_hints));
    }
}
