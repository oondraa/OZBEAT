//! Ctrl+A: the lyric styles on a floating glass bar at the bottom, the way a
//! phone's tab bar does it: an icon and a name for each, the one picked on a
//! soft pill. A click switches at once, so the picture changes behind the
//! bar; "Víc" opens the rest of the modes (music videos, lyrics over the
//! desktop).

use eframe::egui::{self, Align2, Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, Stroke, Vec2};

use crate::anim::ease_out_cubic;
use crate::icons;
use crate::settings::LyricsStyle;
use crate::ui::BOLD;

/// Each style with its icon (from the bundled Phosphor subset) and name.
/// The ones moving with the song first, then those taking the whole screen.
const STYLES: [(LyricsStyle, char, &str); 13] = [
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
    (LyricsStyle::Giant, '\u{e324}', "Obří"),
    (LyricsStyle::Disc, '\u{ecac}', "CD"),
    (LyricsStyle::Player, '\u{e8c2}', "Přehrávač"),
];
/// Where the bar parts its groups: after the moving styles, and before "Víc".
const MOVING: usize = 5;
/// Fade speed, in alpha per second.
const FADE_SPEED: f32 = 6.0;
/// The picked style's color: OZBEAT's blue, lightened to read on the glass.
const PICKED: Color32 = Color32::from_rgb(0x7A, 0xB8, 0xFF);

/// What was clicked on the bar.
pub enum Pick {
    Style(LyricsStyle),
    /// The rest of the modes.
    More,
}

pub struct ModeBar {
    /// How far it is in (0 -> 1).
    shown: f32,
    /// Where it was last drawn, for [`Self::under_pointer`].
    rect: Rect,
}

impl Default for ModeBar {
    fn default() -> Self {
        Self { shown: 0.0, rect: Rect::NOTHING }
    }
}

impl ModeBar {
    /// Whether the pointer is on the bar, so a click there doesn't also count
    /// for what lies under it.
    pub fn under_pointer(&self, ctx: &egui::Context) -> bool {
        self.shown > 0.0
            && ctx
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| self.rect.expand(6.0).contains(p))
    }

    /// Draws the bar over `screen` while `open`, rising in and sinking out,
    /// with `current` picked. Returns what was clicked.
    pub fn show(&mut self, ctx: &egui::Context, screen: Rect, current: LyricsStyle, open: bool) -> Option<Pick> {
        let (hover, click, dt) = ctx.input(|i| {
            let click = i.pointer.primary_clicked().then(|| i.pointer.interact_pos());
            (i.pointer.hover_pos(), click.flatten(), i.stable_dt.min(0.1))
        });
        let step = FADE_SPEED * dt;
        self.shown = if open { (self.shown + step).min(1.0) } else { (self.shown - step).max(0.0) };
        if self.shown == 0.0 {
            self.rect = Rect::NOTHING;
            return None;
        }
        if self.shown < 1.0 {
            ctx.request_repaint();
        }

        let s = (screen.height() / 720.0).clamp(0.8, 1.5);
        let pad = 4.0 * s;
        let gap = 9.0 * s;
        let slots = STYLES.len() + 1;
        let room = screen.width() - 40.0 * s - pad * 2.0 - gap * 2.0;
        let item = (70.0 * s).min(room / slots as f32);
        let size = Vec2::new(pad * 2.0 + gap * 2.0 + item * slots as f32, 62.0 * s);
        let shown = ease_out_cubic(self.shown);
        let bottom = screen.bottom() - 20.0 * s + 24.0 * s * (1.0 - shown);
        self.rect = Rect::from_center_size(Pos2::new(screen.center().x, bottom - size.y / 2.0), size);
        let radius = size.y / 2.0;

        let mut painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("ozbeat-modebar")));
        painter.multiply_opacity(shown);
        let shadow = egui::Shadow {
            offset: [0, (8.0 * s) as i8],
            blur: (48.0 * s) as u8,
            spread: 0,
            color: Color32::from_black_alpha(120),
        };
        painter.add(shadow.as_shape(self.rect, radius));
        painter.rect_filled(self.rect, radius, Color32::from_rgba_unmultiplied(18, 18, 22, 228));
        painter.rect_stroke(self.rect, radius, Stroke::new(1.0, Color32::from_white_alpha(30)), egui::StrokeKind::Inside);

        let interactive = open && self.shown > 0.5;
        let mut picked = None;
        let mut x = self.rect.left() + pad;
        let ink = Color32::from_rgb(246, 243, 238);
        let entries = STYLES.iter().map(|&(style, icon, name)| (Some(style), Some(icon), name)).chain([(None, None, "Víc")]);
        for (k, (style, icon, name)) in entries.enumerate() {
            if k == MOVING || k == STYLES.len() {
                let line = x + gap / 2.0;
                let reach = size.y * 0.24;
                painter.line_segment(
                    [Pos2::new(line, self.rect.center().y - reach), Pos2::new(line, self.rect.center().y + reach)],
                    Stroke::new(1.0, Color32::from_white_alpha(34)),
                );
                x += gap;
            }
            let button = Rect::from_min_size(Pos2::new(x, self.rect.top() + pad), Vec2::new(item, size.y - pad * 2.0));
            x += item;
            let over = interactive && hover.is_some_and(|p| button.contains(p));
            let active = style == Some(current);
            let fill = if active { 22 } else if over { 12 } else { 0 };
            if fill > 0 {
                painter.rect_filled(button, button.height() / 2.0, Color32::from_white_alpha(fill));
            }
            let color = if active { PICKED } else { ink };
            let icon_at = button.center() - Vec2::new(0.0, 7.0 * s);
            match icon {
                Some(icon) => {
                    painter.text(icon_at, Align2::CENTER_CENTER, icon, FontId::new(22.0 * s, FontFamily::Name(icons::FONT.into())), color);
                }
                None => {
                    for dx in [-1.0, 0.0, 1.0] {
                        painter.circle_filled(icon_at + Vec2::new(dx * 6.0 * s, 0.0), 2.2 * s, color);
                    }
                }
            }
            painter.text(
                button.center() + Vec2::new(0.0, 14.0 * s),
                Align2::CENTER_CENTER,
                name,
                FontId::new((10.5 * s).round(), FontFamily::Name(BOLD.into())),
                color,
            );
            if over {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            if interactive && click.is_some_and(|p| button.contains(p)) {
                picked = Some(match style {
                    Some(style) => Pick::Style(style),
                    None => Pick::More,
                });
            }
        }
        picked
    }
}
