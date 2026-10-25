//! Buttons for what plays: back, quieter, pause / resume, louder and next, on a glass
//! pill in the bottom right corner, clear of the lyrics in the middle. Like
//! the title bar they come up when the mouse moves and go once it rests, so
//! the picture keeps the whole window the rest of the time.

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, CursorIcon, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::anim::{ease_out_cubic, remap, spring};
use crate::volume::arc;

/// How long the buttons stay after the mouse stops moving.
const LINGER: Duration = Duration::from_millis(2200);
/// Fade speed, in alpha per second.
const FADE_SPEED: f32 = 5.0;
/// How long a pressed button takes to spring back.
const PRESS: Duration = Duration::from_millis(350);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Press {
    Previous,
    Quieter,
    PlayPause,
    Louder,
    Next,
}

const BUTTONS: [Press; 5] = [Press::Previous, Press::Quieter, Press::PlayPause, Press::Louder, Press::Next];

pub struct Transport {
    last_activity: Instant,
    alpha: f32,
    /// Where the pill was last drawn, for [`Self::under_pointer`].
    pill: Rect,
    pressed: Option<(Press, Instant)>,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            last_activity: Instant::now(),
            alpha: 0.0,
            pill: Rect::NOTHING,
            pressed: None,
        }
    }
}

impl Transport {
    /// Whether the pointer is on the buttons, so a click there doesn't also
    /// count for what lies under them (big lyrics, fullscreen).
    pub fn under_pointer(&self, ctx: &egui::Context) -> bool {
        self.alpha > 0.0
            && ctx
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| self.pill.expand(6.0).contains(p))
    }

    /// Draws the buttons over `screen` while the mouse moves, if `allowed`,
    /// with the cover's `accent`. Returns the button clicked.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        screen: Rect,
        accent: Color32,
        paused: bool,
        allowed: bool,
    ) -> Option<Press> {
        let (hover, moved, click, dt) = ctx.input(|i| {
            (
                i.pointer.hover_pos(),
                i.pointer.delta() != Vec2::ZERO || i.pointer.any_pressed(),
                i.pointer
                    .primary_clicked()
                    .then(|| i.pointer.interact_pos())
                    .flatten(),
                i.stable_dt.min(0.1),
            )
        });
        if moved {
            self.last_activity = Instant::now();
        }
        let over = hover.is_some_and(|p| self.pill.contains(p));
        let wanted = allowed
            && hover.is_some()
            && (over || self.last_activity.elapsed() < LINGER);
        let step = FADE_SPEED * dt;
        self.alpha = if wanted {
            (self.alpha + step).min(1.0)
        } else {
            (self.alpha - step).max(0.0)
        };
        if self.alpha == 0.0 {
            self.pill = Rect::NOTHING;
            return None;
        }

        let s = (screen.height() / 720.0).clamp(0.8, 1.6);
        let button = 46.0 * s;
        let pad = 6.0 * s;
        let size = Vec2::new(button * BUTTONS.len() as f32 + pad * 2.0, button + pad * 2.0);
        // Rises a little into place as it fades in.
        let shown = ease_out_cubic(self.alpha);
        let corner = screen.right_bottom() - Vec2::new(28.0, 30.0 - 10.0 * (1.0 - shown)) * s;
        self.pill = Rect::from_min_max(corner - size, corner);
        let radius = size.y / 2.0;

        let mut painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("ozbeat-transport"),
        ));
        painter.multiply_opacity(shown);
        painter.rect_filled(self.pill, radius, Color32::from_black_alpha(150));
        painter.rect_filled(self.pill, radius, Color32::from_white_alpha(14));
        painter.rect_stroke(
            self.pill,
            radius,
            Stroke::new(1.0, Color32::from_white_alpha(46)),
            egui::StrokeKind::Inside,
        );

        let mut clicked = None;
        for (k, press) in BUTTONS.into_iter().enumerate() {
            let rect = Rect::from_min_size(
                self.pill.min + Vec2::new(pad + button * k as f32, pad),
                Vec2::splat(button),
            );
            let hovered = hover.is_some_and(|p| rect.contains(p));
            if hovered {
                ctx.set_cursor_icon(CursorIcon::PointingHand);
            }
            // The middle one, in the cover's color, is the main action.
            let fill = if press == Press::PlayPause {
                accent.gamma_multiply(if hovered { 0.95 } else { 0.75 })
            } else {
                Color32::from_white_alpha(if hovered { 30 } else { 0 })
            };
            // A press dips the button and lets it spring back.
            let dip = self
                .pressed
                .filter(|(p, at)| *p == press && at.elapsed() < PRESS)
                .map_or(1.0, |(_, at)| {
                    let t = remap(at.elapsed().as_secs_f32(), 0.0, PRESS.as_secs_f32());
                    0.86 + 0.14 * spring(t)
                });
            let center = rect.center();
            painter.circle_filled(center, button * 0.44 * dip, fill);
            let icon = s * dip;
            match press {
                Press::Previous => skip(&painter, center, icon, -1.0),
                Press::Next => skip(&painter, center, icon, 1.0),
                Press::Quieter => speaker(&painter, center, icon, 1),
                Press::Louder => speaker(&painter, center, icon, 3),
                Press::PlayPause if paused => play(&painter, center, icon),
                Press::PlayPause => pause(&painter, center, icon),
            }
            if self.alpha > 0.5 && click.is_some_and(|p| rect.contains(p)) {
                self.pressed = Some((press, Instant::now()));
                clicked = Some(press);
            }
        }
        clicked
    }
}

