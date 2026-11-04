//! Tab: the lyric styles as a grid of pictures on frosted glass over the
//! window, the ones moving with the song apart from those taking the whole
//! screen. The pictures are stills made beforehand (`draws_previews` in
//! `ui.rs`), so the grid costs nothing to draw; a click or
//! Enter picks one and closes the grid, the arrows move between them, and a
//! click beside them (or Tab, or Esc) closes it as it was. "Další módy" opens
//! the rest of the modes (music videos, lyrics over the desktop).

use eframe::egui::{self, Align2, Color32, CursorIcon, FontFamily, FontId, Key, Pos2, Rect, Stroke, Vec2};

use crate::anim::ease_out_cubic;
use crate::glass;
use crate::icons;
use crate::settings::LyricsStyle;
use crate::ui::BOLD;

/// Each style with its icon (from the bundled Phosphor subset) and name.
/// The ones moving with the song first, then those taking the whole screen.
pub const STYLES: [(LyricsStyle, char, &str); 16] = [
    (LyricsStyle::Words, '\u{e16c}', "Po slovech"),
    (LyricsStyle::Drum, '\u{e9fa}', "Válec"),
    (LyricsStyle::Lens, '\u{e220}', "Čočka"),
    (LyricsStyle::Loupe, '\u{e7ba}', "Lupa"),
    (LyricsStyle::Live, '\u{e242}', "Živě"),
    (LyricsStyle::Typing, '\u{e3b4}', "Psaní"),
    (LyricsStyle::Flip, '\u{e92a}', "Střídání"),
    (LyricsStyle::Block, '\u{e0e6}', "Blok"),
    (LyricsStyle::Neon, '\u{e1b4}', "Neon"),
    (LyricsStyle::Crt, '\u{e754}', "CRT"),
    (LyricsStyle::CrtColor, '\u{e6c8}', "CRT barvy"),
    (LyricsStyle::Dance, '\u{ed98}', "Siluety"),
    (LyricsStyle::Pocket, '\u{e2a6}', "MP3"),
    (LyricsStyle::Giant, '\u{e324}', "Obří"),
    (LyricsStyle::Disc, '\u{ecac}', "CD"),
    (LyricsStyle::Player, '\u{e8c2}', "Přehrávač"),
];
/// How many of [`STYLES`] move with the song; the rest take the whole screen.
const MOVING: usize = 5;
/// Fade speed, in alpha per second.
const FADE_SPEED: f32 = 6.0;
/// The picked style's color: OZBEAT's blue, lightened to read on the glass.
const PICKED: Color32 = Color32::from_rgb(0x7A, 0xB8, 0xFF);

/// The style `step` places on from `style` in [`STYLES`], round and round.
pub fn neighbor(style: LyricsStyle, step: i32) -> LyricsStyle {
    let n = STYLES.len() as i32;
    let at = STYLES.iter().position(|(s, _, _)| *s == style).unwrap_or(0) as i32;
    STYLES[(at + step).rem_euclid(n) as usize].0
}

/// The still picture of `style` on the grid, in `assets/previews`.
pub fn preview_file(style: LyricsStyle) -> String {
    format!("{}.jpg", format!("{style:?}").to_lowercase())
}

/// What was picked on the grid.
pub enum Pick {
    Style(LyricsStyle),
    /// The rest of the modes.
    More,
    /// Closed as it was: a click beside the tiles.
    Close,
}

pub struct ModeBar {
    /// How far it is in (0 -> 1).
    shown: f32,
    /// The tile under the arrows (and the mouse), as an index into [`STYLES`].
    selected: Option<usize>,
    /// Where the pointer was last frame, so the arrows win until it moves.
    last_hover: Option<Pos2>,
    /// The stills, in [`STYLES`] order; loaded the first time it opens.
    previews: Vec<egui::TextureHandle>,
}

impl Default for ModeBar {
    fn default() -> Self {
        Self { shown: 0.0, selected: None, last_hover: None, previews: Vec::new() }
    }
}

