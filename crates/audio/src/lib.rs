//! Audio capture and analysis: turns what the computer plays (loopback) or
//! what a microphone hears into smoothed band levels and a soft beat envelope.
//!
//! Everything is smoothed with a fast attack and a slow release, so the
//! visuals swell with the music instead of flickering on every transient.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

const FFT_SIZE: usize = 2048;
const FRAME: Duration = Duration::from_millis(16);
/// Band edges in Hz.
const BASS: (f32, f32) = (30.0, 150.0);
const MID: (f32, f32) = (150.0, 2000.0);
const TREBLE: (f32, f32) = (2000.0, 10000.0);
/// Levels map the loudest recent `DYNAMIC_RANGE_DB` to 0..1.
const DYNAMIC_RANGE_DB: f32 = 36.0;
/// How fast the remembered peak falls when the music gets quieter.
const PEAK_DECAY_DB_PER_SEC: f32 = 4.0;
const ATTACK: Duration = Duration::from_millis(60);
const RELEASE: Duration = Duration::from_millis(350);
const BEAT_RELEASE: Duration = Duration::from_millis(300);
/// Faster than this is not a beat but a roll or noise.
const MIN_BEAT_GAP: Duration = Duration::from_millis(280);
const SILENCE_RMS: f32 = 1e-4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// What this computer is playing.
    Loopback,
    /// The default microphone (for music coming out of other speakers).
    Microphone,
}

/// Current analysis, all 0..1 and already smoothed.
#[derive(Debug, Clone, Copy, Default)]
pub struct Levels {
    pub bass: f32,
    pub mid: f32,
    pub treble: f32,
    /// Jumps toward 1 on a detected beat, then eases back down.
    pub beat: f32,
    /// False while the input is (practically) silent.
    pub signal: bool,
    /// Raw, unsmoothed loudness of the last block (for level meters).
    pub rms: f32,
}

/// A running capture. Cheap to clone; capture stops when the last clone drops.
#[derive(Clone)]
pub struct Capture {
    inner: Arc<Inner>,
}

struct Inner {
    input: Input,
    device: Mutex<String>,
    levels: Mutex<Levels>,
    stop: AtomicBool,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Capture {
    pub fn start(input: Input) -> Result<Self, String> {
        let inner = Arc::new(Inner {
            input,
            device: Mutex::new(String::new()),
            levels: Mutex::new(Levels::default()),
            stop: AtomicBool::new(false),
        });
        let (ready_tx, ready_rx) = mpsc::channel();
        // The worker only holds a weak reference, so dropping every Capture ends it.
        let weak = Arc::downgrade(&inner);
        thread::Builder::new()
            .name(format!("audio-{input:?}").to_lowercase())
            .spawn(move || worker(input, weak, ready_tx))
            .map_err(|e| e.to_string())?;
        let device = ready_rx
            .recv()
            .map_err(|_| "audio thread died".to_owned())??;
        *inner.device.lock().unwrap() = device;
        Ok(Self { inner })
    }

    pub fn input(&self) -> Input {
        self.inner.input
    }

    /// Name of the captured device, e.g. "Microphone (Realtek Audio)".
    pub fn device(&self) -> String {
        self.inner.device.lock().unwrap().clone()
    }

    pub fn levels(&self) -> Levels {
        *self.inner.levels.lock().unwrap()
    }
}

/// Owns the stream (not `Send` on every platform) and runs the analysis loop.
fn worker(
    input: Input,
    inner: std::sync::Weak<Inner>,
    ready: mpsc::Sender<Result<String, String>>,
) {
    let samples = Arc::new(Mutex::new(VecDeque::with_capacity(FFT_SIZE * 2)));
    let (stream, sample_rate, device) = match open_stream(input, Arc::clone(&samples)) {
        Ok(opened) => opened,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(e.to_string()));
        return;
    }
    let _ = ready.send(Ok(device));

    let mut analyzer = Analyzer::new(sample_rate);
    let mut buffer = vec![0.0f32; FFT_SIZE];
    loop {
        thread::sleep(FRAME);
        let Some(inner) = inner.upgrade() else { return };
        if inner.stop.load(Ordering::Relaxed) {
            return;
        }
        {
            let samples = samples.lock().unwrap();
            let skip = FFT_SIZE.saturating_sub(samples.len());
            buffer[..skip].fill(0.0);
            for (dst, src) in buffer[skip..]
                .iter_mut()
                .zip(samples.iter().rev().take(FFT_SIZE).rev())
            {
                *dst = *src;
            }
        }
        let levels = analyzer.process(&buffer);
        *inner.levels.lock().unwrap() = levels;
    }
}

fn open_stream(
    input: Input,
    samples: Arc<Mutex<VecDeque<f32>>>,
) -> Result<(cpal::Stream, f32, String), String> {
    let host = cpal::default_host();
    // An input stream on an *output* device is loopback (WASAPI, CoreAudio on macOS 14.6+).
    let (device, config) = match input {
        Input::Loopback => {
            let device = host.default_output_device().ok_or("no output device")?;
            let config = device.default_output_config().map_err(|e| e.to_string())?;
            (device, config)
        }
        Input::Microphone => {
            let device = host.default_input_device().ok_or("no microphone")?;
            let config = device.default_input_config().map_err(|e| e.to_string())?;
            (device, config)
        }
    };
    let name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unknown device".into());
    let sample_rate = config.sample_rate() as f32;
    let channels = usize::from(config.channels());
    let format = config.sample_format();
    let config = config.config();

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, channels, samples),
        SampleFormat::I16 => build::<i16>(&device, config, channels, samples),
        SampleFormat::I32 => build::<i32>(&device, config, channels, samples),
        SampleFormat::U16 => build::<u16>(&device, config, channels, samples),
        other => return Err(format!("unsupported sample format {other}")),
    }?;
    Ok((stream, sample_rate, name))
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    channels: usize,
    samples: Arc<Mutex<VecDeque<f32>>>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                let mut samples = samples.lock().unwrap();
                for frame in data.chunks(channels.max(1)) {
                    let mono = frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>()
                        / frame.len() as f32;
                    samples.push_back(mono);
                }
                let excess = samples.len().saturating_sub(FFT_SIZE * 2);
                samples.drain(..excess);
            },
            |e| {
                // Xruns are harmless glitches (e.g. when playback pauses); only report real errors.
                if e.kind() != cpal::ErrorKind::Xrun {
                    eprintln!("audio stream error: {e}");
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}

struct Band {
    bins: (usize, usize),
    peak_db: f32,
    level: f32,
}

struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    bands: [Band; 3],
    last_frame: Instant,
    previous_bass: f32,
    flux_history: VecDeque<f32>,
    last_beat: Instant,
    beat: f32,
    silent_since: Option<Instant>,
}

