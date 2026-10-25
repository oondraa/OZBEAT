//! The visualizer window: shader background plus cover, titles, lyrics.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Galley, Mesh, Pos2,
    Rect, Shape, Stroke, TextFormat, TextureHandle, TextureOptions, Vec2, emath::Rot2,
    emath::TSTransform, epaint::Vertex, load::SizedTexture, text::LayoutJob,
};
use eframe::egui_wgpu;
use mv_audio::Capture;
use mv_core::{Device, NowPlaying, PlaybackState};
use mv_enrich::Enrichment;

use crate::Mode;
use crate::anim::{Tween, ease_in_out_cubic, ease_out_cubic, remap, spring};
use crate::art::{Art, DEFAULT_PALETTE, Picture};
use crate::background::{self, Background, Uniforms};
use crate::beam;
use crate::clip::ClipPlayer;
use crate::pipeline::SharedScene;
use crate::settings::{ClipScreen, LyricsStyle, SharedSettings};
use crate::setup::{self, BRAND_BLUE};
use crate::titlebar::TitleBar;
use crate::transport::{Press, Transport};
use crate::update::{self, SharedUpdate};
use crate::control::Control;
use crate::icons;
use crate::volume::VolumeHud;

/// How long the lyrics take to scroll to the next line.
const LYRIC_SCROLL: Duration = Duration::from_millis(450);
/// How long cover, photo, background and colors take to blend into new ones.
const ART_FADE: Duration = Duration::from_millis(700);
const TOAST_VISIBLE: Duration = Duration::from_millis(2400);
const TOAST_FADE: Duration = Duration::from_millis(600);
/// Opening and closing the settings panel.
const SETTINGS_OPEN: Duration = Duration::from_millis(700);
const SETTINGS_CLOSE: Duration = Duration::from_millis(180);
/// The spark crossing to the second screen and switching it on (Ctrl+D).
const MIRROR_OPEN: Duration = Duration::from_millis(1000);
const MIRROR_CLOSE: Duration = Duration::from_millis(800);
/// Frames the second window draws before it is shown and the transition starts.
const MIRROR_READY: u32 = 3;
/// Clicking the lyrics: they grow to fill the window, and shrink back.
const FOCUS_OPEN: Duration = Duration::from_millis(550);
const FOCUS_CLOSE: Duration = Duration::from_millis(400);
/// Lyrics fading in once they have loaded (seconds).
const LYRICS_IN: Duration = Duration::from_millis(450);
/// How long lyrics take to fade out when there is nothing left to sing, while
/// the cover glides into the middle.
const LYRICS_OUT: Duration = Duration::from_millis(800);
/// A new song's titles fading in over the last one's.
const TRACK_FADE: Duration = Duration::from_millis(600);
/// Brightness of the current line's words still to be sung, against the sung ones.
const UNSUNG: f32 = 0.45;
/// The music video fading in over the background once its first frame is in (seconds).
const CLIP_FADE: f32 = 0.8;
/// A second click this soon after a focus toggle is a double-click (fullscreen).
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub(crate) const BOLD: &str = "bold";
pub(crate) const REGULAR: &str = "regular";
/// The lyric words, one at a time: heavier, falling back to [`BOLD`].
const HEAVY: &str = "heavy";
const HEAVY_FONT: &[u8] = include_bytes!("../assets/fonts/Geist-Bold.ttf");
/// Geist, the website's typeface, bundled so the app looks the same everywhere.
const BUNDLED_FONTS: [(&str, &[u8]); 2] = [
    (BOLD, include_bytes!("../assets/fonts/Geist-SemiBold.ttf")),
    (REGULAR, include_bytes!("../assets/fonts/Geist-Regular.ttf")),
];
/// System fonts for scripts Geist lacks (Cyrillic, Greek); egui's built-in
/// font is the last fallback.
const BOLD_FONTS: &[&str] = &[
    "C:\\Windows\\Fonts\\segoeuib.ttf",
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
];
const REGULAR_FONTS: &[&str] = &[
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
];

struct Textures {
    cover: Option<TextureHandle>,
    photo: Option<TextureHandle>,
}

/// Art on screen together with its uploaded textures.
struct Shown {
    art: Arc<Art>,
    textures: Textures,
}

/// The current art fading in over the previous one.
struct ArtFade {
    current: Option<Shown>,
    /// Fading out, with how visible it was when the fade started.
    previous: Option<(Shown, f32)>,
    changed: Instant,
}

impl ArtFade {
    /// 0 -> 1 over [`ART_FADE`], eased.
    fn progress(&self) -> f32 {
        let t = (self.changed.elapsed().as_secs_f32() / ART_FADE.as_secs_f32()).min(1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// How visible the previous art is now.
    fn previous_alpha(&self) -> f32 {
        let Some((_, start)) = &self.previous else {
            return 0.0;
        };
        // Under a new picture it stays put until covered; alone it fades out.
        if self.current.is_some() {
            *start
        } else {
            start * (1.0 - self.progress())
        }
    }

    /// The current and the previous art's texture, with their alpha.
    fn layers<'a>(
        &'a self,
        pick: impl Fn(&'a Textures) -> Option<&'a TextureHandle>,
    ) -> [Option<(&'a TextureHandle, f32)>; 2] {
        let t = self.progress();
        [
            self.previous
                .as_ref()
                .filter(|_| t < 1.0)
                .and_then(|(s, _)| pick(&s.textures))
                .map(|tex| (tex, self.previous_alpha())),
            self.current
                .as_ref()
                .and_then(|s| pick(&s.textures))
                .map(|tex| (tex, t)),
        ]
    }
}

pub struct VisualApp {
    scene: SharedScene,
    settings: SharedSettings,
    settings_open: bool,
    /// Monitor index the visuals are mirrored to (Ctrl+D), if any. Stays
    /// set while the mirror fades out after `mirror_on` went false.
    mirror: Option<usize>,
    mirror_on: bool,
    mirror_anim: Tween,
    /// From the main window's monitor towards the mirror's, for the lyrics' flight.
    mirror_dir: Vec2,
    /// Frames the mirror has drawn; it is shown only once it has a picture,
    /// so a fresh window never flashes white.
    mirror_frames: u32,
    settings_anim: Tween,
    /// Leaving fullscreen should maximize the window again.
    restore_maximized: bool,
    toast: Option<(String, Instant)>,
    icon: Option<Arc<egui::IconData>>,
    started: Instant,
    art: ArtFade,
    lyric_scroll: LyricScroll,
    /// Lyrics clicked: shown big in the middle, cover and titles hidden.
    lyrics_focus: bool,
    lyrics_focus_anim: Tween,
    /// When a click last toggled `lyrics_focus`, to undo it on a double-click.
    focus_toggled: Option<Instant>,
    volume: VolumeHud,
    control: Control,
    /// Animation phase of the background; advances faster with louder mids,
    /// so the flow speeds up smoothly instead of jumping.
    flow: f32,
    last_frame: Instant,
    mode: Mode,
    /// Screensaver: every monitor besides the main window's, each in its own
    /// fullscreen window. Split like Ctrl+D: the first one shows the lyrics,
    /// any further ones the full visual.
    screens: Vec<usize>,
    /// Screensaver: where the pointer was first seen in each window (main
    /// window first, then `screens`), so only a real move closes it.
    wake_origin: Vec<Option<Pos2>>,
    /// Screensaver: last time the main window was pushed into fullscreen.
    fullscreen_retry: Instant,
    /// `None` in the screensaver, which never updates itself.
    update: Option<SharedUpdate>,
    update_announced: bool,
    /// Our own title bar; `None` in the screensaver, which has no frame to replace.
    title_bar: Option<TitleBar>,
    /// The playing song's music video, when clips are on and it is downloaded.
    clip: Option<ClipPlayer>,
    /// The lyrics screen clicked: the video big, the lyrics small below.
    clip_focus: bool,
    clip_focus_anim: Tween,
    /// Which window shows the settings panel: the one Ctrl+S was pressed in.
    settings_screen: Screen,
    /// Lyrics on screen (0 -> 1): in when there is something to sing, out
    /// for the intro, the outro, long breaks and while they load.
    lyrics_shown: Tween,
    /// The lyrics last on screen and where in the song, kept to fade out
    /// when the song changes.
    lyrics_seen: Option<(Arc<Enrichment>, Duration)>,
    /// The last song's lyrics fading out, frozen where they were, how visible
    /// they were and since when.
    outgoing: Option<(Arc<Enrichment>, Duration, f32, Instant)>,
    /// The song on screen, and the one before it while its titles fade out.
    track: Option<NowPlaying>,
    previous_track: Option<(NowPlaying, Instant)>,
    transport: Transport,
    mirror_transport: Transport,
    /// Raised by Ctrl+Alt+T anywhere in Windows; `None` in the screensaver.
    overlay_key: Option<Arc<AtomicBool>>,
    /// The window turned into the lyrics over the desktop, and whether it
    /// was fullscreen and maximized before, to go back to that.
    overlay: Option<(bool, bool)>,
    /// The overlay covers its monitor and lets the mouse through.
    overlay_ready: bool,
    /// Where the window was before the overlay, to put it back.
    overlay_rect: Option<(Pos2, Vec2)>,
}

/// The windows that take keys and clicks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    /// The second screen of Ctrl+D.
    Mirror,
}

/// Screensaver: input right after start is the window settling, not the user.
const WAKE_GRACE: Duration = Duration::from_millis(800);
/// Screensaver: how often to re-request fullscreen for the main window.
const FULLSCREEN_RETRY: Duration = Duration::from_millis(500);
/// Screensaver: pointer travel (points) that counts as the user moving the mouse.
const WAKE_DISTANCE: f32 = 12.0;

/// Eases the lyric list from one line to the next instead of jumping.
struct LyricScroll {
    track: String,
    from: f32,
    target: f32,
    changed: Instant,
}

impl LyricScroll {
    fn new() -> Self {
        Self {
            track: String::new(),
            from: -1.0,
            target: -1.0,
            changed: Instant::now(),
        }
    }

    /// Animated line position for `target` (the current line index, -1 before the first).
    fn position(&mut self, track: &str, target: f32) -> f32 {
        let now = Instant::now();
        if self.track != track {
            // New song: no scrolling across from the previous one.
            *self = Self {
                track: track.to_owned(),
                from: target,
                target,
                changed: now,
            };
        } else if target != self.target {
            self.from = self.at(now);
            self.target = target;
            self.changed = now;
        }
        self.at(now)
    }

    fn at(&self, now: Instant) -> f32 {
        let t = ((now - self.changed).as_secs_f32() / LYRIC_SCROLL.as_secs_f32()).min(1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + (self.target - self.from) * eased
    }
}

impl VisualApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        scene: SharedScene,
        settings: SharedSettings,
        mode: Mode,
    ) -> Self {
        install_fonts(&cc.egui_ctx);
        install_style(&cc.egui_ctx);
        crate::frameless::install(cc);
        if let Some(render_state) = &cc.wgpu_render_state {
            background::init(render_state);
        }
        let settings_open = match mode {
            Mode::Configure => true,
            // Nobody is at the keyboard to go through the setup.
            Mode::Screensaver => false,
            // First run opens the setup panel by itself.
            Mode::Window | Mode::Preview => !settings.lock().unwrap().setup_done,
        };
        // A screensaver covers every screen; the main window sits on the primary one.
        let screens: Vec<usize> = if mode == Mode::Screensaver {
            let displays = display_info::DisplayInfo::all().unwrap_or_default();
            (0..displays.len())
                .filter(|&i| Some(i) != crate::primary_monitor(&displays))
                .collect()
        } else {
            Vec::new()
        };
        let update = (mode == Mode::Window && !update::packaged()).then(|| {
            let update = SharedUpdate::default();
            update::check_in_background(Arc::clone(&update));
            update
        });
        Self {
            update,
            update_announced: false,
            title_bar: (mode != Mode::Screensaver).then(TitleBar::new),
            scene,
            settings,
            settings_open,
            mirror: None,
            mirror_on: false,
            mirror_anim: Tween::new(MIRROR_OPEN, MIRROR_CLOSE),
            mirror_dir: Vec2::X,
            mirror_frames: 0,
            settings_anim: Tween::new(SETTINGS_OPEN, SETTINGS_CLOSE),
            restore_maximized: false,
            toast: None,
            icon: crate::window_icon().map(Arc::new),
            started: Instant::now(),
            art: ArtFade {
                current: None,
                previous: None,
                changed: Instant::now(),
            },
            lyric_scroll: LyricScroll::new(),
            lyrics_focus: false,
            lyrics_focus_anim: Tween::new(FOCUS_OPEN, FOCUS_CLOSE),
            focus_toggled: None,
            volume: VolumeHud::default(),
            control: Control::default(),
            flow: 0.0,
            last_frame: Instant::now(),
            mode,
            wake_origin: vec![None; screens.len() + 1],
            fullscreen_retry: Instant::now(),
            screens,
            clip: None,
            clip_focus: false,
            clip_focus_anim: Tween::new(FOCUS_OPEN, FOCUS_CLOSE),
            settings_screen: Screen::Main,
            lyrics_shown: Tween::new(LYRICS_IN, LYRICS_OUT),
            lyrics_seen: None,
            outgoing: None,
            track: None,
            previous_track: None,
            transport: Transport::default(),
            mirror_transport: Transport::default(),
            overlay_key: (mode == Mode::Window).then(|| crate::hotkey::spawn(cc.egui_ctx.clone())),
            overlay: None,
            overlay_ready: false,
            overlay_rect: None,
        }
    }

    /// Screensaver: true once a key, click, wheel, touch or real mouse move
    /// arrives in window `slot` (0 = main window, then `screens`).
    fn wakes(&mut self, ui: &egui::Ui, slot: usize) -> bool {
        let settling = self.started.elapsed() < WAKE_GRACE;
        let origin = &mut self.wake_origin[slot];
        ui.input(|i| {
            if let Some(pos) = i.pointer.latest_pos()
                && (settling || origin.is_none())
            {
                *origin = Some(pos);
            }
            if settling {
                return false;
            }
            let moved = matches!((*origin, i.pointer.latest_pos()),
                (Some(a), Some(b)) if a.distance(b) > WAKE_DISTANCE);
            moved
                || i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { key, pressed: true, .. } if !is_modifier(*key)
                    ) || matches!(
                        e,
                        egui::Event::PointerButton { pressed: true, .. }
                            | egui::Event::MouseWheel { .. }
                            | egui::Event::Touch { .. }
                    )
                })
        })
    }

    /// Starts a fade when the art changes; textures are uploaded once per art.
    fn sync_textures(&mut self, ctx: &egui::Context, art: Option<&Arc<Art>>) {
        let fade = &mut self.art;
        if fade.progress() >= 1.0 {
            fade.previous = None;
        }
        if fade.current.as_ref().map(|s| s.art.id) == art.map(|a| a.id) {
            return;
        }
        let Some(art) = art else {
            fade.previous = fade.current.take().map(|s| (s, 1.0));
            fade.changed = Instant::now();
            return;
        };
        let upload = |name: &str, picture: &Option<Arc<Picture>>| {
            picture.as_ref().map(|p| {
                let image = egui::ColorImage::from_rgba_unmultiplied(p.size, &p.rgba);
                // Mipmaps keep a 1600px cover crisp when drawn at a few hundred pixels.
                let options = TextureOptions {
                    mipmap_mode: Some(egui::TextureFilter::Linear),
                    ..TextureOptions::LINEAR
                };
                ctx.load_texture(name, image, options)
            })
        };
        let shown = Shown {
            art: Arc::clone(art),
            textures: Textures {
                cover: upload("cover", &art.cover),
                photo: upload("photo", &art.photo),
            },
        };
        // From a picture: it stays under the new one. From nothing: whatever
        // was still fading out stays at the level it had reached.
        fade.previous = match fade.current.take() {
            Some(current) => Some((current, 1.0)),
            None => {
                let alpha = fade.previous_alpha();
                fade.previous.take().map(|(s, _)| (s, alpha))
            }
        };
        fade.current = Some(shown);
        fade.changed = Instant::now();
    }
}

