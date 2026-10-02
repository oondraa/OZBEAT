//! Source-agnostic model of "what is playing right now".
//!
//! Every source adapter (SMTC, BluOS, ...) translates its own data into
//! [`NowPlaying`] and reports it as a [`SourceEvent`]. The [`Registry`] keeps
//! the latest state per source and decides which one is the active one.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Buffering,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Track {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NowPlaying {
    pub track: Track,
    pub state: PlaybackState,
    pub duration: Option<Duration>,
    /// Position as reported by the source, valid at `position_at`.
    /// Sources report it only occasionally; use [`NowPlaying::position_now`].
    pub position: Option<Duration>,
    pub position_at: SystemTime,
    /// Human-readable player name ("Spotify", "Ondra Pulse"), for display only.
    pub device: Option<String>,
}

impl NowPlaying {
    /// Position extrapolated to `now`, clamped to the track duration.
    pub fn position_now(&self, now: SystemTime) -> Option<Duration> {
        let mut pos = self.position?;
        if self.state == PlaybackState::Playing {
            pos += now.duration_since(self.position_at).unwrap_or_default();
        }
        Some(match self.duration {
            Some(d) if !d.is_zero() => pos.min(d),
            _ => pos,
        })
    }
}

/// Identifies one concrete source, e.g. `smtc:Spotify.exe` or `bluos:192.168.1.40`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId {
    pub kind: &'static str,
    pub instance: String,
}

impl SourceId {
    pub fn new(kind: &'static str, instance: impl Into<String>) -> Self {
        Self {
            kind,
            instance: instance.into(),
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.instance)
    }
}

#[derive(Debug, Clone)]
pub enum SourceEvent {
    Updated(SourceId, NowPlaying),
    /// The source has nothing to report anymore (session closed, device offline).
    Gone(SourceId),
    /// The device exists and has this name, whether or not it plays anything.
    /// Lets the setup screen list idle speakers too.
    Present(SourceId, String),
}

/// A known device, for listing in settings.
#[derive(Debug, Clone)]
pub struct Device {
    pub id: SourceId,
    pub name: String,
    pub now_playing: Option<NowPlaying>,
}

impl Device {
    /// Stable across restarts and DHCP changes, unlike the IP in `id`:
    /// "bluos:Ondra Pulse", "smtc:Spotify".
    pub fn key(&self) -> String {
        format!("{}:{}", self.id.kind, self.name)
    }
}

struct Entry {
    now_playing: NowPlaying,
    /// When this source last started playing or switched tracks while playing.
    last_activity: SystemTime,
}

#[derive(Default)]
pub struct Registry {
    entries: HashMap<SourceId, Entry>,
    names: HashMap<SourceId, String>,
}

impl Registry {
    pub fn apply(&mut self, event: SourceEvent, now: SystemTime) {
        match event {
            SourceEvent::Updated(id, np) => {
                if let Some(device) = &np.device {
                    self.names.insert(id.clone(), device.clone());
                }
                let prev = self.entries.get(&id);
                let became_active = np.state == PlaybackState::Playing
                    && prev.is_none_or(|e| {
                        e.now_playing.state != PlaybackState::Playing
                            || e.now_playing.track != np.track
                    });
                let last_activity = match prev {
                    Some(e) if !became_active => e.last_activity,
                    None if !became_active => SystemTime::UNIX_EPOCH,
                    _ => now,
                };
                self.entries.insert(
                    id,
                    Entry {
                        now_playing: np,
                        last_activity,
                    },
                );
            }
            SourceEvent::Gone(id) => {
                self.entries.remove(&id);
            }
            SourceEvent::Present(id, name) => {
                self.names.insert(id, name);
            }
        }
    }

    /// Like [`Registry::active`], but only among sources `allowed` accepts.
    pub fn active_where(
        &self,
        allowed: impl Fn(&Device) -> bool,
    ) -> Option<(&SourceId, &NowPlaying)> {
        self.entries
            .iter()
            .filter(|(id, e)| allowed(&self.device(id, Some(&e.now_playing))))
            .max_by_key(|(_, e)| {
                (
                    e.now_playing.state == PlaybackState::Playing,
                    e.last_activity,
                )
            })
            .map(|(id, e)| (id, &e.now_playing))
    }

    /// Every device seen so far (playing or merely present), sorted by name.
    pub fn devices(&self) -> Vec<Device> {
        let mut ids: Vec<&SourceId> = self.names.keys().chain(self.entries.keys()).collect();
        ids.sort();
        ids.dedup();
        let mut devices: Vec<Device> = ids
            .into_iter()
            .map(|id| self.device(id, self.entries.get(id).map(|e| &e.now_playing)))
            .collect();
        devices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        devices
    }

