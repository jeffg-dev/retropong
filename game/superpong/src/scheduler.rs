//! Tick scheduling (SPEC §13.5): turns rendered-frame time into console
//! ticks, buffers Start/Select presses until a tick can take them, and
//! remembers the last finished game's score. Pure apart from the console it
//! drives, so the whole loop is unit-tested without a window.

use superpong_core::{Console, GameKind, Input, Phase, SoundEvent, Status};

/// Seconds of simulated time each rendered frame adds in `--screenshot`
/// mode, so `--frames N` always reaches the same console state.
pub const HEADLESS_FRAME_DT: f32 = 1.0 / 60.0;
/// Longest wall-clock frame the normal path will catch up on.
pub const MAX_FRAME_DT: f32 = 0.25;

/// Seconds of time a rendered frame adds: a fixed [`HEADLESS_FRAME_DT`] in
/// headless screenshot mode (wall-clock frame times there depend on how
/// fast the machine renders without vsync), otherwise the measured frame
/// time `wall_dt`, capped at [`MAX_FRAME_DT`].
pub fn frame_dt(headless: bool, wall_dt: f32) -> f32 {
    if headless {
        HEADLESS_FRAME_DT
    } else {
        wall_dt.min(MAX_FRAME_DT)
    }
}

/// Owns everything the main loop carries from one rendered frame to the next.
#[derive(Debug, Default)]
pub struct Scheduler {
    /// Seconds of console time owed; each tick costs one frame period of
    /// the current phase.
    accumulator: f64,
    /// Buffered so a Start press is never lost even if a rendered frame
    /// happens to advance zero console ticks.
    pending_start: bool,
    /// Likewise for a Game Select move.
    pending_select: Option<GameKind>,
    /// Final `(left, right)` score of the last game that ended.
    last_final: Option<(u8, u8)>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Buffers a Start press for the first tick of the next run that has one.
    pub fn press_start(&mut self) {
        self.pending_start = true;
    }

    /// Buffers a Game Select move to `kind`, replacing an earlier pending one.
    #[cfg(test)]
    pub fn select(&mut self, kind: GameKind) {
        self.pending_select = Some(kind);
    }

    /// Buffers a move of the Game Select switch to the game after the one it
    /// will be on: the pending selection if one is waiting for a tick,
    /// otherwise `current`, the console's game. Two buffered moves before
    /// a tick therefore move two games on.
    #[cfg(test)]
    pub fn select_next(&mut self, current: GameKind) {
        self.pending_select = Some(self.pending_select.unwrap_or(current).next());
    }

    /// Moves the Game Select switch on `console` at once, for while the
    /// console is paused (the menu, the countdown) and no tick will take a
    /// buffered move. A move still buffered from before the pause is
    /// dropped, so it cannot undo this one on the first tick after it.
    pub fn select_now(&mut self, console: &mut Console, kind: GameKind) {
        self.pending_select = None;
        console.select(kind);
    }

    /// Called on the frame the console is paused (the menu opens). A Start
    /// still buffered is dropped: it was pressed before the pause and would
    /// otherwise restart the game on the first tick after it. A Game Select
    /// move still buffered is made at once, so the menu shows the game the
    /// console is on.
    pub fn pause(&mut self, console: &mut Console) {
        self.pending_start = false;
        if let Some(kind) = self.pending_select {
            self.select_now(console, kind);
        }
    }

    /// Whether a Start press is waiting for a tick.
    #[cfg(test)]
    pub fn pending_start(&self) -> bool {
        self.pending_start
    }

    /// The Game Select move still waiting for a tick, if any.
    #[cfg(test)]
    pub fn pending_select(&self) -> Option<GameKind> {
        self.pending_select
    }

    /// Final score of the most recent game that ended, if one has.
    #[cfg(test)]
    pub fn last_final(&self) -> Option<(u8, u8)> {
        self.last_final
    }