impl eframe::App for VisualApp {
    /// Clear, so the lyrics over the desktop show nothing but themselves;
    /// otherwise the background covers the whole window anyway.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.toggle_overlay_on_key();
        let overlay = self.mode == Mode::Window && self.settings.lock().unwrap().overlay;
        self.set_overlay(&ctx, overlay);
        if overlay {
            crate::frameless::keep_bare();
            let frame = self.prepare_frame(&ctx);
            self.draw_overlay(ui, &frame);
            self.draw_toast(&ctx);
            ctx.request_repaint();
            return;
        }
        if self.mode == Mode::Screensaver {
            // On Windows the builder's fullscreen flag can be lost while the
            // main window is still hidden, and winit then ignores a repeated
            // request because it believes it is fullscreen. Toggling it off
            // and on fixes that; retried until the window covers its monitor.
            let covers = ctx.input(|i| {
                let v = i.viewport();
                matches!((v.outer_rect, v.monitor_size),
                    (Some(r), Some(m)) if r.width() + 1.0 >= m.x && r.height() + 1.0 >= m.y)
            });
            if !covers && self.fullscreen_retry.elapsed() > FULLSCREEN_RETRY {
                self.fullscreen_retry = Instant::now();
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
            ctx.set_cursor_icon(egui::CursorIcon::None);
            if self.wakes(ui, 0) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else {
            self.handle_keys(&ctx);
        }
        self.announce_update();

        let frame = self.prepare_frame(&ctx);
        // Creating the second window stalls a few frames; start once it is up,
        // so the animation doesn't jump ahead meanwhile.
        self.mirror_anim
            .set(self.mirror_on && self.mirror_frames >= MIRROR_READY);
        let mirror = self.mirror_anim.progress();
        if !self.mirror_on && mirror == 0.0 {
            self.mirror = None;
            self.mirror_frames = 0;
            self.settings_screen = Screen::Main;
        }
        // With a second screen the content is split: cover and titles here,
        // lyrics over there.
        let (split, leaving, gone) = if self.screens.is_empty() {
            let leaving = beam::leaving(ui.max_rect(), self.mirror_dir, mirror);
            (beam::split(mirror), leaving, beam::gone(mirror))
        } else {
            (1.0, None, true)
        };
        self.clip_focus_anim.set(self.clip_focus);
        if self.mode != Mode::Screensaver {
            self.handle_lyrics_click(&ctx, ui.max_rect(), &frame, split);
        }
        self.lyrics_focus_anim.set(self.lyrics_focus);
        let focus = self.lyrics_focus_anim.progress();
        self.settings_anim.set(self.settings_open);
        let panel = self.panel_on(Screen::Main);
        // Behind the glass panel only the soft background shows: the sharp
        // cover, titles and lyrics fade back, so the pane reads as frosted.
        ui.scope(|ui| {
            ui.multiply_opacity(1.0 - 0.95 * ease_out_cubic(panel));
            let content = Content::Main {
                split,
                leaving,
                gone,
                focus,
            };
            self.draw_visual(ui, &frame, content);
        });
        if let Some(monitor) = self.mirror {
            self.show_mirror(&ctx, monitor, &frame, mirror);
        }
        for slot in 0..self.screens.len() {
            self.show_screen(&ctx, slot, &frame);
        }
        self.volume.update(frame.now_playing.as_ref());
        self.draw_overlays(ui, Screen::Main, &frame, panel);
        // Opened from the screensaver dialog: done once the settings close.
        if self.mode == Mode::Configure && !self.settings_open && self.settings_anim.progress() == 0.0 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Some(bar) = &mut self.title_bar {
            bar.show(&ctx, self.settings_open && self.settings_screen == Screen::Main);
        }
        self.draw_toast(&ctx);
        ctx.request_repaint();
    }
}

/// What a window shows on top of the background.
#[derive(Clone, Copy)]
enum Content {
    /// Cover and titles. `split` (0 -> 1) is how far the lyrics have moved
    /// to a second screen: at 0 they sit below the cover, at 1 they are gone
    /// and the cover block is big in the middle. They fly off as `leaving`
    /// says, and are out of sight once `gone`. `focus` (0 -> 1) is how far
    /// clicked lyrics have grown to fill the window, with cover and titles
    /// faded out.
    Main {
        split: f32,
        leaving: Option<beam::Flight>,
        gone: bool,
        focus: f32,
    },
    /// Big centered lyrics (second screen), flying in while `arriving`.
    /// `clip_focus` (0 -> 1) is how far a click has brought the video up
    /// and the lyrics down small.
    LyricsOnly {
        arriving: Option<beam::Flight>,
        clip_focus: f32,
    },
}

/// The music video's current picture.
#[derive(Clone, Copy)]
struct ClipFrame {
    texture: egui::TextureId,
    size: Vec2,
    /// Fading in (0 -> 1).
    alpha: f32,
}

/// Everything one frame needs, gathered once and shared by all windows.
struct FrameData {
    now_playing: Option<NowPlaying>,
    enrichment: Option<Arc<Enrichment>>,
    art: Option<Arc<Art>>,
    audio: Option<Capture>,
    devices: Vec<Device>,
    position: Option<Duration>,
    pulse: f32,
    palette: [[f32; 3]; 3],
    lyrics_style: LyricsStyle,
    clip_screen: ClipScreen,
    /// The art fading out, how visible it is before fading, and the fade (0 -> 1).
    previous: Option<(Arc<Art>, f32)>,
    fade: f32,
    /// Stopped or no source: calm idle screen without stale artwork.
    idle: bool,
    clip: Option<ClipFrame>,
    time: f32,
    flow: f32,
    /// How far the lyrics are on screen (0 -> 1), see `VisualApp::lyrics_shown`.
    lyrics: f32,
    /// The last song's lyrics fading out: where they stopped and how visible they are.
    outgoing: Option<(Arc<Enrichment>, Duration, f32)>,
    /// The last song, its titles fading out as the new ones come (0 -> 1).
    previous_track: Option<(NowPlaying, f32)>,
}

impl VisualApp {
    fn prepare_frame(&mut self, ctx: &egui::Context) -> FrameData {
        // Deliberately no on-screen source or status: the picture is the whole UI.
        let (now_playing, enrichment, art, audio, devices, clip_path) = {
            let scene = self.scene.lock().unwrap();
            (
                scene.now_playing.clone(),
                scene.enrichment.clone(),
                scene.art.clone(),
                scene.audio.clone(),
                scene.devices.clone(),
                scene.clip.clone(),
            )
        };
        self.sync_textures(ctx, art.as_ref());

        let time = self.started.elapsed().as_secs_f32();
        let position = now_playing
            .as_ref()
            .and_then(|np| np.position_now(SystemTime::now()));
        let playing = now_playing
            .as_ref()
            .is_some_and(|np| np.state == PlaybackState::Playing);
        // A mic hears the room even when nothing plays, so only trust it during playback.
        let levels = audio
            .as_ref()
            .map(Capture::levels)
            .filter(|l| playing && l.signal);
        let raw_pulse = match levels {
            Some(l) => 0.5 * l.bass + 0.5 * l.beat,
            None => pulse(now_playing.as_ref(), enrichment.as_deref(), position, time),
        };
        // Intensity 0.5 (the default) keeps the tuned amplitude.
        let (intensity, clips, clip_screen, lyrics_style) = {
            let s = self.settings.lock().unwrap();
            (s.intensity, s.youtube, s.clip_screen, s.lyrics)
        };
        let clip = self.sync_clip(ctx, clip_path.filter(|_| clips), position, playing);
        let clip = clip.map(|(texture, size, alpha)| ClipFrame {
            texture,
            size,
            alpha,
        });
        let dt = self.last_frame.elapsed().as_secs_f32().min(0.1);
        self.last_frame = Instant::now();
        self.flow += dt * (1.0 + 0.8 * levels.map_or(0.0, |l| l.mid));

        let fade = self.art.progress();
        let previous = self
            .art
            .previous
            .as_ref()
            .filter(|_| fade < 1.0)
            .map(|(s, alpha)| (Arc::clone(&s.art), *alpha));
        let palette_of = |art: Option<&Arc<Art>>| art.map_or(DEFAULT_PALETTE, |a| a.palette);
        let palette = mix_palette(
            palette_of(previous.as_ref().map(|(a, _)| a)),
            palette_of(art.as_ref()),
            fade,
        );

        let idle = now_playing
            .as_ref()
            .is_none_or(|np| np.state == PlaybackState::Stopped);
        self.follow_track(now_playing.as_ref(), idle);
        // Without a position the lyrics are whole lines, always worth showing.
        let singing = enrichment.as_ref().filter(|e| {
            e.lyrics.as_ref().is_some_and(|l| {
                !l.synced.is_empty() && !idle && position.is_none_or(|p| l.sung_near(p))
            })
        });
        self.lyrics_shown.set(singing.is_some());
        if let Some(e) = singing {
            self.lyrics_seen = Some((Arc::clone(e), position.unwrap_or_default()));
        }
        let outgoing = self.outgoing.as_ref().and_then(|(e, at, alpha, since)| {
            let t = since.elapsed().as_secs_f32() / LYRICS_OUT.as_secs_f32();
            (t < 1.0).then(|| (Arc::clone(e), *at, alpha * (1.0 - ease_out_cubic(t))))
        });
        if outgoing.is_none() {
            self.outgoing = None;
        }
        let previous_track = self.previous_track.as_ref().and_then(|(np, since)| {
            let t = since.elapsed().as_secs_f32() / TRACK_FADE.as_secs_f32();
            (t < 1.0).then(|| (np.clone(), t))
        });
        if previous_track.is_none() {
            self.previous_track = None;
        }

        FrameData {
            idle,
            lyrics: self.lyrics_shown.progress(),
            outgoing,
            previous_track,
            palette,
            lyrics_style,
            clip_screen,
            previous,
            fade,
            pulse: raw_pulse * intensity * 2.0,
            clip,
            now_playing,
            enrichment,
            art,
            audio,
            devices,
            position,
            time,
            flow: self.flow,
        }
    }

    /// Notices a new song: the last one's titles and lyrics fade out from
    /// where they were, instead of the screen cutting to the new one.
    fn follow_track(&mut self, now_playing: Option<&NowPlaying>, idle: bool) {
        let now_playing = now_playing.filter(|_| !idle);
        let same = |a: &NowPlaying, b: &NowPlaying| {
            a.track.title == b.track.title && a.track.artist == b.track.artist
        };
        let changed = match (&self.track, now_playing) {
            (Some(old), Some(new)) => !same(old, new),
            (old, new) => old.is_some() != new.is_some(),
        };
        if changed {
            self.previous_track = self
                .track
                .take()
                .filter(|_| now_playing.is_some())
                .map(|np| (np, Instant::now()));
            let shown = self.lyrics_shown.progress();
            self.outgoing = self
                .lyrics_seen
                .take()
                .filter(|_| shown > 0.01)
                .map(|(e, at)| (e, at, shown, Instant::now()));
            // The new song's lyrics come in fresh rather than taking over the old fade.
            self.lyrics_shown = Tween::new(LYRICS_IN, LYRICS_OUT);
        }
        self.track = now_playing.cloned();
    }

    /// Opens, follows and closes the music video for the song playing: its
    /// texture, size and how far it has faded in.
    fn sync_clip(
        &mut self,
        ctx: &egui::Context,
        clip: Option<mv_video::Clip>,
        position: Option<Duration>,
        playing: bool,
    ) -> Option<(egui::TextureId, Vec2, f32)> {
        if self.clip.as_ref().map(ClipPlayer::clip) != clip.as_ref() {
            self.clip = clip.map(ClipPlayer::open);
        }
        let player = self.clip.as_mut().filter(|c| !c.failed());
        let shown = player.and_then(|player| {
            player.follow(position?, playing);
            player.texture(ctx).map(|t| (t.id(), t.size_vec2()))
        });
        let alpha = ctx.animate_bool_with_time(
            egui::Id::new("ozbeat-clip"),
            shown.is_some(),
            CLIP_FADE,
        );
        shown.map(|(texture, size)| (texture, size, alpha))
    }

