//! Optional background clips from YouTube, through the user's own `yt-dlp`.
//!
//! YouTube's terms forbid downloading, so this is off by default and kept
//! apart from the API-based `mv-enrich`. It is deliberately slow so the
//! user's IP never gets flagged:
//! - picture only (the audio comes from the speaker), at most 1080p
//! - every clip is downloaded once and kept; "no clip" is remembered too
//! - at least [`MIN_GAP`] between two rounds of talking to YouTube
//! - a bot check or 429 silences the module for [`BACKOFF`], across restarts
//!
//! Clips are atmosphere, not a synced video: their edit rarely matches the
//! album version, and that's fine.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mv_enrich::matching::{norm, primary_artist};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;

pub const MIN_GAP: Duration = Duration::from_secs(180);
pub const BACKOFF: Duration = Duration::from_secs(6 * 3600);
const NOT_FOUND_TTL: Duration = Duration::from_secs(14 * 86_400);
const SEARCH_RESULTS: u32 = 5;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// Video-only stream, so no merging (and no ffmpeg) is needed.
const FORMAT: &str = "bv*[height<=1080][ext=mp4]/bv*[height<=1080]";
/// yt-dlp messages meaning "YouTube is onto us": stop, don't retry.
const BLOCK_MARKERS: &[&str] = &["Sign in to confirm", "HTTP Error 429", "Too Many Requests"];
const BACKOFF_FILE: &str = "backoff_until";

#[derive(Debug, Clone)]
pub enum Outcome {
    Ready(PathBuf),
    NotFound,
    BackingOff { until: SystemTime },
    Failed(String),
}

#[derive(Serialize, Deserialize)]
struct IndexEntry {
    checked_at: u64,
    video_id: Option<String>,
}

#[derive(Deserialize)]
struct SearchHit {
    id: String,
    title: String,
    duration: Option<f64>,
    channel: Option<String>,
    uploader: Option<String>,
}

enum RunError {
    Blocked(String),
    Failed(String),
}

pub struct VideoFetcher {
    dir: PathBuf,
    ytdlp: PathBuf,
    /// Held for the whole YouTube round, so yt-dlp never runs twice at once.
    last_round: Mutex<Option<Instant>>,
}

impl VideoFetcher {
    pub fn new(dir: PathBuf, ytdlp: PathBuf) -> Self {
        Self {
            dir,
            ytdlp,
            last_round: Mutex::new(None),
        }
    }

    /// Cache in the user cache dir; `MV_YTDLP` overrides the yt-dlp binary.
    pub fn from_env() -> Self {
        let dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("musicvisual")
            .join("video");
        let ytdlp =
            std::env::var_os("MV_YTDLP").map_or_else(|| PathBuf::from("yt-dlp"), PathBuf::from);
        Self::new(dir, ytdlp)
    }

    pub async fn fetch(&self, artist: &str, title: &str, duration: Option<Duration>) -> Outcome {
        let artist = primary_artist(artist);
        if let Err(e) = tokio::fs::create_dir_all(&self.dir).await {
            return Outcome::Failed(format!("cache dir: {e}"));
        }
        let index_path = self
            .dir
            .join(format!("{}_{}.json", norm(artist), norm(title)));
        let known = read_index(&index_path).await;

        // Everything answerable from disk is answered without touching YouTube.
        let known_id = match known {
            Some(IndexEntry {
                video_id: Some(id), ..
            }) => {
                if let Some(path) = self.find_clip(&id).await {
                    return Outcome::Ready(path);
                }
                Some(id) // found earlier, download didn't finish
            }
            Some(IndexEntry {
                video_id: None,
                checked_at,
            }) if age(checked_at) < NOT_FOUND_TTL => {
                return Outcome::NotFound;
            }
            _ => None,
        };
        if let Some(until) = self.backoff_until().await {
            return Outcome::BackingOff { until };
        }

        let mut last_round = self.last_round.lock().await;
        if let Some(at) = *last_round {
            tokio::time::sleep(MIN_GAP.saturating_sub(at.elapsed())).await;
        }
        let outcome = self
            .round(known_id, artist, title, duration, &index_path)
            .await;
        *last_round = Some(Instant::now());
        outcome
    }

    async fn round(
        &self,
        known_id: Option<String>,
        artist: &str,
        title: &str,
        duration: Option<Duration>,
        index_path: &Path,
    ) -> Outcome {
        let result = async {
            let id = match known_id {
                Some(id) => id,
                None => {
                    let found = self.search(artist, title, duration).await?;
                    write_index(
                        index_path,
                        &IndexEntry {
                            checked_at: unix_now(),
                            video_id: found.clone(),
                        },
                    )
                    .await;
                    match found {
                        Some(id) => id,
                        None => return Ok(Outcome::NotFound),
                    }
                }
            };
            self.download(&id).await.map(Outcome::Ready)
        }
        .await;

        match result {
            Ok(outcome) => outcome,
            Err(RunError::Blocked(reason)) => {
                let until = SystemTime::now() + BACKOFF;
                let secs = until
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let _ = tokio::fs::write(self.dir.join(BACKOFF_FILE), secs.to_string()).await;
                eprintln!(
                    "youtube pushed back, pausing for {}h: {reason}",
                    BACKOFF.as_secs() / 3600
                );
                Outcome::BackingOff { until }
            }
            Err(RunError::Failed(reason)) => Outcome::Failed(reason),
        }
    }

