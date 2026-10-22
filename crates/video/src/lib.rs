//! Optional music videos from YouTube, through `yt-dlp` (see [`ytdlp`]).
//!
//! YouTube's terms forbid downloading, so this is off by default and kept
//! apart from the API-based `mv-enrich`. It is deliberately slow so the
//! user's IP never gets flagged:
//! - picture only (the audio comes from the speaker), at most 1080p
//! - every clip is downloaded once and kept; "no clip" is remembered too
//! - at least [`MIN_GAP`] between two rounds of talking to YouTube
//! - a bot check or 429 silences the module for [`BACKOFF`], across restarts
//!
//! The app plays a clip along with the song's position. Its edit can differ
//! from the album version (an intro, a skit), and that's fine. Labels often
//! put several songs in one video (Milion Plus: "Melanž" holds El Camino,
//! Hroby and Rolls); its chapters say where each song is.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mv_enrich::matching::{norm, primary_artist};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

mod remux;
mod ytdlp;

pub const MIN_GAP: Duration = Duration::from_secs(180);
pub const BACKOFF: Duration = Duration::from_secs(6 * 3600);
const NOT_FOUND_TTL: Duration = Duration::from_secs(14 * 86_400);
const SEARCH_RESULTS: u32 = 5;
/// How many videos of several songs to open looking for the song's chapter.
const ALBUM_CANDIDATES: usize = 2;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// Video-only stream, so no merging (and no ffmpeg) is needed; H.264 in MP4
/// is what Windows decodes everywhere.
const FORMAT: &str = "bv*[height<=1080][vcodec^=avc1][ext=mp4]/bv*[height<=1080][ext=mp4]";
/// yt-dlp messages meaning "YouTube is onto us": stop, don't retry.
const BLOCK_MARKERS: &[&str] = &["Sign in to confirm", "HTTP Error 429", "Too Many Requests"];
const BACKOFF_FILE: &str = "backoff_until";

/// A downloaded video, or the part of one that is the song.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub path: PathBuf,
    /// Where the song starts in the video.
    pub start: Duration,
    /// Where its part ends, when the video goes on with other songs.
    pub end: Option<Duration>,
}

#[derive(Debug, Clone)]
pub enum Outcome {
    Ready(Clip),
    NotFound,
    BackingOff { until: SystemTime },
    Failed(String),
}

#[derive(Serialize, Deserialize)]
struct IndexEntry {
    checked_at: u64,
    video_id: Option<String>,
    /// The song's chapter in the video, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    part: Option<(f64, f64)>,
}

impl IndexEntry {
    fn clip(&self, path: PathBuf) -> Clip {
        Clip {
            path,
            start: self.part.map_or(Duration::ZERO, |(start, _)| secs(start)),
            end: self.part.map(|(_, end)| secs(end)),
        }
    }
}

/// A video found for a song: its id, and its chapter when it holds several songs.
#[derive(Debug, Clone, PartialEq)]
struct Found {
    id: String,
    part: Option<(f64, f64)>,
}

#[derive(Deserialize)]
struct VideoInfo {
    #[serde(default)]
    chapters: Option<Vec<Chapter>>,
}

#[derive(Deserialize)]
struct Chapter {
    start_time: f64,
    end_time: f64,
    title: String,
}

#[derive(Deserialize)]
struct SearchHit {
    id: String,
    title: String,
    duration: Option<f64>,
    channel: Option<String>,
    uploader: Option<String>,
    channel_is_verified: Option<bool>,
    view_count: Option<u64>,
}

enum RunError {
    Blocked(String),
    Failed(String),
}

pub struct VideoFetcher {
    dir: PathBuf,
    /// Held for the whole YouTube round, so yt-dlp never runs twice at once.
    last_round: Mutex<Option<Instant>>,
}

