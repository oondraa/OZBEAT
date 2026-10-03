//! Everything off the UI thread: sources, lookups, image loading, clips.
//! The UI only ever reads the latest [`Scene`].

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use mv_audio::{Capture, Input};
use mv_core::{Device, NowPlaying, PlaybackState, Registry, SourceId, Track};
use mv_enrich::{Enricher, Enrichment, ImageKind, matching};
use mv_sources::bluos;
use mv_video::{Outcome, VideoFetcher};
use tokio::sync::mpsc;

use crate::art::{self, Art};
use crate::settings::{AudioMode, SharedSettings};

const TICK: Duration = Duration::from_millis(100);
/// A track must stay active this long before we look it up, so skipping
/// through a playlist doesn't fire a burst of API requests.
const LOOKUP_DEBOUNCE: Duration = Duration::from_millis(1500);
/// YouTube is only asked about tracks that have really been played for a while.
const VIDEO_MIN_PLAYED: Duration = Duration::from_secs(20);
/// After a capture fails to open (no mic, permission denied), wait before retrying.
const AUDIO_RETRY: Duration = Duration::from_secs(30);

pub struct Options {
    pub bluos_hosts: Vec<String>,
    pub discover: bool,
    pub smtc: bool,
}

/// What the UI draws. Enrichment and art always belong to `now_playing`.
#[derive(Default)]
pub struct Scene {
    pub source: Option<SourceId>,
    pub now_playing: Option<NowPlaying>,
    pub enrichment: Option<Arc<Enrichment>>,
    pub art: Option<Arc<Art>>,
    pub video: Option<String>,
    /// The capture matching the active source; the UI polls it every frame.
    pub audio: Option<Capture>,
    /// Every device seen, enabled or not, for the setup screen.
    pub devices: Vec<Device>,
}

pub type SharedScene = Arc<Mutex<Scene>>;

pub async fn run(opts: Options, scene: SharedScene, settings: SharedSettings) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    #[cfg(windows)]
    if opts.smtc {
        mv_sources::smtc::spawn(tx.clone());
    }
    for host in &opts.bluos_hosts {
        bluos::spawn(host.clone(), tx.clone());
    }
    if opts.discover {
        let known: HashSet<String> = opts.bluos_hosts.iter().cloned().collect();
        if let Err(e) = bluos::discover(tx.clone(), known) {
            eprintln!("mdns discovery disabled: {e}");
        }
    }
    drop(tx);

    let enricher = Arc::new(Enricher::new(mv_enrich::Config::from_env()));
    let (enriched_tx, mut enriched_rx) = mpsc::unbounded_channel::<(String, Arc<Enrichment>)>();
    let (art_tx, mut art_rx) = mpsc::unbounded_channel::<(u64, ArtFor)>();
    let mut lookups = Lookups::default();
    let mut art: Option<ArtFor> = None;
    let mut art_plan = ArtPlan::default();
    let mut art_shown = 0;

    let mut fetcher: Option<Arc<VideoFetcher>> = None;
    let (video_tx, mut video_rx) = mpsc::unbounded_channel::<(String, Outcome)>();
    let mut videos = Videos::default();

    let mut audio = AudioInputs::default();

    let mut registry = Registry::default();
    let mut tick = tokio::time::interval(TICK);
    loop {
        tokio::select! {
            event = rx.recv() => match event {
                Some(event) => registry.apply(event, SystemTime::now()),
                None => {
                    eprintln!("no sources left");
                    return;
                }
            },
            Some((key, enrichment)) = enriched_rx.recv() => {
                // A song we have since moved on from.
                if lookups.requested.as_ref() != Some(&key) {
                    continue;
                }
                art_plan.track(&key);
                art_plan.found(&enrichment);
                art_plan.build(&enricher, &art_tx);
                lookups.done = Some((key, enrichment));
            }
            Some((seq, loaded)) = art_rx.recv() => {
                if seq > art_shown {
                    art_shown = seq;
                    art = Some(loaded);
                }
            }
            Some((key, outcome)) = video_rx.recv() => {
                videos.in_flight = None;
                videos.done = Some((key, outcome));
            }
            _ = tick.tick() => {
                // Settings can change any time from the setup screen.
                let settings = settings.lock().unwrap().clone();
                let active = registry.active_where(|d| settings.allows(d));
                if settings.youtube && fetcher.is_none() {
                    fetcher = Some(Arc::new(VideoFetcher::from_env()));
                }
                let youtube = fetcher.as_ref().filter(|_| settings.youtube);

                if let Some((_, np)) = active {
                    art_plan.track(&track_key(&np.track));
                    art_plan.player = np.track.art_url.clone();
                    art_plan.build(&enricher, &art_tx);
                    lookups.request(np, &enricher, &enriched_tx);
                    if let Some(fetcher) = youtube {
                        videos.request(np, fetcher, &video_tx);
                    }
                }
                let capture = audio.for_source(settings.audio, active.map(|(id, _)| id));
                publish(&scene, active, &lookups, art.as_ref(), youtube.map(|_| &videos));
                let mut scene = scene.lock().unwrap();
                scene.audio = capture;
                scene.devices = registry.devices();
            }
        }
    }
}

