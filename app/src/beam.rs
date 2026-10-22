//! Ctrl+D transition: the lyrics fly off the main window towards the other
//! monitor as they are, trailing fading copies of themselves as they speed
//! up, while the cover and titles grow into the middle. The other screen
//! opens with a wipe from the side they come from, a glowing edge sweeping
//! across, and the lyrics fly in behind it and settle with a spring.
//! Turning the mirror off plays the same timeline backwards.

use eframe::egui::{self, Color32, Mesh, Painter, Pos2, Rect, Stroke, Vec2, emath::TSTransform};

use crate::anim::{ease_in_cubic, ease_in_out_cubic, glow, remap, spring};

/// Parts of the timeline (0 -> 1).
const OUT: (f32, f32) = (0.0, 0.45);
const SPLIT: (f32, f32) = (0.1, 0.7);
const WIPE: (f32, f32) = (0.3, 0.75);
const IN: (f32, f32) = (0.42, 1.0);
/// Fading copies behind moving lyrics.
const GHOSTS: usize = 7;
/// How long a trail is, as a share of the timeline the lyrics cover in it.
const TRAIL: f32 = 0.05;

/// How far the main window has handed the lyrics over (0 -> 1).
pub fn split(t: f32) -> f32 {
    ease_in_out_cubic(remap(t, SPLIT.0, SPLIT.1))
}

/// Lyrics on their way: where they are, off their place, and where they
/// were a moment ago, for the trail.
#[derive(Clone, Copy)]
pub struct Flight {
    offset: Vec2,
    behind: Vec2,
}

/// Main window: the lyrics speeding off through the edge facing `dir`.
/// `None` before they move; [`gone`] once they are out of sight.
pub fn leaving(rect: Rect, dir: Vec2, t: f32) -> Option<Flight> {
    let s = |t: f32| ease_in_cubic(remap(t, OUT.0, OUT.1));
    (s(t) > 0.0).then(|| {
        let far = dir * across(rect, dir);
        Flight {
            offset: far * s(t),
            behind: far * s(t - TRAIL),
        }
    })
}

/// The lyrics have left the main window.
pub fn gone(t: f32) -> bool {
    t >= OUT.1
}

/// Other screen: the lyrics coming in from the side facing the main window
/// and settling in their place; `None` once they are there.
pub fn arriving(rect: Rect, dir: Vec2, t: f32) -> Option<Flight> {
    let s = |t: f32| spring(remap(t, IN.0, IN.1));
    (t < IN.1).then(|| {
        let far = -dir * across(rect, dir);
        Flight {
            offset: far * (1.0 - s(t)),
            behind: far * (1.0 - s(t - TRAIL)),
        }
    })
}

/// Draws what `draw` paints, moved by `flight`, over a trail of fainter copies.
pub fn with_trail(ui: &mut egui::Ui, flight: Flight, mut draw: impl FnMut(&mut egui::Ui)) {
    for k in (1..=GHOSTS).rev() {
        let back = k as f32 / GHOSTS as f32;
        let offset = flight.offset + (flight.behind - flight.offset) * back;
        if (offset - flight.offset).length() < 2.0 {
            continue;
        }
        ui.scope(|ui| {
            ui.multiply_opacity(0.28 * (1.0 - back) * (1.0 - back) + 0.02);
            ui.with_visual_transform(TSTransform::from_translation(offset), &mut draw);
        });
    }
    ui.with_visual_transform(TSTransform::from_translation(flight.offset), &mut draw);
}

/// Other screen: black, then wiped open from the side facing the main window,
/// with a glowing edge in `color` sweeping across.
pub fn draw_wipe(painter: &Painter, rect: Rect, dir: Vec2, t: f32, color: Color32) {
    let w = ease_in_out_cubic(remap(t, WIPE.0, WIPE.1));
    if w >= 1.0 {
        return;
    }
    // Along the main axis of travel, 0 at the edge the lyrics come in by.
    let horizontal = dir.x.abs() >= dir.y.abs();
    let forward = if horizontal { dir.x >= 0.0 } else { dir.y >= 0.0 };
    let length = if horizontal { rect.width() } else { rect.height() };
    let at = |a: f32| {
        let a = if forward { a } else { 1.0 - a };
        if horizontal {
            let x = rect.left() + rect.width() * a;
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())]
        } else {
            let y = rect.top() + rect.height() * a;
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)]
        }
    };
    let soft = 0.18;
    let front = w * (1.0 + soft);
    let black = |a: f32| Color32::from_black_alpha((a * 255.0) as u8);
    let mut mesh = Mesh::default();
    let stops = [(front - soft, 0.0), (front, 1.0), (front.max(1.0) + 1.0, 1.0)];
    for (a, alpha) in stops {
        let [p, q] = at(a.max(0.0));
        mesh.colored_vertex(p, black(alpha));
        mesh.colored_vertex(q, black(alpha));
    }
    for i in 0..(stops.len() as u32 - 1) * 2 {
        mesh.add_triangle(i, i + 1, i + 2);
    }
    painter.add(mesh);

    // The edge itself, brightest mid-sweep.
    let edge = at((front - soft * 0.5).clamp(0.0, 1.0));
    let heat = (std::f32::consts::PI * w).sin();
    let hot = color.lerp_to_gamma(Color32::WHITE, 0.6);
    painter.line_segment(edge, Stroke::new(length * 0.03, glow(color, 0.18 * heat)));
    painter.line_segment(edge, Stroke::new(3.0, glow(hot, 0.9 * heat)));
}

/// How far to move along `dir` to be fully out of `rect`, with a margin.
fn across(rect: Rect, dir: Vec2) -> f32 {
    (dir.x.abs() * rect.width() + dir.y.abs() * rect.height()) * 1.05
}
