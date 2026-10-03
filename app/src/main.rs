// No console window behind the visualizer on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod art;
mod background;
mod cache;
mod pipeline;
mod settings;
mod setup;
mod ui;
mod update;

use std::process::ExitCode;
use std::sync::Arc;

use eframe::egui;
use pipeline::{Options, SharedScene};
use settings::{AudioMode, Settings};

const HELP: &str = "\
musicvisual: visualizes what is playing on this computer and on network speakers

USAGE: musicvisual [OPTIONS]

OPTIONS:
    --bluos <HOST>   Poll a BluOS player at HOST (repeatable)
    --no-discover    Do not look for BluOS players via mDNS
    --no-smtc        Do not read Windows media sessions
    --youtube        Fetch background clips via yt-dlp (off by default)
    --audio <MODE>   auto | loopback | mic | off (default: auto = loopback
                     for music played by this computer, mic for speakers)
    -h, --help       Print this help

SCREENSAVER (rename or copy the exe to OZBEAT.scr, then right-click > Install):
    /s               Run as the screensaver; any input closes it
    /c               Open the settings
    /p <HWND>        Preview in the Windows dialog (not supported, exits)

KEYS:
    Ctrl+S               settings
    Ctrl+D               mirror the visuals on the next screen
    F11 / double-click   toggle fullscreen
    Esc                  leave fullscreen / close settings

--youtube and --audio override the saved settings for this run.";

/// Command-line overrides of saved settings, applied for this run only.
#[derive(Default)]
struct Overrides {
    youtube: bool,
    audio: Option<AudioMode>,
}

fn parse_args(
    mut args: impl Iterator<Item = String>,
) -> Result<Option<(Options, Overrides)>, String> {
    let mut opts = Options {
        bluos_hosts: Vec::new(),
        discover: true,
        smtc: true,
    };
    let mut overrides = Overrides::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bluos" => opts
                .bluos_hosts
                .push(args.next().ok_or("--bluos needs a host")?),
            "--no-discover" => opts.discover = false,
            "--no-smtc" => opts.smtc = false,
            "--youtube" => overrides.youtube = true,
            "--audio" => {
                overrides.audio = Some(match args.next().as_deref() {
                    Some("auto") => AudioMode::Auto,
                    Some("loopback") => AudioMode::Loopback,
                    Some("mic") => AudioMode::Mic,
                    Some("off") => AudioMode::Off,
                    _ => return Err("--audio needs auto, loopback, mic or off".into()),
                })
            }
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some((opts, overrides)))
}

/// How Windows asked us to run when started as a `.scr` screensaver.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// A normal window (no screensaver argument).
    Window,
    /// `/s`: fullscreen on top of everything, closed by any input.
    Screensaver,
    /// `/c`: the screensaver dialog's Settings button.
    Configure,
    /// `/p <HWND>`: the small preview in the screensaver dialog.
    Preview,
}

/// Windows passes `/s`, `/c`, `/c:<HWND>`, `/p <HWND>` or `/p:<HWND>`, in any case.
fn screensaver_mode(first: Option<&str>) -> Mode {
    let Some(arg) = first.and_then(|a| a.strip_prefix('/').or_else(|| a.strip_prefix('-'))) else {
        return Mode::Window;
    };
    let flag = arg
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match flag.as_str() {
        "s" => Mode::Screensaver,
        "c" => Mode::Configure,
        "p" => Mode::Preview,
        _ => Mode::Window,
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = screensaver_mode(args.first().map(String::as_str));
    if mode == Mode::Preview {
        // eframe cannot draw into the dialog's preview window; leave it black.
        return ExitCode::SUCCESS;
    }
    let args = if mode == Mode::Window {
        args
    } else {
        Vec::new()
    };
    let (opts, overrides) = match parse_args(args.into_iter()) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}\n\n{HELP}");
            return ExitCode::FAILURE;
        }
    };

    // The window must own the main thread (macOS requires it); the async
    // pipeline gets its own runtime on a background thread.
    let mut settings = Settings::load();
    settings.youtube |= overrides.youtube;
    if let Some(audio) = overrides.audio {
        settings.audio = audio;
    }
    let settings = Arc::new(std::sync::Mutex::new(settings));
    cache::trim_in_background();
    update::remove_previous();

    let scene = SharedScene::default();
    let pipeline_scene = Arc::clone(&scene);
    let pipeline_settings = Arc::clone(&settings);
    std::thread::Builder::new()
        .name("pipeline".into())
        .spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("failed to start tokio");
            runtime.block_on(pipeline::run(opts, pipeline_scene, pipeline_settings));
        })
        .expect("failed to spawn pipeline thread");

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("OZBEAT")
        .with_inner_size([1280.0, 720.0])
        .with_min_inner_size([640.0, 360.0]);
    if mode == Mode::Screensaver {
        let displays = display_info::DisplayInfo::all().unwrap_or_default();
        if let Some(primary) = primary_monitor(&displays) {
            viewport = viewport.with_monitor(primary);
        }
        viewport = viewport
            .with_fullscreen(true)
            .with_decorations(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
    }
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let result = eframe::run_native(
        "OZBEAT",
        options,
        Box::new(move |cc| Ok(Box::new(ui::VisualApp::new(cc, scene, settings, mode)))),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("window error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Index of the primary monitor, in the order winit lists monitors (on
/// Windows both enumerate via EnumDisplayMonitors).
pub(crate) fn primary_monitor(displays: &[display_info::DisplayInfo]) -> Option<usize> {
    displays.iter().position(|d| d.is_primary)
}

/// Title bar and taskbar icon; the dark variant reads well on both themes.
pub(crate) fn window_icon() -> Option<egui::IconData> {
    let png = include_bytes!("../../assety/OZBEAT_dark_logo.png");
    let image = image::load_from_memory(png).ok()?.to_rgba8();
    Some(egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