    /// The whole visual for one window; used for the main window and the mirror.
    fn draw_visual(&mut self, ui: &mut egui::Ui, f: &FrameData, content: Content) {
        let rect = ui.max_rect();
        let painter = ui.painter().clone();
        let backdrop_of = |art: &Arc<Art>| {
            art.backdrop
                .clone()
                .map(|b| (art.id, b))
                .filter(|_| !f.idle)
        };
        let backdrop = f.art.as_ref().and_then(backdrop_of);
        let prev = f
            .previous
            .as_ref()
            .and_then(|(art, alpha)| Some((backdrop_of(art)?, *alpha)));
        let size_of = |b: Option<&(u64, Arc<Picture>)>| {
            b.map_or([1.0, 1.0], |(_, p)| p.size.map(|v| v as f32))
        };
        painter.add(egui_wgpu::Callback::new_paint_callback(
            rect,
            Background {
                uniforms: Uniforms {
                    resolution: [rect.width(), rect.height()],
                    time: f.flow % 3600.0,
                    pulse: f.pulse,
                    vivid: rgba(f.palette[0]),
                    mid: rgba(f.palette[1]),
                    dark: rgba(f.palette[2]),
                    art_size: size_of(backdrop.as_ref()),
                    has_art: if backdrop.is_some() { 1.0 } else { 0.0 },
                    srgb_target: 0.0, // filled in by the callback
                    prev_size: size_of(prev.as_ref().map(|(b, _)| b)),
                    prev_has_art: prev.as_ref().map_or(0.0, |(_, alpha)| *alpha),
                    fade: f.fade,
                },
                art: backdrop,
                prev: prev.map(|(b, _)| b),
            },
        ));

        let Some(np) = f.now_playing.as_ref().filter(|_| !f.idle) else {
            draw_idle(&painter, rect, f.time);
            return;
        };
        // The video plays where the settings put it; in the main window it
        // gives way as Ctrl+D moves it under the lyrics.
        let share = match content {
            Content::Main { split, .. } => match f.clip_screen {
                ClipScreen::Lyrics => 1.0 - split,
                ClipScreen::Main | ClipScreen::Both => 1.0,
            },
            Content::LyricsOnly { .. } => match f.clip_screen {
                ClipScreen::Main => 0.0,
                ClipScreen::Lyrics | ClipScreen::Both => 1.0,
            },
        };
        let clip = f
            .clip
            .map(|c| ClipFrame {
                alpha: c.alpha * share,
                ..c
            })
            .filter(|c| c.alpha > 0.0);
        if let Some(clip) = clip {
            draw_clip(&painter, rect, clip);
        }
        // Over a video, the cover and titles give way to a small caption.
        let video = clip.map_or(0.0, |c| c.alpha);

        let synced = f
            .enrichment
            .as_ref()
            .and_then(|e| e.lyrics.as_ref())
            .filter(|l| !l.synced.is_empty());
        let scroll = synced.map(|lyrics| {
            let line = f
                .position
                .and_then(|p| lyrics.line_at(p))
                .map_or(-1.0, |i| i as f32);
            self.lyric_scroll.position(&np.track.title, line)
        });
        // Lyrics fade in when they arrive and out when nothing is left to sing.
        let ready = if synced.is_some() { f.lyrics } else { 0.0 };
        let accent = color(f.palette[0]);
        let outgoing = f.outgoing.as_ref().and_then(|(e, at, alpha)| {
            let lyrics = e.lyrics.as_ref()?;
            let line = lyrics.line_at(*at).map_or(-1.0, |i| i as f32);
            Some((lyrics, *at, line, *alpha))
        });

        match content {
            Content::Main {
                split,
                leaving,
                gone,
                focus,
            } => {
                // Without lyrics below (on the other screen, or none to sing
                // right now) the cover block grows into the middle.
                let settle = split.max(1.0 - ready);
                let center = egui::emath::lerp(0.40..=0.5, settle);
                let layout = Layout::new(rect, center, settle);
                let focus = ease_in_out_cubic(focus);
                if focus < 1.0 && video < 1.0 {
                    ui.scope(|ui| {
                        ui.multiply_opacity((1.0 - focus) * (1.0 - video));
                        self.draw_cover(ui, &layout, f.pulse);
                        // A new song's titles rise in as the last one's rise away.
                        let rise = layout.title_size * 0.5;
                        let incoming = f.previous_track.as_ref().map_or(1.0, |(_, t)| ease_out_cubic(*t));
                        if let Some((prev, _)) = &f.previous_track {
                            let shift = TSTransform::from_translation(Vec2::new(0.0, -rise * incoming));
                            ui.with_visual_transform(shift, |ui| {
                                ui.multiply_opacity(1.0 - incoming);
                                self.draw_titles(ui, &layout, prev, None, accent, settle);
                            });
                        }
                        let shift = TSTransform::from_translation(Vec2::new(0.0, rise * (1.0 - incoming)));
                        ui.with_visual_transform(shift, |ui| {
                            ui.multiply_opacity(incoming);
                            self.draw_titles(ui, &layout, np, f.position, accent, settle);
                        });
                    });
                }
                if video > 0.0 && focus < 1.0 {
                    let mut painter = painter.clone();
                    painter.multiply_opacity(video * (1.0 - focus));
                    draw_clip_caption(&painter, rect, np);
                }
                // Big lyrics stay big across songs; until the new song's
                // lyrics are in, show what plays in their place.
                if !gone && focus > 0.0 && ready < 1.0 {
                    let mut painter = painter.clone();
                    painter.multiply_opacity(focus * (1.0 - ready));
                    draw_caption(&painter, rect, np);
                }
                if let (false, Some((lyrics, at, line, alpha))) = (gone, outgoing) {
                    let (center_y, size) = lyrics_placement(rect, focus);
                    draw_outgoing(ui, rect, f, video, lyrics, at, line, center_y, size, alpha);
                }
                if let (false, Some(lyrics), Some(scroll), true) = (gone, synced, scroll, ready > 0.0) {
                    let (center_y, size) = lyrics_placement(rect, focus);
                    draw_lyrics_shade(&painter, rect, f, center_y, size, video * ready);
                    let draw = |ui: &mut egui::Ui| {
                        if focus > 0.0 && focus < 1.0 {
                            // Growing: laid out once at the full size and scaled,
                            // since a new font size every frame stalls on the glyph atlas.
                            let big = big_lyrics_size(rect);
                            let transform = scaled_about(rect.center(), center_y, size / big);
                            ui.with_visual_transform(transform, |ui| {
                                let mut painter = ui.painter().clone();
                                painter.multiply_opacity(ready);
                                let y = rect.center().y;
                                draw_timed_lyrics(&painter, rect, f, f.position, video, lyrics, scroll, y, big);
                            });
                        } else {
                            let mut painter = ui.painter().clone();
                            painter.multiply_opacity(ready);
                            draw_timed_lyrics(&painter, rect, f, f.position, video, lyrics, scroll, center_y, size);
                        }
                    };
                    match leaving {
                        Some(flight) => beam::with_trail(ui, flight, draw),
                        None => draw(ui),
                    }
                }
            }
            Content::LyricsOnly {
                arriving,
                clip_focus,
            } => {
                // Big lyrics over the video dimmed, or (clicked) the video
                // with the lyrics small where they sit under the cover.
                let clip_focus = if video > 0.0 {
                    ease_in_out_cubic(clip_focus)
                } else {
                    0.0
                };
                let dim = 100.0 * video * (1.0 - clip_focus);
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(dim as u8));
                if clip_focus > 0.0 {
                    let mut painter = painter.clone();
                    painter.multiply_opacity(video * clip_focus);
                    draw_clip_caption(&painter, rect, np);
                }
                let big = big_lyrics_size(rect);
                let scale = egui::emath::lerp(1.0..=main_lyrics_size(rect) / big, clip_focus);
                let center_y = egui::emath::lerp(rect.center().y..=main_lyrics_y(rect), clip_focus);
                let leaving = outgoing.map_or(0.0, |o| o.3);
                draw_lyrics_shade(&painter, rect, f, center_y, big * scale, video * ready.max(leaving));
                let draw = |ui: &mut egui::Ui| {
                    let transform = scaled_about(rect.center(), center_y, scale);
                    ui.with_visual_transform(transform, |ui| {
                        let y = rect.center().y;
                        if let Some((lyrics, at, line, alpha)) = outgoing {
                            draw_outgoing(ui, rect, f, video, lyrics, at, line, y, big, alpha);
                        }
                        if let (Some(lyrics), Some(scroll), true) = (synced, scroll, ready > 0.0) {
                            let mut painter = ui.painter().clone();
                            painter.multiply_opacity(ready);
                            draw_timed_lyrics(&painter, rect, f, f.position, video, lyrics, scroll, y, big);
                        }
                        // Nothing to sing: show what plays rather than an empty
                        // screen, once the last song's lyrics have given way.
                        let caption = (1.0 - ready) * (1.0 - leaving);
                        if caption > 0.0 {
                            let mut painter = ui.painter().clone();
                            painter.multiply_opacity(caption);
                            let incoming = f.previous_track.as_ref().map_or(1.0, |(_, t)| ease_out_cubic(*t));
                            if let Some((prev, _)) = &f.previous_track {
                                let mut painter = painter.clone();
                                painter.multiply_opacity(1.0 - incoming);
                                draw_caption(&painter, rect, prev);
                            }
                            painter.multiply_opacity(incoming);
                            draw_caption(&painter, rect, np);
                        }
                    });
                };
                match arriving {
                    Some(flight) => beam::with_trail(ui, flight, draw),
                    None => draw(ui),
                }
            }
        }
        // Big in the middle, the cover has its own progress under the titles.
        let mut bar = painter.clone();
        if let Content::Main { split, .. } = content {
            bar.multiply_opacity(1.0 - split.max(1.0 - ready));
        }
        draw_progress(&bar, rect, np, f.position, accent);
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let escape = self.handle_shortcuts(ctx, Screen::Main);
        // Esc closes the panel first, then the big lyrics; only then does it
        // leave fullscreen.
        if escape && self.settings_open && self.settings_screen == Screen::Main {
            self.close_settings();
            return;
        }
        if escape && self.lyrics_focus {
            self.lyrics_focus = false;
            return;
        }
        // The title bar has its own double-click (maximize), and so many
        // quick clicks on the buttons are no double-click.
        let on_bar = self
            .title_bar
            .as_ref()
            .is_some_and(|bar| bar.under_pointer(ctx));
        let on_buttons = self.transport.under_pointer(ctx);
        let settings_here = self.settings_open && self.settings_screen == Screen::Main;
        self.handle_fullscreen(ctx, !settings_here && !on_bar && !on_buttons);
    }