impl VideoFetcher {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            last_round: Mutex::new(None),
        }
    }

    /// Cache in the user cache dir; `MV_YTDLP` overrides the yt-dlp binary.
    pub fn from_env() -> Self {
        let dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("musicvisual")
            .join("video");
        Self::new(dir)
    }

    /// The clip downloaded earlier for this song, without talking to YouTube.
    pub async fn cached(&self, artist: &str, title: &str) -> Option<Clip> {
        let artist = primary_artist(artist);
        let index_path = self
            .dir
            .join(format!("{}_{}.json", norm(artist), norm(title)));
        let entry = read_index(&index_path).await?;
        let path = self.find_clip(entry.video_id.as_deref()?).await?;
        Some(entry.clip(path))
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
            Some(entry) if entry.video_id.is_some() => {
                let id = entry.video_id.clone().unwrap_or_default();
                if let Some(path) = self.find_clip(&id).await {
                    return Outcome::Ready(entry.clip(path));
                }
                // Found earlier, the download didn't finish.
                Some(Found {
                    id,
                    part: entry.part,
                })
            }
            Some(entry) if age(entry.checked_at) < NOT_FOUND_TTL => {
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
        known_id: Option<Found>,
        artist: &str,
        title: &str,
        duration: Option<Duration>,
        index_path: &Path,
    ) -> Outcome {
        let result = async {
            let ytdlp = ytdlp::resolve(&self.dir).await.map_err(RunError::Failed)?;
            let ytdlp = ytdlp.as_path();
            let found = match known_id {
                Some(found) => found,
                None => {
                    let found = self.search(ytdlp, artist, title, duration).await?;
                    let entry = IndexEntry {
                        checked_at: unix_now(),
                        video_id: found.as_ref().map(|f| f.id.clone()),
                        part: found.as_ref().and_then(|f| f.part),
                    };
                    write_index(index_path, &entry).await;
                    match found {
                        Some(found) => found,
                        None => return Ok(Outcome::NotFound),
                    }
                }
            };
            // Another song of the same video may have brought it already.
            let path = match self.find_clip(&found.id).await {
                Some(path) => path,
                None => self.download(ytdlp, &found.id).await?,
            };
            let entry = IndexEntry {
                checked_at: 0,
                video_id: None,
                part: found.part,
            };
            Ok(Outcome::Ready(entry.clip(path)))
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
        ytdlp: &Path,
        artist: &str,
        title: &str,
        duration: Option<Duration>,
    ) -> Result<Option<Found>, RunError> {
        let query = format!("ytsearch{SEARCH_RESULTS}:{artist} {title} official video");
        let out = self
            .run(
                ytdlp,
                &["--flat-playlist", "--dump-json", "--no-warnings", &query],
                SEARCH_TIMEOUT,
            )
            .await?;
        let hits: Vec<SearchHit> = out
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        if let Some(hit) = pick_clip(&hits, artist, title, duration) {
            return Ok(Some(Found {
                id: hit.id.clone(),
                part: None,
            }));
        }
        // No video of its own: maybe one of several songs, found by its chapters.
        for hit in album_candidates(&hits, artist, duration) {
            let url = format!("https://www.youtube.com/watch?v={}", hit.id);
            let out = self
                .run(
                    ytdlp,
                    &["--skip-download", "--dump-json", "--no-warnings", "--no-playlist", &url],
                    SEARCH_TIMEOUT,
                )
                .await?;
            let chapters = serde_json::from_str::<VideoInfo>(out.trim())
                .ok()
                .and_then(|info| info.chapters)
                .unwrap_or_default();
            if let Some(part) = find_part(&chapters, title) {
                return Ok(Some(Found {
                    id: hit.id.clone(),
                    part: Some(part),
                }));
            }
        }
        Ok(None)
    }

    /// Downloads as `<id>.dash.mp4`, then rewrites it as `<id>.mp4` (see [`remux`]).
    async fn download(&self, ytdlp: &Path, id: &str) -> Result<PathBuf, RunError> {
        let template = self.dir.join(format!("{id}.dash.%(ext)s"));
        let url = format!("https://www.youtube.com/watch?v={id}");
        let out = self
            .run(
                ytdlp,
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
        let dash = match path {
            Some(path) if tokio::fs::try_exists(&path).await.unwrap_or(false) => path,
            _ => {
                return Err(RunError::Failed(
                    "yt-dlp finished but the file is missing".into(),
                ));
            }
        };
        let plain = self.dir.join(format!("{id}.mp4"));
        let part = self.dir.join(format!("{id}.part"));
        let rewritten = {
            let (dash, part) = (dash.clone(), part.clone());
            tokio::task::spawn_blocking(move || {
                let data = std::fs::read(&dash).map_err(|e| e.to_string())?;
                let plain = remux::defragment(&data)?;
                std::fs::write(&part, plain).map_err(|e| e.to_string())
            })
            .await
            .map_err(|e| e.to_string())
            .flatten()
        };
        let _ = tokio::fs::remove_file(&dash).await;
        rewritten
            .and_then(|()| std::fs::rename(&part, &plain).map_err(|e| e.to_string()))
            .map(|()| plain)
            .map_err(|e| RunError::Failed(format!("cannot rewrite the clip: {e}")))
    }

    async fn run(&self, ytdlp: &Path, args: &[&str], timeout: Duration) -> Result<String, RunError> {
        let child = ytdlp::command(ytdlp)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| RunError::Failed(format!("cannot start {}: {e}", ytdlp.display())))?;
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

    /// A finished clip named `<id>.mp4`; partial files are ignored.
    async fn find_clip(&self, id: &str) -> Option<PathBuf> {
        let mut entries = tokio::fs::read_dir(&self.dir).await.ok()?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.file_name().is_some_and(|n| *n == *format!("{id}.mp4")) {
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

/// Whether `channel` is one to take clips from: the artist's own, a verified
/// one (labels) or a VEVO one. Reuploads titled "official video" are common
/// and look bad (watermarks, low resolution), so other channels never count.
fn trusted(hit: &SearchHit, artist: &str) -> bool {
    let channel = norm(hit.channel.as_deref().or(hit.uploader.as_deref()).unwrap_or_default());
    // "Artist - Topic" channels hold the audio over the cover.
    let topic = hit.channel.as_deref().is_some_and(|c| c.ends_with(" - Topic"));
    !topic
        && (hit.channel_is_verified == Some(true)
            || channel.contains(artist)
            || channel.contains("vevo"))
}

/// Trusted videos by the artist that are longer than the song, so may hold
/// it among others; the most watched first.
fn album_candidates<'a>(
    hits: &'a [SearchHit],
    artist: &str,
    duration: Option<Duration>,
) -> Vec<&'a SearchHit> {
    let artist = norm(artist);
    let mut found: Vec<_> = hits
        .iter()
        .filter(|h| trusted(h, &artist) && norm(&h.title).contains(&artist))
        .filter(|h| match (duration, h.duration) {
            (Some(want), Some(got)) => got > want.as_secs_f64() * 1.2,
            _ => true,
        })
        .collect();
    found.sort_by_key(|h| std::cmp::Reverse(h.view_count.unwrap_or(0)));
    found.truncate(ALBUM_CANDIDATES);
    found
}

/// The chapter named after the song ("2.) El Camino (prod. Decky)"), in seconds.
fn find_part(chapters: &[Chapter], title: &str) -> Option<(f64, f64)> {
    let title = norm(title);
    chapters
        .iter()
        .find(|c| !title.is_empty() && norm(&c.title).contains(&title))
        .map(|c| (c.start_time, c.end_time))
}

/// The clip must name the track and the artist (in the title or as the
/// channel), and come from a [`trusted`] channel. Official uploads win,
/// lyric/audio-only uploads lose, then the most watched.
fn pick_clip<'a>(
    hits: &'a [SearchHit],
    artist: &str,
    title: &str,
    duration: Option<Duration>,
) -> Option<&'a SearchHit> {
    let (artist, title) = (norm(artist), norm(title));
    hits.iter()
        .filter(|h| {
            // Hashtags name other songs ("NEJSEM #andez").
            let words = h.title.split_whitespace().filter(|w| !w.starts_with('#'));
            let name = norm(&words.collect::<Vec<_>>().join(" "));
            let channel = norm(
                h.channel
                    .as_deref()
                    .or(h.uploader.as_deref())
                    .unwrap_or_default(),
            );
            trusted(h, &artist)
                && name.contains(&title)
                && (name.contains(&artist) || channel.contains(&artist))
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
            (score, h.view_count.unwrap_or(0))
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

fn secs(s: f64) -> Duration {
    Duration::from_secs_f64(s.max(0.0))
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
            channel_is_verified: None,
            view_count: None,
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
    fn skips_reuploads_from_other_channels() {
        let mut label = hit("label", "Yzomandias - Jedna Dva (official music video)", "Milion Plus", 259.0);
        label.channel_is_verified = Some(true);
        let hits = [
            hit("reupload", "Yzomandias - El Camino (official music video)", "AP music", 163.0),
            hit("topic", "El Camino", "Yzomandias - Topic", 160.0),
            label,
            hit("tagged", "YZOMANDIAS - NEJSEM #andez", "Yzomandias", 180.0),
        ];
        assert!(pick_clip(&hits, "Yzomandias", "El Camino", None).is_none());
        assert!(pick_clip(&hits, "Yzomandias", "Andez", None).is_none());
        let best = pick_clip(&hits, "Yzomandias", "Jedna Dva", None);
        assert_eq!(best.map(|h| h.id.as_str()), Some("label"));
    }

    #[test]
    fn finds_a_song_among_several_in_one_video() {
        let mut album = hit("melanz", "Yzomandias - Melanž feat. Nik Tendo (official music video)", "Milion Plus", 385.0);
        album.channel_is_verified = Some(true);
        let hits = [
            hit("reupload", "Yzomandias - El Camino (official music video)", "AP music", 163.0),
            album,
        ];
        let song = Some(Duration::from_secs(150));
        let ids: Vec<_> = album_candidates(&hits, "Yzomandias", song).iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, ["melanz"]);

        let chapter = |start: f64, end: f64, title: &str| Chapter {
            start_time: start,
            end_time: end,
            title: title.into(),
        };
        let chapters = [
            chapter(0.0, 25.0, "1.) Melanž (intro)"),
            chapter(25.0, 170.0, "2.) El Camino (prod. Decky)"),
            chapter(170.0, 205.0, "3.) Hroby (prod. Konex & Yzomandias)"),
        ];
        assert_eq!(find_part(&chapters, "El Camino"), Some((25.0, 170.0)));
        assert_eq!(find_part(&chapters, "Rolls"), None);
    }

    #[test]
    fn nothing_matching_means_no_clip() {
        let hits = [hit("x", "Something Else", "Someone", 200.0)];
        assert!(pick_clip(&hits, "P T K", "KRÁTKÁ", None).is_none());
    }
}
