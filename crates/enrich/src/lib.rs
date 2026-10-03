//! Looks up artwork, lyrics and tempo for a track from free, official APIs.
//!
//! Only real APIs are used (no page scraping, so no captchas), every response
//! is cached on disk, and each host is rate limited with backoff; see [`http`].

mod audiodb;
mod deezer;
mod fanart;
mod http;
mod itunes;
mod lrc;
mod lrclib;
pub mod matching;

use std::path::PathBuf;
use std::time::Duration;

use mv_core::Track;

pub use http::FetchError;
pub use lrc::{LyricLine, Lyrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// Album cover.
    Cover,
    ArtistPhoto,
    /// Wide artist image (~1920x1080), meant to fill the screen.
    Background,
    /// Artist with a transparent background.
    Cutout,
    Logo,
    Banner,
}

#[derive(Debug, Clone)]
pub struct Image {
    pub kind: ImageKind,
    pub url: String,
    pub source: &'static str,
}

/// Everything found for one track. Within each kind, images are best-first.
#[derive(Debug, Clone, Default)]
pub struct Enrichment {
    pub images: Vec<Image>,
    pub lyrics: Option<Lyrics>,
    pub bpm: Option<f32>,
    /// Lookups that failed (offline, backing off, ...). Missing data is not listed.
    pub problems: Vec<String>,
}

impl Enrichment {
    pub fn best(&self, kind: ImageKind) -> Option<&Image> {
        self.images.iter().find(|i| i.kind == kind)
    }
}

pub struct Config {
    pub cache_dir: PathBuf,
    /// iTunes storefront; local releases are often only listed in their home store.
    pub country: String,
    pub audiodb_key: String,
    pub fanart_key: Option<String>,
}

impl Config {
    /// Defaults, overridable by MV_COUNTRY, THEAUDIODB_API_KEY and FANART_API_KEY.
    pub fn from_env() -> Self {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            cache_dir: dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("musicvisual")
                .join("http"),
            country: env("MV_COUNTRY").unwrap_or_else(|| "cz".into()),
            audiodb_key: env("THEAUDIODB_API_KEY").unwrap_or_else(|| "123".into()),
            fanart_key: env("FANART_API_KEY"),
        }
    }
}

pub struct Enricher {
    http: http::Http,
    config: Config,
}

impl Enricher {
    pub fn new(config: Config) -> Self {
        Self {
            http: http::Http::new(config.cache_dir.clone()),
            config,
        }
    }

    /// `preview` gets Deezer's cover and artist photo as soon as they are known,
    /// so artwork can show up before the slower lookups (iTunes, lyrics, artist
    /// art) finish. It is not called when Deezer has nothing.
    pub async fn enrich(
        &self,
        track: &Track,
        duration: Option<Duration>,
        preview: impl FnOnce(Enrichment),
    ) -> Enrichment {
        let mut out = Enrichment::default();
        let Some(artist) = track
            .artist
            .as_deref()
            .map(matching::primary_artist)
            .filter(|a| !a.is_empty())
        else {
            out.problems
                .push("no artist reported, skipping lookups".into());
            return out;
        };
        let title = track.title.as_str();
        let http = &self.http;

        let deezer = async {
            let found = deezer::search(http, artist, title).await;
            let Ok(Some(found)) = found else {
                return (found, None);
            };
            let mut early = Enrichment::default();
            add_deezer_images(&mut early.images, &found);
            if !early.images.is_empty() {
                preview(early);
            }
            let bpm = deezer::bpm(http, found.id).await;
            (Ok(Some(found)), bpm)
        };
        let ((deezer, bpm), itunes, lyrics, artist_art) = tokio::join!(
            deezer,
            itunes::cover(
                http,
                &self.config.country,
                artist,
                title,
                track.album.as_deref()
            ),
            lrclib::lookup(http, artist, title, duration),
            self.artist_art(artist),
        );
        let deezer = ok_or_note(deezer, "deezer", &mut out.problems);
        let itunes = ok_or_note(itunes, "itunes", &mut out.problems);
        out.lyrics = ok_or_note(lyrics, "lrclib", &mut out.problems);
        let (audiodb, fanart) = artist_art;
        let audiodb = ok_or_note(audiodb, "theaudiodb", &mut out.problems);
        let fanart = ok_or_note(fanart, "fanart.tv", &mut out.problems);

        let mut add = |kind, source, url: Option<String>| {
            if let Some(url) = url {
                out.images.push(Image { kind, url, source });
            }
        };
        // Order = preference: iTunes covers are the largest, fanart.tv backgrounds the best curated.
        add(ImageKind::Cover, "itunes", itunes);
        if let Some(f) = fanart {
            f.backgrounds
                .into_iter()
                .for_each(|u| add(ImageKind::Background, "fanart.tv", Some(u)));
            f.logos
                .into_iter()
                .for_each(|u| add(ImageKind::Logo, "fanart.tv", Some(u)));
            f.thumbs
                .into_iter()
                .for_each(|u| add(ImageKind::ArtistPhoto, "fanart.tv", Some(u)));
        }
        if let Some(a) = audiodb {
            a.fanart
                .into_iter()
                .for_each(|u| add(ImageKind::Background, "theaudiodb", Some(u)));
            add(ImageKind::Cutout, "theaudiodb", a.cutout);
            add(ImageKind::Cutout, "theaudiodb", a.clearart);
            add(ImageKind::Logo, "theaudiodb", a.logo);
            add(ImageKind::Banner, "theaudiodb", a.banner);
        }
        if let Some(d) = deezer {
            add_deezer_images(&mut out.images, &d);
        }
        out.bpm = bpm;
        out
    }

    /// Local path of a downloaded image (cached forever, fetched politely).
    pub async fn image(&self, url: &str) -> Result<PathBuf, FetchError> {
        const POLICY: http::Policy = http::Policy {
            min_interval: Duration::from_millis(200),
            ttl: Duration::MAX,
            is_throttled: http::never_throttled,
        };
        let url = reqwest::Url::parse(url).map_err(|e| FetchError::Parse(e.to_string()))?;
        self.http.get_file(&url, &POLICY).await
    }

    /// TheAudioDB first: besides artwork it provides the MusicBrainz id fanart.tv needs.
    async fn artist_art(
        &self,
        artist: &str,
    ) -> (
        Result<Option<audiodb::ArtistArt>, FetchError>,
        Result<Option<fanart::ArtistArt>, FetchError>,
    ) {
        let audiodb = audiodb::artist(&self.http, &self.config.audiodb_key, artist).await;
        let mbid = audiodb.as_ref().ok().and_then(|a| a.as_ref()?.mbid.clone());
        let fanart = match (&self.config.fanart_key, mbid) {
            (Some(key), Some(mbid)) => fanart::artist(&self.http, key, &mbid).await,
            _ => Ok(None),
        };
        (audiodb, fanart)
    }
}

fn add_deezer_images(images: &mut Vec<Image>, found: &deezer::Found) {
    let kinds = [
        (ImageKind::Cover, &found.cover),
        (ImageKind::ArtistPhoto, &found.artist_picture),
    ];
    for (kind, url) in kinds {
        if let Some(url) = url {
            images.push(Image {
                kind,
                url: url.clone(),
                source: "deezer",
            });
        }
    }
}

fn ok_or_note<T>(
    result: Result<Option<T>, FetchError>,
    source: &str,
    problems: &mut Vec<String>,
) -> Option<T> {
    result.unwrap_or_else(|e| {
        problems.push(format!("{source}: {e}"));
        None
    })
}