    /// The keys that work the same in every window: Ctrl+S opens the settings
    /// there, Ctrl+D, Ctrl+L and the playback keys. Returns whether Esc was
    /// pressed, which each window handles its own way.
    fn handle_shortcuts(&mut self, ctx: &egui::Context, screen: Screen) -> bool {
        let (settings_key, mirror_key, style_key, escape) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::D),
                i.modifiers.command && i.key_pressed(egui::Key::L),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if style_key {
            let style = {
                let mut s = self.settings.lock().unwrap();
                s.lyrics = match s.lyrics {
                    LyricsStyle::Words => LyricsStyle::Drum,
                    LyricsStyle::Drum => LyricsStyle::Lens,
                    LyricsStyle::Lens => LyricsStyle::Words,
                };
                s.save();
                s.lyrics
            };
            self.show_toast(match style {
                LyricsStyle::Words => "Texty po slovech",
                LyricsStyle::Drum => "Texty na válci",
                LyricsStyle::Lens => "Texty pod čočkou",
            });
        }
        if settings_key {
            if !self.settings_open {
                self.settings_open = true;
                self.settings_screen = screen;
            } else if self.settings_screen != screen {
                // Open on the other screen: bring it over here.
                self.settings_screen = screen;
            } else {
                self.close_settings();
            }
        }
        if mirror_key {
            self.toggle_mirror(ctx);
        }
        self.handle_playback_keys(ctx);
        escape
    }

    /// Ctrl+Up/Down turns what plays louder or quieter, Ctrl+Left/Right skips
    /// back or ahead, Ctrl+Space pauses
    /// and resumes it.
    fn handle_playback_keys(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || !self.settings.lock().unwrap().playback_keys {
            return; // typing in a settings field, or switched off
        }
        let press = ctx.input(|i| {
            let ctrl = i.modifiers.command;
            [
                (egui::Key::Space, Press::PlayPause),
                (egui::Key::ArrowUp, Press::Louder),
                (egui::Key::ArrowDown, Press::Quieter),
                (egui::Key::ArrowRight, Press::Next),
                (egui::Key::ArrowLeft, Press::Previous),
            ]
            .into_iter()
            .find(|(key, _)| ctrl && i.key_pressed(*key))
            .map(|(_, press)| press)
        });
        let Some(press) = press else {
            return;
        };
        self.playback(press);
    }

    /// Pauses or turns what plays louder or quieter, from a key or a button.
    fn playback(&mut self, press: Press) {
        let (source, now_playing) = {
            let scene = self.scene.lock().unwrap();
            (scene.source.clone(), scene.now_playing.clone())
        };
        let done = match press {
            Press::PlayPause => self.control.play_pause(source.as_ref()),
            Press::Next | Press::Previous => self.control.skip(source.as_ref(), press == Press::Next),
            Press::Louder | Press::Quieter => self.control.step_volume(
                source.as_ref(),
                now_playing.as_ref(),
                press == Press::Louder,
            ),
        };
        if !done {
            self.show_toast(if source.is_none() {
                "Nic nehraje"
            } else {
                "Tady hlasitost nastavit nejde"
            });
        }
    }

    /// F11 or double-click toggles fullscreen, Esc leaves it. Double-click is
    /// ignored while the settings panel is open (fast checkbox clicks).
    fn handle_fullscreen(&mut self, ctx: &egui::Context, allow_double_click: bool) {
        let (toggle, escape, is_fullscreen, maximized) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F11)
                    || (allow_double_click
                        && i.pointer
                            .button_double_clicked(egui::PointerButton::Primary)),
                i.key_pressed(egui::Key::Escape),
                i.viewport().fullscreen.unwrap_or(false),
                i.viewport().maximized.unwrap_or(false),
            )
        });
        if toggle && !is_fullscreen {
            // winit keeps a frameless maximized window inside the work area,
            // even in fullscreen: the taskbar stays and the picture squashes.
            // So restore first, and maximize again on the way out.
            self.restore_maximized = maximized;
            if maximized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        } else if (toggle || escape) && is_fullscreen {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            if std::mem::take(&mut self.restore_maximized) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
            }
        }
    }

    /// A click on the lyrics shows them big; a click anywhere else goes back.
    fn handle_lyrics_click(&mut self, ctx: &egui::Context, rect: Rect, f: &FrameData, split: f32) {
        // The lyrics are on their way to another screen.
        if split > 0.0 {
            self.lyrics_focus = false;
            return;
        }
        // Without lyrics there is nothing to grow. Big lyrics stay big while
        // the next song's load, though, until a click or Esc closes them.
        if !has_synced_lyrics(f) && !self.lyrics_focus {
            return;
        }
        let on_bar = self
            .title_bar
            .as_ref()
            .is_some_and(|bar| bar.under_pointer(ctx));
        if self.settings_open || on_bar || self.transport.under_pointer(ctx) {
            return;
        }
        let band = lyrics_band(rect, self.lyrics_focus_anim.progress());
        let (click, double, hover) = ctx.input(|i| {
            (
                i.pointer
                    .primary_clicked()
                    .then(|| i.pointer.interact_pos())
                    .flatten(),
                i.pointer.button_double_clicked(egui::PointerButton::Primary),
                i.pointer.hover_pos(),
            )
        });
        if !self.lyrics_focus && hover.is_some_and(|p| band.contains(p)) {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if double {
            // A double-click toggles fullscreen; take back what its first click did.
            if self
                .focus_toggled
                .take()
                .is_some_and(|t| t.elapsed() < DOUBLE_CLICK)
            {
                self.lyrics_focus = !self.lyrics_focus;
            }
            return;
        }
        if let Some(pos) = click
            && band.contains(pos) != self.lyrics_focus
        {
            self.lyrics_focus = !self.lyrics_focus;
            self.focus_toggled = Some(Instant::now());
        }
    }

    fn close_settings(&mut self) {
        self.settings_open = false;
        let mut settings = self.settings.lock().unwrap();
        if !settings.setup_done {
            settings.setup_done = true;
            settings.save();
        }
    }

    /// Ctrl+D: mirror the visuals fullscreen on the next monitor, or stop.
    fn toggle_mirror(&mut self, ctx: &egui::Context) {
        if self.mirror_on {
            self.mirror_on = false;
            return;
        }
        let displays = display_info::DisplayInfo::all().unwrap_or_default();
        if displays.len() < 2 {
            self.show_toast("Druhá obrazovka nenalezena");
            return;
        }
        // Find the monitor the main window is on and take the next one.
        // Assumes display-info lists monitors in the same order as winit,
        // which holds on Windows (both enumerate via EnumDisplayMonitors).
        let (center, ppp) = ctx.input(|i| {
            (
                i.viewport().outer_rect.map(|r| r.center()),
                i.pixels_per_point(),
            )
        });
        let current = center.and_then(|c| {
            let (x, y) = ((c.x * ppp) as i32, (c.y * ppp) as i32);
            displays.iter().position(|d| {
                x >= d.x && x < d.x + d.width as i32 && y >= d.y && y < d.y + d.height as i32
            })
        });
        let next = current.map_or(1, |i| (i + 1) % displays.len());
        let middle = |d: &display_info::DisplayInfo| {
            Vec2::new(
                d.x as f32 + d.width as f32 / 2.0,
                d.y as f32 + d.height as f32 / 2.0,
            )
        };
        self.mirror_dir = current
            .map(|i| (middle(&displays[next]) - middle(&displays[i])).normalized())
            .filter(|d| d.is_finite() && *d != Vec2::ZERO)
            .unwrap_or(Vec2::X);
        self.mirror = Some(next);
        self.mirror_on = true;
    }

    /// `shown` (0 -> 1) is the Ctrl+D transition, see `beam`.
    fn show_mirror(&mut self, ctx: &egui::Context, monitor: usize, frame: &FrameData, shown: f32) {
        let mut builder = egui::ViewportBuilder::default()
            .with_title("OZBEAT")
            .with_monitor(monitor)
            .with_visible(self.mirror_frames >= MIRROR_READY);
        if let Some(icon) = &self.icon {
            builder = builder.with_icon(Arc::clone(icon));
        }
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("ozbeat-mirror"),
            builder,
            |ui, _class| {
                let rect = ui.max_rect();
                let content = Content::LyricsOnly {
                    arriving: beam::arriving(rect, self.mirror_dir, shown),
                    clip_focus: self.clip_focus_anim.progress(),
                };
                let panel = self.panel_on(Screen::Mirror);
                ui.scope(|ui| {
                    ui.multiply_opacity(1.0 - 0.95 * ease_out_cubic(panel));
                    self.draw_visual(ui, frame, content);
                });
                let accent = color(frame.palette[0]);
                beam::draw_wipe(ui.painter(), rect, self.mirror_dir, shown, accent);
                let ctx = ui.ctx().clone();
                let escape = self.handle_shortcuts(&ctx, Screen::Mirror);
                let settings_here = self.settings_open && self.settings_screen == Screen::Mirror;
                self.draw_overlays(ui, Screen::Mirror, frame, panel);
                // A click swaps which is big, the video or the lyrics.
                if frame.clip.is_some()
                    && !settings_here
                    && !self.mirror_transport.under_pointer(&ctx)
                    && self.settings.lock().unwrap().clip_screen != ClipScreen::Main
                {
                    if ui.input(|i| i.pointer.hover_pos().is_some()) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if ui.input(|i| i.pointer.primary_clicked()) {
                        self.clip_focus = !self.clip_focus;
                    }
                }
                let close_requested = ui.input(|i| i.viewport().close_requested());
                if close_requested {
                    // Fade out first; the window goes once that is done.
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                }
                // Esc closes the panel here first, then the second screen.
                if escape && settings_here {
                    self.close_settings();
                }
                close = close_requested || (escape && !settings_here);
            },
        );
        self.mirror_frames = self.mirror_frames.saturating_add(1);
        if close && self.mirror_on {
            self.mirror_on = false;
        }
    }

    /// Screensaver: fullscreen on `screens[slot]`, the lyrics on the first
    /// extra screen (as with Ctrl+D) and the full visual on any further one;
    /// input there closes the whole screensaver.
    fn show_screen(&mut self, ctx: &egui::Context, slot: usize, frame: &FrameData) {
        let monitor = self.screens[slot];
        let builder = egui::ViewportBuilder::default()
            .with_title("OZBEAT")
            .with_monitor(monitor)
            .with_fullscreen(true)
            .with_decorations(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        let mut wake = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of(("ozbeat-screen", monitor)),
            builder,
            |ui, _class| {
                let content = if slot == 0 {
                    Content::LyricsOnly {
                        arriving: None,
                        clip_focus: 0.0,
                    }
                } else {
                    Content::Main {
                        split: 0.0,
                        leaving: None,
                        gone: false,
                        focus: 0.0,
                    }
                };
                self.draw_visual(ui, frame, content);
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                wake = self.wakes(ui, slot + 1) || ui.input(|i| i.viewport().close_requested());
            },
        );
        if wake {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Ctrl+Alt+T: the lyrics over the desktop on or off.
    fn toggle_overlay_on_key(&mut self) {
        let pressed = self
            .overlay_key
            .as_ref()
            .is_some_and(|key| key.swap(false, Ordering::Relaxed));
        if !pressed {
            return;
        }
        let on = {
            let mut s = self.settings.lock().unwrap();
            s.overlay = !s.overlay;
            s.save();
            s.overlay
        };
        self.show_toast(if on {
            "Texty přes plochu · vypneš Ctrl+Alt+T"
        } else {
            "Texty přes plochu vypnuté"
        });
    }

    /// Turns the window into the lyrics over the desktop, or back: see-through,
    /// fullscreen on its monitor, on top of every app and letting the mouse
    /// through to whatever is under it.
    fn set_overlay(&mut self, ctx: &egui::Context, on: bool) {
        use egui::ViewportCommand as Command;
        if on && self.overlay.is_some() {
            self.settle_overlay(ctx);
            return;
        }
        if on == self.overlay.is_some() {
            return;
        }
        if on {
            let (fullscreen, maximized) = ctx.input(|i| {
                let v = i.viewport();
                (v.fullscreen.unwrap_or(false), v.maximized.unwrap_or(false))
            });
            self.overlay = Some((fullscreen, maximized));
            self.overlay_rect = ctx.input(|i| {
                let v = i.viewport();
                Some((v.outer_rect?.min, v.inner_rect?.size()))
            });
            if fullscreen {
                ctx.send_viewport_cmd(Command::Fullscreen(false));
            }
            self.close_settings();
            self.lyrics_focus = false;
            // A maximized frameless window stays inside the work area, see `handle_fullscreen`.
            if maximized {
                ctx.send_viewport_cmd(Command::Maximized(false));
            }
            self.overlay_ready = false;
            crate::frameless::set_glass(true);
        } else if let Some((fullscreen, maximized)) = self.overlay.take() {
            crate::frameless::set_glass(false);
            ctx.send_viewport_cmd(Command::MousePassthrough(false));
            ctx.send_viewport_cmd(Command::WindowLevel(egui::WindowLevel::Normal));
            if let Some((pos, size)) = self.overlay_rect.take() {
                ctx.send_viewport_cmd(Command::OuterPosition(pos));
                ctx.send_viewport_cmd(Command::InnerSize(size));
            }
            ctx.send_viewport_cmd(Command::Fullscreen(fullscreen));
            if maximized {
                ctx.send_viewport_cmd(Command::Maximized(true));
            }
        }
    }

    /// A frame after the switch, the window covers its monitor (sized to it,
    /// not fullscreen, which winit loses next to the other changes), goes on
    /// top and lets the mouse through.
    fn settle_overlay(&mut self, ctx: &egui::Context) {
        use egui::ViewportCommand as Command;
        if !self.overlay_ready {
            // The monitor the window is on, moved over and sized to it.
            let (center, ppp) = ctx.input(|i| {
                (i.viewport().outer_rect.map(|r| r.center()), i.pixels_per_point())
            });
            let displays = display_info::DisplayInfo::all().unwrap_or_default();
            let monitor = center.and_then(|c| {
                let (x, y) = ((c.x * ppp) as i32, (c.y * ppp) as i32);
                displays.iter().find(|d| {
                    x >= d.x && x < d.x + d.width as i32 && y >= d.y && y < d.y + d.height as i32
                })
            });
            if let Some(d) = monitor {
                let scale = d.scale_factor.max(0.5);
                ctx.send_viewport_cmd(Command::OuterPosition(Pos2::new(d.x as f32 / scale, d.y as f32 / scale)));
                ctx.send_viewport_cmd(Command::InnerSize(Vec2::new(d.width as f32 / scale, d.height as f32 / scale)));
            }
            self.overlay_ready = true;
            ctx.send_viewport_cmd(Command::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            ctx.send_viewport_cmd(Command::MousePassthrough(true));
        }
    }

    /// The lyrics alone in the middle of the screen, white with a dark rim so
    /// they read over anything. Nothing shows while there is nothing to sing.
    fn draw_overlay(&mut self, ui: &mut egui::Ui, f: &FrameData) {
        if f.idle {
            return;
        }
        let rect = ui.max_rect();
        let center_y = rect.center().y;
        let size = big_lyrics_size(rect) * 0.8;
        // As over a music video: white words with a dark rim.
        let video = 1.0;
        if let Some((e, at, alpha)) = &f.outgoing
            && let Some(lyrics) = e.lyrics.as_ref()
        {
            let line = lyrics.line_at(*at).map_or(-1.0, |i| i as f32);
            draw_outgoing(ui, rect, f, video, lyrics, *at, line, center_y, size, *alpha);
        }
        let lyrics = f
            .enrichment
            .as_ref()
            .and_then(|e| e.lyrics.as_ref())
            .filter(|l| !l.synced.is_empty());
        if let Some(lyrics) = lyrics.filter(|_| f.lyrics > 0.0) {
            let line = f
                .position
                .and_then(|p| lyrics.line_at(p))
                .map_or(-1.0, |i| i as f32);
            let mut painter = ui.painter().clone();
            painter.multiply_opacity(f.lyrics);
            draw_timed_lyrics(&painter, rect, f, f.position, video, lyrics, line, center_y, size);
        }
    }

    /// How far the settings panel is open in `screen`'s window (0 -> 1).
    fn panel_on(&self, screen: Screen) -> f32 {
        if self.settings_screen == screen {
            self.settings_anim.progress()
        } else {
            0.0
        }
    }

    /// What goes over the visuals in `screen`'s window: the settings panel
    /// (`panel` open), or else the volume hint, and the playback buttons.
    fn draw_overlays(&mut self, ui: &egui::Ui, screen: Screen, f: &FrameData, panel: f32) {
        let ctx = ui.ctx().clone();
        let rect = ui.max_rect();
        let accent = color(f.palette[0]);
        let buttons = self.mode == Mode::Window
            && !f.idle
            && panel == 0.0
            && self.settings.lock().unwrap().buttons;
        let paused = f
            .now_playing
            .as_ref()
            .is_some_and(|np| np.state == PlaybackState::Paused);
        let transport = match screen {
            Screen::Main => &mut self.transport,
            Screen::Mirror => &mut self.mirror_transport,
        };
        if let Some(press) = transport.show(&ctx, rect, accent, paused, buttons) {
            self.playback(press);
        }
        if panel == 0.0 {
            self.volume.draw(&ctx, rect, accent);
        } else {
            // Dim the visuals a little so the panel reads well.
            ui.painter().rect_filled(
                rect,
                0.0,
                Color32::from_black_alpha((70.0 * ease_out_cubic(panel)) as u8),
            );
            let first_run = !self.settings.lock().unwrap().setup_done;
            let keep_open = setup::show(
                &ctx,
                panel,
                self.settings_open,
                &self.settings,
                first_run,
                &f.devices,
                f.audio.as_ref(),
                self.update.as_ref(),
            );
            if !keep_open && self.settings_open {
                self.close_settings();
            }
        }
        if screen == Screen::Mirror {
            // The main window draws its own toast last, over the title bar.
            self.draw_toast(&ctx);
        }
    }

    /// Mentions a new version once; the settings panel has the button.
    fn announce_update(&mut self) {
        if self.update_announced {
            return;
        }
        let available = self
            .update
            .as_ref()
            .and_then(|u| match &*u.lock().unwrap() {
                update::Status::Available(release) => Some(release.version.clone()),
                _ => None,
            });
        if let Some(version) = available {
            self.update_announced = true;
            self.show_toast(&format!(
                "Je tu nová verze {version} · aktualizuješ v nastavení (Ctrl+S)"
            ));
        }
    }

    fn show_toast(&mut self, text: &str) {
        self.toast = Some((text.to_owned(), Instant::now()));
    }

    /// Short confirmation at the bottom, fading out on its own.
    fn draw_toast(&mut self, ctx: &egui::Context) {
        let Some((text, since)) = &self.toast else {
            return;
        };
        let age = since.elapsed();
        if age > TOAST_VISIBLE + TOAST_FADE {
            self.toast = None;
            return;
        }
        let fade = 1.0
            - (age.saturating_sub(TOAST_VISIBLE).as_secs_f32() / TOAST_FADE.as_secs_f32()).min(1.0);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("ozbeat-toast"),
        ));
        let screen = ctx.content_rect();
        let galley = painter.layout_no_wrap(
            text.clone(),
            FontId::new(16.0, FontFamily::Name(REGULAR.into())),
            Color32::from_white_alpha((230.0 * fade) as u8),
        );
        let bg = Rect::from_center_size(
            Pos2::new(screen.center().x, screen.bottom() - 70.0),
            galley.size() + Vec2::new(36.0, 20.0),
        );
        painter.rect_filled(bg, 12.0, Color32::from_black_alpha((170.0 * fade) as u8));
        painter.galley(bg.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }
}

/// Positions derived from the window size, so everything scales together.
struct Layout {
    cover: Rect,
    text_left: f32,
    text_width: f32,
    title_size: f32,
}

impl Layout {
    /// `center` is the vertical position of the cover block, as a fraction of
    /// the height; `grow` (0 -> 1) makes it big, with the lyrics elsewhere.
    fn new(rect: Rect, center: f32, grow: f32) -> Self {
        let margin = rect.width() * 0.06;
        let small = (rect.height() * 0.42).min(rect.width() * 0.30);
        let large = (rect.height() * 0.62).min(rect.width() * 0.38);
        let side = egui::emath::lerp(small..=large, ease_in_out_cubic(grow));
        let cover = Rect::from_min_size(
            Pos2::new(
                rect.left() + margin,
                rect.top() + rect.height() * center - side / 2.0,
            ),
            Vec2::splat(side),
        );
        let text_left = cover.right() + margin * 0.6;
        Self {
            cover,
            text_left,
            text_width: (rect.right() - margin - text_left).max(100.0),
            // In 4 px steps, so growing doesn't fill the glyph atlas.
            title_size: ((side * 0.15).clamp(26.0, 120.0) / 4.0).round() * 4.0,
        }
    }
}

impl VisualApp {
    fn draw_cover(&self, ui: &egui::Ui, layout: &Layout, pulse: f32) {
        let rect = Rect::from_center_size(
            layout.cover.center(),
            layout.cover.size() * (1.0 + 0.006 * pulse),
        );
        let radius = rect.width() * 0.03;
        let shadow = egui::Shadow {
            offset: [0, 18],
            blur: 60,
            spread: 0,
            color: Color32::from_black_alpha(170),
        };
        ui.painter().add(shadow.as_shape(rect, radius));
        ui.painter()
            .rect_filled(rect, radius, Color32::from_white_alpha(18));
        for (tex, alpha) in self.art.layers(|t| t.cover.as_ref()).into_iter().flatten() {
            egui::Image::from_texture(SizedTexture::from_handle(tex))
                .corner_radius(radius)
                .tint(Color32::WHITE.gamma_multiply(alpha))
                .paint_at(ui, rect);
        }
    }

    /// Title, artist and album beside the cover; with `grow` (the cover big
    /// in the middle) also where the song is, as a bar with its times.
    fn draw_titles(
        &self,
        ui: &egui::Ui,
        layout: &Layout,
        np: &NowPlaying,
        position: Option<Duration>,
        accent: Color32,
        grow: f32,
    ) {
        let painter = ui.painter();
        let bold = |size: f32| FontId::new(size, FontFamily::Name(BOLD.into()));
        let regular = |size: f32| FontId::new(size, FontFamily::Name(REGULAR.into()));

        let title = painter.layout(
            np.track.title.clone(),
            bold(layout.title_size),
            Color32::WHITE,
            layout.text_width,
        );
        let artist_size = layout.title_size * 0.5;
        let photo_size = artist_size * 1.5;
        let artist = np.track.artist.clone().unwrap_or_default();
        let photos = self.art.layers(|t| t.photo.as_ref());
        let has_photo = photos.iter().any(Option::is_some);
        let artist_indent = if has_photo {
            photo_size + artist_size * 0.5
        } else {
            0.0
        };
        let artist = painter.layout(
            artist,
            regular(artist_size),
            Color32::from_white_alpha(220),
            layout.text_width - artist_indent,
        );
        let paused = np.state == PlaybackState::Paused;
        let mut album = np.track.album.clone().unwrap_or_default();
        if paused && !album.is_empty() {
            album.push_str("   ·   ");
        }
        let album_size = layout.title_size * 0.3;
        let album_color = Color32::from_white_alpha(130);
        let album = painter.layout(album, regular(album_size), album_color, layout.text_width);
        // The pause sign is drawn, not taken from the font: the font's ⏸
        // is a clumsy glyph that sits off the text's baseline.
        let paused_label = paused.then(|| {
            painter.layout_no_wrap("Pozastaveno".to_owned(), regular(album_size), album_color)
        });

        let gap = layout.title_size * 0.35;
        let artist_row = artist
            .size()
            .y
            .max(if has_photo { photo_size } else { 0.0 });
        let timeline = position.zip(np.duration).filter(|_| grow > 0.0);
        let timeline_height = album_size * 2.6 * grow;
        let total = title.size().y
            + gap
            + artist_row
            + gap * 0.6
            + album.size().y
            + if timeline.is_some() { timeline_height } else { 0.0 };
        let mut y = layout.cover.center().y - total / 2.0;

        painter.galley(
            Pos2::new(layout.text_left, y),
            title.clone(),
            Color32::WHITE,
        );
        y += title.size().y + gap;

        for (photo, alpha) in photos.into_iter().flatten() {
            let rect = Rect::from_min_size(
                Pos2::new(layout.text_left, y + (artist_row - photo_size) / 2.0),
                Vec2::splat(photo_size),
            );
            egui::Image::from_texture(SizedTexture::from_handle(photo))
                .uv(square_crop(photo.size_vec2()))
                .corner_radius(photo_size / 2.0)
                .tint(Color32::WHITE.gamma_multiply(alpha))
                .paint_at(ui, rect);
        }
        painter.galley(
            Pos2::new(
                layout.text_left + artist_indent,
                y + (artist_row - artist.size().y) / 2.0,
            ),
            artist,
            Color32::WHITE,
        );
        y += artist_row + gap * 0.6;
        let album_pos = Pos2::new(layout.text_left, y);
        if let Some(label) = paused_label {
            // Right after the album, on its last line.
            let row = album
                .rows
                .last()
                .map_or(Rect::from_min_size(Pos2::ZERO, Vec2::new(0.0, label.size().y)), |r| {
                    r.rect()
                });
            let height = album_size * 0.62;
            let bar = Vec2::new(height * 0.3, height);
            let left = album_pos.x + if album.is_empty() { 0.0 } else { row.right() };
            let center_y = album_pos.y + row.center().y;
            for x in [left, left + bar.x + height * 0.26] {
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x, center_y - bar.y / 2.0), bar),
                    bar.x / 2.0,
                    album_color,
                );
            }
            let label_x = left + bar.x * 2.0 + height * 0.26 + album_size * 0.45;
            painter.galley(
                Pos2::new(label_x, center_y - label.size().y / 2.0),
                label,
                Color32::WHITE,
            );
        }
        let album_height = album.size().y;
        painter.galley(album_pos, album, Color32::WHITE);

        if let Some((position, duration)) = timeline {
            let y = album_pos.y + album_height + album_size * 1.1;
            let width = layout.text_width.min(layout.title_size * 9.0);
            let fraction = (position.as_secs_f32() / duration.as_secs_f32().max(1.0)).clamp(0.0, 1.0);
            let track = Rect::from_min_size(Pos2::new(layout.text_left, y), Vec2::new(width, 4.0));
            let fade = |c: Color32| c.gamma_multiply(grow);
            painter.rect_filled(track, 2.0, fade(Color32::from_white_alpha(40)));
            let done = Rect::from_min_size(track.min, Vec2::new(width * fraction, 4.0));
            painter.rect_filled(done, 2.0, fade(accent));
            let time = |d: Duration| format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60);
            let font = regular(album_size * 0.85);
            let below = y + album_size * 0.7;
            painter.text(
                Pos2::new(track.left(), below),
                Align2::LEFT_TOP,
                time(position.min(duration)),
                font.clone(),
                fade(album_color),
            );
            painter.text(
                Pos2::new(track.right(), below),
                Align2::RIGHT_TOP,
                time(duration),
                font,
                fade(album_color),
            );
        }
    }
}

