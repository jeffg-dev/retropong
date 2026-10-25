//! RetroPong (crate `superpong`): the macroquad frontend for the Atari
//! Super Pong (C-140) clone. All game rules live in `superpong-core`; this
//! crate only draws, plays sound, reads input devices, runs the pause menu,
//! and ticks the core at the console's frame rate. See
//! `docs/SPEC-superpong.md` and this crate's README.

mod audio;
mod canvas;
mod colour;
mod crt;
mod demo;
mod input;
mod menu;
mod render;
mod routing;
mod scheduler;
mod settings;
mod single_instance;

use clap::Parser;
use gilrs::Button;
use macroquad::prelude::*;

use audio::{Sounds, Tone, DEFAULT_VOLUME, VOLUME_MAX};
use demo::Demo;
use input::InputManager;
use menu::{Action, Command, Item, Menu};
use render::{Renderer, DEFAULT_WINDOW};
use routing::PreviewHold;
use scheduler::{frame_dt, Scheduler};
use settings::{DisplayMode, Settings};
use superpong_core::{Console, GameKind, Input as CoreInput, SoundEvent};

/// CLI flags. Parsed in a plain, synchronous `fn main` before any window is
/// created (task review finding #6), so `--help`/`--version`/an unknown flag
/// print and exit cleanly even with no display available, instead of
/// panicking inside miniquad's X11 setup after a window was already opened.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "retropong",
    about = "RetroPong: a clone of the Atari Super Pong console (model C-140)",
    version
)]
struct Cli {
    /// Initial Game Select position, 1-4 (default 3).
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=4))]
    game: Option<u8>,
    /// Start in fullscreen.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    fullscreen: bool,
    /// Start in black-and-white instead of colour.
    #[arg(long)]
    mono: bool,
    /// Start in this display mode: `pixel` (sharp pixels) or `crt`
    /// (classic video, as a 1970s colour TV showed it).
    #[arg(long, value_enum, default_value_t = DisplayMode::Crt)]
    display: DisplayMode,
    /// Write the four synthesised sounds as WAV files (44.1 kHz, 16-bit
    /// mono) into this directory, then exit.
    #[arg(long, value_name = "DIR")]
    export_sounds: Option<std::path::PathBuf>,
    /// Start muted.
    #[arg(long)]
    mute: bool,
    /// Master volume at start, 0 (silent) to 10.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=i64::from(VOLUME_MAX)), default_value_t = DEFAULT_VOLUME)]
    volume: u8,
    /// Invert the left knob reading (CW up); false selects CCW up.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    invert_left: bool,
    /// Invert the right knob reading (CW up); false selects CCW up.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    invert_right: bool,
    /// Prefer Wayland (with X11 fallback) instead of X11-only.
    #[arg(long)]
    wayland: bool,
    /// Headless testing: write a PNG screenshot here after `--frames`
    /// frames, then exit.
    #[arg(long, hide = true)]
    screenshot: Option<String>,
    /// Headless testing: number of frames to run before the screenshot.
    #[arg(long, default_value_t = 60, hide = true)]
    frames: u64,
    /// Headless testing: press Start on the second frame.
    #[arg(long)]
    auto_start: bool,
    /// Headless testing: drive both knobs from the ball position so
    /// rallies happen on their own, and press Start 3 s after each game
    /// over.
    #[arg(long)]
    demo: bool,
    /// Headless testing: fixed knob values `<left> <right>` (0.0..=1.0),
    /// overriding every other input source every frame.
    // `action = Set` (rather than clap's default `Append` for a
    // `Vec`-typed arg) makes clap reject a repeated `--knobs` outright
    // ("cannot be used multiple times") instead of silently concatenating
    // the two pairs into a 4-element `Vec` that fails the `[l, r]` match
    // below and falls through to physical input (task review
    // finding #3). A plain comment, not a doc comment, so it doesn't leak
    // into `--help`.
    #[arg(long, num_args = 2, value_names = ["LEFT", "RIGHT"], action = clap::ArgAction::Set)]
    knobs: Option<Vec<f32>>,
    /// Headless testing: open the menu on this rendered frame.
    #[arg(long, value_name = "FRAME", hide = true)]
    menu_at: Option<u64>,
    /// Headless testing: the entry the cursor is on when `--menu-at` opens
    /// the menu.
    #[arg(long, value_enum, default_value_t = Item::Resume, hide = true)]
    menu_item: Item,
    /// Headless testing: press `Esc` on this rendered frame (after
    /// `--menu-at`: close the menu, starting the countdown in a game).
    #[arg(long, value_name = "FRAME", hide = true)]
    resume_at: Option<u64>,
}

