//! A deliberately polite HTTP client: the reason we never see bans or captchas.
//!
//! - every response (including "not found") is cached on disk, so a track is
//!   looked up once, not every time it plays
//! - requests to one host are spaced by a per-API minimum interval
//! - 429/503/403 (or an API-specific "quota" body) puts the host on a growing
//!   backoff; while it lasts we don't touch it and serve stale cache instead

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::RETRY_AFTER;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = concat!(
    "musicvisual/",
    env!("CARGO_PKG_VERSION"),
    " (personal music visualizer)"
);
/// "Not found" expires sooner than hits: new releases get indexed eventually.
const MISSING_TTL: Duration = Duration::from_secs(3 * 86_400);
const BASE_BACKOFF: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(3600);
/// Pauses before asking a busy (503) server again, before backing off.
const BUSY_RETRIES: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];

#[derive(Clone, Copy)]
pub(crate) struct Policy {
    /// Minimum gap between two requests to the same host.
    pub min_interval: Duration,
    /// How long a cached response stays fresh.
    pub ttl: Duration,
    /// For APIs that signal rate limiting in a 200 body (looking at you, Deezer).
    pub is_throttled: fn(&str) -> bool,
}

pub(crate) fn never_throttled(_: &str) -> bool {
    false
}

#[derive(Debug)]
pub enum FetchError {
    /// We are voluntarily staying away from this host after it pushed back.
    Backoff {
        host: String,
        retry_in: Duration,
    },
    Throttled {
        host: String,
    },
    Network(String),
    Status(StatusCode),
    Parse(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backoff { host, retry_in } => {
                write!(f, "{host}: backing off for {}s", retry_in.as_secs())
            }
            Self::Throttled { host } => write!(f, "{host}: rate limited, backing off"),
            Self::Network(e) => write!(f, "network: {e}"),
            Self::Status(s) => write!(f, "http {s}"),
            Self::Parse(e) => write!(f, "unexpected response: {e}"),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    fetched_at: u64,
    /// `None` records a 404, so we don't ask again.
    body: Option<String>,
}

impl CacheEntry {
    fn is_fresh(&self, ttl: Duration) -> bool {
        let ttl = if self.body.is_some() {
            ttl
        } else {
            ttl.min(MISSING_TTL)
        };
        unix_now().saturating_sub(self.fetched_at) < ttl.as_secs()
    }
}

struct HostState {
    next_slot: Instant,
    blocked_until: Option<Instant>,
    strikes: u32,
}

pub(crate) struct Http {
    client: reqwest::Client,
    cache_dir: PathBuf,
    hosts: Mutex<HashMap<String, HostState>>,
}

impl Http {
    pub fn new(cache_dir: PathBuf) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(20))
            .build()
            .expect("failed to build http client");
        Self {
            client,
            cache_dir,
            hosts: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the response body, or `None` if the resource does not exist.
    pub async fn get_text(&self, url: &Url, policy: &Policy) -> Result<Option<String>, FetchError> {
        let path = self
            .cache_dir
            .join(format!("{:016x}.json", fnv1a(url.as_str())));
        let cached: Option<CacheEntry> = tokio::fs::read_to_string(&path)
            .await
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        if let Some(entry) = &cached
            && entry.is_fresh(policy.ttl)
        {
            return Ok(entry.body.clone());
        }

        match self.fetch(url, policy).await {
            Ok(body) => {
                let entry = CacheEntry {
                    fetched_at: unix_now(),
                    body: body.map(|b| String::from_utf8_lossy(&b).into_owned()),
                };
                self.store(path, &entry).await;
                Ok(entry.body)
            }
            // Stale data beats no data, and beats hammering a host that pushed back.
            Err(e) => cached.map(|c| c.body).ok_or(e),
        }
    }

    /// Downloads a file once and keeps it: image URLs are content-addressed,
    /// so a cached copy never goes stale. A cached copy's modified time is
    /// bumped on every use, so cache trimming removes the least used first.
    pub async fn get_file(&self, url: &Url, policy: &Policy) -> Result<PathBuf, FetchError> {
        let path = self
            .cache_dir
            .join("files")
            .join(format!("{:016x}", fnv1a(url.as_str())));
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            let touched = path.clone();
            tokio::task::spawn_blocking(move || touch(&touched));
            return Ok(path);
        }
        let bytes = self
            .fetch(url, policy)
            .await?
            .ok_or(FetchError::Status(StatusCode::NOT_FOUND))?;
        let tmp = path.with_extension("tmp");
        let written = async {
            tokio::fs::create_dir_all(path.parent().expect("has parent")).await?;
            tokio::fs::write(&tmp, &bytes).await?;
            tokio::fs::rename(&tmp, &path).await
        }
        .await;
        written.map_err(|e| FetchError::Network(format!("cache write: {e}")))?;
        Ok(path)
    }