/// Shrinks `font` so `text` fits `max_width` on one line. Sizes are snapped
/// to 4 px steps so long lines don't fill the glyph atlas with odd sizes.
fn fit_font(painter: &egui::Painter, text: &str, font: &FontId, max_width: f32) -> FontId {
    let width = painter
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    shrunk(font, width, max_width).unwrap_or_else(|| font.clone())
}

/// A smaller `font` for text `width` wide to fit `max_width`, if it doesn't.
fn shrunk(font: &FontId, width: f32, max_width: f32) -> Option<FontId> {
    if width <= max_width {
        return None;
    }
    let size = ((font.size * max_width / width) / 4.0).floor() * 4.0;
    Some(FontId::new(size.max(12.0), font.family.clone()))
}

/// The video filling `rect` (cropped, not stretched), darkened towards the
/// bottom so the lyrics over it stay readable.
fn draw_clip(painter: &egui::Painter, rect: Rect, clip: ClipFrame) {
    let fill = (rect.width() / clip.size.x).max(rect.height() / clip.size.y);
    let shown = rect.size() / (clip.size * fill);
    let uv = Rect::from_center_size(Pos2::new(0.5, 0.5), shown);
    painter.image(clip.texture, rect, uv, Color32::WHITE.gamma_multiply(clip.alpha));

    let shade = |a: f32| Color32::from_black_alpha((a * clip.alpha) as u8);
    let mid = rect.top() + rect.height() * 0.55;
    let mut mesh = egui::Mesh::default();
    for (y, a) in [(rect.top(), 90.0), (mid, 40.0), (rect.bottom(), 215.0)] {
        mesh.colored_vertex(Pos2::new(rect.left(), y), shade(a));
        mesh.colored_vertex(Pos2::new(rect.right(), y), shade(a));
    }
    for i in 0..4 {
        mesh.add_triangle(i, i + 1, i + 2);
    }
    painter.add(mesh);
}

/// Title and artist small in the corner, over the music video.
fn draw_clip_caption(painter: &egui::Painter, rect: Rect, np: &NowPlaying) {
    let size = (rect.height() * 0.03).clamp(16.0, 34.0);
    let title = painter.text(
        rect.left_top() + Vec2::new(size * 1.6, size * 1.4),
        Align2::LEFT_TOP,
        &np.track.title,
        FontId::new(size, FontFamily::Name(BOLD.into())),
        Color32::WHITE,
    );
    if let Some(artist) = &np.track.artist {
        painter.text(
            Pos2::new(title.left(), title.bottom() + size * 0.2),
            Align2::LEFT_TOP,
            artist,
            FontId::new(size * 0.7, FontFamily::Name(REGULAR.into())),
            Color32::from_white_alpha(190),
        );
    }
}

/// Title and artist, centered: the lyrics screen when there are no timed lyrics.
fn draw_caption(painter: &egui::Painter, rect: Rect, np: &NowPlaying) {
    let size = (rect.height() * 0.08).clamp(28.0, 110.0);
    let max_width = rect.width() * 0.9;
    let title = FontId::new(size, FontFamily::Name(BOLD.into()));
    let title = fit_font(painter, &np.track.title, &title, max_width);
    painter.text(
        rect.center() - Vec2::new(0.0, size * 0.35),
        Align2::CENTER_CENTER,
        &np.track.title,
        title,
        Color32::WHITE,
    );
    if let Some(artist) = &np.track.artist {
        painter.text(
            rect.center() + Vec2::new(0.0, size * 0.6),
            Align2::CENTER_CENTER,
            artist,
            FontId::new(size * 0.45, FontFamily::Name(REGULAR.into())),
            Color32::from_white_alpha(200),
        );
    }
}

/// Shown when nothing plays: a gently breathing hint instead of an empty window.
fn draw_idle(painter: &egui::Painter, rect: Rect, time: f32) {
    let breath = 0.5 + 0.5 * (time * 0.8).sin();
    let title_size = (rect.height() * 0.07).clamp(28.0, 72.0);
    painter.text(
        rect.center() - Vec2::new(0.0, title_size * 0.35),
        Align2::CENTER_CENTER,
        "Teď nic nehraje",
        FontId::new(title_size, FontFamily::Name(BOLD.into())),
        Color32::from_white_alpha((150.0 + 60.0 * breath) as u8),
    );
    painter.text(
        rect.center() + Vec2::new(0.0, title_size * 0.6),
        Align2::CENTER_CENTER,
        "Pusť hudbu ve Spotify nebo na reproduktoru",
        FontId::new(title_size * 0.32, FontFamily::Name(REGULAR.into())),
        Color32::from_white_alpha(110),
    );
}

/// Lyrics under the cover on the main window: their size and current line.
fn main_lyrics_size(rect: Rect) -> f32 {
    (rect.height() * 0.045).clamp(20.0, 54.0)
}

fn main_lyrics_y(rect: Rect) -> f32 {
    rect.top() + rect.height() * 0.83
}

/// Lyrics filling a window: the second screen, or clicked on the main one.
fn big_lyrics_size(rect: Rect) -> f32 {
    (rect.height() * 0.075).clamp(28.0, 96.0)
}

/// Current line and font size of the main window's lyrics, `focus` (0 -> 1,
/// eased) of the way from under the cover to big in the middle.
fn lyrics_placement(rect: Rect, focus: f32) -> (f32, f32) {
    let y = egui::emath::lerp(main_lyrics_y(rect)..=rect.center().y, focus);
    let size = egui::emath::lerp(main_lyrics_size(rect)..=big_lyrics_size(rect), focus);
    (y, size)
}

/// Where a click counts as on the lyrics: the current line and one either side.
fn lyrics_band(rect: Rect, focus: f32) -> Rect {
    let (y, size) = lyrics_placement(rect, ease_in_out_cubic(focus));
    Rect::from_center_size(
        Pos2::new(rect.center().x, y),
        Vec2::new(rect.width() * 0.9, size * 1.3 * 3.0),
    )
}

fn has_synced_lyrics(f: &FrameData) -> bool {
    !f.idle
        && f.enrichment
            .as_ref()
            .and_then(|e| e.lyrics.as_ref())
            .is_some_and(|l| !l.synced.is_empty())
}

/// Scales by `scale` about `anchor`, which ends up at height `y`.
fn scaled_about(anchor: Pos2, y: f32, scale: f32) -> TSTransform {
    TSTransform::new(
        Vec2::new(anchor.x * (1.0 - scale), y - scale * anchor.y),
        scale,
    )
}

/// Over a music video (`video`, 0 -> 1 as it fades in), a dark band behind
/// the lyrics at `center_y` keeps them readable whatever the picture. Drawn
/// apart from the lyrics, since their moves and scaling would crop it.
fn draw_lyrics_shade(
    painter: &egui::Painter,
    rect: Rect,
    f: &FrameData,
    center_y: f32,
    size: f32,
    video: f32,
) {
    if video <= 0.0 {
        return;
    }
    let reach = match (f.position, f.lyrics_style) {
        (None, _) => size * 2.4,
        (Some(_), LyricsStyle::Words | LyricsStyle::Lens) => size * WORD_SCALE * 1.7,
        (Some(_), LyricsStyle::Drum) => size * DRUM_SCALE * 3.2,
    };
    draw_band(painter, rect, center_y, reach, 175.0 * video);
}

/// The lyrics in the chosen style, also when their timing is only a guess
/// from plain lyrics; whole lines when the player doesn't say where in the
/// song it is. Over a music video (`video`) the words turn white.
#[allow(clippy::too_many_arguments)]
fn draw_timed_lyrics(
    painter: &egui::Painter,
    rect: Rect,
    f: &FrameData,
    position: Option<Duration>,
    video: f32,
    lyrics: &mv_enrich::Lyrics,
    scroll: f32,
    center_y: f32,
    size: f32,
) {
    let Some(position) = position else {
        draw_lyrics(painter, rect, lyrics, None, scroll, center_y, size);
        return;
    };
    let vivid = f.palette[0];
    match f.lyrics_style {
        LyricsStyle::Words => draw_words(painter, rect, lyrics, position, center_y, size, vivid, video, None),
        LyricsStyle::Drum => draw_drum(painter, rect, lyrics, position, center_y, size, vivid, video),
        LyricsStyle::Lens => draw_lens(painter, rect, lyrics, position, center_y, size, vivid, video),
    }
}

/// The last song's lyrics, frozen where they stopped (`at`, on `line`),
/// drifting up and away at `alpha` as the new song begins.
#[allow(clippy::too_many_arguments)]
fn draw_outgoing(
    ui: &mut egui::Ui,
    rect: Rect,
    f: &FrameData,
    video: f32,
    lyrics: &mv_enrich::Lyrics,
    at: Duration,
    line: f32,
    center_y: f32,
    size: f32,
    alpha: f32,
) {
    let lift = TSTransform::from_translation(Vec2::new(0.0, -size * 1.5 * (1.0 - alpha)));
    ui.with_visual_transform(lift, |ui| {
        let mut painter = ui.painter().clone();
        painter.multiply_opacity(alpha);
        draw_timed_lyrics(&painter, rect, f, Some(at), video, lyrics, line, center_y, size);
    });
}

/// Black across `rect`, `alpha` at `center_y` and fading out `reach` above and below.
fn draw_band(painter: &egui::Painter, rect: Rect, center_y: f32, reach: f32, alpha: f32) {
    let shade = |a: f32| Color32::from_black_alpha(a.round().clamp(0.0, 255.0) as u8);
    let mut mesh = Mesh::default();
    let stops = [(-1.0, 0.0), (-0.45, 0.8), (0.0, 1.0), (0.45, 0.8), (1.0, 0.0)];
    for (y, a) in stops {
        let y = (center_y + y * reach).clamp(rect.top(), rect.bottom());
        mesh.colored_vertex(Pos2::new(rect.left(), y), shade(alpha * a));
        mesh.colored_vertex(Pos2::new(rect.right(), y), shade(alpha * a));
    }
    for i in 0..(stops.len() as u32 - 1) * 2 {
        mesh.add_triangle(i, i + 1, i + 2);
    }
    painter.add(mesh);
}

/// How long the stack takes to move up by one word, unless words come faster.
const WORD_STEP: f32 = 0.3;
/// Size of the word in the middle, against the lines' size at the same spot.
const WORD_SCALE: f32 = 2.6;

// A word's look by its place in the stack, `d` words below the middle one
// (0 is the word being sung, at the bottom; -1 the big one in the middle),
// each as a curve through (d, value) points.
const WORD_SIZE: &[(f32, f32)] = &[(-3.0, 0.42), (-2.0, 0.55), (-1.0, 1.0), (0.0, 0.6), (1.0, 0.45)];
/// Height of the word's center below the middle, in big font sizes.
const WORD_Y: &[(f32, f32)] = &[(-3.0, -1.25), (-2.0, -0.78), (-1.0, 0.0), (0.0, 0.8), (1.0, 1.2)];
const WORD_ALPHA: &[(f32, f32)] = &[
    (-3.2, 0.0),
    (-2.6, 0.25),
    (-2.0, 0.7),
    (-1.0, 1.0),
    (0.1, 1.0),
    (0.7, 0.0),
];
/// The highlight behind the word being sung, gone by the time it's big.
const WORD_BOX: &[(f32, f32)] = &[(-0.75, 0.0), (0.0, 1.0), (0.15, 1.0), (0.6, 0.0)];

