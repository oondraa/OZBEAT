//! Times how long a track takes until its cover is ready, with an empty cache.
//!
//! cargo run --release -p mv-enrich --example cover_timing

use std::sync::Mutex;
use std::time::{Duration, Instant};

use mv_core::Track;
use mv_enrich::{Config, Enricher, ImageKind};

const TRACKS: &[(&str, &str)] = &[
    ("The Weeknd", "Blinding Lights"),
    ("Daft Punk", "Get Lucky"),
    ("Dua Lipa", "Levitating"),
    ("Arctic Monkeys", "Do I Wanna Know?"),
    ("Billie Eilish", "bad guy"),
];

#[tokio::main]
async fn main() {
    let mut config = Config::from_env();
    config.cache_dir = std::env::temp_dir().join(format!("ozbeat-timing-{}", std::process::id()));
    let enricher = Enricher::new(config);

    for (artist, title) in TRACKS {
        let track = Track {
            title: (*title).into(),
            artist: Some((*artist).into()),
            ..Default::default()
        };
        let start = Instant::now();
        let early = Mutex::new(None);
        let preview = |e: mv_enrich::Enrichment| {
            *early.lock().unwrap() = Some((start.elapsed(), e));
        };
        let enrichment = enricher.enrich(&track, None, preview).await;
        let looked_up = start.elapsed();

        // The app downloads the preview's cover right away, while the rest is still looked up.
        let first = match early.into_inner().unwrap() {
            Some((at, e)) => match e.best(ImageKind::Cover) {
                Some(image) => {
                    let (took, size) = download(&enricher, &image.url).await;
                    format!(
                        "{:.1}s ({} {size} KB)",
                        (at + took).as_secs_f32(),
                        image.source
                    )
                }
                None => "-".into(),
            },
            None => "-".into(),
        };
        let full = match enrichment.best(ImageKind::Cover) {
            Some(image) => {
                let (took, size) = download(&enricher, &image.url).await;
                format!(
                    "{:.1}s ({} {size} KB)",
                    (looked_up + took).as_secs_f32(),
                    image.source
                )
            }
            None => "no cover".into(),
        };
        println!("{artist} - {title}: first cover {first}, after all lookups {full}");
    }
}

async fn download(enricher: &Enricher, url: &str) -> (Duration, u64) {
    let start = Instant::now();
    let size = enricher
        .image(url)
        .await
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .map_or(0, |m| m.len());
    (start.elapsed(), size / 1024)
}
