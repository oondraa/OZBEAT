//! Drives a WLED light from the music. The audio analysis goes out as WLED
//! "Audio Sync" packets, so its sound-reactive effects move with whatever
//! OZBEAT hears, and the cover's colors become the light's colors.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use mv_audio::Levels;
use tokio::net::UdpSocket;

use crate::pipeline::SharedScene;
use crate::settings::SharedSettings;

/// WLED's AudioReactive usermod listens here (Sync → Receive).
const AUDIO_SYNC_PORT: u16 = 11988;
/// WLED's own microphone runs at about this rate.
const SEND_EVERY: Duration = Duration::from_millis(20);
/// A name that did not resolve is tried again after this long.
const RESOLVE_RETRY: Duration = Duration::from_secs(10);
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn run(scene: SharedScene, settings: SharedSettings) {
    let socket = match UdpSocket::bind("0.0.0.0:0").await {
        Ok(socket) => socket,
        Err(e) => {
            eprintln!("wled disabled: {e}");
            return;
        }
    };
    let client = match reqwest::Client::builder().timeout(HTTP_TIMEOUT).build() {
        Ok(client) => client,
        Err(e) => {
            eprintln!("wled disabled: {e}");
            return;
        }
    };
    let mut target: Option<Target> = None;
    // The host and art the light's colors were last set for.
    let mut colored: Option<(String, u64)> = None;
    let mut beats = 0;
    let mut tick = tokio::time::interval(SEND_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let host = host_of(&settings.lock().unwrap().wled).to_owned();
        if host.is_empty() {
            target = None;
            continue;
        }
        if target.as_ref().is_none_or(|t| t.stale(&host)) {
            target = Some(Target::resolve(host.clone()).await);
        }
        let Some(addr) = target.as_ref().and_then(|t| t.addr) else {
            continue;
        };

        let (audio, art) = {
            let scene = scene.lock().unwrap();
            (scene.audio.clone(), scene.art.clone())
        };
        if let Some(audio) = audio {
            let levels = audio.levels();
            let beat = levels.beats != beats;
            beats = levels.beats;
            if let Err(e) = socket
                .send_to(&audio_sync_packet(&levels, beat), addr)
                .await
            {
                eprintln!("wled audio sync to {addr}: {e}");
            }
        }
        if let Some(art) = art
            && colored.as_ref() != Some(&(host.clone(), art.id))
        {
            colored = Some((host.clone(), art.id));
            let body = colors_json(art.palette);
            let url = format!("http://{host}/json/state");
            let client = client.clone();
            tokio::spawn(async move {
                let sent = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .body(body.to_string())
                    .send()
                    .await;
                if let Err(e) = sent.and_then(|r| r.error_for_status()) {
                    eprintln!("wled colors: {e}");
                }
            });
        }
    }
}

/// The light's address, looked up once per host.
struct Target {
    host: String,
    addr: Option<SocketAddr>,
    at: Instant,
}

impl Target {
    async fn resolve(host: String) -> Self {
        let addr = match tokio::net::lookup_host((host.as_str(), AUDIO_SYNC_PORT)).await {
            Ok(mut addrs) => addrs.find(SocketAddr::is_ipv4),
            Err(e) => {
                eprintln!("wled {host}: {e}");
                None
            }
        };
        Self {
            host,
            addr,
            at: Instant::now(),
        }
    }

    fn stale(&self, host: &str) -> bool {
        self.host != host || (self.addr.is_none() && self.at.elapsed() >= RESOLVE_RETRY)
    }
}

/// Accepts what people paste: "http://192.168.1.50/" works as well.
fn host_of(input: &str) -> &str {
    let input = input.trim();
    let input = input
        .strip_prefix("http://")
        .or_else(|| input.strip_prefix("https://"))
        .unwrap_or(input);
    input.split('/').next().unwrap_or_default()
}

/// WLED's Audio Sync v2 packet (`audioSyncPacket` in its audio_reactive.h):
/// 44 bytes, little-endian, loudness on 0..255 like its own microphone.
fn audio_sync_packet(levels: &Levels, beat: bool) -> [u8; 44] {
    let mut p = [0u8; 44];
    p[..6].copy_from_slice(b"00002\0");
    p[8..12].copy_from_slice(&(levels.volume_raw * 255.0).to_le_bytes());
    p[12..16].copy_from_slice(&(levels.volume * 255.0).to_le_bytes());
    p[16] = u8::from(beat);
    for (dst, level) in p[18..34].iter_mut().zip(levels.spectrum) {
        *dst = (level.clamp(0.0, 1.0) * 255.0).round() as u8;
    }
    // WLED's effects read the magnitude on about 0..1023.
    p[36..40].copy_from_slice(&(levels.peak_level * 1023.0).to_le_bytes());
    p[40..44].copy_from_slice(&levels.peak_hz.to_le_bytes());
    p
}

/// The cover's vivid and average tones as the first segment's colors, at
/// full brightness (the light's own brightness setting still applies).
fn colors_json(palette: [[f32; 3]; 3]) -> serde_json::Value {
    let full = |c: [f32; 3]| {
        let max = c[0].max(c[1]).max(c[2]).max(0.01);
        c.map(|v| ((v / max).clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    serde_json::json!({ "seg": [{ "col": [full(palette[0]), full(palette[1])] }] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_matches_wled_layout() {
        let mut levels = Levels {
            volume_raw: 0.5,
            volume: 1.0,
            peak_hz: 440.0,
            peak_level: 1.0,
            ..Levels::default()
        };
        levels.spectrum[0] = 1.0;
        levels.spectrum[15] = 0.5;
        let p = audio_sync_packet(&levels, true);
        assert_eq!(&p[..6], b"00002\0");
        assert_eq!(f32::from_le_bytes(p[8..12].try_into().unwrap()), 127.5);
        assert_eq!(f32::from_le_bytes(p[12..16].try_into().unwrap()), 255.0);
        assert_eq!(p[16], 1);
        assert_eq!((p[18], p[33]), (255, 128));
        assert_eq!(f32::from_le_bytes(p[36..40].try_into().unwrap()), 1023.0);
        assert_eq!(f32::from_le_bytes(p[40..44].try_into().unwrap()), 440.0);
    }

    #[test]
    fn host_accepts_pasted_urls() {
        assert_eq!(host_of(" 192.168.1.50 "), "192.168.1.50");
        assert_eq!(host_of("http://192.168.1.50/"), "192.168.1.50");
        assert_eq!(host_of("wled.local/settings"), "wled.local");
    }

    #[test]
    fn colors_are_brightened() {
        let json = colors_json([[0.5, 0.25, 0.0], [0.2, 0.2, 0.2], [0.0; 3]]);
        assert_eq!(json["seg"][0]["col"][0], serde_json::json!([255, 128, 0]));
        assert_eq!(json["seg"][0]["col"][1], serde_json::json!([255, 255, 255]));
    }
}
