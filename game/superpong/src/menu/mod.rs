//! The RetroPong pause menu and the resume countdown (SPEC §13.6). Not part
//! of the original console, which had no pause.
//!
//! [`Menu`] is a pure state machine: [`Menu::command`] takes one navigation
//! command and changes the settings it owns, [`Menu::advance`] runs the
//! countdown on rendered-frame time, and [`Menu::runs_console`] tells the
//! main loop whether the console may tick. What the menu cannot change
//! itself (the Game Select switch on the console, quitting, a feedback
//! beep) it returns as an [`Action`]. Drawing lives in [`screen`].

pub mod font;
pub mod screen;

use superpong_core::{GameKind, Phase, Status};

use crate::audio::VOLUME_MAX;
use crate::settings::Settings;

/// Seconds each step of the 3, 2, 1, GO countdown is shown.
pub const COUNT_STEP_S: f32 = 0.5;
/// The number the countdown starts from.
pub const COUNT_FROM: u8 = 3;

/// The menu's entries, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Item {
    Resume,
    GameMode,
    Color,
    Crt,
    Volume,
    LeftDirection,
    RightDirection,
    Exit,
}

impl Item {
    /// Every entry in screen order.
    pub const ALL: [Item; 8] = [
        Item::Resume,
        Item::GameMode,
        Item::Color,
        Item::Crt,
        Item::Volume,
        Item::LeftDirection,
        Item::RightDirection,
        Item::Exit,
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// The entry `step` places down (negative: up), wrapping at either end.
    fn moved(self, step: i32) -> Item {
        let n = Item::ALL.len() as i32;
        Item::ALL[(self.index() as i32 + step).rem_euclid(n) as usize]
    }

    /// The entry's label, in the menu font's characters.
    pub fn label(self) -> &'static str {
        match self {
            Item::Resume => "RESUME",
            Item::GameMode => "GAME MODE",
            Item::Color => "COLOR",
            Item::Crt => "CRT",
            Item::Volume => "VOLUME",
            Item::LeftDirection => "LEFT",
            Item::RightDirection => "RIGHT",
            Item::Exit => "EXIT GAME",
        }
    }
}

/// One navigation input, from whichever key or button gave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// `Esc` or a gamepad's Start: opens the menu, closes it, and during
    /// the countdown opens it again.
    Toggle,
    /// A gamepad's East button: closes the menu if it is open.
    Back,
    Up,
    Down,
    Left,
    Right,
    /// `Enter` or a gamepad's South button.
    Activate,
}

/// What the menu asks of the main loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Move the console's Game Select switch to this game now.
    Select(GameKind),
    /// The volume changed: play a beep at the new level.
    Volume,
    /// Quit the program.
    Exit,
}

/// Where the menu is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// Not shown; the console runs.
    Closed,
    /// Shown over the blanked playfield; the console is paused.
    Open,
    /// Closed again, and counting down `elapsed` seconds into the 3, 2, 1, GO
    /// before the console runs again; `shown` is the number already
    /// announced (beeped), u8::MAX before the first.
    Countdown { elapsed: f32, shown: u8 },
}

/// The menu: its state and the entry under the cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Menu {
    state: State,
    cursor: Item,
}

impl Default for Menu {
    fn default() -> Self {
        Self::new()
    }
}

impl Menu {
    /// A closed menu.
    pub fn new() -> Self {
        Self { state: State::Closed, cursor: Item::Resume }
    }

    #[cfg(test)]
    pub fn state(&self) -> State {
        self.state
    }

    /// The entry under the cursor.
    pub fn cursor(&self) -> Item {
        self.cursor
    }

    /// Whether the menu is on screen.
    pub fn is_open(&self) -> bool {
        self.state == State::Open
    }

    /// Whether the console may tick: only with the menu closed and no
    /// countdown running. Everything else is a pause.
    pub fn runs_console(&self) -> bool {
        self.state == State::Closed
    }

    /// Opens the menu with the cursor on `item` (headless `--menu-item`).
    pub fn open_at(&mut self, item: Item) {
        self.state = State::Open;
        self.cursor = item;
    }

    /// The countdown number on screen, if one is running.
    pub fn countdown(&self) -> Option<u8> {
        match self.state {
            State::Countdown { elapsed, .. } => Some(count_at(elapsed)),
            _ => None,
        }
    }

