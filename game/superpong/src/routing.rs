//! What one rendered frame does with the keys and with the picture, taken
//! out of the main loop so it is unit-tested without a window (SPEC §13.6):
//! which key presses are menu commands and which are console Start or Game
//! Select, and which console's picture is shown, with which overlay.
//!
//! The rule behind most of it: while the menu is open or counting down the
//! console is paused, and then nothing the player presses may reach it as a
//! Start (it would restart the match).

use macroquad::prelude::KeyCode;
use superpong_core::{Console, Input as CoreInput};

use crate::menu::screen::{Countdown, MenuView, Overlay};
use crate::menu::{Command, Menu};
use crate::settings::Settings;

/// The keys that press the console's Start (and activate in the menu).
pub const ACTIVATE_KEYS: &[KeyCode] = &[KeyCode::Enter, KeyCode::KpEnter];

/// The menu's keys while it is open (SPEC §13.6); `Esc` is handled on its
/// own, since it also opens the menu.
const MENU_KEYS: [(&[KeyCode], Command); 5] = [
    (&[KeyCode::Up], Command::Up),
    (&[KeyCode::Down], Command::Down),
    (&[KeyCode::Left], Command::Left),
    (&[KeyCode::Right], Command::Right),
    (ACTIVATE_KEYS, Command::Activate),
];

/// The menu commands the keys pressed this frame (`pressed`) give. `Esc`
/// always toggles; the others only count while the menu is open
/// (`menu_open` as the frame began), so the arrow keys do not navigate a
/// game and the Enter that resumes is not also a Start.
pub fn menu_commands(menu_open: bool, pressed: impl Fn(KeyCode) -> bool) -> Vec<Command> {
    let mut commands = Vec::new();
    if pressed(KeyCode::Escape) {
        commands.push(Command::Toggle);
    }
    if menu_open {
        for (keys, command) in MENU_KEYS {
            if keys.iter().any(|&key| pressed(key)) {
                commands.push(command);
            }
        }
    }
    commands
}

/// Whether the console gets a Start this frame: the Start keys (or the
/// headless `auto_start`), but never while the console is `paused` (menu
/// open or counting down). The scheduler would keep that Start for the
/// first tick after the pause, and Start restarts the match.
pub fn start_wanted(paused: bool, auto_start: bool, pressed: impl Fn(KeyCode) -> bool) -> bool {
    !paused && (auto_start || ACTIVATE_KEYS.iter().any(|&key| pressed(key)))
}

/// The frame the console's next tick would draw with the knobs at `left`
/// and `right` now, made on a copy that is then thrown away.
pub fn preview(console: &Console, left: f32, right: f32) -> Console {
    let mut next = console.clone();
    next.tick(&CoreInput { left, right, ..CoreInput::default() }, &mut Vec::new());
    next
}

/// Decides, frame by frame, whether the countdown's [`preview`] or the real
/// console is drawn. During the countdown it is the preview. After it the
/// preview stays up until a real console tick has run: a rendered frame can
/// run no tick at all, and the real console is still where it was before
/// the pause, so showing it then would snap the paddles back for a frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct PreviewHold {
    held: bool,
}

impl PreviewHold {
    /// Whether to draw the preview this frame. `counting_down` and
    /// `menu_open` are the menu's state after this frame's commands, and
    /// `ticks` the number of console ticks run this frame.
    pub fn show_preview(&mut self, counting_down: bool, menu_open: bool, ticks: u32) -> bool {
        if counting_down {
            self.held = true;
        } else if menu_open || ticks > 0 {
            self.held = false;
        }
        self.held
    }
}

/// What is drawn over the picture `shown`: the countdown (with the ball of
/// `shown`, so the digit cannot hide it), the menu, or nothing. `console` is
/// the real console, whose game the menu and the countdown size by.
/// `crt_shown` is whether classic video is on screen, not just wanted.
pub fn overlay(
    menu: &Menu,
    console: &Console,
    shown: &Console,
    settings: &Settings,
    crt_shown: bool,
) -> Overlay {
    match menu.countdown() {
        Some(number) => {
            let ball = shown.status().ball.filter(|b| b.visible).map(|b| (b.x, b.row));
            Overlay::Countdown(Countdown { number, game: console.kind(), ball })
        }
        None if menu.is_open() => Overlay::Menu(MenuView {
            cursor: menu.cursor(),
            game: console.kind(),
            color: settings.color,
            crt: crt_shown,
            volume: settings.volume,
            muted: settings.muted,
            invert_left: settings.invert_left,
            invert_right: settings.invert_right,
        }),
        None => Overlay::None,
    }
}

