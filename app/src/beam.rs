//! Ctrl+D transition: the lyrics pour letter by letter into a spark that
//! gathers on the current line, the spark shoots off towards the other
//! monitor (taking the last letters along), crosses that screen to its middle,
//! and the screen switches on like an old CRT (a glowing line that opens into
//! the picture). Turning the mirror off plays the same timeline backwards,
//! so the spark brings the lyrics back.

use eframe::egui::{Color32, Painter, Pos2, Rect, Stroke, Vec2};

use crate::anim::{ease_in_cubic, ease_in_out_cubic, ease_out_cubic, glow, remap};

/// Parts of the timeline (0 -> 1).
const GATHER: (f32, f32) = (0.0, 0.38);
const CHARGE: (f32, f32) = (0.06, 0.28);
const OUT: (f32, f32) = (0.26, 0.5);
const IN: (f32, f32) = (0.46, 0.7);
const ON: (f32, f32) = (0.66, 1.0);
/// How much of its path the spark's tail covers.
const TRAIL: f32 = 0.22;
/// How far past the window edge the spark flies before it is gone.
const OVERSHOOT: f32 = 80.0;

/// How far the main window has handed the lyrics over (0 -> 1).
pub fn split(t: f32) -> f32 {
    ease_in_out_cubic(remap(t, 0.1, 0.6))
}

/// Main window: the spark charging at `from` (the current lyric line), then
/// flying out through the edge facing `dir`.
pub fn draw_outgoing(painter: &Painter, rect: Rect, from: Pos2, dir: Vec2, t: f32, color: Color32) {
    let s = remap(t, OUT.0, OUT.1);
    if s <= 0.0 {
        let charge = ease_out_cubic(remap(t, CHARGE.0, CHARGE.1));
        if charge > 0.0 {
            let hot = color.lerp_to_gamma(Color32::WHITE, 0.6);
            let flicker = 0.85 + 0.15 * (t * 180.0).sin();
            for (radius, amount) in [(46.0, 0.10), (24.0, 0.22), (12.0, 0.5), (5.0, 1.0)] {
                painter.circle_filled(from, radius * charge, glow(hot, amount * flicker));
            }
            painter.circle_filled(from, 2.5 * charge, Color32::WHITE);
        }
        return;
    }
    if s >= 1.0 {
        return;
    }
    let to = exit_point(rect, from, dir) + dir * OVERSHOOT;
    draw_spark(painter, |x| from.lerp(to, ease_in_cubic(x)), s, color);
}

/// The lyrics being pulled into the spark on the main window.
#[derive(Clone, Copy)]
pub struct Pull {
    /// How far the pouring has got (0 -> 1).
    amount: f32,
    /// Where the spark is now: the letters fly there.
    head: Pos2,
    /// Where the spark gathers; letters close to it go first.
    origin: Pos2,
    reach: f32,
    color: Color32,
}

/// A letter on its way into the spark.
pub struct Grain {
    pub pos: Pos2,
    pub opacity: f32,
    pub scale: f32,
    pub color: Color32,
}

/// `None` until the lyrics start to move; once they are all in the spark it
/// stays `done` (the lyrics stay hidden) for as long as the mirror is on.
pub fn pull(rect: Rect, from: Pos2, dir: Vec2, t: f32, color: Color32) -> Option<Pull> {
    let amount = remap(t, GATHER.0, GATHER.1);
    if amount <= 0.0 {
        return None;
    }
    let s = remap(t, OUT.0, OUT.1);
    let to = exit_point(rect, from, dir) + dir * OVERSHOOT;
    Some(Pull {
        amount,
        head: from.lerp(to, ease_in_cubic(s)),
        origin: from,
        reach: rect.width() * 0.5,
        color: color.lerp_to_gamma(Color32::WHITE, 0.6),
    })
}

impl Pull {
    /// Every letter is in the spark.
    pub fn done(&self) -> bool {
        self.amount >= 1.0
    }

    /// Where the letter at `pos` is now; `None` once the spark has it.
    /// `seed` keeps each letter's own delay and swirl from frame to frame.
    pub fn grain(&self, pos: Pos2, seed: u32) -> Option<Grain> {
        let near = ((pos - self.origin).length() / self.reach).min(1.0);
        let delay = 0.45 * near + 0.12 * hash(seed);
        let g = remap(self.amount, delay, delay + 0.4);
        if g >= 1.0 {
            return None;
        }
        let toward = self.head - pos;
        let swirl = toward.normalized().rot90() * (hash(seed ^ 0x5bd1) - 0.5) * 90.0;
        Some(Grain {
            pos: pos + toward * (g * g) + swirl * (std::f32::consts::PI * g).sin(),
            opacity: 1.0 - g * g * g,
            scale: 1.0 - 0.7 * g,
            color: Color32::WHITE.lerp_to_gamma(self.color, ease_out_cubic(g)),
        })
    }
}

/// A stable pseudo-random number in 0..1.
fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x & 0xFFFF) as f32 / 65535.0
}

