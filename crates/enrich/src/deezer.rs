//! Deezer public API: no key, good coverage of small/local artists.
//! Limit is 50 requests / 5 s; we stay far below.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use crate::http::{FetchError, Http, Policy};
use crate::matching::{same_artist, same_title};

const API: &str = "https://api.deezer.com";

const POLICY: Policy = Policy {
    min_interval: Duration::from_millis(250),
    ttl: Duration::from_secs(7 * 86_400),
    is_throttled,
};

/// Deezer answers quota errors with HTTP 200 and `{"error":{"code":4,...}}`.
fn is_throttled(body: &str) -> bool {
    body.starts_with("{\"error\"") && body.contains("\"code\":4")
}

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    data: Vec<SearchTrack>,
}

#[derive(Deserialize)]
struct SearchTrack {
    id: u64,
    title: String,
    artist: Artist,
    album: Album,
}

#[derive(Deserialize)]
struct Artist {
    name: String,
    picture_xl: Option<String>,
}

#[derive(Deserialize)]
struct Album {
    cover_xl: Option<String>,
}

#[derive(Deserialize)]
struct TrackDetail {
    #[serde(default)]
    bpm: f32,
}

pub(crate) struct Found {
    /// Deezer track id, for [`bpm`].
    pub id: u64,
    pub artist_picture: Option<String>,
    pub cover: Option<String>,
}

pub(crate) async fn search(
    http: &Http,
    artist: &str,
    title: &str,
) -> Result<Option<Found>, FetchError> {
    // Field search (artist:"..." track:"...") misses artists with spaces in the name,
    // so search loosely and verify the match ourselves.
    let query = format!("{artist} {title}");
    let url = Url::parse_with_params(
        &format!("{API}/search"),
        [("q", query.as_str()), ("limit", "10")],
    )
    .expect("valid url");
    let Some(body) = http.get_text(&url, &POLICY).await? else {
        return Ok(None);
    };
    let search: Search =
        serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))?;
    let Some(track) = search
        .data
        .into_iter()
        .find(|t| same_artist(&t.artist.name, artist) && same_title(&t.title, title))
    else {
        return Ok(None);
    };
    Ok(Some(Found {
        id: track.id,
        artist_picture: track.artist.picture_xl.map(display_size),
        cover: track.album.cover_xl.map(display_size),
    }))
}

/// BPM is only on the track detail and often 0 (= unknown); a miss here is fine.
pub(crate) async fn bpm(http: &Http, id: u64) -> Option<f32> {
    let url = Url::parse(&format!("{API}/track/{id}")).expect("valid url");
    match http.get_text(&url, &POLICY).await {
        Ok(Some(body)) => serde_json::from_str::<TrackDetail>(&body)
            .ok()
            .map(|d| d.bpm),
        _ => None,
    }
    .filter(|bpm| *bpm > 0.0)
}

/// `*_xl` URLs are 1000x1000; the CDN scales to the size asked for, up to the
/// original upload (e.g. 1200x1200). The app shows covers at most 1600px.
fn display_size(url: String) -> String {
    url.replace("/1000x1000-", "/1600x1600-")
}