    /// Applies one command. `settings` is changed in place (colour, display,
    /// volume); `status` is the console as it stands, for the selected game
    /// and for whether closing needs a countdown (a game in progress:
    /// `Waiting` or `Rally`).
    pub fn command(&mut self, command: Command, settings: &mut Settings, status: &Status) -> Action {
        match (self.state, command) {
            (State::Closed, Command::Toggle) | (State::Countdown { .. }, Command::Toggle) => {
                self.open_at(Item::Resume);
                Action::None
            }
            (State::Open, Command::Toggle | Command::Back) => {
                self.close(status);
                Action::None
            }
            (State::Open, _) => self.navigate(command, settings, status),
            _ => Action::None,
        }
    }

    /// Advances the countdown by `dt` seconds of rendered-frame time and
    /// returns the number that has just come up, if one has, so the caller
    /// can beep. Once all four steps have run the menu is closed.
    pub fn advance(&mut self, dt: f32) -> Option<u8> {
        let State::Countdown { elapsed, shown } = self.state else { return None };
        let elapsed = elapsed + dt;
        if elapsed >= COUNT_STEP_S * f32::from(COUNT_FROM + 1) {
            self.state = State::Closed;
            return None;
        }
        let now = count_at(elapsed);
        self.state = State::Countdown { elapsed, shown: now };
        (now != shown).then_some(now)
    }

    fn close(&mut self, status: &Status) {
        let in_game = matches!(status.phase, Phase::Waiting | Phase::Rally);
        self.state = if in_game { State::Countdown { elapsed: 0.0, shown: u8::MAX } } else { State::Closed };
    }

    fn navigate(&mut self, command: Command, settings: &mut Settings, status: &Status) -> Action {
        match command {
            Command::Up => self.cursor = self.cursor.moved(-1),
            Command::Down => self.cursor = self.cursor.moved(1),
            Command::Left | Command::Right | Command::Activate => {
                return self.change(command, settings, status);
            }
            Command::Toggle | Command::Back => {}
        }
        Action::None
    }

    /// Left, Right or Activate on the entry under the cursor.
    fn change(&mut self, command: Command, settings: &mut Settings, status: &Status) -> Action {
        let step = match command {
            Command::Left => -1,
            Command::Right => 1,
            _ => 0,
        };
        match self.cursor {
            Item::Resume => {
                if command == Command::Activate {
                    self.close(status);
                }
                Action::None
            }
            Item::GameMode => {
                // Left/Right slide the switch and stop at its ends, like the
                // real one; Activate steps on and wraps 4 -> 1.
                let n = status.kind.number();
                let to = if step == 0 { status.kind.next() } else { slide(n, step) };
                if to == status.kind {
                    Action::None
                } else {
                    Action::Select(to)
                }
            }
            Item::Color => {
                settings.color = !settings.color;
                Action::None
            }
            Item::Crt => {
                settings.display = settings.display.toggled();
                Action::None
            }
            Item::Volume => {
                let level = (i32::from(settings.volume) + step).clamp(0, i32::from(VOLUME_MAX)) as u8;
                if step == 0 || (level == settings.volume && !settings.muted) {
                    return Action::None;
                }
                // Turning the volume also unmutes.
                settings.volume = level;
                settings.muted = false;
                Action::Volume
            }
            Item::LeftDirection => {
                settings.invert_left = !settings.invert_left;
                Action::None
            }
            Item::RightDirection => {
                settings.invert_right = !settings.invert_right;
                Action::None
            }
            Item::Exit => {
                if command == Command::Activate {
                    Action::Exit
                } else {
                    Action::None
                }
            }
        }
    }
}

/// The game `step` switch positions from `number`, stopping at 1 and 4.
fn slide(number: u8, step: i32) -> GameKind {
    let to = (i32::from(number) + step).clamp(1, GameKind::ALL.len() as i32);
    GameKind::from_number(to as u8).expect("1..=4 is a switch position")
}