fn publish(
    scene: &SharedScene,
    active: Option<(&SourceId, &NowPlaying)>,
    lookups: &Lookups,
    art: Option<&(String, Arc<Art>)>,
    videos: Option<&Videos>,
) {
    let key = active.map(|(_, np)| track_key(&np.track));
    let for_track = |k: &String| key.as_ref() == Some(k);

    let mut scene = scene.lock().unwrap();
    scene.source = active.map(|(id, _)| id.clone());
    scene.now_playing = active.map(|(_, np)| np.clone());
    scene.enrichment = lookups
        .done
        .as_ref()
        .filter(|(k, _)| for_track(k))
        .map(|(_, e)| Arc::clone(e));
    scene.art = art
        .filter(|(k, _)| for_track(k))
        .map(|(_, a)| Arc::clone(a));
    scene.video = videos.zip(active).map(|(v, (_, np))| v.status(&np.track));
}

/// Opens captures lazily (the mic only once a speaker is actually playing) and keeps them.
#[derive(Default)]
struct AudioInputs {
    loopback: Option<Capture>,
    mic: Option<Capture>,
    failed: Vec<(Input, Instant)>,
}

impl AudioInputs {
    fn for_source(&mut self, mode: AudioMode, source: Option<&SourceId>) -> Option<Capture> {
        match mode {
            AudioMode::Off => None,
            AudioMode::Loopback => self.get(Input::Loopback),
            AudioMode::Mic => self.get(Input::Microphone),
            // Music from this computer can be tapped directly; a network
            // speaker's sound never passes through it, so listen instead.
            AudioMode::Auto => match source?.kind {
                "smtc" => {
                    // Spotify also shows a session while it only remote-controls
                    // another device; then the computer is silent and the room
                    // mic is the better bet.
                    let loopback = self.get(Input::Loopback);
                    if loopback.as_ref().is_some_and(|c| c.levels().signal) {
                        return loopback;
                    }
                    self.get(Input::Microphone)
                        .filter(|c| c.levels().signal)
                        .or(loopback)
                }
                _ => self.get(Input::Microphone),
            },
        }
    }

    /// The capture for `input`, opening it on first use.
    fn get(&mut self, input: Input) -> Option<Capture> {
        let slot = match input {
            Input::Loopback => &mut self.loopback,
            Input::Microphone => &mut self.mic,
        };
        if slot.is_none() {
            self.failed.retain(|(_, at)| at.elapsed() < AUDIO_RETRY);
            if self.failed.iter().any(|(i, _)| *i == input) {
                return None;
            }
            match Capture::start(input) {
                Ok(capture) => *slot = Some(capture),
                Err(e) => {
                    eprintln!("audio {input:?} unavailable: {e}");
                    self.failed.push((input, Instant::now()));
                }
            }
        }
        slot.clone()
    }
}

/// The images one [`Art`] is built from.
#[derive(Clone, Default, PartialEq)]
struct ArtUrls {
    cover: Option<String>,
    photo: Option<String>,
    background: Option<String>,
}

impl ArtUrls {
    fn of(enrichment: &Enrichment) -> Self {
        let url = |kind| enrichment.best(kind).map(|i| i.url.clone());
        Self {
            cover: url(ImageKind::Cover),
            photo: url(ImageKind::ArtistPhoto),
            background: url(ImageKind::Background),
        }
    }

    /// Keeps the images found first: a better match that arrives later only
    /// fills the gaps instead of swapping a picture mid-song.
    fn keeping(self, shown: &Self) -> Self {
        Self {
            cover: shown.cover.clone().or(self.cover),
            photo: shown.photo.clone().or(self.photo),
            background: shown.background.clone().or(self.background),
        }
    }
}

/// Picks the images for the current song and rebuilds the art when they change.
#[derive(Default)]
struct ArtPlan {
    key: String,
    /// The player's own cover. Shown at once, but it is often small, so a
    /// cover found by the lookups replaces it.
    player: Option<String>,
    found: ArtUrls,
    built: Option<ArtUrls>,
    /// Numbers the builds, so a slow early one can't replace a later one.
    seq: u64,
}

impl ArtPlan {
    fn track(&mut self, key: &str) {
        if self.key != key {
            *self = Self {
                key: key.to_owned(),
                seq: self.seq,
                ..Self::default()
            };
        }
    }

    fn found(&mut self, enrichment: &Enrichment) {
        self.found = ArtUrls::of(enrichment).keeping(&self.found);
    }

    fn build(&mut self, enricher: &Arc<Enricher>, tx: &mpsc::UnboundedSender<(u64, ArtFor)>) {
        let mut want = self.found.clone();
        want.cover = want.cover.or_else(|| self.player.clone());
        if want == ArtUrls::default() || self.built.as_ref() == Some(&want) {
            return;
        }
        self.seq += 1;
        spawn_art(
            self.seq,
            self.key.clone(),
            want.clone(),
            Arc::clone(enricher),
            tx.clone(),
        );
        self.built = Some(want);
    }
}