/// Startup hints survive menu visits, but disappear permanently after the first start.
pub fn launch_overlay(first_launch: &mut bool, phase: superpong_core::Phase, overlay: Overlay) -> Overlay {
    if phase != superpong_core::Phase::Attract {
        *first_launch = false;
    }
    if *first_launch && overlay == Overlay::None {
        Overlay::Launch
    } else {
        overlay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::Item;
    use crate::scheduler::Scheduler;
    use superpong_core::GameKind;
    use superpong_core::{Phase, WIN_SCORE};

    #[test]
    fn launch_hints_survive_menu_visits_but_never_return_after_start() {
        let mut first = true;
        assert_eq!(launch_overlay(&mut first, Phase::Attract, Overlay::None), Overlay::Launch);
        let menu = Overlay::Menu(MenuView {
            cursor: Item::Resume,
            game: GameKind::Pong,
            color: true,
            crt: true,
            muted: false,
            volume: 8,
            invert_left: false,
            invert_right: false,
        });
        assert_eq!(launch_overlay(&mut first, Phase::Attract, menu), menu);
        assert_eq!(launch_overlay(&mut first, Phase::Attract, Overlay::None), Overlay::Launch);
        assert_eq!(launch_overlay(&mut first, Phase::Waiting, Overlay::None), Overlay::None);
        assert_eq!(launch_overlay(&mut first, Phase::Attract, Overlay::None), Overlay::None);
    }

    #[test]
    fn removed_shortcuts_are_ignored_by_keyboard_routing() {
        let removed = [
            KeyCode::Space,
            KeyCode::W,
            KeyCode::A,
            KeyCode::S,
            KeyCode::D,
            KeyCode::Key1,
            KeyCode::Key2,
            KeyCode::Key3,
            KeyCode::Key4,
            KeyCode::Tab,
            KeyCode::M,
            KeyCode::C,
            KeyCode::V,
            KeyCode::H,
            KeyCode::F,
            KeyCode::I,
            KeyCode::F12,
        ];
        assert!(menu_commands(true, keys(&removed)).is_empty());
        assert!(menu_commands(false, keys(&removed)).is_empty());
        assert!(!start_wanted(false, false, keys(&removed)));
    }

    /// A `pressed` that reports exactly these keys.
    fn keys(down: &[KeyCode]) -> impl Fn(KeyCode) -> bool + '_ {
        move |key| down.contains(&key)
    }

    fn in_game(kind: GameKind) -> Console {
        let mut console = Console::new(kind);
        console.press_start();
        console
    }

    #[test]
    fn esc_always_toggles_and_the_rest_only_navigate_an_open_menu() {
        assert_eq!(menu_commands(false, keys(&[KeyCode::Escape])), [Command::Toggle]);
        assert_eq!(menu_commands(true, keys(&[KeyCode::Escape])), [Command::Toggle]);
        let all = &[KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right, KeyCode::KpEnter];
        assert_eq!(
            menu_commands(true, keys(all)),
            [Command::Up, Command::Down, Command::Left, Command::Right, Command::Activate]
        );
        assert_eq!(menu_commands(false, keys(all)), []);
        assert_eq!(menu_commands(true, keys(&[KeyCode::Space])), []);
        assert_eq!(menu_commands(true, keys(&[KeyCode::Q])), []);
    }

    #[test]
    fn start_keys_start_only_a_running_console() {
        for key in ACTIVATE_KEYS {
            let down = std::slice::from_ref(key);
            assert!(start_wanted(false, false, keys(down)), "{key:?}");
            assert!(!start_wanted(true, false, keys(down)), "{key:?} while paused");
        }
        assert!(!start_wanted(false, false, keys(&[KeyCode::A])));
        assert!(start_wanted(false, true, keys(&[])), "the headless Start");
        assert!(!start_wanted(true, true, keys(&[])), "not even that while paused");
    }

    /// A frame of the main loop's Start handling, on a console held at
    /// match point: returns the left score after `frames` more frames.
    fn left_score_after(menu: &mut Menu, frames: &[&[KeyCode]]) -> u8 {
        let mut settings = Settings::default();
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = in_game(GameKind::Pong);
        let off_the_right = superpong_core::BallState { x: 173, row: 100, dirx: 1, vy: 0, visible: true };
        let miss = CoreInput { left: 0.0, right: 0.0, ..CoreInput::default() };
        for _ in 0..WIN_SCORE - 1 {
            console.debug_set_ball(off_the_right, 0);
            for _ in 0..4 {
                console.tick(&miss, &mut events);
            }
        }
        assert_eq!(console.scores(), (WIN_SCORE - 1, 0));
        for &down in frames {
            let paused = !menu.runs_console();
            let menu_open = menu.is_open();
            if start_wanted(paused, false, keys(down)) {
                scheduler.press_start();
            }
            for command in menu_commands(menu_open, keys(down)) {
                menu.command(command, &mut settings, &console.status());
            }
            if !paused && !menu.runs_console() {
                scheduler.pause(&mut console);
            }
            menu.advance(1.0 / 60.0);
            if menu.runs_console() {
                scheduler.run(&mut console, 1.0 / 60.0, &mut events, |_| CoreInput::default());
            }
        }
        console.scores().0
    }

    #[test]
    fn enter_during_the_countdown_does_not_restart_the_match() {
        // Esc, Enter on RESUME, then an impatient Enter and Space during the
        // countdown, then the countdown runs out and the game goes on.
        let mut menu = Menu::new();
        let mut frames: Vec<&[KeyCode]> =
            vec![&[KeyCode::Escape], &[KeyCode::Enter], &[KeyCode::Space], &[KeyCode::Enter]];
        frames.extend(std::iter::repeat_n(&[][..], 300));
        let score = left_score_after(&mut menu, &frames);
        assert!(menu.runs_console(), "the countdown is over");
        assert!(score >= WIN_SCORE - 1, "the score is kept: {score}");
    }

    #[test]
    fn a_start_pressed_with_esc_does_not_survive_the_pause() {
        let mut menu = Menu::new();
        let mut frames: Vec<&[KeyCode]> = vec![&[KeyCode::Escape, KeyCode::Enter], &[KeyCode::Escape]];
        frames.extend(std::iter::repeat_n(&[][..], 200));
        assert!(left_score_after(&mut menu, &frames) >= WIN_SCORE - 1);
    }

    #[test]
    fn a_start_while_running_still_restarts() {
        // The same console without the menu: Enter is a Start.
        let mut menu = Menu::new();
        let frames: Vec<&[KeyCode]> = vec![&[KeyCode::Enter], &[], &[]];
        assert_eq!(left_score_after(&mut menu, &frames), 0);
    }

    #[test]
    fn the_preview_is_the_next_tick_and_leaves_the_console_alone() {
        let console = in_game(GameKind::Pong);
        let before = console.status();
        let shown = preview(&console, 0.9, 0.1);
        assert_eq!(console.status(), before);
        let mut real = console.clone();
        real.tick(&CoreInput { left: 0.9, right: 0.1, ..CoreInput::default() }, &mut Vec::new());
        assert_eq!(shown.status(), real.status());
        assert_ne!((shown.status().left_top, shown.status().right_top), (before.left_top, before.right_top));
    }

    #[test]
    fn the_preview_stays_up_until_a_real_tick_has_run() {
        let mut hold = PreviewHold::default();
        assert!(!hold.show_preview(false, false, 3), "an ordinary frame");
        for _ in 0..180 {
            assert!(hold.show_preview(true, false, 0), "counting down");
        }
        // The countdown has just ended and this frame ran no tick: still the
        // preview, so the paddles do not jump back for a frame.
        assert!(hold.show_preview(false, false, 0));
        assert!(hold.show_preview(false, false, 0));
        // The first real tick draws the same frame, and the preview is done.
        assert!(!hold.show_preview(false, false, 1));
        assert!(!hold.show_preview(false, false, 0), "it does not come back");
    }

    #[test]
    fn opening_the_menu_ends_the_hold() {
        let mut hold = PreviewHold::default();
        assert!(hold.show_preview(true, false, 0));
        assert!(!hold.show_preview(false, true, 0), "Esc during the countdown");
        assert!(!hold.show_preview(false, false, 0), "and a close without a countdown");
    }

    #[test]
    fn the_overlay_follows_the_menu() {
        let settings = Settings::default();
        let console = in_game(GameKind::Pong);
        let mut menu = Menu::new();
        assert_eq!(overlay(&menu, &console, &console, &settings, false), Overlay::None);
        menu.open_at(Item::Volume);
        let Overlay::Menu(view) = overlay(&menu, &console, &console, &settings, false) else {
            panic!("the menu is open")
        };
        assert_eq!((view.cursor, view.game, view.crt), (Item::Volume, GameKind::Pong, false));
        // The CRT row says what is on screen.
        let Overlay::Menu(view) = overlay(&menu, &console, &console, &settings, true) else { panic!() };
        assert!(view.crt);
        menu.command(Command::Toggle, &mut Settings::default(), &console.status());
        assert!(matches!(
            overlay(&menu, &console, &console, &settings, false),
            Overlay::Countdown(Countdown { number: 3, game: GameKind::Pong, ball: None })
        ));
    }

    #[test]
    fn the_countdown_overlay_carries_the_ball_of_the_shown_picture() {
        let mut console = in_game(GameKind::Solitaire);
        while console.phase() != Phase::Rally {
            console.tick(&CoreInput::default(), &mut Vec::new());
        }
        let shown = preview(&console, 0.5, 0.5);
        let ball = shown.status().ball.expect("a rally has a ball");
        let mut menu = Menu::new();
        menu.command(Command::Toggle, &mut Settings::default(), &console.status());
        menu.command(Command::Toggle, &mut Settings::default(), &console.status());
        let Overlay::Countdown(countdown) = overlay(&menu, &console, &shown, &Settings::default(), false)
        else {
            panic!("counting down")
        };
        assert_eq!(countdown.ball, Some((ball.x, ball.row)));
        assert_eq!(countdown.game, GameKind::Solitaire);
        // A hidden ball is not drawn.
        let waiting = in_game(GameKind::Solitaire);
        let Overlay::Countdown(countdown) = overlay(&menu, &waiting, &waiting, &Settings::default(), false)
        else {
            panic!("counting down")
        };
        assert_eq!(countdown.ball, None);
    }
}