/// A speaker with `waves` waves (1: quieter, 3: louder), scaled by `s`.
fn speaker(painter: &Painter, at: Pos2, s: f32, waves: usize) {
    let white = Color32::from_white_alpha(235);
    let at = at - Vec2::new(4.0 * s, 0.0);
    let body = Rect::from_center_size(at - Vec2::new(5.0 * s, 0.0), Vec2::new(5.0, 8.0) * s);
    painter.rect_filled(body, 1.2 * s, white);
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(body.right() - 0.5, body.top()),
            Pos2::new(at.x + 2.0 * s, at.y - 8.0 * s),
            Pos2::new(at.x + 2.0 * s, at.y + 8.0 * s),
            Pos2::new(body.right() - 0.5, body.bottom()),
        ],
        white,
        Stroke::NONE,
    ));
    for radius in [5.0, 9.0, 13.0].into_iter().take(waves) {
        arc(painter, at + Vec2::new(2.0 * s, 0.0), radius * s, 0.75, Stroke::new(1.8 * s, white));
    }
}

/// A triangle against a bar, pointing `dir` (1: next, -1: back).
fn skip(painter: &Painter, at: Pos2, s: f32, dir: f32) {
    let x = |v: f32| at.x + dir * v * s;
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(x(-6.0), at.y - 7.0 * s),
            Pos2::new(x(5.0), at.y),
            Pos2::new(x(-6.0), at.y + 7.0 * s),
        ],
        Color32::WHITE,
        Stroke::NONE,
    ));
    let bar = Rect::from_center_size(Pos2::new(x(6.5), at.y), Vec2::new(3.0, 14.0) * s);
    painter.rect_filled(bar, 1.5 * s, Color32::WHITE);
}

fn play(painter: &Painter, at: Pos2, s: f32) {
    // Nudged right, so it looks centered by its weight.
    let at = at + Vec2::new(1.5 * s, 0.0);
    painter.add(Shape::convex_polygon(
        vec![
            at + Vec2::new(-6.0, -8.0) * s,
            at + Vec2::new(8.0, 0.0) * s,
            at + Vec2::new(-6.0, 8.0) * s,
        ],
        Color32::WHITE,
        Stroke::NONE,
    ));
}

fn pause(painter: &Painter, at: Pos2, s: f32) {
    for x in [-4.0, 4.0] {
        let bar = Rect::from_center_size(at + Vec2::new(x * s, 0.0), Vec2::new(4.0, 16.0) * s);
        painter.rect_filled(bar, 2.0 * s, Color32::WHITE);
    }
}
