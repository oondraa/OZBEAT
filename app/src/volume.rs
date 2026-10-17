//! A short hint while the speaker's volume changes: a glass pill at the
//! top, clear of the lyrics, with a speaker whose waves fill in as it gets louder and a bar in
//! the cover's color that glides to the new level. Each step sends a soft
//! ring out of the speaker (louder) or into it (quieter).

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2,
};
use mv_core::NowPlaying;

use crate::anim::{ease_in_cubic, ease_out_cubic, glow, remap, spring};

/// How long the hint stays after the last change.
const HOLD: Duration = Duration::from_millis(1400);
const APPEAR: Duration = Duration::from_millis(450);
const FADE: Duration = Duration::from_millis(450);
/// How fast the bar glides to the new level (time constant).
const GLIDE: Duration = Duration::from_millis(90);
const RIPPLE: Duration = Duration::from_millis(700);
const MAX_RIPPLES: usize = 4;

#[derive(Default)]
pub struct VolumeHud {
    /// The player and its volume last seen; a new player is not a change.
    seen: Option<(Option<String>, f32, bool)>,
    /// Since when the hint is up, and when the volume last changed.
    shown_at: Option<Instant>,
    changed: Option<Instant>,
    level: f32,
    target: f32,
    muted: bool,
    /// When each ring started and whether it goes out (louder) or in.
    ripples: Vec<(Instant, bool)>,
    last_frame: Option<Instant>,
}

