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
    pub youtube: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            setup_done: false,
            disabled_sources: BTreeSet::new(),
            audio: AudioMode::Auto,
            intensity: 0.5,
            youtube: false,
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
