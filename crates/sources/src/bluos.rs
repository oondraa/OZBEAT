//! Bluesound / BluOS players over their local HTTP API (port 11000).
//!
//! `/Status` supports long polling: pass back the `etag` from the previous
//! response and the player holds the request until something changes or
//! `timeout` seconds pass.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use mv_core::{NowPlaying, PlaybackState, SourceEvent, SourceId, Track};

use crate::EventSender;

const PORT: u16 = 11000;
const LONG_POLL_SECS: u64 = 100;
const RETRY_DELAY: Duration = Duration::from_secs(5);
const MDNS_SERVICE: &str = "_musc._tcp.local.";

/// Polls one player until the receiver goes away.
pub fn spawn(host: String, tx: EventSender) {
    tokio::spawn(run(host, tx));
}

/// Finds players on the LAN via mDNS and spawns a poller for each new one.
/// `known` hosts (e.g. configured manually) are skipped.
pub fn discover(tx: EventSender, mut known: HashSet<String>) -> Result<(), mdns_sd::Error> {
    let mdns = ServiceDaemon::new()?;
    let events = mdns.browse(MDNS_SERVICE)?;
    tokio::spawn(async move {
        let _mdns = mdns; // dropping the daemon stops the browse
        while let Ok(event) = events.recv_async().await {
            let ServiceEvent::ServiceResolved(service) = event else {
                continue;
            };
            // A player may answer on several interfaces; one IPv4 address is enough.
            let Some(ip) = service
                .addresses
                .iter()
                .map(|a| a.to_ip_addr())
                .find(|ip| ip.is_ipv4())
            else {
                continue;
            };
            let host = ip.to_string();
            if known.insert(host.clone()) {
                spawn(host, tx.clone());
            }
        }
    });
    Ok(())
}

async fn run(host: String, tx: EventSender) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(LONG_POLL_SECS + 20))
        .build()
        .expect("failed to build http client");
    let id = SourceId::new("bluos", host.clone());
    let mut etag: Option<String> = None;
    let mut last: Option<NowPlaying> = None;
    let mut name: Option<String> = None;

    loop {
        if name.is_none() {
            name = fetch_name(&client, &host).await;
            if let Some(name) = &name
                && tx
                    .send(SourceEvent::Present(id.clone(), name.clone()))
                    .is_err()
            {
                return;
            }
        }

        let mut url = format!("http://{host}:{PORT}/Status?timeout={LONG_POLL_SECS}");
        if let Some(tag) = &etag {
            url.push_str("&etag=");
            url.push_str(tag);
        }

        let status = match fetch(&client, &url).await {
            Ok(body) => parse_status(&body, &host, name.as_deref(), SystemTime::now()),
            Err(e) => Err(e),
        };
        let status = match status {
            Ok(status) => status,
            Err(e) => {
                eprintln!("bluos {host}: {e}");
                etag = None;
                if last.take().is_some() && tx.send(SourceEvent::Gone(id.clone())).is_err() {
                    return;
                }
                tokio::time::sleep(RETRY_DELAY).await;
                continue;
            }
        };

        etag = status.etag;
        if status.now_playing != last {
            let event = match &status.now_playing {
                Some(np) => SourceEvent::Updated(id.clone(), np.clone()),
                None => SourceEvent::Gone(id.clone()),
            };
            if tx.send(event).is_err() {
                return;
            }
            last = status.now_playing;
        }
    }
}

/// The name set in the BluOS app ("Living Room"), from `/SyncStatus`.
async fn fetch_name(client: &reqwest::Client, host: &str) -> Option<String> {
    let body = fetch(client, &format!("http://{host}:{PORT}/SyncStatus"))
        .await
        .ok()?;
    let doc = roxmltree::Document::parse(&body).ok()?;
    let name = doc.root_element().attribute("name")?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client.get(url).send().await.map_err(|e| e.to_string())?;
    let response = response.error_for_status().map_err(|e| e.to_string())?;
    response.text().await.map_err(|e| e.to_string())
}

#[derive(Debug)]
struct Status {
    etag: Option<String>,
    now_playing: Option<NowPlaying>,
}