/// The countdown number `elapsed` seconds in: 3, then 2, then 1, then 0 (GO).
fn count_at(elapsed: f32) -> u8 {
    let step = (elapsed / COUNT_STEP_S).floor().max(0.0) as u8;
    COUNT_FROM.saturating_sub(step)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::DEFAULT_VOLUME;
    use crate::scheduler::Scheduler;
    use crate::settings::DisplayMode;
    use superpong_core::{Console, Input};

    /// The light show (no countdown on resume) and a game in progress.
    fn attract(kind: GameKind) -> Status {
        Console::new(kind).status()
    }

    fn in_game(kind: GameKind) -> Status {
        let mut console = Console::new(kind);
        console.press_start();
        console.status()
    }

    fn open() -> Menu {
        let mut menu = Menu::new();
        menu.command(Command::Toggle, &mut Settings::default(), &attract(GameKind::Pong));
        menu
    }

    /// Runs `commands` on `menu` and returns the last action.
    fn run(menu: &mut Menu, settings: &mut Settings, status: &Status, commands: &[Command]) -> Action {
        commands.iter().fold(Action::None, |_, &c| menu.command(c, settings, status))
    }

    #[test]
    fn esc_opens_the_menu_on_resume_and_closes_it() {
        let mut menu = Menu::new();
        assert!(menu.runs_console() && !menu.is_open());
        let status = attract(GameKind::Pong);
        menu.command(Command::Toggle, &mut Settings::default(), &status);
        assert!(menu.is_open() && !menu.runs_console());
        assert_eq!(menu.cursor(), Item::Resume);
        // Over the light show there is nothing to count down to.
        menu.command(Command::Toggle, &mut Settings::default(), &status);
        assert_eq!(menu.state(), State::Closed);
        assert!(menu.runs_console());
    }

    #[test]
    fn back_closes_but_never_opens() {
        let mut menu = Menu::new();
        let status = attract(GameKind::Pong);
        menu.command(Command::Back, &mut Settings::default(), &status);
        assert_eq!(menu.state(), State::Closed);
        let mut menu = open();
        menu.command(Command::Back, &mut Settings::default(), &status);
        assert_eq!(menu.state(), State::Closed);
    }

    #[test]
    fn navigation_is_ignored_while_closed() {
        let (mut menu, mut settings) = (Menu::new(), Settings::default());
        let before = (menu, settings.color, settings.display, settings.volume);
        let status = in_game(GameKind::Pong);
        for c in [Command::Up, Command::Down, Command::Left, Command::Right, Command::Activate] {
            assert_eq!(menu.command(c, &mut settings, &status), Action::None);
        }
        assert_eq!((menu, settings.color, settings.display, settings.volume), before);
    }

    #[test]
    fn up_and_down_walk_the_entries_and_wrap() {
        let (mut menu, mut settings) = (open(), Settings::default());
        let status = attract(GameKind::Pong);
        let mut seen = vec![menu.cursor()];
        for _ in 0..Item::ALL.len() {
            menu.command(Command::Down, &mut settings, &status);
            seen.push(menu.cursor());
        }
        let mut want = Item::ALL.to_vec();
        want.push(Item::Resume);
        assert_eq!(seen, want);
        menu.command(Command::Up, &mut settings, &status);
        assert_eq!(menu.cursor(), Item::Exit, "up from the top wraps to the bottom");
        menu.command(Command::Up, &mut settings, &status);
        assert_eq!(menu.cursor(), Item::RightDirection);
    }

    #[test]
    fn the_cursor_is_back_on_resume_each_time_the_menu_opens() {
        let (mut menu, mut settings) = (open(), Settings::default());
        let status = attract(GameKind::Pong);
        run(
            &mut menu,
            &mut settings,
            &status,
            &[Command::Down, Command::Down, Command::Toggle, Command::Toggle],
        );
        assert_eq!((menu.state(), menu.cursor()), (State::Open, Item::Resume));
    }

    #[test]
    fn game_mode_slides_between_1_and_4_and_stops_at_the_ends() {
        let (mut menu, mut settings) = (open(), Settings::default());
        menu.command(Command::Down, &mut settings, &attract(GameKind::Pong));
        let mode = |menu: &mut Menu, kind, c| menu.command(c, &mut Settings::default(), &attract(kind));
        assert_eq!(mode(&mut menu, GameKind::SuperPong, Command::Left), Action::Select(GameKind::Solitaire));
        assert_eq!(mode(&mut menu, GameKind::SuperPong, Command::Right), Action::Select(GameKind::Pong));
        assert_eq!(mode(&mut menu, GameKind::Catch, Command::Left), Action::None, "stops at 1");
        assert_eq!(mode(&mut menu, GameKind::Pong, Command::Right), Action::None, "stops at 4");
        // Activate steps on and wraps, for a pad with one button.
        assert_eq!(mode(&mut menu, GameKind::Pong, Command::Activate), Action::Select(GameKind::Catch));
        assert_eq!(mode(&mut menu, GameKind::Catch, Command::Activate), Action::Select(GameKind::Solitaire));
        // The menu itself stays open and changes no setting.
        assert!(menu.is_open());
        assert_eq!(settings.volume, DEFAULT_VOLUME);
    }

    #[test]
    fn color_and_crt_toggle_their_settings_from_any_change_key() {
        let (mut menu, mut settings) = (open(), Settings::default());
        let status = attract(GameKind::Pong);
        run(&mut menu, &mut settings, &status, &[Command::Down, Command::Down]);
        assert_eq!(menu.cursor(), Item::Color);
        for (c, colour) in [(Command::Activate, false), (Command::Left, true), (Command::Right, false)] {
            assert_eq!(menu.command(c, &mut settings, &status), Action::None);
            assert_eq!(settings.color, colour, "{c:?}");
        }
        menu.command(Command::Down, &mut settings, &status);
        assert_eq!(menu.cursor(), Item::Crt);
        for (c, display) in [(Command::Right, DisplayMode::Pixel), (Command::Activate, DisplayMode::Crt)] {
            menu.command(c, &mut settings, &status);
            assert_eq!(settings.display, display, "{c:?}");
        }
        assert!(!settings.color, "CRT leaves colour alone");
    }

    #[test]
    fn directions_toggle_independently_from_any_change_key() {
        let (mut menu, mut settings) = (open(), Settings::default());
        let status = attract(GameKind::Pong);
        for item in [Item::LeftDirection, Item::RightDirection] {
            menu.open_at(item);
            for command in [Command::Activate, Command::Left, Command::Right] {
                let before = settings.apply(0.2, 0.7);
                assert_eq!(menu.command(command, &mut settings, &status), Action::None);
                let after = settings.apply(0.2, 0.7);
                if item == Item::LeftDirection {
                    assert!((after.0 - (1.0 - before.0)).abs() < 1e-6);
                    assert_eq!(after.1, before.1);
                } else {
                    assert_eq!(after.0, before.0);
                    assert!((after.1 - (1.0 - before.1)).abs() < 1e-6);
                }
            }
        }
        assert!(!settings.invert_left && !settings.invert_right);
    }

    #[test]
    fn volume_steps_within_0_to_10_and_beeps_only_on_a_change() {
        let (mut menu, mut settings) = (open(), Settings::default());
        let status = attract(GameKind::Pong);
        menu.open_at(Item::Volume);
        let start = settings.volume;
        assert_eq!(menu.command(Command::Right, &mut settings, &status), Action::Volume);
        assert_eq!(settings.volume, start + 1);
        for _ in 0..20 {
            menu.command(Command::Right, &mut settings, &status);
        }
        assert_eq!(settings.volume, VOLUME_MAX);
        assert_eq!(menu.command(Command::Right, &mut settings, &status), Action::None, "already at the top");
        for _ in 0..20 {
            menu.command(Command::Left, &mut settings, &status);
        }
        assert_eq!(settings.volume, 0);
        assert_eq!(menu.command(Command::Left, &mut settings, &status), Action::None, "already silent");
        assert_eq!(menu.command(Command::Activate, &mut settings, &status), Action::None);
    }

    #[test]
    fn turning_the_volume_unmutes() {
        let (mut menu, mut settings) = (open(), Settings { muted: true, ..Settings::default() });
        let status = attract(GameKind::Pong);
        menu.open_at(Item::Volume);
        settings.volume = VOLUME_MAX;
        // Even at the stop: the press is heard, so it is not lost.
        assert_eq!(menu.command(Command::Right, &mut settings, &status), Action::Volume);
        assert!(!settings.muted);
        assert_eq!(settings.volume, VOLUME_MAX);
    }

    #[test]
    fn exit_quits_only_on_activate() {
        let (mut menu, mut settings) = (open(), Settings::default());
        let status = attract(GameKind::Pong);
        menu.command(Command::Up, &mut settings, &status);
        assert_eq!(menu.cursor(), Item::Exit);
        assert_eq!(menu.command(Command::Left, &mut settings, &status), Action::None);
        assert_eq!(menu.command(Command::Right, &mut settings, &status), Action::None);
        assert_eq!(menu.command(Command::Activate, &mut settings, &status), Action::Exit);
    }

    #[test]
    fn resume_closes_like_esc() {
        let (mut menu, mut settings) = (open(), Settings::default());
        assert_eq!(menu.command(Command::Right, &mut settings, &attract(GameKind::Pong)), Action::None);
        assert!(menu.is_open(), "only Activate resumes");
        menu.command(Command::Activate, &mut settings, &attract(GameKind::Pong));
        assert_eq!(menu.state(), State::Closed);
        let mut menu = open();
        menu.command(Command::Activate, &mut settings, &in_game(GameKind::Pong));
        assert_eq!(menu.countdown(), Some(3));
    }

    #[test]
    fn closing_during_a_game_counts_down_3_2_1() {
        let (mut menu, mut settings) = (open(), Settings::default());
        menu.command(Command::Toggle, &mut settings, &in_game(GameKind::Pong));
        assert_eq!(menu.countdown(), Some(3));
        assert!(!menu.runs_console() && !menu.is_open());
        // Each step lasts half a second; the console resumes after GO at two seconds.
        let mut announced = Vec::new();
        let mut frames = 0;
        while !menu.runs_console() {
            frames += 1;
            if let Some(n) = menu.advance(1.0 / 60.0) {
                announced.push((frames, n));
            }
        }
        // (Summing 1/60 in f32 may land a frame either side of a boundary.)
        let numbers: Vec<u8> = announced.iter().map(|&(_, n)| n).collect();
        assert_eq!(numbers, [3, 2, 1, 0]);
        assert_eq!(announced[0].0, 1, "3 comes up on the first frame");
        assert!((29..=31).contains(&announced[1].0), "{announced:?}");
        assert!((59..=61).contains(&announced[2].0), "{announced:?}");
        assert!((119..=121).contains(&frames), "{frames} frames");
        assert_eq!(menu.state(), State::Closed);
        assert_eq!(menu.countdown(), None);
    }

    #[test]
    fn a_long_frame_can_reach_go_then_finish() {
        let mut menu = open();
        menu.command(Command::Toggle, &mut Settings::default(), &in_game(GameKind::Pong));
        assert_eq!(menu.advance(1.5), Some(0));
        assert_eq!(menu.advance(0.4), None);
        assert_eq!(menu.countdown(), Some(0));
        assert_eq!(menu.advance(0.2), None);
        assert!(menu.runs_console());
    }

    #[test]
    fn esc_during_the_countdown_reopens_the_menu() {
        let mut menu = open();
        let status = in_game(GameKind::Pong);
        menu.command(Command::Toggle, &mut Settings::default(), &status);
        menu.advance(0.75);
        assert_eq!(menu.countdown(), Some(2));
        // Navigation and Back do nothing to a countdown...
        for c in [Command::Down, Command::Activate, Command::Back] {
            menu.command(c, &mut Settings::default(), &status);
            assert_eq!(menu.countdown(), Some(2), "{c:?}");
        }
        // ... Esc opens the menu, and closing it starts again from 3.
        menu.command(Command::Toggle, &mut Settings::default(), &status);
        assert!(menu.is_open());
        assert_eq!(menu.advance(5.0), None, "the open menu does not count");
        menu.command(Command::Toggle, &mut Settings::default(), &status);
        assert_eq!(menu.advance(0.0), Some(3));
    }

    #[test]
    fn the_console_does_not_tick_while_paused_or_counting_down() {
        // The main loop's rule: run the scheduler only when the menu says so.
        let (mut menu, mut settings) = (Menu::new(), Settings::default());
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = Console::new(GameKind::Pong);
        console.press_start();
        let dt = 1.0 / 60.0;
        let mut frame = |menu: &mut Menu, console: &mut Console| {
            menu.advance(dt);
            if menu.runs_console() {
                scheduler.run(console, f64::from(dt), &mut events, |_| Input::default());
            }
        };
        for _ in 0..30 {
            frame(&mut menu, &mut console);
        }
        let paused_at = console.status();
        assert!(paused_at.frame > 0);
        menu.command(Command::Toggle, &mut settings, &console.status());
        for _ in 0..600 {
            frame(&mut menu, &mut console);
        }
        assert_eq!(console.status(), paused_at, "ten seconds in the menu");
        menu.command(Command::Toggle, &mut settings, &console.status());
        let mut counted = 0;
        while menu.countdown().is_some() {
            frame(&mut menu, &mut console);
            counted += 1;
            if menu.countdown().is_some() {
                assert_eq!(console.status(), paused_at, "frame {counted} of the countdown");
            }
        }
        for _ in 0..30 {
            frame(&mut menu, &mut console);
        }
        assert!(console.status().frame > paused_at.frame + 25, "runs again after the countdown");
    }
}
