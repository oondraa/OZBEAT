//! fanart.tv: HD artist backgrounds (1920x1080) and transparent HD logos.
//! Needs a free personal key (FANART_API_KEY) and a MusicBrainz id.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use crate::http::{FetchError, Http, Policy, never_throttled};

const POLICY: Policy = Policy {
    min_interval: Duration::from_secs(1),
    ttl: Duration::from_secs(30 * 86_400),
    is_throttled: never_throttled,
};

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    artistbackground: Vec<Image>,
    #[serde(default)]
    hdmusiclogo: Vec<Image>,
    #[serde(default)]
    artistthumb: Vec<Image>,
}

#[derive(Deserialize)]
struct Image {
    url: String,
    #[serde(default)]
    likes: String,
}

#[derive(Default)]
pub(crate) struct ArtistArt {
    pub backgrounds: Vec<String>,
    pub logos: Vec<String>,
    pub thumbs: Vec<String>,
}

pub(crate) async fn artist(
    http: &Http,
    key: &str,
    mbid: &str,
) -> Result<Option<ArtistArt>, FetchError> {
    let url = Url::parse_with_params(
        &format!("https://webservice.fanart.tv/v3/music/{mbid}"),
        [("api_key", key)],
    )
    .expect("valid url");
    let Some(body) = http.get_text(&url, &POLICY).await? else {
        return Ok(None);
    };
    let response: Response =
        serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))?;
    Ok(Some(ArtistArt {
        backgrounds: most_liked(response.artistbackground),
        logos: most_liked(response.hdmusiclogo),
        thumbs: most_liked(response.artistthumb),
    }))
}

fn most_liked(mut images: Vec<Image>) -> Vec<String> {
    images.sort_by_key(|i| std::cmp::Reverse(i.likes.parse::<u32>().unwrap_or(0)));
    images.into_iter().map(|i| i.url).collect()
}