impl VolumeHud {
    /// Notices volume changes of what plays. Call once per frame.
    pub fn update(&mut self, now_playing: Option<&NowPlaying>) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .replace(now)
            .map_or(0.0, |t| (now - t).as_secs_f32().min(0.1));
        let current = now_playing.and_then(|np| Some((np.device.clone(), np.volume?, np.muted)));
        if let Some((device, volume, muted)) = &current {
            let change = match &self.seen {
                Some((seen_device, seen_volume, seen_muted)) if seen_device == device => {
                    (seen_volume != volume || seen_muted != muted).then_some(volume > seen_volume)
                }
                _ => None,
            };
            if let Some(louder) = change {
                let old = self.seen.as_ref().map_or(*volume, |s| s.1);
                if self.shown_at.is_none() {
                    self.shown_at = Some(now);
                    // Start from the old level, so the bar visibly moves.
                    self.level = old;
                }
                self.changed = Some(now);
                self.target = *volume;
                self.muted = *muted;
                // Muting alone sends no ring.
                if old != *volume {
                    self.ripples.push((now, louder));
                    if self.ripples.len() > MAX_RIPPLES {
                        self.ripples.remove(0);
                    }
                }
            }
        }
        self.seen = current;
        self.level += (self.target - self.level) * (1.0 - (-dt / GLIDE.as_secs_f32()).exp());
        self.ripples.retain(|(at, _)| at.elapsed() < RIPPLE);
        if self.changed.is_some_and(|t| t.elapsed() > HOLD + FADE) {
            self.shown_at = None;
            self.changed = None;
        }
    }

    /// Draws the hint over everything in `screen`, tinted with `accent`.
    pub fn draw(&self, ctx: &egui::Context, screen: Rect, accent: Color32) {
        let (Some(shown_at), Some(changed)) = (self.shown_at, self.changed) else {
            return;
        };
        let appear = (shown_at.elapsed().as_secs_f32() / APPEAR.as_secs_f32()).min(1.0);
        let gone = remap(
            changed.elapsed().as_secs_f32(),
            HOLD.as_secs_f32(),
            (HOLD + FADE).as_secs_f32(),
        );
        let opacity = ease_out_cubic(remap(appear, 0.0, 0.4)) * (1.0 - ease_in_cubic(gone));
        if opacity <= 0.0 {
            return;
        }
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("ozbeat-volume"),
        ));
        let painter = {
            let mut p = painter;
            p.multiply_opacity(opacity);
            p
        };

        // Scales with the window, pops in on a spring and rises away as it fades.
        let s = (screen.height() / 720.0).clamp(0.8, 1.8);
        let grow = egui::emath::lerp(0.85..=1.0, spring(appear));
        let size = Vec2::new(320.0, 60.0) * s * grow;
        let center = Pos2::new(
            screen.center().x,
            screen.top() + 76.0 * s - 12.0 * s * ease_in_cubic(gone),
        );
        let pill = Rect::from_center_size(center, size);
        let radius = size.y / 2.0;

        // Glass: a dark see-through pane and a thin rim.
        painter.rect_filled(pill, radius, Color32::from_black_alpha(150));
        painter.rect_filled(pill, radius, Color32::from_white_alpha(14));
        painter.rect_stroke(
            pill,
            radius,
            Stroke::new(1.0, Color32::from_white_alpha(46)),
            egui::StrokeKind::Inside,
        );

        let icon = Pos2::new(pill.left() + radius * 1.05, pill.center().y);
        self.draw_ripples(&painter, icon, s, accent);
        self.draw_speaker(&painter, icon, s);

        // The level bar between the speaker and the number.
        let track = Rect::from_min_max(
            Pos2::new(pill.left() + radius * 2.0, pill.center().y - 3.0 * s),
            Pos2::new(pill.right() - radius * 1.9, pill.center().y + 3.0 * s),
        );
        painter.rect_filled(track, 3.0 * s, Color32::from_white_alpha(38));
        let level = if self.muted {
            0.0
        } else {
            self.level.clamp(0.0, 1.0)
        };
        let fill_color = if self.muted {
            Color32::from_white_alpha(90)
        } else {
            brighten(accent)
        };
        let end = track.left() + track.width() * level;
        if level > 0.0 {
            let filled = Rect::from_min_max(
                track.min,
                Pos2::new(end.max(track.left() + 6.0 * s), track.bottom()),
            );
            painter.rect_filled(filled, 3.0 * s, fill_color);
        }
        // A soft light at the end of the bar, brightest right after a step.
        let flash = 1.0 - remap(changed.elapsed().as_secs_f32(), 0.0, 0.6);
        if !self.muted {
            let tip = Pos2::new(end, track.center().y);
            for (r, a) in [(16.0, 0.10), (10.0, 0.18), (6.0, 0.35)] {
                painter.circle_filled(tip, r * s, glow(fill_color, a * (0.4 + 0.6 * flash)));
            }
            painter.circle_filled(tip, 5.0 * s, Color32::WHITE);
        }

        let text = if self.muted {
            "ztlumeno".to_owned()
        } else {
            format!("{}", (self.target * 100.0).round() as i32)
        };
        painter.text(
            Pos2::new(pill.right() - radius * 0.95, pill.center().y),
            Align2::CENTER_CENTER,
            text,
            FontId::new(
                if self.muted { 13.0 } else { 20.0 } * s,
                FontFamily::Name(crate::ui::REGULAR.into()),
            ),
            Color32::from_white_alpha(235),
        );
    }

    /// A speaker cone with up to three waves, lit by how loud it is.
    fn draw_speaker(&self, painter: &Painter, at: Pos2, s: f32) {
        let white = Color32::from_white_alpha(235);
        let body = Rect::from_center_size(at - Vec2::new(9.0 * s, 0.0), Vec2::new(6.0, 10.0) * s);
        painter.rect_filled(body, 1.5 * s, white);
        painter.add(Shape::convex_polygon(
            vec![
                Pos2::new(body.right() - 0.5, body.top()),
                Pos2::new(at.x + 2.0 * s, at.y - 10.0 * s),
                Pos2::new(at.x + 2.0 * s, at.y + 10.0 * s),
                Pos2::new(body.right() - 0.5, body.bottom()),
            ],
            white,
            Stroke::NONE,
        ));
        if self.muted {
            let c = at + Vec2::new(12.0 * s, 0.0);
            let d = 4.5 * s;
            let stroke = Stroke::new(2.0 * s, white);
            painter.line_segment([c - Vec2::splat(d), c + Vec2::splat(d)], stroke);
            painter.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
            return;
        }
        for (i, radius) in [6.0, 11.0, 16.0].into_iter().enumerate() {
            // Each wave lights up over its third of the range.
            let lit = remap(self.level, i as f32 / 3.0, (i as f32 + 0.6) / 3.0);
            let alpha = 0.22 + 0.78 * lit;
            arc(
                painter,
                at + Vec2::new(2.0 * s, 0.0),
                radius * s,
                0.75,
                Stroke::new(2.0 * s, Color32::from_white_alpha((235.0 * alpha) as u8)),
            );
        }
    }

    /// Rings travelling out of the speaker (louder) or into it (quieter).
    fn draw_ripples(&self, painter: &Painter, at: Pos2, s: f32, accent: Color32) {
        for (started, louder) in &self.ripples {
            let t = started.elapsed().as_secs_f32() / RIPPLE.as_secs_f32();
            let travel = ease_out_cubic(t);
            let (from, to) = (8.0 * s, 25.0 * s);
            let radius = if *louder {
                egui::emath::lerp(from..=to, travel)
            } else {
                egui::emath::lerp(to..=from, travel)
            };
            let fade = (1.0 - t).max(0.0);
            painter.circle_stroke(
                at,
                radius,
                Stroke::new(1.5 * s, glow(brighten(accent), 0.5 * fade)),
            );
        }
    }
}