/// Window configuration, built from the already-parsed [`Cli`].
fn conf(cli: &Cli) -> Conf {
    Conf {
        window_title: "RetroPong".to_owned(),
        window_width: DEFAULT_WINDOW.0,
        window_height: DEFAULT_WINDOW.1,
        high_dpi: true,
        window_resizable: true,
        fullscreen: cli.fullscreen,
        sample_count: 1,
        platform: miniquad::conf::Platform {
            linux_backend: if cli.wayland {
                miniquad::conf::LinuxBackend::WaylandWithX11Fallback
            } else {
                miniquad::conf::LinuxBackend::X11Only
            },
            linux_wm_class: "retropong",
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Internal QA capture of the current framebuffer, with IO failures reported.
fn export_screenshot(path: &str) {
    let image = get_screen_data();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| image.export_png(path))) {
        Ok(()) => eprintln!("retropong: screenshot saved to {path}"),
        Err(_) => eprintln!("retropong: failed to save screenshot to {path}"),
    }
}

/// A plain, synchronous entry point (not `#[macroquad::main]`) so
/// `Cli::parse()` runs, and can exit on `--help`/`--version`/a bad flag,
/// before any window is created (task review finding #6).
fn main() {
    let cli = Cli::parse();
    if let Some(dir) = &cli.export_sounds {
        match audio::export(dir) {
            Ok(paths) => paths.iter().for_each(|p| eprintln!("retropong: wrote {}", p.display())),
            Err(e) => {
                eprintln!("retropong: failed to export sounds to {}: {e}", dir.display());
                std::process::exit(1);
            }
        }
        return;
    }
    let _instance = match single_instance::InstanceGuard::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            eprintln!(
                "retropong: RetroPong is already running. Close its window before starting another game."
            );
            return;
        }
        Err(error) => {
            eprintln!("retropong: could not reserve the game instance: {error}");
            std::process::exit(1);
        }
    };
    macroquad::Window::from_config(conf(&cli), amain(cli));
}