/// Art and the track key it belongs to.
type ArtFor = (String, Arc<Art>);

/// Downloads the cover, photo and background, then decodes them off the async threads.
fn spawn_art(
    seq: u64,
    key: String,
    urls: ArtUrls,
    enricher: Arc<Enricher>,
    tx: mpsc::UnboundedSender<(u64, ArtFor)>,
) {
    tokio::spawn(async move {
        let fetch = |url: Option<String>| {
            let enricher = &enricher;
            async move {
                match enricher.image(&url?).await {
                    Ok(path) => Some(path),
                    Err(e) => {
                        eprintln!("image download failed: {e}");
                        None
                    }
                }
            }
        };
        let (cover, photo, background) =
            tokio::join!(fetch(urls.cover), fetch(urls.photo), fetch(urls.background));
        if let Ok(art) =
            tokio::task::spawn_blocking(move || art::build(cover, photo, background)).await
        {
            let _ = tx.send((seq, (key, Arc::new(art))));
        }
    });
}

/// Tracks which song we want enriched, which one is in flight and the last result.
#[derive(Default)]
struct Lookups {
    candidate: Option<(String, Instant)>,
    requested: Option<String>,
    done: Option<(String, Arc<Enrichment>)>,
}

impl Lookups {
    fn request(
        &mut self,
        np: &NowPlaying,
        enricher: &Arc<Enricher>,
        tx: &mpsc::UnboundedSender<(String, Arc<Enrichment>)>,
    ) {
        let key = track_key(&np.track);
        if self.candidate.as_ref().is_none_or(|(k, _)| *k != key) {
            self.candidate = Some((key, Instant::now()));
            return;
        }
        let Some((key, since)) = &self.candidate else {
            return;
        };
        if since.elapsed() < LOOKUP_DEBOUNCE || self.requested.as_ref() == Some(key) {
            return;
        }
        self.requested = Some(key.clone());

        let (key, track, duration) = (key.clone(), np.track.clone(), np.duration);
        let (enricher, tx) = (Arc::clone(enricher), tx.clone());
        tokio::spawn(async move {
            let progress = |so_far| {
                let _ = tx.send((key.clone(), Arc::new(so_far)));
            };
            let enrichment = enricher.enrich(&track, duration, progress).await;
            let _ = tx.send((key, Arc::new(enrichment)));
        });
    }
}

/// One YouTube round at a time; when it finishes, whatever plays then is next.
#[derive(Default)]
struct Videos {
    in_flight: Option<String>,
    requested: HashSet<String>,
    done: Option<(String, Outcome)>,
}

impl Videos {
    fn request(
        &mut self,
        np: &NowPlaying,
        fetcher: &Arc<VideoFetcher>,
        tx: &mpsc::UnboundedSender<(String, Outcome)>,
    ) {
        let played_enough = np.state == PlaybackState::Playing
            && np
                .position_now(SystemTime::now())
                .is_some_and(|p| p >= VIDEO_MIN_PLAYED);
        let key = track_key(&np.track);
        let Some(artist) = np.track.artist.clone() else {
            return;
        };
        if !played_enough || self.in_flight.is_some() || self.requested.contains(&key) {
            return;
        }
        self.requested.insert(key.clone());
        self.in_flight = Some(key.clone());

        let (title, duration) = (np.track.title.clone(), np.duration);
        let (fetcher, tx) = (Arc::clone(fetcher), tx.clone());
        tokio::spawn(async move {
            let outcome = fetcher.fetch(&artist, &title, duration).await;
            let _ = tx.send((key, outcome));
        });
    }

    fn status(&self, track: &Track) -> String {
        let key = track_key(track);
        if self.in_flight.as_ref() == Some(&key) {
            return "searching / downloading (spaced out on purpose)".into();
        }
        match self.done.as_ref().filter(|(k, _)| *k == key) {
            Some((_, Outcome::Ready(path))) => format!("ready: {}", path.display()),
            Some((_, Outcome::NotFound)) => "no matching clip".into(),
            Some((_, Outcome::BackingOff { until })) => {
                let mins = until
                    .duration_since(SystemTime::now())
                    .unwrap_or_default()
                    .as_secs()
                    / 60;
                format!("youtube pushed back, pausing for {mins} more min")
            }
            Some((_, Outcome::Failed(reason))) => format!("failed: {reason}"),
            None if self.in_flight.is_some() => "queued behind another clip".into(),
            None => format!("waits until {}s played", VIDEO_MIN_PLAYED.as_secs()),
        }
    }
}

/// The same song reported by two sources (e.g. Spotify Connect to a BluOS
/// speaker) maps to the same key, so it is looked up once.
fn track_key(track: &Track) -> String {
    let artist = track
        .artist
        .as_deref()
        .map(matching::primary_artist)
        .unwrap_or_default();
    format!(
        "{}\u{1f}{}",
        matching::norm(artist),
        matching::norm(&track.title)
    )
}
