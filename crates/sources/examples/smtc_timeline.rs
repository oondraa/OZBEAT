//! Prints the raw timeline each media session reports, to see how often
//! players update it and how far the extrapolated position drifts.
//!
//! cargo run -p mv-sources --example smtc_timeline -- [seconds]

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager;

    const UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);
    let manager = Manager::RequestAsync()?.join()?;
    let started = Instant::now();
    let mut last = Vec::new();
    while started.elapsed() < Duration::from_secs(seconds) {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64();
        let mut seen = Vec::new();
        for session in manager.GetSessions()? {
            let app = session.SourceAppUserModelId()?.to_string();
            let timeline = session.GetTimelineProperties()?;
            let position = timeline.Position()?.Duration as f64 / 1e7;
            let end = timeline.EndTime()?.Duration as f64 / 1e7;
            let updated = (timeline.LastUpdatedTime()?.UniversalTime - UNIX_EPOCH_TICKS) as f64 / 1e7;
            let title = session.TryGetMediaPropertiesAsync()?.join()?.Title()?.to_string();
            seen.push((app.clone(), position, updated));
            // Print only when the session reports something new.
            if !last.iter().any(|(a, p, u)| *a == app && *p == position && *u == updated) {
                println!(
                    "t={:6.2}s {app} \"{title}\" pos={position:7.2} end={end:7.2} updated {:+6.2}s ago -> now≈{:7.2}",
                    started.elapsed().as_secs_f64(),
                    now - updated,
                    position + (now - updated),
                );
            }
        }
        last = seen;
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