/// The still of `style`, as made by `draws_previews`.
fn preview_bytes(style: LyricsStyle) -> &'static [u8] {
    macro_rules! still {
        ($name:literal) => {
            include_bytes!(concat!("../assets/previews/", $name, ".jpg"))
        };
    }
    match style {
        LyricsStyle::Words => still!("words"),
        LyricsStyle::Drum => still!("drum"),
        LyricsStyle::Lens => still!("lens"),
        LyricsStyle::Loupe => still!("loupe"),
        LyricsStyle::Live => still!("live"),
        LyricsStyle::Typing => still!("typing"),
        LyricsStyle::Flip => still!("flip"),
        LyricsStyle::Block => still!("block"),
        LyricsStyle::Neon => still!("neon"),
        LyricsStyle::Crt => still!("crt"),
        LyricsStyle::CrtColor => still!("crtcolor"),
        LyricsStyle::Dance => still!("dance"),
        LyricsStyle::Pocket => still!("pocket"),
        LyricsStyle::Giant => still!("giant"),
        LyricsStyle::Disc => still!("disc"),
        LyricsStyle::Player => still!("player"),
    }
}

fn load_preview(ctx: &egui::Context, style: LyricsStyle) -> egui::TextureHandle {
    let image = image::load_from_memory(preview_bytes(style)).map(|i| i.to_rgba8()).unwrap_or_default();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    ctx.load_texture(format!("ozbeat-style-{}", preview_file(style)), pixels, egui::TextureOptions::LINEAR)
}

/// Where the tiles go in `screen`: the title row's rect, each group's
/// heading and the tiles' rects, in [`STYLES`] order.
struct Layout {
    title: Rect,
    headings: [Pos2; 2],
    tiles: Vec<Rect>,
    columns: usize,
    label: f32,
}