/// Second screen: black until the spark arrives, then the switch-on.
pub fn draw_incoming(painter: &Painter, rect: Rect, dir: Vec2, t: f32, color: Color32) {
    draw_power(painter, rect, remap(t, ON.0, ON.1), color);
    let s = remap(t, IN.0, IN.1);
    if s > 0.0 && s < 1.0 {
        let center = rect.center();
        let from = exit_point(rect, center, -dir) - dir * OVERSHOOT;
        draw_spark(painter, |x| from.lerp(center, ease_out_cubic(x)), s, color);
    }
}

/// A glowing head with a fading tail and a few embers, at `s` along `path`.
fn draw_spark(painter: &Painter, path: impl Fn(f32) -> Pos2, s: f32, color: Color32) {
    let hot = color.lerp_to_gamma(Color32::WHITE, 0.6);
    const SEGMENTS: usize = 24;
    for i in 0..SEGMENTS {
        let k0 = i as f32 / SEGMENTS as f32;
        let k1 = (i + 1) as f32 / SEGMENTS as f32;
        let (a, b) = (s - TRAIL * (1.0 - k0), s - TRAIL * (1.0 - k1));
        if b <= 0.0 {
            continue;
        }
        let stroke = Stroke::new(1.0 + 7.0 * k1, glow(color, 0.9 * k1 * k1));
        painter.line_segment([path(a.max(0.0)), path(b)], stroke);
    }

    let head = path(s);
    let heading = (head - path((s - 0.01).max(0.0))).normalized();
    let across = heading.rot90();
    for j in 0..6 {
        let k = (j as f32 + 0.5) / 6.0;
        let at = s - TRAIL * 0.8 * k;
        if at <= 0.0 {
            continue;
        }
        let wobble = ((j as f32 * 12.99 + s * 40.0).sin()) * 12.0 * k;
        painter.circle_filled(path(at) + across * wobble, 1.6, glow(hot, 0.9 * (1.0 - k)));
    }

    for (radius, amount) in [(40.0, 0.10), (22.0, 0.22), (11.0, 0.5), (5.0, 1.0)] {
        painter.circle_filled(head, radius, glow(hot, amount));
    }
    painter.circle_filled(head, 2.5, Color32::WHITE);
}

/// The CRT switch-on at `on` (0 -> 1): a line grows across the middle, then
/// opens up into the picture, with a flash and a ring where the spark hit.
fn draw_power(painter: &Painter, rect: Rect, on: f32, color: Color32) {
    if on >= 1.0 {
        return;
    }
    let black = Color32::BLACK;
    if on <= 0.0 {
        painter.rect_filled(rect, 0.0, black);
        return;
    }
    let hot = color.lerp_to_gamma(Color32::WHITE, 0.7);
    let wide = ease_out_cubic(remap(on, 0.0, 0.35));
    let tall = ease_in_out_cubic(remap(on, 0.3, 1.0));
    let open = Rect::from_center_size(
        rect.center(),
        Vec2::new(rect.width() * wide, (rect.height() * tall).max(3.0)),
    );

    // Black everywhere outside the opening.
    for cover in [
        Rect::from_min_max(rect.min, Pos2::new(rect.right(), open.top())),
        Rect::from_min_max(Pos2::new(rect.left(), open.bottom()), rect.max),
        Rect::from_min_max(
            Pos2::new(rect.left(), open.top()),
            Pos2::new(open.left(), open.bottom()),
        ),
        Rect::from_min_max(
            Pos2::new(open.right(), open.top()),
            Pos2::new(rect.right(), open.bottom()),
        ),
    ] {
        if cover.is_positive() {
            painter.rect_filled(cover, 0.0, black);
        }
    }

    let fading = 1.0 - tall;
    painter.rect_filled(open, 0.0, glow(hot, 0.35 * fading));
    for y in [open.top(), open.bottom()] {
        let edge = [Pos2::new(open.left(), y), Pos2::new(open.right(), y)];
        painter.line_segment(edge, Stroke::new(16.0, glow(color, 0.25 * fading)));
        painter.line_segment(edge, Stroke::new(2.0, glow(hot, fading)));
    }

    let ring = remap(on, 0.0, 0.4);
    if ring < 1.0 {
        painter.circle_stroke(
            rect.center(),
            20.0 + ease_out_cubic(ring) * rect.width() * 0.35,
            Stroke::new(0.5 + 3.0 * (1.0 - ring), glow(hot, 0.7 * (1.0 - ring))),
        );
    }
}

/// Where a ray from `from` (inside `rect`) along `dir` leaves it.
fn exit_point(rect: Rect, from: Pos2, dir: Vec2) -> Pos2 {
    let along = |pos: f32, d: f32, min: f32, max: f32| {
        if d > 0.0 {
            (max - pos) / d
        } else if d < 0.0 {
            (min - pos) / d
        } else {
            f32::INFINITY
        }
    };
    let tx = along(from.x, dir.x, rect.left(), rect.right());
    let ty = along(from.y, dir.y, rect.top(), rect.bottom());
    from + dir * tx.min(ty)
}