/// Lyrics one word at a time, the way Verci shows them: each word pops in
/// at the bottom on a highlight the moment it is sung, rises into a big word
/// in the middle, then shrinks and fades on its way up. Words get a slight
/// tilt each and the icon of what they name. They are laid out at two sizes
/// and drawn as scaled meshes, so they grow smoothly without filling the
/// glyph atlas with a font size per frame. Over a music video (`video`) the
/// words turn white with a dark rim instead of the cover's tone. Everything
/// drawn goes through `bend`, if given, with glyphs cut finer to curve.
#[allow(clippy::too_many_arguments)]
fn draw_words(
    painter: &egui::Painter,
    rect: Rect,
    lyrics: &mv_enrich::Lyrics,
    position: Duration,
    center_y: f32,
    size: f32,
    vivid: [f32; 3],
    video: f32,
    bend: Option<&dyn Fn(Pos2) -> Pos2>,
) {
    struct Word {
        text: String,
        at: Duration,
        seed: u64,
    }
    // The line before, for the words still rising out of view, and a few after.
    let first = lyrics.line_at(position).map_or(0, |i| i.saturating_sub(1));
    let mut words = Vec::new();
    for (i, line) in lyrics.synced.iter().enumerate().skip(first).take(4) {
        for (k, (range, at)) in lyrics.word_starts(i).into_iter().enumerate() {
            words.push(Word {
                text: line.text.chars().skip(range.start).take(range.len()).collect(),
                at,
                seed: (i as u64) << 20 | k as u64,
            });
        }
    }
    let sung = words.partition_point(|w| w.at <= position);
    let Some(current) = sung.checked_sub(1) else {
        return;
    };
    let since = (position - words[current].at).as_secs_f32();
    let step = words.get(current + 1).map_or(WORD_STEP, |w| {
        (w.at - words[current].at).as_secs_f32().min(WORD_STEP)
    });
    let scroll = current as f32 - 1.0 + ease_out_cubic((since / step.max(0.05)).min(1.0));
    // In a pause between lines, the words already sung fade back.
    let resting = 1.0 - 0.6 * remap(since, 3.0, 4.0);

    // Rasters snapped to 4 px steps, so a resized window doesn't keep adding sizes.
    let big = ((size * WORD_SCALE / 4.0).round() * 4.0).max(16.0);
    let small = big * 0.6;
    // A bend swells the middle, so its words are laid out bigger to stay sharp.
    let sharpen = if bend.is_some() { 1.6 } else { 1.0 };
    // Set tight, like display type.
    let layout = |text: &str, px: f32| {
        let format = TextFormat {
            font_id: FontId::new(px, FontFamily::Name(HEAVY.into())),
            extra_letter_spacing: -0.035 * px,
            ..Default::default()
        };
        painter.layout_job(LayoutJob::single_section(text.to_owned(), format))
    };
    let icon_font = |px: f32| FontId::new(px, FontFamily::Name(icons::FONT.into()));
    let (text_color, box_color) = word_colors(vivid);
    let text_color = lerp_color(text_color, Color32::WHITE, video);
    let max_width = rect.width() * 0.9;

    struct Placed {
        galley: Arc<Galley>,
        /// The icon, which side it's on (-1 or 1) and how far it has popped in.
        icon: Option<(Arc<Galley>, f32, f32)>,
        center: Pos2,
        scale: f32,
        angle: f32,
        alpha: f32,
        highlight: f32,
    }
    let mut placed = Vec::new();
    for (i, word) in words.iter().enumerate() {
        let d = i as f32 - scroll - 1.0;
        let mut alpha = curve(WORD_ALPHA, d);
        if i < current {
            alpha *= resting;
        }
        if alpha <= 0.0 || word.text.is_empty() {
            continue;
        }
        let px = big * curve(WORD_SIZE, d);
        let raster = if px <= small * 1.05 { small } else { big } * sharpen;
        let galley = layout(&word.text, raster);
        // Long words shrink to fit the width at their biggest.
        let fit = (max_width / (galley.size().x * big / raster)).min(1.0);
        let scale = px / raster * fit;
        // Beside a word under a bend, an icon would be crushed at the rim.
        let icon = icons::for_word(&word.text).filter(|_| bend.is_none()).map(|icon| {
            let galley = painter.layout_no_wrap(icon.to_string(), icon_font(raster), Color32::WHITE);
            // Pops in with a little overshoot as its word is sung.
            let pop = spring(remap(-d, -0.6, 0.6));
            let side = if wobble(word.seed ^ 0x5eed) > 0.0 { 1.0 } else { -1.0 };
            (galley, side, pop)
        });
        let center = Pos2::new(
            rect.center().x + wobble(word.seed ^ 0xa11) * 0.1 * px,
            // The stack, rather than its middle word, centered.
            center_y + big * (curve(WORD_Y, d) - 0.1),
        );
        placed.push(Placed {
            galley,
            icon,
            center,
            scale,
            angle: wobble(word.seed) * 0.05,
            alpha,
            highlight: curve(WORD_BOX, d),
        });
    }

    // UVs are in texels of the font atlas, read once everything is laid out.
    let atlas = Vec2::from(painter.fonts(|f| f.font_image_size()).map(|v| v as f32));
    let glyphs = |mesh: &mut Mesh, galley: &Galley, at: Pos2, scale: f32, angle: f32, color: Color32| match bend {
        Some(bend) => {
            let turn = Rot2::from_angle(angle);
            let middle = galley.size() / 2.0;
            warped_mesh(mesh, galley, atlas, |v| bend(at + turn * ((v - middle) * scale)), color, 4);
        }
        None => galley_mesh(mesh, galley, atlas, at, scale, angle, color),
    };
    for word in placed {
        let turn = Rot2::from_angle(word.angle);
        let place = |v: Vec2| word.center + turn * (v * word.scale);
        let half = word.galley.size() / 2.0;
        if word.highlight > 0.0 {
            let ink = word
                .galley
                .rows
                .iter()
                .map(|r| r.visuals.mesh_bounds.translate(r.pos.to_vec2()))
                .fold(Rect::NOTHING, Rect::union);
            let pad = Vec2::new(0.1, 0.05) * word.galley.size().y;
            let ink = ink.expand2(pad).translate(-half);
            let fill = box_color.gamma_multiply(word.alpha * word.highlight);
            if let Some(bend) = bend {
                painter.add(bent_rect(ink, |v| bend(place(v)), fill));
            } else {
                let corners = [ink.left_top(), ink.right_top(), ink.right_bottom(), ink.left_bottom()];
                painter.add(Shape::convex_polygon(
                    corners.map(|c| place(c.to_vec2())).to_vec(),
                    fill,
                    Stroke::NONE,
                ));
            }
        }
        let icon = word.icon.filter(|i| i.2 > 0.0).map(|(icon, side, pop)| {
            let gap = 0.25 * word.galley.size().y;
            let offset = side * ((word.galley.size().x + icon.size().x) / 2.0 + gap);
            (icon, place(Vec2::new(offset, 0.0)), pop)
        });
        // Over a video: a dark rim, as copies of the glyphs around them.
        let rim = 0.12 * word.alpha * video * (1.0 - word.highlight);
        if rim > 0.0 {
            let mut shadow = Mesh::default();
            let shade = Color32::from_black_alpha((rim * 255.0) as u8);
            let radius = 0.045 * word.galley.size().y * word.scale;
            for k in 0..8 {
                let offset = Vec2::angled(k as f32 * TAU / 8.0) * radius;
                let at = place(Vec2::ZERO) + offset;
                glyphs(&mut shadow, &word.galley, at, word.scale, word.angle, shade);
                if let Some((icon, at_icon, pop)) = &icon {
                    let at = *at_icon + offset;
                    glyphs(&mut shadow, icon, at, word.scale * pop, word.angle, shade);
                }
            }
            painter.add(Shape::mesh(shadow));
        }
        // Light text on its highlight, the cover's tone once it's big.
        let color = lerp_color(text_color, Color32::WHITE, word.highlight).gamma_multiply(word.alpha);
        let mut mesh = Mesh::default();
        glyphs(&mut mesh, &word.galley, place(Vec2::ZERO), word.scale, word.angle, color);
        if let Some((icon, at, pop)) = icon {
            let tint = lerp_color(lerp_color(text_color, box_color, 0.5), Color32::WHITE, video * 0.5);
            let color = tint.gamma_multiply(word.alpha * pop.min(1.0));
            glyphs(&mut mesh, &icon, at, word.scale * pop, word.angle, color);
        }
        painter.add(Shape::mesh(mesh));
    }
}

/// Size of the line on the front of the drum, against the lines' size.
const DRUM_SCALE: f32 = 1.35;
/// Turn of the drum from one line to the next, in radians.
const DRUM_STEP: f32 = 0.42;
/// How long the lyrics take to move on to the next line, unless it comes sooner.
const LINE_TURN: f32 = 0.45;

/// How far the lyrics have moved on, in lines, `now` being the line sung:
/// easing from the line before to it as it starts.
fn line_scroll(lyrics: &mv_enrich::Lyrics, now: usize, position: Duration) -> f32 {
    let line_start = lyrics.synced[now].at;
    let since = (position - line_start).as_secs_f32();
    let turn = lyrics.synced.get(now + 1).map_or(LINE_TURN, |next| {
        (next.at - line_start).as_secs_f32().min(LINE_TURN)
    });
    now as f32 - 1.0 + ease_out_cubic((since / turn.max(0.05)).min(1.0))
}

/// A word of a line laid out in a row, or the icon after it.
struct Piece {
    galley: Arc<Galley>,
    /// Where it starts along the row.
    x: f32,
    word: usize,
    icon: bool,
}

/// Line `i`'s words and their icons laid out in a row, and the row's width.
fn line_pieces(
    painter: &egui::Painter,
    lyrics: &mv_enrich::Lyrics,
    i: usize,
    font: &FontId,
    icon_font: &FontId,
) -> (Vec<Piece>, f32) {
    let text = &lyrics.synced[i].text;
    let space = painter.layout_no_wrap(" ".into(), font.clone(), Color32::WHITE).size().x;
    let mut pieces = Vec::new();
    let mut x = 0.0;
    for (k, (range, _)) in lyrics.word_starts(i).into_iter().enumerate() {
        let word: String = text.chars().skip(range.start).take(range.len()).collect();
        let galley = painter.layout_no_wrap(word.clone(), font.clone(), Color32::WHITE);
        let width = galley.size().x;
        pieces.push(Piece { galley, x, word: k, icon: false });
        x += width;
        if let Some(icon) = icons::for_word(&word) {
            let galley = painter.layout_no_wrap(icon.to_string(), icon_font.clone(), Color32::WHITE);
            x += space * 0.5;
            let width = galley.size().x;
            pieces.push(Piece { galley, x, word: k, icon: true });
            x += width;
        }
        x += space;
    }
    (pieces, (x - space).max(1.0))
}

/// Color, highlight and brightness of word `k` of a line. On the line being
/// sung (`front`) `sung` of its words have started, the latest `since`
/// seconds ago; other lines are either `past` or still to come.
fn word_look(
    front: bool,
    past: bool,
    k: usize,
    sung: usize,
    since: Option<f32>,
    text_color: Color32,
) -> (Color32, f32, f32) {
    if !front {
        return if past {
            (text_color, 0.0, 0.55)
        } else {
            (Color32::WHITE, 0.0, 0.35)
        };
    }
    if k + 1 < sung {
        (text_color, 0.0, 1.0)
    } else if k + 1 == sung {
        let pop = since.map_or(1.0, |t| remap(t, 0.0, 0.12));
        (lerp_color(text_color, Color32::WHITE, pop), pop, 1.0)
    } else {
        (Color32::WHITE, 0.0, 0.5)
    }
}

/// Lyrics on a turning drum: the lines sit around a cylinder lying across
/// the screen, the one being sung on its front, big and facing you, and the
/// others tipping away above and below it in perspective. The words of the
/// front line light up as they're sung, the current one on a highlight.
/// Every glyph is projected, so the lines really lean back rather than just
/// shrink.
#[allow(clippy::too_many_arguments)]
fn draw_drum(
    painter: &egui::Painter,
    rect: Rect,
    lyrics: &mv_enrich::Lyrics,
    position: Duration,
    center_y: f32,
    size: f32,
    vivid: [f32; 3],
    video: f32,
) {
    let Some(now) = lyrics.line_at(position) else {
        return;
    };
    let scroll = line_scroll(lyrics, now, position);

    let px = ((size * DRUM_SCALE / 4.0).round() * 4.0).max(16.0);
    let radius = px * 3.0;
    // The camera's distance from the drum's front; nearer bends harder.
    let eye = radius * 2.6;
    let (text_color, box_color) = word_colors(vivid);
    let text_color = lerp_color(text_color, Color32::WHITE, video);
    let max_width = rect.width() * 0.88;
    let font = FontId::new(px, FontFamily::Name(HEAVY.into()));
    let icon_font = FontId::new(px * 0.85, FontFamily::Name(icons::FONT.into()));
    let atlas = Vec2::from(painter.fonts(|f| f.font_image_size()).map(|v| v as f32));

    let mut shapes = Vec::new();
    let reach = (std::f32::consts::FRAC_PI_2 / DRUM_STEP).ceil() as usize;
    for i in now.saturating_sub(reach)..(now + reach + 1).min(lyrics.synced.len()) {
        let angle = (i as f32 - scroll) * DRUM_STEP;
        // Lines fade as they turn away, gone before they'd face backward.
        let alpha = remap(angle.cos(), 0.2, 0.85);
        if alpha <= 0.0 || lyrics.synced[i].text.trim().is_empty() {
            continue;
        }
        let (pieces, width) = line_pieces(painter, lyrics, i, &font, &icon_font);
        let fit = (max_width / width).min(1.0);

        let starts = lyrics.word_starts(i);
        let sung = starts.partition_point(|(_, at)| *at <= position);
        let since = sung.checked_sub(1).map(|k| (position - starts[k].1).as_secs_f32());

        // A point `local` raster px from the line's middle, on the drum.
        let project = |local: Vec2| {
            let a = angle + local.y * fit / radius;
            let depth = radius * (1.0 - a.cos());
            let s = eye / (eye + depth);
            Pos2::new(
                rect.center().x + local.x * fit * s,
                center_y + radius * a.sin() * s,
            )
        };
        let mut mesh = Mesh::default();
        for piece in &pieces {
            let (color, highlight, dim) = word_look(i == now, i < now, piece.word, sung, since, text_color);
            let offset = Vec2::new(piece.x - width / 2.0, -piece.galley.size().y / 2.0);
            if highlight > 0.0 && !piece.icon {
                let ink = piece
                    .galley
                    .rows
                    .iter()
                    .map(|r| r.visuals.mesh_bounds.translate(r.pos.to_vec2()))
                    .fold(Rect::NOTHING, Rect::union);
                let pad = Vec2::new(0.12, 0.06) * piece.galley.size().y;
                let ink = ink.expand2(pad).translate(offset);
                let corners = [ink.left_top(), ink.right_top(), ink.right_bottom(), ink.left_bottom()];
                shapes.push(Shape::convex_polygon(
                    corners.map(|c| project(c.to_vec2())).to_vec(),
                    box_color.gamma_multiply(alpha * highlight),
                    Stroke::NONE,
                ));
            }
            let tint = if piece.icon {
                lerp_color(color, box_color, 0.35)
            } else {
                color
            };
            projected_mesh(&mut mesh, &piece.galley, atlas, |v| project(v + offset), tint.gamma_multiply(alpha * dim));
        }
        shapes.push(Shape::mesh(mesh));
    }
    painter.extend(shapes);
}

/// Words under the fisheye are laid out this much smaller, as it swells
/// them back up in the middle.
const LENS_SIZE: f32 = 0.82;
/// How hard the fisheye bends: the middle grows by this over its tanh.
const LENS_BEND: f32 = 1.9;

