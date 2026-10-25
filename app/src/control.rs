//! Controlling what plays, from the keyboard and the on-screen buttons:
//! the volume, pausing and skipping. BluOS speakers are told directly; for
//! apps on this computer (SMTC) the volume is Windows' own, as with the
//! volume keys.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use mv_core::{NowPlaying, SourceId};

/// One Ctrl+Up/Down step on a BluOS speaker, in percent.
const STEP: f32 = 0.03;
/// While keys are held, the speaker reports back a little late; steps go on
/// from the level we asked for until then.
const TARGET_KEPT: Duration = Duration::from_millis(1500);

enum Command {
    Volume(SourceId, u8),
    PlayPause(SourceId),
    Skip(SourceId, bool),
    #[cfg(windows)]
    SystemVolume(bool),
}

pub struct Control {
    tx: Sender<Command>,
    /// The BluOS level last asked for, and when.
    target: Option<(f32, Instant)>,
}

impl Default for Control {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("control".into())
            .spawn(move || run(&rx))
            .expect("failed to spawn control thread");
        Self { tx, target: None }
    }
}

impl Control {
    /// One step louder or quieter. `false` when there is nothing to control.
    pub fn step_volume(
        &mut self,
        source: Option<&SourceId>,
        now_playing: Option<&NowPlaying>,
        louder: bool,
    ) -> bool {
        let Some(source) = source else {
            return false;
        };
        match source.kind {
            "bluos" => {
                let recent = self
                    .target
                    .filter(|(_, at)| at.elapsed() < TARGET_KEPT)
                    .map(|(level, _)| level);
                let Some(current) = recent.or_else(|| now_playing?.volume) else {
                    // A fixed-level output: nothing to turn.
                    return false;
                };
                let step = if louder { STEP } else { -STEP };
                let level = (current + step).clamp(0.0, 1.0);
                self.target = Some((level, Instant::now()));
                let percent = (level * 100.0).round() as u8;
                self.tx.send(Command::Volume(source.clone(), percent)).is_ok()
            }
            #[cfg(windows)]
            "smtc" => self.tx.send(Command::SystemVolume(louder)).is_ok(),
            _ => false,
        }
    }

    /// Pauses or resumes. `false` when there is nothing to control.
    pub fn play_pause(&self, source: Option<&SourceId>) -> bool {
        source.is_some_and(|s| self.tx.send(Command::PlayPause(s.clone())).is_ok())
    }

    /// The next song, or (`forward` false) the previous one. `false` when
    /// there is nothing to control.
    pub fn skip(&self, source: Option<&SourceId>, forward: bool) -> bool {
        source.is_some_and(|s| self.tx.send(Command::Skip(s.clone(), forward)).is_ok())
    }
}

fn run(rx: &Receiver<Command>) {
    while let Ok(mut command) = rx.recv() {
        // Held keys queue steps faster than a speaker answers; only the last
        // level matters.
        while let Ok(next) = rx.try_recv() {
            match (&command, next) {
                (Command::Volume(..), next @ Command::Volume(..)) => command = next,
                (_, next) => {
                    execute(command);
                    command = next;
                }
            }
        }
        execute(command);
    }
}

fn execute(command: Command) {
    let result = match &command {
        Command::Volume(source, level) => {
            mv_sources::bluos::set_volume(&source.instance, *level)
        }
        Command::PlayPause(source) => match source.kind {
            "bluos" => mv_sources::bluos::toggle_pause(&source.instance),
            #[cfg(windows)]
            "smtc" => {
                mv_sources::smtc::toggle_play_pause(&source.instance).map_err(|e| e.to_string())
            }
            _ => Ok(()),
        },
        Command::Skip(source, forward) => match source.kind {
            "bluos" => mv_sources::bluos::skip(&source.instance, *forward),
            #[cfg(windows)]
            "smtc" => mv_sources::smtc::skip(&source.instance, *forward).map_err(|e| e.to_string()),
            _ => Ok(()),
        },
        #[cfg(windows)]
        Command::SystemVolume(louder) => {
            press_volume_key(*louder);
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("control: {e}");
    }
}

/// Same as the keyboard's volume keys: Windows changes the volume and shows
/// its own indicator.
#[cfg(windows)]
fn press_volume_key(louder: bool) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_VOLUME_DOWN,
        VK_VOLUME_UP,
    };

    let key = if louder { VK_VOLUME_UP } else { VK_VOLUME_DOWN };
    let input = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [input(0), input(KEYEVENTF_KEYUP)];
    // SAFETY: a valid array of INPUT, with its length and element size.
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32);
    }
}
