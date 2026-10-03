//! Prints what Windows media sessions report, including the player's cover file.
//!
//! cargo run -p mv-sources --example smtc_art -- [seconds]

#[cfg(windows)]
fn main() {
    use std::time::{Duration, Instant};

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    mv_sources::smtc::spawn(tx);
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        while let Ok(event) = rx.try_recv() {
            if let mv_core::SourceEvent::Updated(id, np) = event {
                println!(
                    "{id}: {} - {} | art: {}",
                    np.track.artist.as_deref().unwrap_or("?"),
                    np.track.title,
                    np.track.art_url.as_deref().unwrap_or("none"),
                );
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(not(windows))]
fn main() {}