fn layout(screen: Rect, s: f32) -> Layout {
    let margin = Vec2::new(screen.width() * 0.06, screen.height() * 0.06);
    let area = screen.shrink2(margin);
    let title_h = 34.0 * s;
    let heading_h = 26.0 * s;
    let label = 26.0 * s;
    let gap = 14.0 * s;
    let aspect = screen.height() / screen.width();
    let counts = [MOVING, STYLES.len() - MOVING];
    // The most columns' worth of the biggest tiles that still fit the height.
    let (columns, width) = (3..=7)
        .map(|c| {
            let by_width = (area.width() - gap * (c as f32 - 1.0)) / c as f32;
            let rows: usize = counts.iter().map(|n| n.div_ceil(c)).sum();
            let room = area.height() - title_h - heading_h * 2.0 - gap * 2.0;
            let by_height = (room / rows as f32 - label - gap) / aspect;
            (c, by_width.min(by_height))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((6, 100.0));
    let tile = Vec2::new(width, width * aspect);
    let row_h = tile.y + label + gap;
    let total_w = tile.x * columns as f32 + gap * (columns as f32 - 1.0);
    let rows: usize = counts.iter().map(|n| n.div_ceil(columns)).sum();
    let total_h = title_h + heading_h * 2.0 + gap + row_h * rows as f32;
    let left = screen.center().x - total_w / 2.0;
    let mut y = screen.center().y - total_h / 2.0;
    let title = Rect::from_min_size(Pos2::new(left, y), Vec2::new(total_w, title_h));
    y += title_h;
    let mut headings = [Pos2::ZERO; 2];
    let mut tiles = Vec::new();
    for (g, &count) in counts.iter().enumerate() {
        if g > 0 {
            y += gap;
        }
        headings[g] = Pos2::new(left, y + heading_h * 0.5);
        y += heading_h;
        for k in 0..count {
            let (row, col) = (k / columns, k % columns);
            tiles.push(Rect::from_min_size(Pos2::new(left + col as f32 * (tile.x + gap), y + row as f32 * row_h), tile));
        }
        y += row_h * count.div_ceil(columns) as f32;
    }
    Layout { title, headings, tiles, columns, label }
}

impl ModeBar {
    /// Whether the grid is up, taking every click in the window.
    pub fn under_pointer(&self, _ctx: &egui::Context) -> bool {
        self.shown > 0.0
    }

    /// Draws the grid over `screen` while `open`, fading in and out, with
    /// `current` marked. Returns what was picked.
    pub fn show(&mut self, ctx: &egui::Context, screen: Rect, current: LyricsStyle, open: bool) -> Option<Pick> {
        let (hover, click, dt) = ctx.input(|i| {
            let click = i.pointer.primary_clicked().then(|| i.pointer.interact_pos());
            (i.pointer.hover_pos(), click.flatten(), i.stable_dt.min(0.1))
        });
        let step = FADE_SPEED * dt;
        self.shown = if open { (self.shown + step).min(1.0) } else { (self.shown - step).max(0.0) };
        if self.shown == 0.0 {
            self.selected = None;
            return None;
        }
        if self.previews.is_empty() {
            self.previews = STYLES.iter().map(|(style, _, _)| load_preview(ctx, *style)).collect();
        }
        if self.shown < 1.0 {
            ctx.request_repaint();
        }
        let current_at = STYLES.iter().position(|(s, _, _)| *s == current).unwrap_or(0);
        if open && self.selected.is_none() {
            self.selected = Some(current_at);
        }

        let s = (screen.height() / 720.0).clamp(0.8, 1.6);
        let l = layout(screen, s);
        let shown = ease_out_cubic(self.shown);
        let interactive = open && self.shown > 0.5;

        // The arrows move between the tiles, Enter picks; the mouse takes over once it moves.
        let mut picked = None;
        if interactive {
            let (dx, dy, enter) = ctx.input_mut(|i| {
                let mut take = |key| i.consume_key(egui::Modifiers::NONE, key);
                let dx = take(Key::ArrowRight) as i32 - take(Key::ArrowLeft) as i32;
                let dy = take(Key::ArrowDown) as i32 - take(Key::ArrowUp) as i32;
                (dx, dy, take(Key::Enter) || take(Key::Space))
            });
            let mut at = self.selected.unwrap_or(current_at) as i32;
            at = (at + dx).rem_euclid(STYLES.len() as i32);
            if dy != 0 {
                // A row up or down, staying in the same column where there is one.
                let (group_start, group_len) = if at < MOVING as i32 { (0, MOVING as i32) } else { (MOVING as i32, (STYLES.len() - MOVING) as i32) };
                let k = at - group_start;
                let col = k % l.columns as i32;
                let next = k + dy * l.columns as i32;
                at = if (0..group_len).contains(&next) {
                    group_start + next
                } else if dy > 0 && group_start == 0 {
                    MOVING as i32 + col.min((STYLES.len() - MOVING) as i32 - 1)
                } else if dy < 0 && group_start > 0 {
                    let last_row = (MOVING as i32 - 1) / l.columns as i32;
                    (last_row * l.columns as i32 + col).min(MOVING as i32 - 1)
                } else {
                    at
                };
            }
            self.selected = Some(at as usize);
            if enter {
                picked = Some(Pick::Style(STYLES[at as usize].0));
            }
        }
        let moved = hover.is_some() && hover != self.last_hover;
        self.last_hover = hover;

        let mut painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("ozbeat-modebar")));
        painter.multiply_opacity(shown);
        // Frosted glass over the whole window, dark enough for the stills to stand out.
        glass::pane(&painter, screen, 0.0, glass::Look::PANEL.tint(Color32::from_rgba_premultiplied(6, 6, 10, 200)));
        let ink = Color32::from_rgb(246, 243, 238);
        let dim = Color32::from_white_alpha(120);

        painter.text(
            Pos2::new(l.title.left(), l.title.center().y),
            Align2::LEFT_CENTER,
            "Styly textů",
            FontId::new((22.0 * s).round(), FontFamily::Name(BOLD.into())),
            ink,
        );
        // "Další módy" on the right, with the keys beside it.
        let more_font = FontId::new((13.0 * s).round(), FontFamily::Name(BOLD.into()));
        let more_galley = painter.layout_no_wrap("Další módy  ›".into(), more_font, ink);
        let more = Rect::from_min_size(
            Pos2::new(l.title.right() - more_galley.size().x - 24.0 * s, l.title.center().y - 15.0 * s),
            Vec2::new(more_galley.size().x + 24.0 * s, 30.0 * s),
        );
        let over_more = interactive && hover.is_some_and(|p| more.contains(p));
        glass::pane(&painter, more, more.height() / 2.0, glass::Look::PILL.tint(Color32::from_white_alpha(if over_more { 40 } else { 14 })));
        painter.galley(more.center() - more_galley.size() / 2.0, more_galley, ink);
        painter.text(
            Pos2::new(more.left() - 18.0 * s, l.title.center().y),
            Align2::RIGHT_CENTER,
            "šipky a Enter vyberou · Tab zavře · ← → přepíná i bez mřížky",
            FontId::new((11.5 * s).round(), FontFamily::Name(BOLD.into())),
            dim,
        );
        for (heading, text) in l.headings.iter().zip(["S textem v pohybu", "Přes celou obrazovku"]) {
            painter.text(*heading, Align2::LEFT_CENTER, text, FontId::new((12.5 * s).round(), FontFamily::Name(BOLD.into())), dim);
        }

        let mut on_tile = false;
        for (k, (&rect, &(style, icon, name))) in l.tiles.iter().zip(STYLES.iter()).enumerate() {
            // The name under a tile counts as the tile.
            let hit = Rect::from_min_max(rect.min, rect.max + Vec2::new(0.0, l.label));
            let over = interactive && hover.is_some_and(|p| hit.contains(p));
            if over && moved {
                self.selected = Some(k);
            }
            on_tile |= over;
            let selected = self.selected == Some(k);
            let active = style == current;
            // A frame round the tile: bright on the one under the arrows, blue on the one in use.
            // The still, cropped to the tile (it is 16:9, the window may not be).
            let side = rect.size() / Vec2::new(16.0, 9.0);
            let fit = side.x.min(side.y) / side.x.max(side.y);
            let uv = if rect.width() / rect.height() > 16.0 / 9.0 {
                Rect::from_center_size(Pos2::new(0.5, 0.5), Vec2::new(1.0, fit))
            } else {
                Rect::from_center_size(Pos2::new(0.5, 0.5), Vec2::new(fit, 1.0))
            };
            let still = self.previews[k].id();
            painter.add(egui::epaint::RectShape::filled(rect, 6.0 * s, Color32::WHITE).with_texture(still, uv));
            let ring = if selected {
                Some(Stroke::new(3.0 * s, ink))
            } else if active {
                Some(Stroke::new(2.0 * s, PICKED))
            } else {
                None
            };
            if let Some(ring) = ring {
                painter.rect_stroke(rect.expand(4.0 * s), 9.0 * s, ring, egui::StrokeKind::Inside);
            }
            let color = if active { PICKED } else if selected { ink } else { Color32::from_white_alpha(190) };
            let label_y = rect.bottom() + l.label * 0.55;
            let galley = painter.layout_no_wrap(name.into(), FontId::new((12.0 * s).round(), FontFamily::Name(BOLD.into())), color);
            let icon_w = 18.0 * s;
            let left = rect.center().x - (galley.size().x + icon_w) / 2.0;
            painter.text(
                Pos2::new(left + icon_w / 2.0 - 2.0 * s, label_y),
                Align2::CENTER_CENTER,
                icon,
                FontId::new((14.0 * s).round(), FontFamily::Name(icons::FONT.into())),
                color,
            );
            painter.galley(Pos2::new(left + icon_w, label_y - galley.size().y / 2.0), galley, color);
            if over {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            if interactive && click.is_some_and(|p| hit.contains(p)) {
                picked = Some(Pick::Style(style));
            }
        }
        if over_more {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if interactive && picked.is_none() {
            if click.is_some_and(|p| more.contains(p)) {
                picked = Some(Pick::More);
            } else if click.is_some() && !on_tile {
                picked = Some(Pick::Close);
            }
        }
        picked
    }
}