async fn amain(cli: Cli) {
    let mut settings = Settings {
        color: !cli.mono,
        display: cli.display,
        muted: cli.mute,
        volume: cli.volume,
        invert_left: cli.invert_left,
        invert_right: cli.invert_right,
    };
    let mut first_launch = true;

    let kind = cli.game.and_then(GameKind::from_number).unwrap_or(GameKind::SuperPong);
    let mut console = Console::new(kind);

    let mut renderer = Renderer::new(&settings);
    let mut sounds = Sounds::load().await;
    let mut input_manager = InputManager::new();

    let fixed_knobs = cli.knobs.as_deref().and_then(|v| match v {
        [l, r] => Some((*l, *r)),
        _ => None,
    });

    let mut scheduler = Scheduler::new();
    let mut demo = Demo::new();
    let mut menu = Menu::new();
    let mut preview_hold = PreviewHold::default();
    let mut frame_counter: u64 = 0;
    let mut events: Vec<SoundEvent> = Vec::new();

    loop {
        frame_counter += 1;
        let dt = frame_dt(cli.screenshot.is_some(), get_frame_time());
        // Keys are routed by where the menu was when the frame began, so
        // the Enter that resumes from the menu is not also a Start.
        let menu_open = menu.is_open();
        let paused = !menu.runs_console();

        // The headless `--resume-at` presses Esc too.
        let pressed = |key: KeyCode| {
            is_key_pressed(key) || (key == KeyCode::Escape && cli.resume_at == Some(frame_counter))
        };
        let mut commands = routing::menu_commands(menu_open, pressed);
        if cli.menu_at == Some(frame_counter) {
            menu.open_at(cli.menu_item);
        }
        // And no Start while paused: it would be kept for the first tick
        // after the countdown, and restart the match.
        let auto_start_now = cli.auto_start && frame_counter == 2;
        if routing::start_wanted(paused, auto_start_now, is_key_pressed) {
            scheduler.press_start();
        }

        let use_demo = cli.demo && fixed_knobs.is_none();
        let (left, right) = if let Some(pair) = fixed_knobs {
            pair
        } else if cli.demo {
            (0.5, 0.5)
        } else {
            input_manager.poll(&settings)
        };
        commands.extend(input_manager.take_buttons().into_iter().filter_map(pad_command));

        // Controller buttons mean what the menu makes them as each press
        // arrives (SPEC §13.6), so they are mapped after the other commands.
        let paddle_buttons = input_manager.take_paddle_buttons().into_iter().map(Err);
        for next in commands.into_iter().map(Ok).chain(paddle_buttons) {
            let Some(command) = next.map_or_else(|button| menu.paddle_command(button), Some) else {
                continue;
            };
            match menu.command(command, &mut settings, &console.status()) {
                Action::None => {}
                Action::Select(kind) => scheduler.select_now(&mut console, kind),
                Action::Volume => sounds.beep(Tone::Bounce, settings.gain()),
                Action::Exit => return,
            }
        }
        if !paused && !menu.runs_console() {
            // The pause cuts the speaker off, as the console would stop, and
            // leaves no Start buffered from before it.
            sounds.stop();
            scheduler.pause(&mut console);
        }
        if menu.advance(dt).is_some() {
            sounds.beep(Tone::Bounce, settings.gain());
        }

        let mut ticks = 0;
        if menu.runs_console() {
            // The demo steers every tick from the console as it then
            // stands; the other sources hold one reading for all of this
            // frame's ticks.
            ticks = scheduler.run(&mut console, dt as f64, &mut events, |status| {
                if use_demo {
                    demo.input(status)
                } else {
                    CoreInput { left, right, ..CoreInput::default() }
                }
            });
            sounds.play(&events, settings.gain());
        }

        // During the countdown the console stays frozen, but the knobs are
        // live so the players can get their paddles in place: the picture
        // is the frame the next tick would draw with the knobs as they are
        // now, from a copy that is then thrown away. The real tick after
        // the countdown draws that same frame, so the preview stays up
        // until that tick has run, and nothing jumps.
        let preview;
        let shown = if preview_hold.show_preview(menu.countdown().is_some(), menu.is_open(), ticks) {
            let (l, r) = if use_demo { demo::knobs(&console.status()) } else { (left, right) };
            preview = routing::preview(&console, l, r);
            &preview
        } else {
            &console
        };
        let crt_shown = renderer.shown_mode(&settings) == DisplayMode::Crt;
        let overlay = routing::overlay(&menu, &console, shown, &settings, crt_shown);

        let overlay = routing::launch_overlay(&mut first_launch, console.phase(), overlay);
        renderer.draw(shown, &overlay, &settings);

        if let Some(path) = &cli.screenshot {
            if frame_counter >= cli.frames {
                export_screenshot(path);
                break;
            }
        }

        next_frame().await;
    }
}

