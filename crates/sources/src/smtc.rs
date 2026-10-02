//! Windows System Media Transport Controls: whatever shows up in the
//! media flyout (Spotify, browsers, Apple Music, ...).
//!
//! Polls the session list instead of subscribing to WinRT events. It is
//! simpler, survives apps coming and going, and the cost is negligible.

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, SystemTime};

use mv_core::{NowPlaying, PlaybackState, SourceEvent, SourceId, Track};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

use crate::EventSender;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// 1601-01-01 (WinRT `DateTime` epoch) to 1970-01-01, in 100 ns ticks.
const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

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

    loop {
        let mut current = HashMap::new();
        for session in manager.GetSessions()? {
            let Ok(app) = session.SourceAppUserModelId() else {
                continue;
            };
            // A session can fail mid-read while its app is closing; skip it this round.
            let app = app.to_string();
            if let Ok(Some(np)) = read_session(&session, &app) {
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
        for event in events {
            if tx.send(event).is_err() {
                return Ok(()); // receiver dropped, app is shutting down
            }
        }

        last = current;
        thread::sleep(POLL_INTERVAL);
    }
}

fn read_session(session: &Session, app: &str) -> windows::core::Result<Option<NowPlaying>> {
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let title = props.Title()?.to_string();
    if title.is_empty() {
        return Ok(None);
    }

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
        track: Track {
            title,
            artist: non_empty(props.Artist()?.to_string()),
            album: non_empty(props.AlbumTitle()?.to_string()),
            // TODO: the thumbnail is a stream, not a URL; needs to be served locally.
            art_url: None,
        },
        state,
        duration,
        position,
        position_at,
        device: Some(app_name(app)),
    }))
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