impl Analyzer {
    fn new(sample_rate: f32) -> Self {
        let bin =
            |hz: f32| ((hz / sample_rate * FFT_SIZE as f32) as usize).clamp(1, FFT_SIZE / 2 - 1);
        let band = |(lo, hi): (f32, f32)| Band {
            bins: (bin(lo), bin(hi).max(bin(lo) + 1)),
            peak_db: -60.0,
            level: 0.0,
        };
        let window = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        Self {
            fft: FftPlanner::new().plan_fft_forward(FFT_SIZE),
            window,
            spectrum: vec![Complex::default(); FFT_SIZE],
            bands: [band(BASS), band(MID), band(TREBLE)],
            last_frame: Instant::now(),
            previous_bass: 0.0,
            flux_history: VecDeque::with_capacity(64),
            last_beat: Instant::now(),
            beat: 0.0,
            silent_since: None,
        }
    }

    fn process(&mut self, samples: &[f32]) -> Levels {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().clamp(0.001, 0.1);
        self.last_frame = now;

        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        if rms < SILENCE_RMS {
            self.silent_since.get_or_insert(now);
        } else {
            self.silent_since = None;
        }
        let signal = self
            .silent_since
            .is_none_or(|t| now - t < Duration::from_secs(1));

        for (dst, (s, w)) in self
            .spectrum
            .iter_mut()
            .zip(samples.iter().zip(&self.window))
        {
            *dst = Complex::new(s * w, 0.0);
        }
        self.fft.process(&mut self.spectrum);

        let mut energies = [0.0f32; 3];
        for (band, energy) in self.bands.iter_mut().zip(&mut energies) {
            let (lo, hi) = band.bins;
            *energy = self.spectrum[lo..hi]
                .iter()
                .map(|c| c.norm_sqr())
                .sum::<f32>()
                / (hi - lo) as f32;
            let db = 10.0 * (*energy + 1e-12).log10();
            band.peak_db = (band.peak_db - PEAK_DECAY_DB_PER_SEC * dt).max(db);
            let target = if signal {
                ((db - (band.peak_db - DYNAMIC_RANGE_DB)) / DYNAMIC_RANGE_DB).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let tau = if target > band.level { ATTACK } else { RELEASE };
            band.level += (target - band.level) * smoothing(dt, tau);
        }

        // Beats: bass energy rising well above its recent behaviour.
        let bass = energies[0].sqrt();
        let flux = (bass - self.previous_bass).max(0.0);
        self.previous_bass = bass;
        if self.flux_history.len() == 43 {
            self.flux_history.pop_front();
        }
        self.flux_history.push_back(flux);
        let n = self.flux_history.len() as f32;
        let mean = self.flux_history.iter().sum::<f32>() / n;
        let std = (self
            .flux_history
            .iter()
            .map(|f| (f - mean).powi(2))
            .sum::<f32>()
            / n)
            .sqrt();
        let is_beat = signal && n >= 20.0 && flux > mean + 1.5 * std && flux > 0.0;
        if is_beat && now - self.last_beat >= MIN_BEAT_GAP {
            self.last_beat = now;
            self.beat = 1.0;
        } else {
            self.beat -= self.beat * smoothing(dt, BEAT_RELEASE);
        }

        Levels {
            bass: self.bands[0].level,
            mid: self.bands[1].level,
            treble: self.bands[2].level,
            beat: self.beat,
            signal,
            rms,
        }
    }
}

/// Fraction to move toward a target this frame for time constant `tau`.
fn smoothing(dt: f32, tau: Duration) -> f32 {
    1.0 - (-dt / tau.as_secs_f32()).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, amp: f32, sample_rate: f32) -> Vec<f32> {
        (0..FFT_SIZE)
            .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / sample_rate).sin())
            .collect()
    }

    #[test]
    fn bass_tone_lights_up_bass_band_only() {
        let mut analyzer = Analyzer::new(48_000.0);
        let mut levels = Levels::default();
        for _ in 0..60 {
            analyzer.last_frame = Instant::now() - FRAME;
            levels = analyzer.process(&tone(80.0, 0.5, 48_000.0));
        }
        assert!(levels.signal);
        assert!(levels.bass > 0.8, "bass {}", levels.bass);
        assert!(levels.treble < 0.3, "treble {}", levels.treble);
    }

    #[test]
    fn silence_reports_no_signal() {
        let mut analyzer = Analyzer::new(48_000.0);
        analyzer.silent_since = Some(Instant::now() - Duration::from_secs(2));
        let levels = analyzer.process(&vec![0.0; FFT_SIZE]);
        assert!(!levels.signal);
        assert_eq!(levels.bass, 0.0);
    }
}
