//! TheAudioDB: community artwork (fanart, cutouts, logos) for bigger artists.
//! The public test key "123" is limited; a supporter key goes in THEAUDIODB_API_KEY.

use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use crate::http::{FetchError, Http, Policy, never_throttled};
use crate::matching::same_artist;

const POLICY: Policy = Policy {
    min_interval: Duration::from_millis(2500),
    ttl: Duration::from_secs(30 * 86_400),
    is_throttled: never_throttled,
};

#[derive(Deserialize)]
struct Response {
    artists: Option<Vec<Artist>>,
}

#[derive(Deserialize)]
struct Artist {
    #[serde(rename = "strArtist")]
    name: String,
    #[serde(rename = "strMusicBrainzID")]
    mbid: Option<String>,
    #[serde(rename = "strArtistFanart")]
    fanart1: Option<String>,
    #[serde(rename = "strArtistFanart2")]
    fanart2: Option<String>,
    #[serde(rename = "strArtistFanart3")]
    fanart3: Option<String>,
    #[serde(rename = "strArtistFanart4")]
    fanart4: Option<String>,
    #[serde(rename = "strArtistCutout")]
    cutout: Option<String>,
    #[serde(rename = "strArtistClearart")]
    clearart: Option<String>,
    #[serde(rename = "strArtistLogo")]
    logo: Option<String>,
    #[serde(rename = "strArtistWideThumb")]
    wide_thumb: Option<String>,
    #[serde(rename = "strArtistBanner")]
    banner: Option<String>,
}

#[derive(Default)]
pub(crate) struct ArtistArt {
    pub mbid: Option<String>,
    pub fanart: Vec<String>,
    pub cutout: Option<String>,
    pub clearart: Option<String>,
    pub logo: Option<String>,
    pub banner: Option<String>,
}

pub(crate) async fn artist(
    http: &Http,
    key: &str,
    artist: &str,
) -> Result<Option<ArtistArt>, FetchError> {
    let url = Url::parse_with_params(
        &format!("https://www.theaudiodb.com/api/v1/json/{key}/search.php"),
        [("s", artist)],
    )
    .expect("valid url");
    let Some(body) = http.get_text(&url, &POLICY).await? else {
        return Ok(None);
    };
    let response: Response =
        serde_json::from_str(&body).map_err(|e| FetchError::Parse(e.to_string()))?;
    let Some(a) = response
        .artists
        .into_iter()
        .flatten()
        .find(|a| same_artist(&a.name, artist))
    else {
        return Ok(None);
    };

    let fanart = [a.fanart1, a.fanart2, a.fanart3, a.wide_thumb, a.fanart4];
    Ok(Some(ArtistArt {
        mbid: present(a.mbid),
        fanart: fanart.into_iter().filter_map(present).collect(),
        cutout: present(a.cutout),
        clearart: present(a.clearart),
        logo: present(a.logo),
        banner: present(a.banner),
    }))
}

/// TheAudioDB uses both null and "" for missing values.
fn present(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.trim().is_empty())
}
