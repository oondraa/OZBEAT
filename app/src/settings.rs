//! User choices, made in the setup screen (Ctrl+S) and kept across restarts.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use mv_core::Device;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioMode {
    /// Loopback when this computer plays, microphone for network speakers.
    #[default]
    Auto,
    Loopback,
    Mic,
    Off,
}

/// How timed lyrics are shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsStyle {
    /// One word at a time, stacking up as each is sung. Settings saved with
    /// the whole-lines style, since dropped, come back as this.
    #[default]
    #[serde(alias = "lines")]
    Words,
    /// Lines around a turning drum, in perspective, lit word by word.
    Drum,
    /// One word at a time like `Words`, seen through a fisheye lens.
    Lens,
}

/// Where the music video plays once Ctrl+D (or the screensaver) puts the
/// lyrics on a second screen. With one screen it plays in the main window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipScreen {
    /// Under the lyrics; the main window shows the cover big instead.
    #[default]
    Lyrics,
    /// In the main window; the lyrics screen has only the lyrics.
    Main,
    /// On every screen.
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// False until the first-run setup screen was confirmed.
    pub setup_done: bool,
    /// Device keys ("bluos:Ondra Pulse") the user switched off. New devices
    /// default to on, so a newly bought speaker just works.
    pub disabled_sources: BTreeSet<String>,
    pub audio: AudioMode,
    /// 0..1; 0.5 is the default amount of beat-driven motion.
    pub intensity: f32,
    pub lyrics: LyricsStyle,
    /// Download music videos from YouTube and play them with the song.
    pub youtube: bool,
    pub clip_screen: ClipScreen,
    /// IP address or host name of a WLED light to drive; empty = off.
    pub wled: String,
    /// Buttons for the volume and pausing that show while the mouse moves.
    pub buttons: bool,
    /// Ctrl+Up/Down, Ctrl+Left/Right and Ctrl+Space control what plays.
    pub playback_keys: bool,
    /// The lyrics alone over the desktop and every app (Ctrl+Alt+T).
    pub overlay: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            setup_done: false,
            disabled_sources: BTreeSet::new(),
            audio: AudioMode::Auto,
            intensity: 0.5,
            lyrics: LyricsStyle::Words,
            youtube: false,
            clip_screen: ClipScreen::Lyrics,
            wled: String::new(),
            buttons: true,
            playback_keys: true,
            overlay: false,
        }
    }
}

pub type SharedSettings = Arc<Mutex<Settings>>;

impl Settings {
    fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("OZBEAT").join("settings.json"))
    }

    /// Saved settings, or defaults when there are none (or they are unreadable).
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Self::path() else { return };
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                std::fs::write(
                    &path,
                    serde_json::to_string_pretty(self).unwrap_or_default(),
                )
            });
        if let Err(e) = written {
            eprintln!("cannot save settings to {}: {e}", path.display());
        }
    }

    pub fn allows(&self, device: &Device) -> bool {
        !self.disabled_sources.contains(&device.key())
    }
}