    /// Advances `console` by `dt` seconds of rendered-frame time, one tick at
    /// a time at the frame rate of the phase the console is in *when that tick
    /// starts* (SPEC §13.5; a tick can change the phase, so the rate is
    /// re-read after each). `input` supplies the knobs (and, for the demo, a
    /// Start) for each tick from the console as it then stands. A buffered
    /// Start and Select go into the first tick only, so they stay buffered
    /// while a frame runs no tick at all. `events` is replaced with the
    /// sounds of the whole run, in order. Returns the number of ticks run.
    pub fn run(
        &mut self,
        console: &mut Console,
        dt: f64,
        events: &mut Vec<SoundEvent>,
        mut input: impl FnMut(&Status) -> Input,
    ) -> u32 {
        self.accumulator += dt;
        events.clear();
        let mut ticks = 0;
        loop {
            let tick_dt = 1.0 / console.frame_hz();
            if self.accumulator < tick_dt {
                break;
            }
            let mut tick_input = input(&console.status());
            if ticks == 0 {
                tick_input.start |= std::mem::take(&mut self.pending_start);
                tick_input.select = self.pending_select.take();
            } else {
                tick_input.select = None;
            }
            let was_playing = console.phase() != Phase::Attract;
            console.tick(&tick_input, events);
            self.accumulator -= tick_dt;
            ticks += 1;
            // A game just ended (the 15th point): remember its final score.
            if was_playing && console.phase() == Phase::Attract {
                self.last_final = Some(console.scores());
            }
        }
        ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use superpong_core::{BallState, FRAME_HZ_ATTRACT, FRAME_HZ_PLAY, WIN_SCORE};

    fn rest(_: &Status) -> Input {
        Input::default()
    }

    /// A console in `Waiting`, so ticks run at the play rate.
    fn playing(kind: GameKind) -> Console {
        let mut console = Console::new(kind);
        console.press_start();
        console
    }

    #[test]
    fn headless_frames_advance_fixed_simulated_time() {
        for wall in [0.0, 0.001, 0.5] {
            assert_eq!(frame_dt(true, wall), 1.0 / 60.0);
        }
        assert_eq!(frame_dt(false, 0.01), 0.01);
        assert_eq!(frame_dt(false, 3.0), MAX_FRAME_DT);
    }

    #[test]
    fn headless_ticks_track_rendered_frames() {
        // 600 headless frames are 10 s of simulated time, whatever the
        // machine's real frame times: a fixed number of console ticks.
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = playing(GameKind::Pong);
        let ticks: u32 = (0..600)
            .map(|_| scheduler.run(&mut console, frame_dt(true, 0.0) as f64, &mut events, rest))
            .sum();
        assert_eq!(ticks, 599);
        assert_eq!(console.status().frame, 599);
    }

    #[test]
    fn ticks_run_at_the_rate_of_the_current_phase() {
        let mut events = Vec::new();
        let mut console = Console::new(GameKind::Pong);
        let ticks = Scheduler::new().run(&mut console, 10.0, &mut events, rest);
        assert_eq!(ticks, (10.0 * FRAME_HZ_ATTRACT) as u32);

        let mut console = playing(GameKind::Pong);
        let ticks = Scheduler::new().run(&mut console, 10.0, &mut events, rest);
        assert_eq!(ticks, (10.0 * FRAME_HZ_PLAY) as u32);
    }

    #[test]
    fn the_rate_is_re_read_after_each_tick() {
        // Start while in Attract: the first tick costs an attract period,
        // the rest a (longer) play period. One second of time therefore
        // gives 59 ticks, not the 60 an attract-rate run would.
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = Console::new(GameKind::Pong);
        scheduler.press_start();
        assert_eq!(scheduler.run(&mut console, 1.0, &mut events, rest), 59);
        assert_eq!(console.phase(), Phase::Waiting);
        // The same second without the Start is 60 attract ticks.
        let mut console = Console::new(GameKind::Pong);
        assert_eq!(Scheduler::new().run(&mut console, 1.0, &mut events, rest), 60);
    }

    #[test]
    fn start_and_select_wait_for_a_frame_that_runs_a_tick() {
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = Console::new(GameKind::SuperPong);
        // 1 ms is much less than a tick: nothing runs, nothing is lost.
        scheduler.press_start();
        scheduler.select(GameKind::Catch);
        assert_eq!(scheduler.run(&mut console, 0.001, &mut events, rest), 0);
        assert_eq!(console.phase(), Phase::Attract);
        assert_eq!(console.kind(), GameKind::SuperPong);
        assert_eq!(scheduler.pending_select(), Some(GameKind::Catch));
        // The next frame with a tick takes both, Select first.
        assert_eq!(scheduler.run(&mut console, 0.02, &mut events, rest), 1);
        assert_eq!(console.phase(), Phase::Waiting);
        assert_eq!(console.kind(), GameKind::Catch);
        assert_eq!(scheduler.pending_select(), None);
    }

    #[test]
    fn start_and_select_go_into_the_first_tick_only() {
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = Console::new(GameKind::Pong);
        scheduler.press_start();
        scheduler.select(GameKind::Solitaire);
        let ticks = scheduler.run(&mut console, 0.1, &mut events, rest) as i32;
        assert!(ticks > 3, "{ticks}");
        // Start places the hidden ball at x 112; every tick moves it 1. A
        // Start on every tick would hold it at 113.
        assert_eq!(console.status().ball.map(|b| b.x), Some(112 + ticks));
        assert_eq!(console.kind(), GameKind::Solitaire);
        // Both were consumed: a further run starts nothing.
        let more = scheduler.run(&mut console, 0.1, &mut events, rest) as i32;
        assert_eq!(console.status().ball.map(|b| b.x), Some(112 + ticks + more));
    }

    #[test]
    fn input_is_asked_for_on_every_tick_from_the_console_as_it_stands() {
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = playing(GameKind::Pong);
        let mut frames = Vec::new();
        let ticks = scheduler.run(&mut console, 0.1, &mut events, |status| {
            frames.push(status.frame);
            Input { left: 1.0, ..Input::default() }
        });
        assert_eq!(frames, (0..ticks as u64).collect::<Vec<_>>());
        assert_eq!(console.status().left_top, superpong_core::TOP_MAX);
        // A Start the input source itself asks for is honoured on any tick.
        let mut console = Console::new(GameKind::Pong);
        scheduler.run(&mut console, 0.1, &mut events, |s| Input { start: s.frame == 3, ..Input::default() });
        assert_eq!(console.phase(), Phase::Waiting);
    }

    #[test]
    fn tab_cycles_from_the_pending_selection() {
        let mut scheduler = Scheduler::new();
        // Nothing pending: from the console's game.
        scheduler.select_next(GameKind::SuperPong);
        assert_eq!(scheduler.pending_select(), Some(GameKind::Pong));
        // `3`, then Tab before any tick has applied it: Catch, then Solitaire.
        scheduler.select(GameKind::Catch);
        scheduler.select_next(GameKind::SuperPong);
        assert_eq!(scheduler.pending_select(), Some(GameKind::Solitaire));
        scheduler.select_next(GameKind::SuperPong);
        assert_eq!(scheduler.pending_select(), Some(GameKind::SuperPong));
    }

    #[test]
    fn select_now_moves_the_switch_without_a_tick_and_drops_a_buffered_move() {
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = playing(GameKind::Pong);
        scheduler.select(GameKind::Catch);
        scheduler.select_now(&mut console, GameKind::Solitaire);
        assert_eq!(console.kind(), GameKind::Solitaire);
        assert_eq!(console.status().frame, 0, "no tick ran");
        assert_eq!(scheduler.pending_select(), None);
        scheduler.run(&mut console, 0.1, &mut events, rest);
        assert_eq!(console.kind(), GameKind::Solitaire);
    }

    /// A Pong console one reflection-free frame from the right-hand point
    /// that wins game 15-0... made by playing 14 points first.
    #[test]
    fn pausing_drops_a_buffered_start_and_makes_a_buffered_select() {
        let mut events = Vec::new();
        // Without the pause the buffered Start wipes the score ...
        let mut console = match_point_console();
        let mut scheduler = Scheduler::new();
        scheduler.press_start();
        scheduler.run(&mut console, 0.1, &mut events, rest);
        assert_eq!(console.scores().0, 0);
        // ... with it, the match goes on, and the switch is where the
        // buffered Select put it.
        let mut console = match_point_console();
        scheduler.press_start();
        scheduler.select(GameKind::Pong);
        scheduler.pause(&mut console);
        assert!(!scheduler.pending_start());
        assert_eq!(scheduler.pending_select(), None);
        scheduler.run(&mut console, 0.1, &mut events, rest);
        assert!(console.scores().0 >= WIN_SCORE - 1, "{:?}", console.scores());
        let mut console = playing(GameKind::Pong);
        scheduler.select(GameKind::Catch);
        scheduler.pause(&mut console);
        assert_eq!(console.kind(), GameKind::Catch);
    }

    fn match_point_console() -> Console {
        let mut console = Console::new(GameKind::Pong);
        let mut events = Vec::new();
        console.press_start();
        let off_the_right = BallState { x: 173, row: 100, dirx: 1, vy: 0, visible: true };
        // Park both paddles out of the way and let the ball exit right: the
        // left player scores each time.
        let miss = Input { left: 0.0, right: 0.0, ..Input::default() };
        for _ in 0..WIN_SCORE - 1 {
            console.debug_set_ball(off_the_right, 0);
            for _ in 0..4 {
                console.tick(&miss, &mut events);
            }
        }
        assert_eq!((console.scores(), console.phase()), ((WIN_SCORE - 1, 0), Phase::Waiting));
        console.debug_set_ball(off_the_right, 0);
        console
    }

    #[test]
    fn last_final_is_the_score_of_the_game_that_just_ended() {
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        assert_eq!(scheduler.last_final(), None);
        let mut console = match_point_console();
        // Plenty of time for the winning point and the light show after it.
        scheduler.run(&mut console, 0.2, &mut events, rest);
        assert_eq!(console.phase(), Phase::Attract);
        assert_eq!(scheduler.last_final(), Some((WIN_SCORE, 0)));
        // The light show running on does not overwrite it, and a restart
        // does not clear it.
        scheduler.press_start();
        scheduler.run(&mut console, 1.0, &mut events, rest);
        assert_eq!(console.phase(), Phase::Waiting);
        assert_eq!(scheduler.last_final(), Some((WIN_SCORE, 0)));
    }

    #[test]
    fn last_final_is_kept_when_the_game_ends_and_restarts_in_one_run() {
        // The game ending and a Start landing in the same rendered frame
        // (a long frame, or the demo's own restart) still records it.
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = match_point_console();
        let mut ticks_in_attract = 0;
        scheduler.run(&mut console, 2.0, &mut events, |status| {
            ticks_in_attract += (status.phase == Phase::Attract) as u32;
            Input { start: ticks_in_attract == 5, ..Input::default() }
        });
        assert_eq!(scheduler.last_final(), Some((WIN_SCORE, 0)));
        assert_ne!(console.phase(), Phase::Attract);
    }

    #[test]
    fn a_game_in_progress_leaves_last_final_alone() {
        let (mut scheduler, mut events) = (Scheduler::new(), Vec::new());
        let mut console = playing(GameKind::Pong);
        scheduler.run(&mut console, 5.0, &mut events, rest);
        assert_eq!(scheduler.last_final(), None);
    }
}
