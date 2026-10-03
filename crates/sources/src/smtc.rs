//! Windows System Media Transport Controls: whatever shows up in the
//! media flyout (Spotify, browsers, Apple Music, ...).
//!
//! Polls the session list instead of subscribing to WinRT events. It is
//! simpler, survives apps coming and going, and the cost is negligible.

use std::collections::HashMap;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, SystemTime};

use mv_core::{NowPlaying, PlaybackState, SourceEvent, SourceId, Track};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Properties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;

use crate::EventSender;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 1601-01-01 (WinRT `DateTime` epoch) to 1970-01-01, in 100 ns ticks.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
/// Polls to wait for a new song's thumbnail before taking whatever the player has.
const THUMBNAIL_CHECKS: u32 = 6;

pub fn spawn(tx: EventSender) {
    thread::Builder::new()
        .name("smtc".into())
        .spawn(move || {
            if let Err(e) = run(&tx) {
                eprintln!("smtc: {e}");
            }
        })
        .expect("failed to spawn smtc thread");
}

fn run(tx: &EventSender) -> windows::core::Result<()> {
    let manager = Manager::RequestAsync()?.join()?;
    let mut last: HashMap<String, NowPlaying> = HashMap::new();
    let mut thumbnails: HashMap<String, Thumbnail> = HashMap::new();

    loop {
        let mut current = HashMap::new();
        for session in manager.GetSessions()? {
            let Ok(app) = session.SourceAppUserModelId() else {
                continue;
            };
            // A session can fail mid-read while its app is closing; skip it this round.
            let app = app.to_string();
            let thumbnail = thumbnails.entry(app.clone()).or_default();
            if let Ok(Some(np)) = read_session(&session, &app, thumbnail) {
                current.insert(app, np);
            }
        }

        let mut events = Vec::new();
        for (app, np) in &current {
            if last.get(app) != Some(np) {
                events.push(SourceEvent::Updated(SourceId::new("smtc", app), np.clone()));
            }
        }
        for app in last.keys().filter(|app| !current.contains_key(*app)) {
            events.push(SourceEvent::Gone(SourceId::new("smtc", app)));
        }
        thumbnails.retain(|app, _| current.contains_key(app));
        for event in events {
            if tx.send(event).is_err() {
                return Ok(()); // receiver dropped, app is shutting down
            }
        }

        last = current;
        thread::sleep(POLL_INTERVAL);
    }
}

fn read_session(
    session: &Session,
    app: &str,
    thumbnail: &mut Thumbnail,
) -> windows::core::Result<Option<NowPlaying>> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let title = props.Title()?.to_string();
    if title.is_empty() {
        return Ok(None);
    }
    let mut track = Track {
        title,
        artist: non_empty(props.Artist()?.to_string()),
        album: non_empty(props.AlbumTitle()?.to_string()),
        art_url: None,
    };
    track.art_url = thumbnail.url_for(&track, &props);

    let state = match session.GetPlaybackInfo()?.PlaybackStatus()? {
        Status::Playing => PlaybackState::Playing,
        Status::Paused => PlaybackState::Paused,
        Status::Opened | Status::Changing => PlaybackState::Buffering,
        _ => PlaybackState::Stopped,
    };

    // Browsers often report an all-zero timeline; treat that as "unknown".
    let timeline = session.GetTimelineProperties()?;
    let start = timeline.StartTime()?.Duration;
    let end = timeline.EndTime()?.Duration;
    let (duration, position) = if end > start {
        let pos = (timeline.Position()?.Duration - start).max(0);
        (Some(ticks(end - start)), Some(ticks(pos)))
    } else {
        (None, None)
    };

    let updated = timeline.LastUpdatedTime()?.UniversalTime;
    let position_at = if updated > UNIX_EPOCH_TICKS {
        SystemTime::UNIX_EPOCH + ticks(updated - UNIX_EPOCH_TICKS)
    } else {
        SystemTime::now()
    };

    Ok(Some(NowPlaying {
        track,
        state,
        duration,
        position,
        position_at,
        device: Some(app_name(app)),
    }))
}

/// The player's own cover for the current song, saved to a file so it can
/// travel as a `file:` URL. Read once per song, not on every poll.
#[derive(Default)]
struct Thumbnail {
    /// The song (without artwork) this state belongs to.
    track: Track,
    url: Option<String>,
    hash: Option<u64>,
    /// The previous song's thumbnail and album.
    previous: Option<(u64, Option<String>)>,
    checks: u32,
}

impl Thumbnail {
    fn url_for(&mut self, track: &Track, props: &Properties) -> Option<String> {
        if self.track != *track {
            let previous = self.hash.map(|h| (h, self.track.album.clone()));
            *self = Self {
                track: track.clone(),
                previous,
                ..Self::default()
            };
        }
        if self.url.is_none() && self.checks < THUMBNAIL_CHECKS {
            self.checks += 1;
            if let Some(bytes) = read_thumbnail(props) {
                let hash = fnv1a(&bytes);
                // Players update the title before the picture, so right after a
                // change the old song's cover can still be there. The same
                // picture is fine on the same album, or once we waited enough.
                let stale = self.previous.as_ref().is_some_and(|(h, album)| {
                    *h == hash && (album.is_none() || *album != track.album)
                });
                if !stale || self.checks == THUMBNAIL_CHECKS {
                    self.hash = Some(hash);
                    self.url = save(hash, &bytes);
                }
            }
        }
        self.url.clone()
    }
}

/// The thumbnail's bytes; `None` when the player has none.
fn read_thumbnail(props: &Properties) -> Option<Vec<u8>> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.join().ok()?;
    let size = u32::try_from(stream.Size().ok()?).ok().filter(|s| *s > 0)?;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
    reader.LoadAsync(size).ok()?.join().ok()?;
    let mut bytes = vec![0; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

/// Where player covers are kept; named by content, so a file never changes.
pub fn thumbnail_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("musicvisual")
        .join("player-art")
}

fn save(hash: u64, bytes: &[u8]) -> Option<String> {
    let dir = thumbnail_dir();
    let path = dir.join(format!("{hash:016x}"));
    if path.exists() {
        // Marks it as recently used for cache trimming.
        let _ = std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(SystemTime::now()));
    } else {
        let tmp = path.with_extension("tmp");
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&tmp, bytes))
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            eprintln!("smtc: cannot save thumbnail: {e}");
            return None;
        }
    }
    reqwest::Url::from_file_path(&path).ok().map(String::from)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// "Spotify.exe" -> "Spotify", "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic" -> "ZuneMusic".
fn app_name(app_id: &str) -> String {
    let base = app_id.split(['_', '!']).next().unwrap_or(app_id);
    let base = base.strip_suffix(".exe").unwrap_or(base);
    base.rsplit('.').next().unwrap_or(base).to_owned()
}

fn ticks(t: i64) -> Duration {
    Duration::from_nanos(t as u64 * 100)
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}
