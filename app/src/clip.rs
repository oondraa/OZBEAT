//! Plays the song's YouTube clip, picture only: the sound comes from the
//! player, so the clip follows the song's position instead of a clock of its
//! own. When the song is one part of a longer video, only that part plays.
//! Decoded on a thread of its own by Windows' Media Foundation.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use mv_video::Clip;

/// Further apart than this, the clip seeks instead of decoding its way there.
const SEEK_AHEAD: Duration = Duration::from_secs(2);
/// A clip this far ahead of the song (the song was seeked back) seeks too.
const SEEK_BACK: Duration = Duration::from_millis(500);
/// How often the decoder looks again while it waits for a frame's time.
const IDLE: Duration = Duration::from_millis(8);

/// Where the song is: its position when last told, and whether it moves on.
#[derive(Clone, Copy)]
struct Clock {
    position: Duration,
    at: Instant,
    playing: bool,
}

impl Clock {
    fn now(&self) -> Duration {
        match self.playing {
            true => self.position + self.at.elapsed(),
            false => self.position,
        }
    }
}

struct Frame {
    size: [usize; 2],
    pixels: Vec<Color32>,
}

#[derive(Default)]
struct Shared {
    clock: Mutex<Option<Clock>>,
    /// The newest frame due, taken by the UI.
    frame: Mutex<Option<Frame>>,
    /// The clip can't be played (unsupported codec, broken file).
    failed: AtomicBool,
    stop: AtomicBool,
}

/// One clip being played into a texture.
pub struct ClipPlayer {
    clip: Clip,
    shared: Arc<Shared>,
    texture: Option<TextureHandle>,
}

impl ClipPlayer {
    pub fn open(clip: Clip) -> Self {
        let shared = Arc::new(Shared::default());
        let decoder = {
            let (clip, shared) = (clip.clone(), Arc::clone(&shared));
            move || {
                if let Err(e) = decode(&clip, &shared) {
                    eprintln!("clip {}: {e}", clip.path.display());
                    shared.failed.store(true, Ordering::Relaxed);
                }
            }
        };
        if let Err(e) = std::thread::Builder::new()
            .name("clip".into())
            .spawn(decoder)
        {
            eprintln!("clip: {e}");
            shared.failed.store(true, Ordering::Relaxed);
        }
        Self {
            clip,
            shared,
            texture: None,
        }
    }

    pub fn clip(&self) -> &Clip {
        &self.clip
    }

    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Relaxed)
    }

    /// Tells the decoder where the song is; called every frame.
    pub fn follow(&self, position: Duration, playing: bool) {
        *self.shared.clock.lock().unwrap() = Some(Clock {
            position,
            at: Instant::now(),
            playing,
        });
    }

    /// The clip's current picture, once it has one.
    pub fn texture(&mut self, ctx: &egui::Context) -> Option<&TextureHandle> {
        if let Some(frame) = self.shared.frame.lock().unwrap().take() {
            let image = ColorImage::new(frame.size, frame.pixels);
            match &mut self.texture {
                Some(texture) => texture.set(image, TextureOptions::LINEAR),
                None => self.texture = Some(ctx.load_texture("clip", image, TextureOptions::LINEAR)),
            }
        }
        self.texture.as_ref()
    }
}

impl Drop for ClipPlayer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(not(windows))]
fn decode(_clip: &Clip, _shared: &Shared) -> Result<(), String> {
    Err("clips play only on Windows".into())
}