    fn device(&self, id: &SourceId, now_playing: Option<&NowPlaying>) -> Device {
        let name = self
            .names
            .get(id)
            .cloned()
            .or_else(|| now_playing.and_then(|np| np.device.clone()))
            .unwrap_or_else(|| id.instance.clone());
        Device {
            id: id.clone(),
            name,
            now_playing: now_playing.cloned(),
        }
    }

    /// The source to visualize: playing beats paused, then most recent activity wins.
    pub fn active(&self) -> Option<(&SourceId, &NowPlaying)> {
        self.entries
            .iter()
            .max_by_key(|(_, e)| {
                (
                    e.now_playing.state == PlaybackState::Playing,
                    e.last_activity,
                )
            })
            .map(|(id, e)| (id, &e.now_playing))
    }

    /// All sources, sorted by id.
    pub fn sources(&self) -> Vec<(&SourceId, &NowPlaying)> {
        let mut all: Vec<_> = self
            .entries
            .iter()
            .map(|(id, e)| (id, &e.now_playing))
            .collect();
        all.sort_by(|a, b| a.0.cmp(b.0));
        all
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn np(title: &str, state: PlaybackState, pos: u64, at: SystemTime) -> NowPlaying {
        NowPlaying {
            track: Track {
                title: title.into(),
                ..Default::default()
            },
            state,
            duration: Some(Duration::from_secs(200)),
            position: Some(Duration::from_secs(pos)),
            position_at: at,
            device: None,
        }
    }

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000 + secs)
    }

    #[test]
    fn position_advances_only_while_playing() {
        let playing = np("a", PlaybackState::Playing, 10, t(0));
        assert_eq!(playing.position_now(t(5)), Some(Duration::from_secs(15)));

        let paused = np("a", PlaybackState::Paused, 10, t(0));
        assert_eq!(paused.position_now(t(5)), Some(Duration::from_secs(10)));
    }

    #[test]
    fn position_is_clamped_to_duration() {
        let playing = np("a", PlaybackState::Playing, 190, t(0));
        assert_eq!(playing.position_now(t(60)), Some(Duration::from_secs(200)));
    }

    #[test]
    fn most_recently_started_playing_source_is_active() {
        let mut reg = Registry::default();
        let a = SourceId::new("smtc", "a");
        let b = SourceId::new("bluos", "b");

        reg.apply(
            SourceEvent::Updated(a.clone(), np("x", PlaybackState::Playing, 0, t(0))),
            t(0),
        );
        reg.apply(
            SourceEvent::Updated(b.clone(), np("y", PlaybackState::Playing, 0, t(1))),
            t(1),
        );
        assert_eq!(reg.active().unwrap().0, &b);

        // A re-report of the same track must not steal focus back.
        reg.apply(
            SourceEvent::Updated(a.clone(), np("x", PlaybackState::Playing, 3, t(3))),
            t(3),
        );
        assert_eq!(reg.active().unwrap().0, &b);

        // Pausing hands focus to whatever still plays.
        reg.apply(
            SourceEvent::Updated(b.clone(), np("y", PlaybackState::Paused, 4, t(4))),
            t(4),
        );
        assert_eq!(reg.active().unwrap().0, &a);

        reg.apply(SourceEvent::Gone(a), t(5));
        assert_eq!(reg.active().unwrap().0, &b);
    }

    #[test]
    fn disabled_sources_are_skipped_and_idle_devices_listed() {
        let mut reg = Registry::default();
        let spotify = SourceId::new("smtc", "Spotify.exe");
        let kitchen = SourceId::new("bluos", "10.0.0.7");
        let mut np_spotify = np("x", PlaybackState::Playing, 0, t(0));
        np_spotify.device = Some("Spotify".into());
        reg.apply(SourceEvent::Updated(spotify.clone(), np_spotify), t(0));
        reg.apply(
            SourceEvent::Present(kitchen.clone(), "Kitchen".into()),
            t(0),
        );

        let names: Vec<_> = reg.devices().iter().map(Device::key).collect();
        assert_eq!(names, ["bluos:Kitchen", "smtc:Spotify"]);

        assert!(reg.active_where(|d| d.key() != "smtc:Spotify").is_none());
        assert_eq!(reg.active_where(|_| true).unwrap().0, &spotify);
    }
}