/// The word stack of [`draw_words`] seen through a fisheye lens, the way
/// Verci does it: big and bulging in the middle, words curving and
/// shrinking as they rise toward the rim.
#[allow(clippy::too_many_arguments)]
fn draw_lens(
    painter: &egui::Painter,
    rect: Rect,
    lyrics: &mv_enrich::Lyrics,
    position: Duration,
    center_y: f32,
    size: f32,
    vivid: [f32; 3],
    video: f32,
) {
    let center = Pos2::new(rect.center().x, center_y);
    // About the stack's own size, so every word in it curves.
    let reach = Vec2::splat(size * WORD_SCALE * 1.45);
    let bend = fisheye(center, reach);
    draw_words(painter, rect, lyrics, position, center_y, size * LENS_SIZE, vivid, video, Some(&bend));
}

/// A fisheye centered on `center`: points `reach` away (an ellipse) stay
/// put, nearer ones are pushed out, farther ones pulled in, toward a rim
/// they never cross.
fn fisheye(center: Pos2, reach: Vec2) -> impl Fn(Pos2) -> Pos2 {
    move |p| {
        let off = p - center;
        let r = (off / reach).length();
        if r < 1e-4 {
            return center + off * (LENS_BEND / LENS_BEND.tanh());
        }
        center + off * ((LENS_BEND * r).tanh() / (LENS_BEND.tanh() * r))
    }
}

/// A flat `color` mesh over `rect`, cut into a grid so `place` can bend it.
fn bent_rect(rect: Rect, place: impl Fn(Vec2) -> Pos2, color: Color32) -> Mesh {
    const STEPS: u32 = 8;
    let mut mesh = Mesh::default();
    for y in 0..=STEPS {
        for x in 0..=STEPS {
            let t = Vec2::new(x as f32, y as f32) / STEPS as f32;
            mesh.colored_vertex(place((rect.min + rect.size() * t).to_vec2()), color);
        }
    }
    for y in 0..STEPS {
        for x in 0..STEPS {
            let a = y * (STEPS + 1) + x;
            let b = a + STEPS + 1;
            mesh.add_triangle(a, a + 1, b);
            mesh.add_triangle(a + 1, b + 1, b);
        }
    }
    mesh
}

/// Like [`projected_mesh`], but each glyph is first cut into a `steps` by
/// `steps` grid, so a strong bend curves it instead of just moving its corners.
fn warped_mesh(
    mesh: &mut Mesh,
    galley: &Galley,
    atlas: Vec2,
    place: impl Fn(Vec2) -> Pos2,
    color: Color32,
    steps: u32,
) {
    let bounds = |points: &mut dyn Iterator<Item = Pos2>| {
        points.fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(p, p)))
    };
    for row in &galley.rows {
        // Glyphs come as axis-aligned quads of four vertices.
        for quad in row.visuals.mesh.vertices.chunks_exact(4) {
            let pos = bounds(&mut quad.iter().map(|v| v.pos));
            let uv = bounds(&mut quad.iter().map(|v| v.uv));
            let base = mesh.vertices.len() as u32;
            for y in 0..=steps {
                for x in 0..=steps {
                    let t = Vec2::new(x as f32, y as f32) / steps as f32;
                    let at = pos.min + pos.size() * t;
                    let uv = uv.min + uv.size() * t;
                    mesh.vertices.push(Vertex {
                        pos: place(row.pos.to_vec2() + at.to_vec2()),
                        uv: Pos2::new(uv.x / atlas.x, uv.y / atlas.y),
                        color,
                    });
                }
            }
            for y in 0..steps {
                for x in 0..steps {
                    let a = base + y * (steps + 1) + x;
                    let b = a + steps + 1;
                    mesh.add_triangle(a, a + 1, b);
                    mesh.add_triangle(a + 1, b + 1, b);
                }
            }
        }
    }
}

/// Adds `galley`'s glyphs to `mesh`, each vertex placed by `place` from its
/// position in the galley, all in `color`. `atlas` is the font atlas size in
/// texels.
fn projected_mesh(
    mesh: &mut Mesh,
    galley: &Galley,
    atlas: Vec2,
    place: impl Fn(Vec2) -> Pos2,
    color: Color32,
) {
    for row in &galley.rows {
        let glyphs = &row.visuals.mesh;
        let base = mesh.vertices.len() as u32;
        mesh.indices.extend(glyphs.indices.iter().map(|i| base + i));
        mesh.vertices.extend(glyphs.vertices.iter().map(|v| Vertex {
            pos: place(row.pos.to_vec2() + v.pos.to_vec2()),
            uv: Pos2::new(v.uv.x / atlas.x, v.uv.y / atlas.y),
            color,
        }));
    }
}

/// Adds `galley`'s glyphs to `mesh`, centered on `center`, scaled and turned
/// by `angle` radians, all in `color`. `atlas` is the font atlas size in texels.
fn galley_mesh(
    mesh: &mut Mesh,
    galley: &Galley,
    atlas: Vec2,
    center: Pos2,
    scale: f32,
    angle: f32,
    color: Color32,
) {
    let turn = Rot2::from_angle(angle);
    let middle = galley.size() / 2.0;
    for row in &galley.rows {
        let glyphs = &row.visuals.mesh;
        let base = mesh.vertices.len() as u32;
        mesh.indices.extend(glyphs.indices.iter().map(|i| base + i));
        mesh.vertices.extend(glyphs.vertices.iter().map(|v| Vertex {
            pos: center + turn * ((row.pos.to_vec2() + v.pos.to_vec2() - middle) * scale),
            uv: Pos2::new(v.uv.x / atlas.x, v.uv.y / atlas.y),
            color,
        }));
    }
}

/// The value at `x` on a curve through `points` (sorted by x), flat past its ends.
fn curve(points: &[(f32, f32)], x: f32) -> f32 {
    let i = points.partition_point(|&(px, _)| px <= x);
    match (i.checked_sub(1).and_then(|i| points.get(i)), points.get(i)) {
        (Some(&(x0, y0)), Some(&(x1, y1))) => y0 + (y1 - y0) * (x - x0) / (x1 - x0),
        (Some(&(_, y)), None) | (None, Some(&(_, y))) => y,
        (None, None) => 0.0,
    }
}

/// A steady pseudo-random number in -1..1 for `seed`, so each word keeps its tilt.
fn wobble(seed: u64) -> f32 {
    let mut x = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

/// Word colors from the cover's vivid tone: a light one to read on the dark
/// background, and a deeper one behind white text.
fn word_colors(vivid: [f32; 3]) -> (Color32, Color32) {
    let luma = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let lifted = |c: [f32; 3], to: f32| {
        let l = luma(c);
        let t = ((to - l) / (1.0 - l).max(1e-3)).clamp(0.0, 1.0);
        c.map(|v| v + (1.0 - v) * t)
    };
    let l = luma(vivid);
    let deep = if l > 0.36 {
        vivid.map(|v| v * 0.36 / l)
    } else {
        lifted(vivid, 0.3)
    };
    (color(lifted(vivid, 0.62)), color(deep))
}

fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(mix(from.r(), to.r()), mix(from.g(), to.g()), mix(from.b(), to.b()))
}

/// A scrolling column of lines centered on `scroll` (a fractional line index).
/// Lines grow as they reach the center by crossfading two font sizes, which
/// keeps the glyph atlas small compared to animating the size itself.
/// The line at the center lights up word by word as it's sung, unless the
/// timing is only estimated.
fn draw_lyrics(
    painter: &egui::Painter,
    rect: Rect,
    lyrics: &mv_enrich::Lyrics,
    position: Option<Duration>,
    scroll: f32,
    center_y: f32,
    size: f32,
) {
    let max_width = rect.width() * 0.9;
    let spacing = size * 1.3;
    let big = FontId::new(size, FontFamily::Name(BOLD.into()));
    let small = FontId::new(size * 0.6, FontFamily::Name(BOLD.into()));

    let first = (scroll.floor() as i64 - 2).max(0);
    let last = scroll.ceil() as i64 + 2;
    for i in first..=last {
        let Some(line) = lyrics.synced.get(i as usize).filter(|l| !l.text.is_empty()) else {
            continue;
        };
        let offset = i as f32 - scroll;
        let distance = offset.abs();
        if distance > 2.2 {
            continue;
        }
        let emphasis = (1.0 - distance).clamp(0.0, 1.0);
        let mut alpha = if distance <= 1.0 {
            110.0 + 145.0 * emphasis
        } else {
            110.0 * (2.2 - distance) / 1.2
        };
        if offset < 0.0 {
            alpha *= 0.7; // already sung
        }

        let pos = Pos2::new(rect.center().x, center_y + offset * spacing);
        // Words light up only on the line growing into the center.
        let icons = icons::for_line(&line.text);
        let sung = position.filter(|_| !lyrics.estimated && emphasis > 0.0).map(|p| {
            sung_glyphs(lyrics.sung_chars(i as usize, p), &icons)
        });
        for (font, weight, sweeps) in [(&big, emphasis, true), (&small, 1.0 - emphasis, false)] {
            let a = alpha * weight;
            if a < 1.0 {
                continue;
            }
            let galley = lyric_galley(painter, &line.text, &icons, font, max_width);
            match sung.filter(|_| sweeps) {
                Some(sung) => {
                    // Unsung words dim as the line grows, so it lights up from
                    // the size and brightness the line had on its way in.
                    let dim = egui::emath::lerp(1.0..=UNSUNG, emphasis);
                    draw_swept(painter, galley, pos, a, dim, sung);
                }
                _ => {
                    let origin = pos - galley.size() / 2.0;
                    let color = Color32::from_white_alpha(a as u8);
                    painter.galley_with_override_text_color(origin, galley, color);
                }
            }
        }
    }
}

/// A lyric line with a space and an icon after each word in `icons`, shrunk
/// like [`fit_font`] when it is wider than `max_width`.
fn lyric_galley(
    painter: &egui::Painter,
    text: &str,
    icons: &[(usize, char)],
    font: &FontId,
    max_width: f32,
) -> Arc<Galley> {
    let layout = |font: &FontId| {
        let format = |font_id| TextFormat {
            font_id,
            color: Color32::WHITE,
            valign: egui::Align::Center,
            ..Default::default()
        };
        let icon_font = FontId::new(font.size, FontFamily::Name(icons::FONT.into()));
        let byte = |chars: usize| text.char_indices().nth(chars).map_or(text.len(), |(b, _)| b);
        let mut job = LayoutJob::default();
        let mut from = 0;
        for &(end, icon) in icons {
            let to = byte(end);
            job.append(&text[from..to], 0.0, format(font.clone()));
            job.append(" ", 0.0, format(font.clone()));
            job.append(&icon.to_string(), 0.0, format(icon_font.clone()));
            from = to;
        }
        job.append(&text[from..], 0.0, format(font.clone()));
        painter.layout_job(job)
    };
    let galley = layout(font);
    match shrunk(font, galley.size().x, max_width) {
        Some(smaller) => layout(&smaller),
        None => galley,
    }
}

/// `sung` characters of a line as glyphs of its [`lyric_galley`], which has
/// two more (a space and the icon) after each word in `icons`. An icon lights
/// up once its word is sung.
fn sung_glyphs(sung: f32, icons: &[(usize, char)]) -> f32 {
    let done = sung.max(0.0).floor() as usize;
    sung + 2.0 * icons.iter().filter(|&&(end, _)| end <= done).count() as f32
}

/// One line with its first `sung` glyphs at `alpha` and the rest at
/// `alpha * dim`, blending over a soft edge inside the word being sung.
fn draw_swept(
    painter: &egui::Painter,
    galley: Arc<Galley>,
    center: Pos2,
    alpha: f32,
    dim: f32,
    sung: f32,
) {
    /// Slices of the soft edge; each is drawn as its own clipped copy.
    const SLICES: usize = 6;
    let height = galley.rows.first().map_or(galley.size().y, |r| r.size.y);
    let feather = height * 0.3;
    let origin = center - galley.size() / 2.0;
    let white = |a: f32| Color32::from_white_alpha(a.round() as u8);

    let mut x = origin.x + galley.size().x;
    if let Some(row) = galley.rows.first() {
        let k = sung.max(0.0).floor() as usize;
        if let Some(glyph) = row.glyphs.get(k) {
            x = origin.x + row.pos.x + glyph.pos.x + glyph.advance_width * sung.fract();
        }
    }
    let bounds = Rect::from_min_size(origin, galley.size()).expand(feather * 2.0);
    let band = |from: f32, to: f32| {
        Rect::from_x_y_ranges(from..=to, bounds.y_range()).intersect(bounds)
    };
    let draw = |from: f32, to: f32, a: f32| {
        if to > from && a >= 1.0 {
            painter
                .with_clip_rect(band(from, to))
                .galley_with_override_text_color(origin, galley.clone(), white(a));
        }
    };

    let (left, right) = (x - feather, x + feather);
    draw(bounds.left(), left, alpha);
    let step = (right - left) / SLICES as f32;
    for j in 0..SLICES {
        let t = (j as f32 + 0.5) / SLICES as f32;
        let from = left + step * j as f32;
        draw(from, from + step, alpha * egui::emath::lerp(1.0..=dim, t));
    }
    draw(right, bounds.right(), alpha * dim);
}

fn draw_progress(
    painter: &egui::Painter,
    rect: Rect,
    np: &NowPlaying,
    position: Option<std::time::Duration>,
    accent: Color32,
) {
    let (Some(position), Some(duration)) = (position, np.duration) else {
        return;
    };
    let fraction = (position.as_secs_f32() / duration.as_secs_f32().max(1.0)).clamp(0.0, 1.0);
    let bar = Rect::from_min_max(
        Pos2::new(rect.left(), rect.bottom() - 4.0),
        rect.right_bottom(),
    );
    painter.rect_filled(bar, 0.0, Color32::from_white_alpha(25));
    let filled = Rect::from_min_max(
        bar.min,
        Pos2::new(bar.left() + bar.width() * fraction, bar.bottom()),
    );
    painter.rect_filled(filled, 0.0, accent);
}

/// Beat pulse when the tempo is known, otherwise a slow breath while playing.
fn pulse(
    np: Option<&NowPlaying>,
    enrichment: Option<&Enrichment>,
    position: Option<std::time::Duration>,
    time: f32,
) -> f32 {
    if np.is_none_or(|np| np.state != PlaybackState::Playing) {
        return 0.0;
    }
    match (enrichment.and_then(|e| e.bpm), position) {
        (Some(bpm), Some(pos)) => {
            // A soft swell peaking on the beat; a sharp attack flickers too much.
            let beat = pos.as_secs_f32() * bpm / 60.0;
            let swell = 0.5 + 0.5 * (beat.fract() * TAU).cos();
            0.6 * swell * swell
        }
        _ => 0.12 * (0.5 + 0.5 * (time * 1.6).sin()),
    }
}

/// Windows replays a lone AltGr/Alt/Ctrl when the window gains focus, so
/// modifiers alone never close the screensaver.
fn is_modifier(key: egui::Key) -> bool {
    use egui::Key::*;
    matches!(
        key,
        ShiftLeft
            | ShiftRight
            | ControlLeft
            | ControlRight
            | AltLeft
            | AltRight
            | SuperLeft
            | SuperRight
    )
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let systems = [BOLD_FONTS, REGULAR_FONTS];
    for ((name, bundled), candidates) in BUNDLED_FONTS.into_iter().zip(systems) {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bundled)));
        let mut family = vec![name.to_owned()];
        let system = format!("{name}-system");
        if let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts
                .font_data
                .insert(system.clone(), Arc::new(FontData::from_owned(bytes)));
            family.push(system);
        }
        family.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }
    let mut heavy = vec![HEAVY.to_owned()];
    heavy.extend(fonts.families[&FontFamily::Name(BOLD.into())].iter().cloned());
    fonts
        .font_data
        .insert(HEAVY.into(), Arc::new(FontData::from_static(HEAVY_FONT)));
    fonts.families.insert(FontFamily::Name(HEAVY.into()), heavy);
    fonts.font_data.insert(
        icons::FONT.into(),
        Arc::new(FontData::from_static(icons::FONT_BYTES)),
    );
    fonts
        .families
        .insert(FontFamily::Name(icons::FONT.into()), vec![icons::FONT.into()]);
    ctx.set_fonts(fonts);
}