#[cfg(windows)]
fn decode(clip: &Clip, shared: &Shared) -> Result<(), String> {
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::core::{GUID, HSTRING};

    let video = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
    fn err(what: &'static str) -> impl Fn(windows::core::Error) -> String {
        move |e| format!("{what}: {e}")
    }
    // SAFETY: COM and Media Foundation are set up for this thread and torn
    // down when it is done; every pointer passed is valid for its call, and
    // the locked buffer is only read between Lock and Unlock.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(err("COM"))?;
        MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(err("Media Foundation"))?;
        let result = (|| {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1).map_err(err("attributes"))?;
            let attributes = attributes.ok_or("no attributes")?;
            // Lets the reader convert whatever the codec gives into RGB.
            attributes
                .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
                .map_err(err("attributes"))?;
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(clip.path.as_os_str()), &attributes)
                .map_err(err("open"))?;
            reader
                .SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)
                .map_err(err("streams"))?;
            reader
                .SetStreamSelection(video, true)
                .map_err(err("no video stream"))?;
            let wanted = MFCreateMediaType().map_err(err("media type"))?;
            wanted
                .SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                .map_err(err("media type"))?;
            wanted
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                .map_err(err("media type"))?;
            reader
                .SetCurrentMediaType(video, None, &wanted)
                .map_err(err("this video format can't be decoded"))?;
            let actual = reader
                .GetCurrentMediaType(video)
                .map_err(err("media type"))?;
            let packed = actual.GetUINT64(&MF_MT_FRAME_SIZE).map_err(err("frame size"))?;
            let (width, height) = ((packed >> 32) as usize, (packed & 0xffff_ffff) as usize);
            // Bottom-up rows have a negative stride.
            let stride = actual
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .map_or(width as i32 * 4, |s| s as i32);

            // A decoded frame not yet due, with its time in the clip.
            let mut pending: Option<(Duration, Frame)> = None;
            let mut shown: Option<Duration> = None;
            // A seek lands on the keyframe before its target; the frames up
            // to the target are decoded but not shown.
            let mut seeked: Option<Duration> = None;
            let mut ended = false;
            while !shared.stop.load(Ordering::Relaxed) {
                let Some(clock) = *shared.clock.lock().unwrap() else {
                    std::thread::sleep(IDLE);
                    continue;
                };
                // The song's part of the video; past its end, its last frame.
                let target = clock.now() + clip.start;
                let target = clip.end.map_or(target, |end| target.min(end));
                let last = pending.as_ref().map(|(at, _)| *at).or(shown);
                if seeked.zip(last).is_some_and(|(to, at)| at >= to) {
                    seeked = None;
                }
                // Past its end the clip holds its last frame, until the song
                // goes back.
                let far = seeked.or(last).is_some_and(|at| {
                    at > target + SEEK_BACK || (!ended && at + SEEK_AHEAD < target)
                });
                if far {
                    // 100 ns units.
                    let to = PROPVARIANT::from(target.as_nanos() as i64 / 100);
                    reader
                        .SetCurrentPosition(&GUID::zeroed(), &to)
                        .map_err(err("seek"))?;
                    pending = None;
                    shown = None;
                    seeked = Some(target);
                    ended = false;
                }
                if pending.is_none() && !ended {
                    let (mut flags, mut stamp, mut sample) = (0u32, 0i64, None);
                    reader
                        .ReadSample(video, 0, None, Some(&mut flags), Some(&mut stamp), Some(&mut sample))
                        .map_err(err("decode"))?;
                    if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                        ended = true;
                    }
                    if let Some(sample) = sample {
                        let buffer = sample.ConvertToContiguousBuffer().map_err(err("frame"))?;
                        let (mut data, mut len) = (std::ptr::null_mut(), 0u32);
                        buffer.Lock(&mut data, None, Some(&mut len)).map_err(err("frame"))?;
                        let bytes = std::slice::from_raw_parts(data, len as usize);
                        let frame = to_frame(bytes, width, height, stride);
                        buffer.Unlock().map_err(err("frame"))?;
                        let at = Duration::from_nanos(stamp.max(0) as u64 * 100);
                        pending = frame.map(|f| (at, f));
                    }
                    continue;
                }
                match pending.take() {
                    Some((at, frame)) if at <= target => {
                        if seeked.is_none() {
                            *shared.frame.lock().unwrap() = Some(frame);
                        }
                        shown = Some(at);
                    }
                    waiting => {
                        pending = waiting;
                        std::thread::sleep(IDLE);
                    }
                }
            }
            Ok(())
        })();
        let _ = MFShutdown();
        CoUninitialize();
        result
    }
}

/// RGB32 (BGRX) rows `stride` bytes apart into opaque pixels, top row first.
fn to_frame(bytes: &[u8], width: usize, height: usize, stride: i32) -> Option<Frame> {
    let row_len = stride.unsigned_abs() as usize;
    if row_len < width * 4 || bytes.len() < row_len * height {
        return None;
    }
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        let row = if stride < 0 { height - 1 - y } else { y };
        let row = &bytes[row * row_len..][..width * 4];
        pixels.extend(row.chunks_exact(4).map(|p| Color32::from_rgb(p[2], p[1], p[0])));
    }
    Some(Frame {
        size: [width, height],
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bgrx_rows() {
        // 2x2, rows padded to 12 bytes.
        let bytes = [
            1, 2, 3, 0, 4, 5, 6, 0, 9, 9, 9, 9, //
            7, 8, 9, 0, 10, 11, 12, 0, 9, 9, 9, 9,
        ];
        let frame = to_frame(&bytes, 2, 2, 12).unwrap();
        assert_eq!(frame.pixels[0], Color32::from_rgb(3, 2, 1));
        assert_eq!(frame.pixels[3], Color32::from_rgb(12, 11, 10));
        // Bottom-up: the last row in memory is the top one.
        let flipped = to_frame(&bytes, 2, 2, -12).unwrap();
        assert_eq!(flipped.pixels[0], Color32::from_rgb(9, 8, 7));
        assert!(to_frame(&bytes[..10], 2, 2, 12).is_none());
    }

    /// Plays a real clip: `MV_TEST_CLIP=path\to\clip.mp4 cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn decodes_a_clip() {
        let path = std::env::var_os("MV_TEST_CLIP").expect("MV_TEST_CLIP");
        let player = ClipPlayer::open(Clip {
            path: path.into(),
            start: Duration::ZERO,
            end: None,
        });
        player.follow(Duration::from_secs(30), true);
        let started = Instant::now();
        let mut frames = 0;
        while started.elapsed() < Duration::from_secs(3) {
            if let Some(frame) = player.shared.frame.lock().unwrap().take() {
                assert!(frame.size[0] > 0 && frame.pixels.iter().any(|p| *p != Color32::BLACK));
                frames += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!player.failed());
        eprintln!("{frames} frames in 3 s");
        assert!(frames > 30, "{frames} frames in 3 s");
    }
}
