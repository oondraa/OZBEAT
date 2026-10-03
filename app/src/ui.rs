//! The visualizer window: shader background plus cover, titles, lyrics.

use std::f32::consts::TAU;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect,
    TextureHandle, TextureOptions, Vec2, load::SizedTexture,
};
use eframe::egui_wgpu;
use mv_audio::Capture;
use mv_core::{Device, NowPlaying, PlaybackState};
use mv_enrich::Enrichment;

use crate::Mode;
use crate::art::{Art, DEFAULT_PALETTE, Picture};
use crate::background::{self, Background, Uniforms};
use crate::pipeline::SharedScene;
use crate::settings::SharedSettings;
use crate::setup::{self, BRAND_BLUE};

/// How long the lyrics take to scroll to the next line.
const LYRIC_SCROLL: Duration = Duration::from_millis(450);
/// How long cover, photo, background and colors take to blend into new ones.
const ART_FADE: Duration = Duration::from_millis(700);
const TOAST_VISIBLE: Duration = Duration::from_millis(2400);
const TOAST_FADE: Duration = Duration::from_millis(600);

const BOLD: &str = "bold";
const REGULAR: &str = "regular";
/// Nicer system fonts when present; egui's built-in font is the fallback.
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
    /// Monitor index the visuals are mirrored to (Ctrl+D), if any.
    mirror: Option<usize>,
    toast: Option<(String, Instant)>,
    logo: Option<TextureHandle>,
    icon: Option<Arc<egui::IconData>>,
    started: Instant,
    art: ArtFade,
    lyric_scroll: LyricScroll,
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
        Self {
            scene,
            settings,
            settings_open,
            mirror: None,
            toast: None,
            logo: load_logo(&cc.egui_ctx),
            icon: crate::window_icon().map(Arc::new),
            started: Instant::now(),
            art: ArtFade {
                current: None,
                previous: None,
                changed: Instant::now(),
            },
            lyric_scroll: LyricScroll::new(),
            flow: 0.0,
            last_frame: Instant::now(),
            mode,
            wake_origin: vec![None; screens.len() + 1],
            fullscreen_retry: Instant::now(),
            screens,
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
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

        let frame = self.prepare_frame(&ctx);
        // With a second screen the content is split: cover and titles here,
        // lyrics over there.
        let content = if self.mirror.is_some() || !self.screens.is_empty() {
            Content::CoverOnly
        } else {
            Content::Full
        };
        self.draw_visual(ui, &frame, content);
        if let Some(monitor) = self.mirror {
            self.show_mirror(&ctx, monitor, &frame);
        }
        for slot in 0..self.screens.len() {
            self.show_screen(&ctx, slot, &frame);
        }
        if self.settings_open {
            // Dim the visuals so the panel reads well.
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, Color32::from_black_alpha(110));
            let first_run = !self.settings.lock().unwrap().setup_done;
            let keep_open = setup::show(
                &ctx,
                &self.settings,
                first_run,
                &frame.devices,
                frame.audio.as_ref(),
                self.logo.as_ref(),
            );
            if !keep_open {
                self.close_settings();
            }
        }
        // Opened from the screensaver dialog: done once the settings close.
        if self.mode == Mode::Configure && !self.settings_open {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.draw_toast(&ctx);
        ctx.request_repaint();
    }
}

/// What a window shows on top of the background.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Content {
    /// Cover, titles and lyrics (single screen).
    Full,
    /// Cover and titles, centered (main screen while mirroring).
    CoverOnly,
    /// Big centered lyrics (second screen).
    LyricsOnly,
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
    /// The art fading out, how visible it is before fading, and the fade (0 -> 1).
    previous: Option<(Arc<Art>, f32)>,
    fade: f32,
    /// Stopped or no source: calm idle screen without stale artwork.
    idle: bool,
    time: f32,
    flow: f32,
}