/// Larger text and the brand accent for the settings panel.
fn install_style(ctx: &egui::Context) {
    use egui::TextStyle;
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(26.0, FontFamily::Name(BOLD.into())),
            ),
            (
                TextStyle::Body,
                FontId::new(16.0, FontFamily::Name(REGULAR.into())),
            ),
            (
                TextStyle::Button,
                FontId::new(16.0, FontFamily::Name(REGULAR.into())),
            ),
            (
                TextStyle::Small,
                FontId::new(13.0, FontFamily::Name(REGULAR.into())),
            ),
            (TextStyle::Monospace, FontId::monospace(14.0)),
        ]
        .into();
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = BRAND_BLUE;
        // Glass controls for the settings panel: white at low alpha, so the
        // panel's frosted look carries through instead of grey-blue boxes.
        let glass = Color32::from_white_alpha;
        let widgets = &mut style.visuals.widgets;
        widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, glass(26));
        for (state, fill, stroke) in [
            (&mut widgets.inactive, 26, 34),
            (&mut widgets.hovered, 44, 80),
            (&mut widgets.active, 60, 110),
            (&mut widgets.open, 44, 80),
        ] {
            state.bg_fill = glass(fill);
            state.weak_bg_fill = glass(fill);
            state.bg_stroke = egui::Stroke::new(1.0, glass(stroke));
            state.corner_radius = egui::CornerRadius::same(8);
        }
        style.visuals.extreme_bg_color = glass(16);
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
        // The scroll fade paints the panel's own fill as a box, which on
        // see-through glass shows up as a lighter rectangle with hard edges.
        style.spacing.scroll.fade.strength = 0.0;
    });
}

/// UV rect of the centered square, for round crops of non-square photos.
fn square_crop(size: Vec2) -> Rect {
    let side = size.x.min(size.y);
    let (w, h) = (side / size.x, side / size.y);
    Rect::from_min_size(Pos2::new((1.0 - w) / 2.0, (1.0 - h) / 2.0), Vec2::new(w, h))
}

fn mix_palette(from: [[f32; 3]; 3], to: [[f32; 3]; 3], t: f32) -> [[f32; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|c| from[i][c] + (to[i][c] - from[i][c]) * t))
}

fn rgba(c: [f32; 3]) -> [f32; 4] {
    [c[0], c[1], c[2], 1.0]
}

fn color(c: [f32; 3]) -> Color32 {
    let [r, g, b] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
    Color32::from_rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mv_enrich::{LyricLine, Lyrics};

    /// Draws the word stack a few moments into a line, rasterized on the CPU
    /// into PNGs, to look at without opening a window:
    /// `MV_TEST_WORDS=dir cargo test -p musicvisual draws_words -- --ignored`.
    #[test]
    #[ignore]
    fn draws_words() {
        let dir = std::path::PathBuf::from(std::env::var_os("MV_TEST_WORDS").expect("MV_TEST_WORDS"));
        let line = |at: f64, text: &str| LyricLine {
            at: Duration::from_secs_f64(at),
            text: text.to_owned(),
            words: Vec::new(),
            end: None,
        };
        let lyrics = Lyrics {
            synced: vec![
                line(6.0, "Celou noc jedu městem"),
                line(10.0, "Na rohu stojím a srdce mi hoří"),
                line(14.0, "Peníze v kapse, auto je rychlý"),
                line(17.0, "Celá noc a já pořád jedu dál"),
                line(20.0, "Srdce bije, město nespí"),
            ],
            plain: None,
            estimated: false,
        };
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let size = Vec2::new(960.0, 540.0);
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut atlas = Atlas::default();
        let palettes = [[0.95, 0.45, 0.35], [0.2, 0.45, 0.95]];
        let shots = [10.05, 10.4, 11.2, 12.1, 13.0, 15.0];
        let styles = [LyricsStyle::Words, LyricsStyle::Drum, LyricsStyle::Lens];
        for (n, (style, at)) in styles
            .into_iter()
            .flat_map(|style| shots.map(|at| (style, at)))
            .enumerate()
        {
            let vivid = palettes[n % 2];
            let draw = |ctx: &egui::Context| {
                let painter = ctx.layer_painter(egui::LayerId::background());
                let rect = Rect::from_min_size(Pos2::ZERO, size);
                painter.rect_filled(rect, 0.0, Color32::from_rgb(14, 10, 16));
                let position = Duration::from_secs_f64(at);
                let (center, size) = (rect.center().y, big_lyrics_size(rect));
                let video = if n % shots.len() >= 4 { 1.0 } else { 0.0 };
                if video > 0.0 {
                    // A bright, busy picture in place of a clip.
                    for k in 0..12 {
                        let band = Rect::from_min_size(Pos2::new(k as f32 * 80.0, 0.0), Vec2::new(80.0, rect.height()));
                        let c = if k % 2 == 0 { Color32::from_rgb(235, 225, 200) } else { Color32::from_rgb(90, 160, 220) };
                        painter.rect_filled(band, 0.0, c);
                    }
                    draw_band(&painter, rect, center, size * WORD_SCALE * 1.7, 175.0);
                }
                match style {
                    LyricsStyle::Words => draw_words(&painter, rect, &lyrics, position, center, size, vivid, video, None),
                    LyricsStyle::Drum => draw_drum(&painter, rect, &lyrics, position, center, size, vivid, video),
                    LyricsStyle::Lens => draw_lens(&painter, rect, &lyrics, position, center, size, vivid, video),
                }
            };
            // The first run loads the fonts.
            let warm_up = ctx.run_ui(input(), |ui| draw(ui.ctx()));
            let out = ctx.run_ui(input(), |ui| draw(ui.ctx()));
            for (id, delta) in warm_up.textures_delta.set.iter().chain(&out.textures_delta.set) {
                atlas.apply(*id, delta);
            }
            let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
            let mut image = image::RgbaImage::new(size.x as u32, size.y as u32);
            for p in primitives {
                if let egui::epaint::Primitive::Mesh(mesh) = p.primitive {
                    rasterize(&mut image, &mesh, &atlas);
                }
            }
            let kind = match style {
                LyricsStyle::Words => "words",
                LyricsStyle::Drum => "drum",
                LyricsStyle::Lens => "lens",
            };
            image.save(dir.join(format!("{kind}{}.png", n % shots.len()))).unwrap();
        }
    }

    /// The Ctrl+D handoff, rasterized like [`draws_words`]: the main window
    /// as the lyrics fly off, then the other screen wiping open as they come.
    /// `MV_TEST_WORDS=dir cargo test -p musicvisual draws_handoff -- --ignored`.
    #[test]
    #[ignore]
    fn draws_handoff() {
        let dir = std::path::PathBuf::from(std::env::var_os("MV_TEST_WORDS").expect("MV_TEST_WORDS"));
        let line = |at: f64, text: &str| LyricLine {
            at: Duration::from_secs_f64(at),
            text: text.to_owned(),
            words: Vec::new(),
            end: None,
        };
        let lyrics = Lyrics {
            synced: vec![line(10.0, "Na rohu stojím a srdce mi hoří")],
            plain: None,
            estimated: false,
        };
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let size = Vec2::new(960.0, 540.0);
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut atlas = Atlas::default();
        let dir_x = Vec2::X;
        let position = Duration::from_secs_f64(11.2);
        let frames = [(false, 0.12), (false, 0.25), (false, 0.38), (true, 0.4), (true, 0.5), (true, 0.6), (true, 0.72), (true, 0.85)];
        for (n, (second, t)) in frames.into_iter().enumerate() {
            let draw = |ui: &mut egui::Ui| {
                let rect = Rect::from_min_size(Pos2::ZERO, size);
                ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(30, 20, 40));
                let words = |ui: &mut egui::Ui, y: f32| {
                    let s = if second { big_lyrics_size(rect) } else { main_lyrics_size(rect) };
                    draw_words(ui.painter(), rect, &lyrics, position, y, s, [0.95, 0.45, 0.35], 0.0, None);
                };
                if second {
                    match beam::arriving(rect, dir_x, t) {
                        Some(flight) => beam::with_trail(ui, flight, |ui| words(ui, rect.center().y)),
                        None => words(ui, rect.center().y),
                    }
                    beam::draw_wipe(ui.painter(), rect, dir_x, t, Color32::from_rgb(240, 110, 90));
                } else if let Some(flight) = beam::leaving(rect, dir_x, t) {
                    beam::with_trail(ui, flight, |ui| words(ui, main_lyrics_y(rect)));
                }
            };
            let warm_up = ctx.run_ui(input(), draw);
            let out = ctx.run_ui(input(), draw);
            for (id, delta) in warm_up.textures_delta.set.iter().chain(&out.textures_delta.set) {
                atlas.apply(*id, delta);
            }
            let mut image = image::RgbaImage::new(size.x as u32, size.y as u32);
            for p in ctx.tessellate(out.shapes, out.pixels_per_point) {
                if let egui::epaint::Primitive::Mesh(mesh) = p.primitive {
                    rasterize(&mut image, &mesh, &atlas);
                }
            }
            image.save(dir.join(format!("handoff{n}.png"))).unwrap();
        }
    }

    /// The settings panel (Ctrl+S) over a stand-in for the visuals, rasterized
    /// like [`draws_words`] into `settings.png`.
    #[test]
    #[ignore]
    fn draws_settings() {
        let dir = std::path::PathBuf::from(std::env::var_os("MV_TEST_WORDS").expect("MV_TEST_WORDS"));
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        install_style(&ctx);
        let settings: SharedSettings = Arc::new(std::sync::Mutex::new(crate::settings::Settings {
            youtube: true,
            wled: "192.168.12.127".into(),
            ..Default::default()
        }));
        let playing = NowPlaying {
            track: mv_core::Track {
                title: "El Camino".into(),
                artist: Some("Yzomandias".into()),
                album: None,
                art_url: None,
            },
            state: PlaybackState::Playing,
            duration: None,
            position: None,
            position_at: SystemTime::now(),
            device: None,
            volume: None,
            muted: false,
        };
        let devices = [
            Device {
                id: mv_core::SourceId::new("smtc", "Spotify"),
                name: "Spotify".into(),
                now_playing: Some(playing),
            },
            Device {
                id: mv_core::SourceId::new("bluos", "192.168.12.40"),
                name: "Ondra Pulse".into(),
                now_playing: None,
            },
        ];
        let size = Vec2::new(1280.0, 1000.0);
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut atlas = Atlas::default();
        let mut out = None;
        // Time moves on, so the window's fade-in finishes and its size settles.
        for frame in 0..20 {
            let mut raw = input();
            raw.time = Some(frame as f64 * 0.5);
            let output = ctx.run_ui(raw, |ui| {
                let rect = Rect::from_min_size(Pos2::ZERO, size);
                let painter = ui.painter();
                painter.add(egui::Shape::mesh({
                    let mut mesh = Mesh::default();
                    for (pos, c) in [
                        (rect.left_top(), Color32::from_rgb(60, 30, 90)),
                        (rect.right_top(), Color32::from_rgb(140, 50, 70)),
                        (rect.right_bottom(), Color32::from_rgb(20, 20, 40)),
                        (rect.left_bottom(), Color32::from_rgb(30, 60, 90)),
                    ] {
                        mesh.colored_vertex(pos, c);
                    }
                    mesh.add_triangle(0, 1, 2);
                    mesh.add_triangle(0, 2, 3);
                    mesh
                }));
                painter.rect_filled(rect, 0.0, Color32::from_black_alpha(70));
                setup::show(ui.ctx(), 1.0, true, &settings, false, &devices, None, None);
            });
            for (id, delta) in &output.textures_delta.set {
                atlas.apply(*id, delta);
            }
            out = Some(output);
        }
        let out = out.unwrap();
        let mut image = image::RgbaImage::new(size.x as u32, size.y as u32);
        for p in ctx.tessellate(out.shapes, out.pixels_per_point) {
            if let egui::epaint::Primitive::Mesh(mesh) = p.primitive {
                rasterize(&mut image, &mesh, &atlas);
            }
        }
        image.save(dir.join("settings.png")).unwrap();
    }

    /// The textures egui has sent, by id: the font atlas, images.
    #[derive(Default)]
    struct Atlas(std::collections::HashMap<egui::TextureId, ColorImageBuf>);

    impl Atlas {
        fn apply(&mut self, id: egui::TextureId, delta: &egui::epaint::ImageDelta) {
            self.0.entry(id).or_default().apply(delta);
        }
    }

    #[derive(Default)]
    struct ColorImageBuf {
        size: [usize; 2],
        pixels: Vec<Color32>,
    }

    impl ColorImageBuf {
        fn apply(&mut self, delta: &egui::epaint::ImageDelta) {
            let egui::ImageData::Color(image) = &delta.image;
            let [x0, y0] = delta.pos.unwrap_or([0, 0]);
            if delta.pos.is_none() {
                self.size = image.size;
                self.pixels = vec![Color32::TRANSPARENT; image.size[0] * image.size[1]];
            }
            for y in 0..image.size[1] {
                for x in 0..image.size[0] {
                    self.pixels[(y0 + y) * self.size[0] + x0 + x] = image.pixels[y * image.size[0] + x];
                }
            }
        }

        fn sample(&self, uv: Pos2) -> [f32; 4] {
            let x = ((uv.x * self.size[0] as f32) as usize).min(self.size[0] - 1);
            let y = ((uv.y * self.size[1] as f32) as usize).min(self.size[1] - 1);
            self.pixels[y * self.size[0] + x].to_array().map(|c| c as f32 / 255.0)
        }
    }

    /// Premultiplied triangles over `image`, without antialiasing beyond egui's own feathering.
    fn rasterize(image: &mut image::RgbaImage, mesh: &Mesh, atlas: &Atlas) {
        let Some(atlas) = atlas.0.get(&mesh.texture_id) else {
            return;
        };
        for t in mesh.indices.chunks_exact(3) {
            let v = [0, 1, 2].map(|k| mesh.vertices[t[k] as usize]);
            let (a, b, c) = (v[0].pos, v[1].pos, v[2].pos);
            let area = (b - a).x * (c - a).y - (b - a).y * (c - a).x;
            if area.abs() < 1e-6 {
                continue;
            }
            let min = a.min(b).min(c);
            let max = a.max(b).max(c);
            let x_range = (min.x.floor().max(0.0) as u32)..(max.x.ceil().min(image.width() as f32) as u32);
            for y in (min.y.floor().max(0.0) as u32)..(max.y.ceil().min(image.height() as f32) as u32) {
                for x in x_range.clone() {
                    let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((b - p).x * (c - p).y - (b - p).y * (c - p).x) / area;
                    let w1 = ((c - p).x * (a - p).y - (c - p).y * (a - p).x) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let uv = Pos2::new(
                        w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x,
                        w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y,
                    );
                    let tex = atlas.sample(uv);
                    let colors = v.map(|v| v.color.to_array().map(|c| c as f32 / 255.0));
                    let src: [f32; 4] = std::array::from_fn(|k| {
                        (w0 * colors[0][k] + w1 * colors[1][k] + w2 * colors[2][k]) * tex[k]
                    });
                    let dst = image.get_pixel_mut(x, y);
                    for k in 0..4 {
                        let d = dst.0[k] as f32 / 255.0;
                        dst.0[k] = ((src[k] + d * (1.0 - src[3])) * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
}