/// An arc of `radius` around `center`, opening to the right, `spread` radians to each side.
fn arc(painter: &Painter, center: Pos2, radius: f32, spread: f32, stroke: Stroke) {
    const STEPS: usize = 16;
    let points = (0..=STEPS)
        .map(|i| {
            let a = -spread + 2.0 * spread * i as f32 / STEPS as f32;
            center + radius * Vec2::new(a.cos(), a.sin())
        })
        .collect();
    painter.add(Shape::line(points, stroke));
}

/// The cover's accent, lifted so it reads on the dark glass.
fn brighten(c: Color32) -> Color32 {
    let max = c.r().max(c.g()).max(c.b()).max(1) as f32;
    let k = (230.0 / max).max(1.0);
    let lift = |v: u8| (v as f32 * k).min(255.0) as u8;
    Color32::from_rgb(lift(c.r()), lift(c.g()), lift(c.b()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mv_core::{PlaybackState, Track};
    use std::time::SystemTime;

    fn playing(device: &str, volume: f32) -> NowPlaying {
        NowPlaying {
            track: Track::default(),
            state: PlaybackState::Playing,
            duration: None,
            position: None,
            position_at: SystemTime::now(),
            device: Some(device.into()),
            volume: Some(volume),
            muted: false,
        }
    }

    #[test]
    fn shows_only_on_a_change() {
        let mut hud = VolumeHud::default();
        hud.update(Some(&playing("Ondra Pulse", 0.13)));
        assert!(hud.shown_at.is_none(), "first sight is not a change");
        hud.update(Some(&playing("Kiki Pulse", 0.40)));
        assert!(hud.shown_at.is_none(), "another player is not a change");
        hud.update(Some(&playing("Kiki Pulse", 0.42)));
        assert!(hud.shown_at.is_some());
        assert_eq!(hud.target, 0.42);
        assert!(hud.ripples.last().is_some_and(|(_, louder)| *louder));
    }

    #[test]
    fn quieter_rings_go_inward() {
        let mut hud = VolumeHud::default();
        hud.update(Some(&playing("Ondra Pulse", 0.30)));
        hud.update(Some(&playing("Ondra Pulse", 0.28)));
        assert!(hud.ripples.last().is_some_and(|(_, louder)| !louder));
    }
}