    async fn search(
        &self,
        artist: &str,
        title: &str,
        duration: Option<Duration>,
    ) -> Result<Option<String>, RunError> {
        let query = format!("ytsearch{SEARCH_RESULTS}:{artist} {title} official video");
        let out = self
            .run(
                &["--flat-playlist", "--dump-json", "--no-warnings", &query],
                SEARCH_TIMEOUT,
            )
            .await?;
        let hits: Vec<SearchHit> = out
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        Ok(pick_clip(&hits, artist, title, duration).map(|h| h.id.clone()))
    }

    async fn download(&self, id: &str) -> Result<PathBuf, RunError> {
        let template = self.dir.join(format!("{id}.%(ext)s"));
        let url = format!("https://www.youtube.com/watch?v={id}");
        let out = self
            .run(
                &[
                    "-f",
                    FORMAT,
                    "--no-playlist",
                    "--no-warnings",
                    "--no-progress",
                    "--no-simulate",
                    "--print",
                    "after_move:filepath",
                    "-o",
                    &template.to_string_lossy(),
                    &url,
                ],
                DOWNLOAD_TIMEOUT,
            )
            .await?;
        let path = out
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .map(PathBuf::from);
        match path {
            Some(path) if tokio::fs::try_exists(&path).await.unwrap_or(false) => Ok(path),
            _ => Err(RunError::Failed(
                "yt-dlp finished but the file is missing".into(),
            )),
        }
    }

    async fn run(&self, args: &[&str], timeout: Duration) -> Result<String, RunError> {
        let child = Command::new(&self.ytdlp)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| RunError::Failed(format!("cannot start {}: {e}", self.ytdlp.display())))?;
        let output = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| RunError::Failed("yt-dlp timed out".into()))?
            .map_err(|e| RunError::Failed(e.to_string()))?;

        let stderr = String::from_utf8_lossy(&output.stderr);
        let last_line = stderr
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("")
            .to_owned();
        if BLOCK_MARKERS.iter().any(|m| stderr.contains(m)) {
            return Err(RunError::Blocked(last_line));
        }
        if !output.status.success() {
            return Err(RunError::Failed(last_line));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// A finished download named `<id>.<ext>`; partial files are ignored.
    async fn find_clip(&self, id: &str) -> Option<PathBuf> {
        let mut entries = tokio::fs::read_dir(&self.dir).await.ok()?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let stem_matches = path.file_stem().is_some_and(|s| s == id);
            let finished = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "mp4" | "webm" | "mkv"));
            if stem_matches && finished {
                return Some(path);
            }
        }
        None
    }

    async fn backoff_until(&self) -> Option<SystemTime> {
        let secs: u64 = tokio::fs::read_to_string(self.dir.join(BACKOFF_FILE))
            .await
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let until = UNIX_EPOCH + Duration::from_secs(secs);
        (until > SystemTime::now()).then_some(until)
    }
}

/// The clip must name the track and the artist (in the title or as the
/// channel); official uploads win, lyric/audio-only uploads lose.
fn pick_clip<'a>(
    hits: &'a [SearchHit],
    artist: &str,
    title: &str,
    duration: Option<Duration>,
) -> Option<&'a SearchHit> {
    let (artist, title) = (norm(artist), norm(title));
    hits.iter()
        .filter(|h| {
            let name = norm(&h.title);
            let channel = norm(
                h.channel
                    .as_deref()
                    .or(h.uploader.as_deref())
                    .unwrap_or_default(),
            );
            name.contains(&title) && (name.contains(&artist) || channel.contains(&artist))
        })
        .filter(|h| match (duration, h.duration) {
            // Clips may add an intro or outro, but not be a different song or a compilation.
            (Some(want), Some(got)) => {
                (want.as_secs_f64() * 0.8..=want.as_secs_f64() * 2.0).contains(&got)
            }
            _ => true,
        })
        .max_by_key(|h| {
            let t = h.title.to_lowercase();
            let mut score = 0;
            if t.contains("official") || t.contains("oficiální") {
                score += 2;
            }
            if t.contains("video") || t.contains("klip") {
                score += 1;
            }
            for weak in ["lyric", "audio", "visualizer", "text", "reaction", "cover"] {
                if t.contains(weak) {
                    score -= 3;
                }
            }
            score
        })
}

async fn read_index(path: &Path) -> Option<IndexEntry> {
    let text = tokio::fs::read_to_string(path).await.ok()?;
    serde_json::from_str(&text).ok()
}

async fn write_index(path: &Path, entry: &IndexEntry) {
    if let Ok(json) = serde_json::to_string(entry) {
        let _ = tokio::fs::write(path, json).await;
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn age(unix_secs: u64) -> Duration {
    Duration::from_secs(unix_now().saturating_sub(unix_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: &str, title: &str, channel: &str, secs: f64) -> SearchHit {
        SearchHit {
            id: id.into(),
            title: title.into(),
            duration: Some(secs),
            channel: Some(channel.into()),
            uploader: None,
        }
    }

    #[test]
    fn prefers_official_clip_and_rejects_wrong_matches() {
        let hits = [
            hit("lyrics", "P T K - KRÁTKÁ (Lyrics)", "Fan Lyrics", 160.0),
            hit("official", "KRÁTKÁ (Official Video)", "P T K", 175.0),
            hit("other", "Úplně jiná písnička", "P T K", 160.0),
            hit("mix", "P T K - KRÁTKÁ | 1 hour mix", "Mixes", 3600.0),
        ];
        let best = pick_clip(&hits, "P T K", "KRÁTKÁ", Some(Duration::from_secs(160)));
        assert_eq!(best.map(|h| h.id.as_str()), Some("official"));
    }

    #[test]
    fn nothing_matching_means_no_clip() {
        let hits = [hit("x", "Something Else", "Someone", 200.0)];
        assert!(pick_clip(&hits, "P T K", "KRÁTKÁ", None).is_none());
    }
}