    async fn fetch(&self, url: &Url, policy: &Policy) -> Result<Option<Vec<u8>>, FetchError> {
        let host = url.host_str().unwrap_or_default().to_owned();
        let mut busy_retries = BUSY_RETRIES.iter();
        let response = loop {
            self.wait_turn(&host, policy.min_interval).await?;
            let response = self
                .client
                .get(url.clone())
                .send()
                .await
                .map_err(|e| FetchError::Network(e.to_string()))?;
            // A busy server (LRCLIB often is) usually answers a moment later;
            // backing off right away would cost every track for a while.
            match busy_retries.next() {
                Some(pause)
                    if response.status() == StatusCode::SERVICE_UNAVAILABLE
                        && !response.headers().contains_key(RETRY_AFTER) =>
                {
                    tokio::time::sleep(*pause).await;
                }
                _ => break response,
            }
        };
        let status = response.status();

        if status == StatusCode::NOT_FOUND {
            self.forgive(&host);
            return Ok(None);
        }
        if matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE | StatusCode::FORBIDDEN
        ) {
            let retry_after = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            self.penalize(&host, retry_after);
            return Err(FetchError::Throttled { host });
        }
        if !status.is_success() {
            return Err(FetchError::Status(status));
        }

        let body = response
            .bytes()
            .await
            .map_err(|e| FetchError::Network(e.to_string()))?
            .to_vec();
        if (policy.is_throttled)(&String::from_utf8_lossy(&body)) {
            self.penalize(&host, None);
            return Err(FetchError::Throttled { host });
        }
        self.forgive(&host);
        Ok(Some(body))
    }

    /// Reserves the next free slot for `host` and sleeps until it comes.
    async fn wait_turn(&self, host: &str, min_interval: Duration) -> Result<(), FetchError> {
        let wait = {
            let now = Instant::now();
            let mut hosts = self.hosts.lock().unwrap();
            let state = hosts.entry(host.to_owned()).or_insert(HostState {
                next_slot: now,
                blocked_until: None,
                strikes: 0,
            });
            if let Some(until) = state.blocked_until
                && until > now
            {
                return Err(FetchError::Backoff {
                    host: host.to_owned(),
                    retry_in: until - now,
                });
            }
            let slot = state.next_slot.max(now);
            state.next_slot = slot + min_interval;
            slot - now
        };
        tokio::time::sleep(wait).await;
        Ok(())
    }

    fn penalize(&self, host: &str, retry_after: Option<Duration>) {
        let mut hosts = self.hosts.lock().unwrap();
        if let Some(state) = hosts.get_mut(host) {
            state.strikes += 1;
            let exponential = BASE_BACKOFF * 2u32.saturating_pow(state.strikes - 1);
            let backoff = retry_after.unwrap_or(exponential).min(MAX_BACKOFF);
            state.blocked_until = Some(Instant::now() + backoff);
        }
    }

    fn forgive(&self, host: &str) {
        if let Some(state) = self.hosts.lock().unwrap().get_mut(host) {
            state.strikes = 0;
            state.blocked_until = None;
        }
    }

    /// Best effort: a failed cache write only costs a future request.
    async fn store(&self, path: PathBuf, entry: &CacheEntry) {
        let Ok(json) = serde_json::to_string(entry) else {
            return;
        };
        let tmp = path.with_extension("tmp");
        if tokio::fs::create_dir_all(&self.cache_dir).await.is_ok()
            && tokio::fs::write(&tmp, json).await.is_ok()
        {
            let _ = tokio::fs::rename(&tmp, &path).await;
        }
    }
}

/// Best effort: a file that is not touched is only trimmed a little sooner.
pub(crate) fn touch(path: &std::path::Path) {
    let _ = std::fs::File::options()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(SystemTime::now()));
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Stable across Rust versions, unlike `DefaultHasher`; cache file names depend on it.
fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}