impl VisualApp {
    fn prepare_frame(&mut self, ctx: &egui::Context) -> FrameData {
        // Deliberately no on-screen source or status: the picture is the whole UI.
        let (now_playing, enrichment, art, audio, devices) = {
            let scene = self.scene.lock().unwrap();
            (
                scene.now_playing.clone(),
                scene.enrichment.clone(),
                scene.art.clone(),
                scene.audio.clone(),
                scene.devices.clone(),
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
        let intensity = self.settings.lock().unwrap().intensity;
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

        FrameData {
            idle: now_playing
                .as_ref()
                .is_none_or(|np| np.state == PlaybackState::Stopped),
            palette,
            previous,
            fade,
            pulse: raw_pulse * intensity * 2.0,
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

        match content {
            Content::Full | Content::CoverOnly => {
                // Without lyrics below, the cover block moves to the middle.
                let center = if content == Content::Full { 0.40 } else { 0.5 };
                let layout = Layout::new(rect, center);
                self.draw_cover(ui, &layout, f.pulse);
                self.draw_titles(ui, &layout, np);
                if let (Content::Full, Some(lyrics), Some(scroll)) = (content, synced, scroll) {
                    let size = (rect.height() * 0.045).clamp(20.0, 54.0);
                    let center_y = rect.top() + rect.height() * 0.83;
                    draw_lyrics(&painter, rect, lyrics, scroll, center_y, size);
                }
            }
            Content::LyricsOnly => match (synced, scroll) {
                (Some(lyrics), Some(scroll)) => {
                    let size = (rect.height() * 0.075).clamp(28.0, 96.0);
                    draw_lyrics(&painter, rect, lyrics, scroll, rect.center().y, size);
                }
                // No timed lyrics: show what plays rather than an empty screen.
                _ => draw_caption(&painter, rect, np),
            },
        }
        draw_progress(&painter, rect, np, f.position, color(f.palette[0]));
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let (settings_key, mirror_key, escape) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(egui::Key::S),
                i.modifiers.command && i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if settings_key {
            if self.settings_open {
                self.close_settings();
            } else {
                self.settings_open = true;
            }
        }
        if mirror_key {
            self.toggle_mirror(ctx);
        }
        // Esc closes the panel first; only then does it leave fullscreen.
        if escape && self.settings_open {
            self.close_settings();
            return;
        }
        handle_fullscreen(ctx, !self.settings_open);
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
        if self.mirror.take().is_some() {
            self.show_toast("Druhá obrazovka vypnuta");
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
        self.mirror = Some(next);
        self.show_toast("Vizuál i na druhé obrazovce · Ctrl+D vypne");
    }

    fn show_mirror(&mut self, ctx: &egui::Context, monitor: usize, frame: &FrameData) {
        let mut builder = egui::ViewportBuilder::default()
            .with_title("OZBEAT")
            .with_monitor(monitor);
        if let Some(icon) = &self.icon {
            builder = builder.with_icon(Arc::clone(icon));
        }
        let mut close = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("ozbeat-mirror"),
            builder,
            |ui, _class| {
                self.draw_visual(ui, frame, Content::LyricsOnly);
                close = ui.input(|i| {
                    i.viewport().close_requested()
                        || i.key_pressed(egui::Key::Escape)
                        || (i.modifiers.command && i.key_pressed(egui::Key::D))
                });
            },
        );
        if close {
            self.mirror = None;
            self.show_toast("Druhá obrazovka vypnuta");
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
                    Content::LyricsOnly
                } else {
                    Content::Full
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
    /// `center` is the vertical position of the cover block, as a fraction of the height.
    fn new(rect: Rect, center: f32) -> Self {
        let margin = rect.width() * 0.06;
        let side = (rect.height() * 0.42).min(rect.width() * 0.30);
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
            title_size: (side * 0.15).clamp(26.0, 96.0),
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

    fn draw_titles(&self, ui: &egui::Ui, layout: &Layout, np: &NowPlaying) {
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
        let mut album = np.track.album.clone().unwrap_or_default();
        if np.state == PlaybackState::Paused {
            album = if album.is_empty() {
                "⏸  Pozastaveno".to_owned()
            } else {
                format!("{album}   ·   ⏸  Pozastaveno")
            };
        }
        let album = painter.layout(
            album,
            regular(layout.title_size * 0.3),
            Color32::from_white_alpha(130),
            layout.text_width,
        );

        let gap = layout.title_size * 0.35;
        let artist_row = artist
            .size()
            .y
            .max(if has_photo { photo_size } else { 0.0 });
        let total = title.size().y + gap + artist_row + gap * 0.6 + album.size().y;
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
        painter.galley(Pos2::new(layout.text_left, y), album, Color32::WHITE);
    }
}

/// Shrinks `font` so `text` fits `max_width` on one line. Sizes are snapped
/// to 4 px steps so long lines don't fill the glyph atlas with odd sizes.
fn fit_font(painter: &egui::Painter, text: &str, font: &FontId, max_width: f32) -> FontId {
    let width = painter
        .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
        .size()
        .x;
    if width <= max_width {
        return font.clone();
    }
    let size = ((font.size * max_width / width) / 4.0).floor() * 4.0;
    FontId::new(size.max(12.0), font.family.clone())
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

/// A scrolling column of lines centered on `scroll` (a fractional line index).
/// Lines grow as they reach the center by crossfading two font sizes, which
/// keeps the glyph atlas small compared to animating the size itself.
fn draw_lyrics(
    painter: &egui::Painter,
    rect: Rect,
    lyrics: &mv_enrich::Lyrics,
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
        for (font, weight) in [(&big, emphasis), (&small, 1.0 - emphasis)] {
            let a = (alpha * weight) as u8;
            if a > 0 {
                let font = fit_font(painter, &line.text, font, max_width);
                painter.text(
                    pos,
                    Align2::CENTER_CENTER,
                    &line.text,
                    font,
                    Color32::from_white_alpha(a),
                );
            }
        }
    }
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

/// F11 or double-click toggles fullscreen, Esc leaves it. Double-click is
/// ignored while the settings panel is open (fast checkbox clicks).
fn handle_fullscreen(ctx: &egui::Context, allow_double_click: bool) {
    let (toggle, escape, is_fullscreen) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::F11)
                || (allow_double_click
                    && i.pointer
                        .button_double_clicked(egui::PointerButton::Primary)),
            i.key_pressed(egui::Key::Escape),
            i.viewport().fullscreen.unwrap_or(false),
        )
    });
    if toggle {
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fullscreen));
    } else if escape && is_fullscreen {
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
    }
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    for (name, candidates) in [(BOLD, BOLD_FONTS), (REGULAR, REGULAR_FONTS)] {
        let mut family = Vec::new();
        if let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts
                .font_data
                .insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            family.push(name.to_owned());
        }
        // The default fonts stay as fallback for glyphs the system font lacks.
        family.extend(fallback.iter().cloned());
        fonts.families.insert(FontFamily::Name(name.into()), family);
    }
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
        style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    });
}

/// The OZBEAT wordmark card for the settings panel.
fn load_logo(ctx: &egui::Context) -> Option<TextureHandle> {
    let png = include_bytes!("../../assety/OZBEAT_dark_big.png");
    let mut image = image::load_from_memory(png).ok()?.to_rgba8();
    // The card is light-on-black; key the black out so it sits on any background.
    for pixel in image.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        let key = r.max(g).max(b);
        if key > 0 {
            let lift = |c: u8| (u16::from(c) * 255 / u16::from(key)) as u8;
            pixel.0 = [
                lift(r),
                lift(g),
                lift(b),
                (u16::from(key) * u16::from(a) / 255) as u8,
            ];
        } else {
            pixel.0 = [0, 0, 0, 0];
        }
    }
    let size = [image.width() as usize, image.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    let options = TextureOptions {
        mipmap_mode: Some(egui::TextureFilter::Linear),
        ..TextureOptions::LINEAR
    };
    Some(ctx.load_texture("ozbeat-logo", image, options))
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