fn parse_status(
    body: &str,
    host: &str,
    name: Option<&str>,
    now: SystemTime,
) -> Result<Status, String> {
    let doc = roxmltree::Document::parse(body).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    let etag = root.attribute("etag").map(str::to_owned);
    let field = |name: &str| {
        root.children()
            .find(|n| n.has_tag_name(name))
            .and_then(|n| n.text())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let seconds = |name: &str| {
        field(name)
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|s| *s >= 0.0)
            .map(Duration::from_secs_f64)
    };

    // Local library tracks have name/artist/album; streams and radio often only title1-3.
    let Some(title) = field("name").or_else(|| field("title1")) else {
        return Ok(Status {
            etag,
            now_playing: None,
        });
    };

    let state = match field("state").as_deref() {
        Some("play" | "stream") => PlaybackState::Playing,
        Some("pause") => PlaybackState::Paused,
        Some("connecting") => PlaybackState::Buffering,
        _ => PlaybackState::Stopped,
    };

    let art_url = field("image").map(|img| {
        if img.starts_with("http://") || img.starts_with("https://") {
            img
        } else {
            let sep = if img.starts_with('/') { "" } else { "/" };
            format!("http://{host}:{PORT}{sep}{img}")
        }
    });

    Ok(Status {
        etag,
        now_playing: Some(NowPlaying {
            track: Track {
                title,
                artist: field("artist").or_else(|| field("title2")),
                album: field("album").or_else(|| field("title3")),
                art_url,
            },
            state,
            duration: seconds("totlen").filter(|d| !d.is_zero()),
            position: seconds("secs"),
            position_at: now,
            device: Some(name.unwrap_or(host).to_owned()),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_track() {
        let body = r#"<status etag="4e266c9fbfba6d13d1a4d6ff4bd2e1e6">
            <album>Discovery</album>
            <artist>Daft Punk</artist>
            <name>One More Time</name>
            <image>/Artwork?service=LocalMusic&amp;songid=42</image>
            <secs>83</secs>
            <totlen>320</totlen>
            <state>play</state>
        </status>"#;
        let now = SystemTime::now();
        let status = parse_status(body, "10.0.0.5", Some("Ondra Pulse"), now).unwrap();
        assert_eq!(
            status.etag.as_deref(),
            Some("4e266c9fbfba6d13d1a4d6ff4bd2e1e6")
        );

        let np = status.now_playing.unwrap();
        assert_eq!(np.track.title, "One More Time");
        assert_eq!(np.track.artist.as_deref(), Some("Daft Punk"));
        assert_eq!(np.track.album.as_deref(), Some("Discovery"));
        assert_eq!(
            np.track.art_url.as_deref(),
            Some("http://10.0.0.5:11000/Artwork?service=LocalMusic&songid=42")
        );
        assert_eq!(np.state, PlaybackState::Playing);
        assert_eq!(np.position, Some(Duration::from_secs(83)));
        assert_eq!(np.duration, Some(Duration::from_secs(320)));
        assert_eq!(np.position_at, now);
        assert_eq!(np.device.as_deref(), Some("Ondra Pulse"));
    }

    #[test]
    fn falls_back_to_title_lines_for_streams() {
        let body = r#"<status etag="1">
            <title1>Radio Wave</title1>
            <title2>Some Artist - Some Song</title2>
            <state>stream</state>
            <secs>12</secs>
        </status>"#;
        let np = parse_status(body, "h", None, SystemTime::now())
            .unwrap()
            .now_playing
            .unwrap();
        assert_eq!(np.track.title, "Radio Wave");
        assert_eq!(np.track.artist.as_deref(), Some("Some Artist - Some Song"));
        assert_eq!(np.state, PlaybackState::Playing);
        assert_eq!(np.duration, None);
    }

    #[test]
    fn idle_player_reports_nothing() {
        let body = r#"<status etag="2"><state>stop</state></status>"#;
        let status = parse_status(body, "h", None, SystemTime::now()).unwrap();
        assert_eq!(status.etag.as_deref(), Some("2"));
        assert!(status.now_playing.is_none());
    }
}