/// The menu command a gamepad button gives (SPEC §13.6): the d-pad
/// navigates, South activates, East closes the menu and Start opens and
/// closes it like `Esc`. The left stick is left out: on a RetroController
/// seen as a gamepad it is the two knobs.
fn pad_command(button: Button) -> Option<Command> {
    Some(match button {
        Button::DPadUp => Command::Up,
        Button::DPadDown => Command::Down,
        Button::DPadLeft => Command::Left,
        Button::DPadRight => Command::Right,
        Button::South => Command::Activate,
        Button::East => Command::Back,
        Button::Start => Command::Toggle,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("retropong").chain(args.iter().copied()))
    }

    #[test]
    fn cli_defaults() {
        let cli = parse(&[]).unwrap();
        assert!(!cli.mono);
        assert!(cli.fullscreen);
        assert_eq!(cli.display, DisplayMode::Crt);
        assert_eq!(cli.export_sounds, None);
        assert_eq!(cli.game, None);
    }

    #[test]
    fn fullscreen_can_be_explicit_or_disabled_and_qa_capture_is_hidden() {
        assert!(parse(&["--fullscreen"]).unwrap().fullscreen);
        assert!(!parse(&["--fullscreen", "false"]).unwrap().fullscreen);
        let help = parse(&["--help"]).unwrap_err().to_string();
        assert!(!help.contains("--screenshot") && !help.contains("--frames"));
    }

    #[test]
    fn cli_new_flags() {
        let cli = parse(&["--mono", "--display", "crt", "--export-sounds", "out/sounds"]).unwrap();
        assert!(cli.mono);
        assert_eq!(cli.display, DisplayMode::Crt);
        assert_eq!(parse(&["--display", "pixel"]).unwrap().display, DisplayMode::Pixel);
        assert!(parse(&["--display", "tv"]).is_err());
        assert_eq!(cli.export_sounds.as_deref(), Some(std::path::Path::new("out/sounds")));
    }

    #[test]
    fn cli_volume_and_menu_flags() {
        assert_eq!(parse(&[]).unwrap().volume, DEFAULT_VOLUME);
        assert_eq!(parse(&["--volume", "0"]).unwrap().volume, 0);
        assert_eq!(parse(&["--volume", "10"]).unwrap().volume, 10);
        assert!(parse(&["--volume", "11"]).is_err());
        let cli = parse(&["--menu-at", "30", "--menu-item", "game-mode", "--resume-at", "40"]).unwrap();
        assert_eq!((cli.menu_at, cli.menu_item, cli.resume_at), (Some(30), Item::GameMode, Some(40)));
        let cli = parse(&[]).unwrap();
        assert_eq!((cli.menu_at, cli.menu_item, cli.resume_at), (None, Item::Resume, None));
        assert!(parse(&["--menu-item", "quit"]).is_err());
    }

    #[test]
    fn cli_is_named_retropong() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        assert_eq!(cmd.get_name(), "retropong");
        assert!(cmd.get_about().unwrap().to_string().starts_with("RetroPong"));
        // The headless menu flags stay out of `--help`.
        let help = cmd.clone().render_help().to_string();
        assert!(!help.contains("menu-at") && !help.contains("resume-at"), "{help}");
        assert!(help.contains("--volume"));
    }

    #[test]
    fn gamepad_buttons_drive_the_menu() {
        assert_eq!(pad_command(Button::DPadUp), Some(Command::Up));
        assert_eq!(pad_command(Button::DPadDown), Some(Command::Down));
        assert_eq!(pad_command(Button::DPadLeft), Some(Command::Left));
        assert_eq!(pad_command(Button::DPadRight), Some(Command::Right));
        assert_eq!(pad_command(Button::South), Some(Command::Activate));
        assert_eq!(pad_command(Button::East), Some(Command::Back));
        assert_eq!(pad_command(Button::Start), Some(Command::Toggle));
        assert_eq!(pad_command(Button::North), None);
    }

    #[test]
    fn cli_removed_flags_are_rejected() {
        assert!(parse(&["--color"]).is_err());
        assert!(parse(&["--seed", "3"]).is_err());
        assert!(parse(&["--sounds", "c140"]).is_err());
        assert!(parse(&["--sound-levels", "video"]).is_err());
        // Folded into `--display crt`.
        assert!(parse(&["--scanlines"]).is_err());
        assert!(parse(&["--stripes"]).is_err());
        assert!(parse(&["--swap"]).is_err());
    }

    #[test]
    fn cli_knobs_default_to_clockwise_up_and_allow_independent_overrides() {
        let cli = parse(&[]).unwrap();
        assert!(cli.invert_left && cli.invert_right);
        let cli = parse(&["--invert-left", "false"]).unwrap();
        assert!(!cli.invert_left && cli.invert_right);
        let cli = parse(&["--invert-right=false"]).unwrap();
        assert!(cli.invert_left && !cli.invert_right);
    }

    #[test]
    fn cli_kept_flags() {
        let cli = parse(&[
            "--game",
            "4",
            "--fullscreen",
            "true",
            "--mute",
            "--invert-left",
            "--invert-right",
            "--wayland",
            "--screenshot",
            "shot.png",
            "--frames",
            "600",
            "--auto-start",
            "--demo",
            "--knobs",
            "0.5",
            "0.25",
        ])
        .unwrap();
        assert_eq!(cli.game, Some(4));
        assert!(cli.fullscreen && cli.mute && cli.wayland);
        assert!(cli.invert_left && cli.invert_right && cli.auto_start && cli.demo);
        assert_eq!((cli.screenshot.as_deref(), cli.frames), (Some("shot.png"), 600));
        assert_eq!(cli.knobs, Some(vec![0.5, 0.25]));
        assert!(parse(&["--game", "5"]).is_err());
        assert!(parse(&["--knobs", "0.1", "0.2", "--knobs", "0.3", "0.4"]).is_err());
    }
}
