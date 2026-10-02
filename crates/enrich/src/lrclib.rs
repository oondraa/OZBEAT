//! LRCLIB: free, keyless, community database of time-synced lyrics.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use crate::http::{FetchError, Http, Policy, never_throttled};
use crate::lrc::{self, Lyrics};
use crate::matching::{same_artist, same_title};

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
    if let Some(body) = http.get_text(&get, &POLICY).await? {
        let record: Record =
            serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))?;
        return Ok(to_lyrics(record));
    }

    // Fallback: search, keep verified matches, prefer synced and the closest duration.
    let search = Url::parse_with_params(
        &format!("{API}/search"),
        [("artist_name", artist), ("track_name", title)],
    )
    .expect("valid url");
    let Some(body) = http.get_text(&search, &POLICY).await? else {
        return Ok(None);
    };
    let records: Vec<Record> =
        serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))?;
    let best = records
        .into_iter()
        .filter(|r| same_artist(&r.artist_name, artist) && same_title(&r.track_name, title))
        .min_by_key(|r| {
            let off_by = match (duration, r.duration) {
                (Some(want), Some(got)) => (want.as_secs_f64() - got).abs() as u64,
                _ => u64::MAX / 2,
            };
            (r.synced_lyrics.is_none(), off_by)
        });
    Ok(best.and_then(to_lyrics))
}

fn to_lyrics(record: Record) -> Option<Lyrics> {
    if record.instrumental {
        return None;
    }
    let synced = record
        .synced_lyrics
        .as_deref()
        .map(lrc::parse)
        .unwrap_or_default();
    let plain = record.plain_lyrics.filter(|p| !p.trim().is_empty());
    (!synced.is_empty() || plain.is_some()).then_some(Lyrics { synced, plain })
}
