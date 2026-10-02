//! iTunes Search API: no key, album artwork at its original resolution
//! (often 3000x3000). Apple allows roughly 20 requests per minute.
//!
//! Song search misses a lot of local releases, so we resolve the artist first
//! and then scan their catalogue, which also caches well per artist.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use crate::http::{FetchError, Http, Policy, never_throttled};
use crate::matching::{norm, same_artist, same_title};

const API: &str = "https://itunes.apple.com";

const POLICY: Policy = Policy {
    min_interval: Duration::from_secs(3),
    ttl: Duration::from_secs(7 * 86_400),
    is_throttled: never_throttled,
};

#[derive(Deserialize)]
struct Results<T> {
    results: Vec<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtistResult {
    artist_id: u64,
    artist_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogItem {
    wrapper_type: String,
    track_name: Option<String>,
    collection_name: Option<String>,
    artwork_url100: Option<String>,
}

pub(crate) async fn cover(
    http: &Http,
    country: &str,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<String>, FetchError> {
    let search = Url::parse_with_params(
        &format!("{API}/search"),
        [
            ("term", artist),
            ("entity", "musicArtist"),
            ("country", country),
            ("limit", "10"),
        ],
    )
    .expect("valid url");
    let Some(body) = http.get_text(&search, &POLICY).await? else {
        return Ok(None);
    };
    let artists: Results<ArtistResult> = parse(&body)?;
    let Some(found) = artists
        .results
        .into_iter()
        .find(|a| same_artist(&a.artist_name, artist))
    else {
        return Ok(None);
    };

    let lookup = Url::parse_with_params(
        &format!("{API}/lookup"),
        [
            ("id", found.artist_id.to_string().as_str()),
            ("entity", "song"),
            ("country", country),
            ("limit", "200"),
        ],
    )
    .expect("valid url");
    let Some(body) = http.get_text(&lookup, &POLICY).await? else {
        return Ok(None);
    };
    let catalog: Results<CatalogItem> = parse(&body)?;
    let songs: Vec<_> = catalog
        .results
        .into_iter()
        .filter(|i| i.wrapper_type == "track")
        .collect();

    // Exact track first; otherwise any song from the same album has the same cover.
    let by_title = songs.iter().find(|s| {
        s.track_name
            .as_deref()
            .is_some_and(|t| same_title(t, title))
    });
    let by_album = || {
        let album = norm(album?);
        songs.iter().find(|s| {
            s.collection_name
                .as_deref()
                .is_some_and(|c| norm(c) == album)
        })
    };
    Ok(by_title
        .or_else(by_album)
        .and_then(|s| s.artwork_url100.as_deref())
        .map(original_size))
}

fn parse<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, FetchError> {
    serde_json::from_str(body).map_err(|e| FetchError::Parse(e.to_string()))
}

/// Artwork URLs end in `100x100bb.jpg`; requesting a huge size returns the
/// original upload instead (the CDN never upscales).
fn original_size(url: &str) -> String {
    url.replace("/100x100bb.", "/5000x5000bb.")
}
