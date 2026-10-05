//! LRCLIB: free, keyless, community database of time-synced lyrics.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use crate::http::{FetchError, Http, Policy, never_throttled};
use crate::lrc::{self, Lyrics};
use crate::matching::{base_title, same_artist, same_title};

const API: &str = "https://lrclib.net/api";

const POLICY: Policy = Policy {
    min_interval: Duration::from_millis(500),
    // Lyrics don't change; only "not found" (capped by the cache) is worth retrying.
    ttl: Duration::from_secs(90 * 86_400),
    is_throttled: never_throttled,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    artist_name: String,
    track_name: String,
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

pub(crate) async fn lookup(
    http: &Http,
    artist: &str,
    title: &str,
    duration: Option<Duration>,
) -> Result<Option<Lyrics>, FetchError> {
    // `get` is an exact match (LRCLIB tolerates ±2 s on duration).
    let mut params = vec![
        ("artist_name", artist.to_owned()),
        ("track_name", title.to_owned()),
    ];
    if let Some(d) = duration {
        params.push(("duration", d.as_secs().to_string()));
    }
    let get = Url::parse_with_params(&format!("{API}/get"), &params).expect("valid url");
    let exact = match http.get_text(&get, &POLICY).await? {
        Some(body) => {
            let record: Record =
                serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))?;
            if record.instrumental {
                return Ok(None);
            }
            Some(record)
        }
        None => None,
    };
    // The exact entry often has only plain lyrics while another upload of the
    // same song is synced, so search unless we already have timing.
    if exact.as_ref().is_some_and(|r| r.synced_lyrics.is_some()) {
        return Ok(exact.and_then(|r| to_lyrics(r, duration)));
    }

    // Search by the bare title ("Song (Live 2024)" -> "Song"): uploads are
    // usually of the plain version. Keep verified matches, prefer usable
    // timing, then the closest duration.
    let search = Url::parse_with_params(
        &format!("{API}/search"),
        [("artist_name", artist), ("track_name", base_title(title))],
    )
    .expect("valid url");
    let found = match http.get_text(&search, &POLICY).await? {
        Some(body) => {
            let records: Vec<Record> =
                serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))?;
            records
                .into_iter()
                .filter(|r| {
                    !r.instrumental
                        && same_artist(&r.artist_name, artist)
                        && same_title(&r.track_name, title)
                })
                .min_by_key(|r| {
                    let off_by = off_by(r, duration).map_or(u64::MAX / 2, |s| s as u64);
                    (!timing_fits(r, duration), off_by)
                })
        }
        None => None,
    };
    let best = match (exact, found) {
        (exact, Some(found)) if timing_fits(&found, duration) || exact.is_none() => Some(found),
        (exact, _) => exact,
    };
    Ok(best.and_then(|r| to_lyrics(r, duration)))
}

/// Seconds between the song's length and the record's, when both are known.
fn off_by(record: &Record, duration: Option<Duration>) -> Option<f64> {
    Some((duration?.as_secs_f64() - record.duration?).abs())
}

/// Synced lyrics whose timing can be trusted: a version of a different
/// length (live, extended) would drift, and estimated timing beats that.
fn timing_fits(record: &Record, duration: Option<Duration>) -> bool {
    const TOLERANCE: f64 = 8.0;
    record.synced_lyrics.is_some() && off_by(record, duration).is_none_or(|s| s <= TOLERANCE)
}

/// Plain lyrics get estimated timing (see [`lrc::estimate`]) from the song's
/// length, the player's or else LRCLIB's.
fn to_lyrics(record: Record, duration: Option<Duration>) -> Option<Lyrics> {
    if record.instrumental {
        return None;
    }
    let mut synced = record
        .synced_lyrics
        .as_deref()
        .map(lrc::parse)
        .unwrap_or_default();
    let plain = record.plain_lyrics.filter(|p| !p.trim().is_empty());
    let listed = record.duration.filter(|d| d.is_finite() && *d > 0.0);
    let song = duration.or(listed.map(Duration::from_secs_f64));
    if let (true, Some(plain), Some(song)) = (synced.is_empty(), &plain, song) {
        synced = lrc::estimate(plain, song);
    }
    (!synced.is_empty() || plain.is_some()).then_some(Lyrics { synced, plain })
}
