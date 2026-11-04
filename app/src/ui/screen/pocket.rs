//! A little pocket player lying in the dark, its white earphones beside it,
//! the line being sung on its backlit screen in plain blue type, or over the
//! music video on it when there is one.

use std::f32::consts::TAU;

use eframe::egui::{
    Align2, Color32, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2,
    text::{LayoutJob, TextFormat, TextWrapping},
};

use super::super::live::looks::SANS;
use super::{Scene, glow, snap, vignette};

/// A smooth curve through `points`, as a polyline.
fn spline(points: &[Pos2], steps: usize) -> Vec<Pos2> {
    let mut out = Vec::new();
    for i in 0..points.len().saturating_sub(1) {
        let p0 = points[i.saturating_sub(1)];
        let (p1, p2) = (points[i], points[i + 1]);
        let p3 = points[(i + 2).min(points.len() - 1)];
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let v = (p1.to_vec2() * 2.0
                + (p2 - p0) * t
                + (p0.to_vec2() * 2.0 - p1.to_vec2() * 5.0 + p2.to_vec2() * 4.0 - p3.to_vec2()) * t2
                + (p3.to_vec2() - p0.to_vec2() + p1.to_vec2() * 3.0 - p2.to_vec2() * 3.0) * t3)
                * 0.5;
            out.push(v.to_pos2());
        }
    }
    out.extend(points.last());
    out
}

/// A white cord along `points`, shaded round.
fn cord(painter: &Painter, points: &[Pos2], width: f32) {
    let line = spline(points, 16);
    painter.add(Shape::line(line.clone(), Stroke::new(width * 1.6, Color32::from_black_alpha(70))));
    painter.add(Shape::line(line.clone(), Stroke::new(width, Color32::from_rgb(196, 196, 192))));
    let lit: Vec<Pos2> = line.iter().map(|p| *p + Vec2::new(-0.2, -0.25) * width).collect();
    painter.add(Shape::line(lit, Stroke::new(width * 0.45, Color32::from_rgb(246, 246, 242))));
}

/// An earbud lying at `at`, its stem pointing along `dir` to where the cord goes.
fn earbud(painter: &Painter, at: Pos2, dir: Vec2, r: f32) -> Pos2 {
    let dir = dir.normalized();
    let stem_end = at + dir * r * 2.4;
    painter.circle_filled(at + Vec2::new(r * 0.08, r * 0.14), r * 1.05, Color32::from_black_alpha(90));
    painter.add(Shape::line_segment([at, stem_end], Stroke::new(r * 0.62, Color32::from_rgb(200, 200, 196))));
    painter.add(Shape::line_segment([at, stem_end - dir * r * 0.1], Stroke::new(r * 0.4, Color32::from_rgb(238, 238, 234))));
    painter.circle_filled(at, r, Color32::from_rgb(214, 214, 210));
    painter.circle_filled(at - Vec2::new(r * 0.12, r * 0.14), r * 0.8, Color32::from_rgb(240, 240, 236));
    // The grille, a little darker.
    let grille = at - dir * r * 0.15;
    painter.circle_filled(grille, r * 0.42, Color32::from_rgb(178, 178, 176));
    stem_end
}

/// The pocket player.
pub(in crate::ui) fn draw_pocket(painter: &Painter, s: &Scene) {
    let rect = s.rect;
    // The dark it lies in, a lamp off to one side.
    painter.rect_filled(rect, 0.0, Color32::from_rgb(14, 14, 16));
    glow(painter, rect.left_top() + rect.size() * Vec2::new(0.42, 0.3), rect.size() * Vec2::new(0.75, 0.9), Color32::from_rgb(38, 37, 40));

    let h = rect.height() * 0.78;
    let w = h * 0.74;
    let body = Rect::from_center_size(rect.center() + Vec2::new(rect.width() * 0.06, 0.0), Vec2::new(w, h));
    let round = w * 0.09;

    // The earphones, lying out to the left, their cord running to the jack.
    let r = w * 0.11;
    let buds = [
        (body.left_top() + Vec2::new(-w * 0.95, h * 0.05), Vec2::new(0.6, 1.0)),
        (body.left_top() + Vec2::new(-w * 0.45, -h * 0.02), Vec2::new(-0.2, 1.0)),
    ];
    let ends: Vec<Pos2> = buds.iter().map(|&(at, dir)| earbud(painter, at, dir, r)).collect();
    let split = body.left_bottom() + Vec2::new(-w * 0.55, -h * 0.22);
    let thin = (w * 0.018).max(1.5);
    cord(painter, &[ends[0], ends[0] + Vec2::new(0.0, h * 0.15), split + Vec2::new(-w * 0.1, -h * 0.05), split], thin);
    cord(painter, &[ends[1], ends[1] + Vec2::new(-w * 0.02, h * 0.2), split + Vec2::new(w * 0.06, -h * 0.12), split], thin);
    let jack = Pos2::new(body.left() + w * 0.2, body.bottom());
    cord(
        painter,
        &[split, split + Vec2::new(w * 0.05, h * 0.2), Pos2::new(jack.x - w * 0.1, rect.bottom() - h * 0.02), Pos2::new(jack.x, body.bottom() + h * 0.06), jack],
        thin * 1.15,
    );
    painter.rect_filled(Rect::from_center_size(Pos2::new(jack.x, body.bottom() + h * 0.018), Vec2::new(w * 0.05, h * 0.04)), w * 0.01, Color32::from_rgb(226, 226, 222));

    // The body: a wide soft shadow, then dark glossy aluminium.
    for (offset, blur, alpha) in [(0.04, 0.12, 120), (0.01, 0.025, 140)] {
        let shadow = eframe::egui::Shadow {
            offset: [(h * offset * 0.4).min(127.0) as i8, (h * offset).min(127.0) as i8],
            blur: (h * blur).min(255.0) as u8,
            spread: 0,
            color: Color32::from_black_alpha(alpha),
        };
        painter.add(shadow.as_shape(body, round));
    }
    painter.rect_filled(body, round, Color32::from_rgb(30, 30, 33));
    // Light catching the top, falling off down the body.
    let mut sheen = Mesh::default();
    let inner = Rect::from_min_max(body.min + Vec2::new(w * 0.012, round), body.max - Vec2::splat(w * 0.012));
    for (t, a) in [(0.0, 18u8), (0.3, 6), (1.0, 0)] {
        let y = inner.top() + inner.height() * t;
        sheen.colored_vertex(Pos2::new(inner.left(), y), Color32::from_white_alpha(a));
        sheen.colored_vertex(Pos2::new(inner.right(), y), Color32::from_white_alpha(a));
    }
    for i in 0..4 {
        sheen.add_triangle(i, i + 1, i + 2);
    }
    painter.add(sheen);
    painter.rect_stroke(body, round, Stroke::new((w * 0.006).max(1.0), Color32::from_rgb(78, 78, 84)), StrokeKind::Inside);

    // The screen behind its glass.
    let glass = Rect::from_min_size(body.min + Vec2::new(w * 0.08, w * 0.08), Vec2::new(w * 0.84, w * 0.6));
    painter.rect_filled(glass, w * 0.02, Color32::from_rgb(6, 6, 8));
    let screen = glass.shrink(w * 0.035);
    let beat = 1.0 + 0.04 * s.beat;
    let backlight = Color32::from_rgb(232, 238, 247);
    match s.clip {
        Some(clip) => {
            let fill = (screen.width() / clip.size.x).max(screen.height() / clip.size.y);
            let shown = screen.size() / (clip.size * fill);
            let uv = Rect::from_center_size(Pos2::new(0.5, 0.5), shown);
            painter.rect_filled(screen, 0.0, Color32::BLACK);
            painter.image(clip.texture, screen, uv, Color32::from_rgb(235, 240, 250).gamma_multiply(clip.alpha));
            painter.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(210, 225, 255, 28));
        }
        None => {
            painter.rect_filled(screen, 0.0, backlight);
        }
    }
    // The backlight spilling out onto the glass around it.
    glow(painter, screen.center(), screen.size() * 0.72 * beat, Color32::from_rgba_unmultiplied(200, 215, 255, 20));

    // The line being sung, word by word, in plain type.
    if let Some(i) = s.lyrics.line_at(s.position) {
        let starts = s.lyrics.word_starts(i);
        let text = s.lyrics.synced[i].text.trim_end().to_lowercase();
        let chars: Vec<char> = text.chars().collect();
        let sung_to = starts.iter().filter(|(_, at)| *at <= s.position).map(|(r, _)| r.end).max().unwrap_or(0).min(chars.len());
        let ink = if s.clip.is_some() { Color32::from_rgb(10, 10, 12) } else { Color32::from_rgb(26, 72, 170) };
        let (head, tail): (String, String) = (chars[..sung_to].iter().collect(), chars[sung_to..].iter().collect());
        let layout = |px: f32| {
            let mut job = LayoutJob {
                wrap: TextWrapping { max_width: screen.width() * 0.84, ..Default::default() },
                halign: eframe::egui::Align::Center,
                ..Default::default()
            };
            let format = |color: Color32| TextFormat { font_id: FontId::new(px, FontFamily::Name(SANS.into())), color, ..Default::default() };
            job.append(&head, 0.0, format(ink));
            job.append(&tail, 0.0, format(Color32::TRANSPARENT));
            painter.layout_job(job)
        };
        // Long lines get smaller type until the whole line fits the screen.
        let mut px = snap(screen.height() * 0.2, 2.0);
        let mut galley = layout(px);
        while px > 10.0 && (galley.rect.height() > screen.height() * 0.86 || galley.rect.width() > screen.width() * 0.92) {
            px = snap(px * 0.88, 2.0).max(10.0);
            galley = layout(px);
        }
        let at = screen.center() - galley.rect.center().to_vec2();
        let painter = painter.with_clip_rect(screen);
        if s.clip.is_some() {
            // A thin light edge so it reads over the picture.
            for k in 0..8 {
                let off = Vec2::angled(k as f32 / 8.0 * TAU) * (px * 0.05);
                painter.galley_with_override_text_color(at + off, galley.clone(), Color32::from_white_alpha(120));
            }
        }
        painter.galley(at, galley, ink);
    }
    // The LCD's grain, and the glass catching the lamp.
    let pitch = (screen.height() / 120.0).max(2.0);
    let mut grain = Mesh::default();
    let mut y = screen.top();
    while y < screen.bottom() {
        grain.add_colored_rect(Rect::from_min_size(Pos2::new(screen.left(), y), Vec2::new(screen.width(), pitch * 0.3)), Color32::from_black_alpha(10));
        y += pitch;
    }
    painter.add(grain);
    let mut gloss = Mesh::default();
    let g = glass;
    gloss.colored_vertex(g.left_top(), Color32::from_white_alpha(22));
    gloss.colored_vertex(g.left_top() + Vec2::new(g.width() * 0.55, 0.0), Color32::from_white_alpha(10));
    gloss.colored_vertex(g.left_top() + Vec2::new(0.0, g.height() * 0.7), Color32::from_white_alpha(0));
    gloss.add_triangle(0, 1, 2);
    painter.add(gloss);

    // The wheel, MENU at its top, the buttons round it.
    let center = Pos2::new(body.center().x, body.bottom() - w * 0.355);
    let wheel = w * 0.27;
    painter.circle_filled(center, wheel, Color32::from_rgb(40, 40, 44));
    painter.circle_stroke(center, wheel, Stroke::new((w * 0.005).max(1.0), Color32::from_rgb(64, 64, 70)));
    painter.circle_filled(center, wheel * 0.38, Color32::from_rgb(34, 34, 37));
    painter.circle_stroke(center, wheel * 0.38, Stroke::new((w * 0.004).max(1.0), Color32::from_rgb(58, 58, 64)));
    let label = Color32::from_rgb(170, 170, 176);
    painter.text(
        center - Vec2::new(0.0, wheel * 0.72),
        Align2::CENTER_CENTER,
        "MENU",
        FontId::new(snap(wheel * 0.15, 1.0), FontFamily::Name(SANS.into())),
        label,
    );
    let k = wheel * 0.07;
    let tri = |c: Pos2, dir: f32| {
        Shape::convex_polygon(vec![c + Vec2::new(-k * dir, -k), c + Vec2::new(k * dir, 0.0), c + Vec2::new(-k * dir, k)], label, Stroke::NONE)
    };
    for dir in [-1.0f32, 1.0] {
        let c = center + Vec2::new(dir * wheel * 0.72, 0.0);
        painter.add(tri(c + Vec2::new(-k * 0.5 * dir, 0.0), dir));
        painter.add(tri(c + Vec2::new(k * 1.1 * dir, 0.0), dir));
        painter.rect_filled(Rect::from_center_size(c + Vec2::new(k * 2.2 * dir, 0.0), Vec2::new(k * 0.35, k * 2.0)), 0.0, label);
    }
    let play = center + Vec2::new(-k * 1.0, wheel * 0.72);
    painter.add(tri(play, 1.0));
    for dx in [1.6, 2.4] {
        painter.rect_filled(Rect::from_center_size(play + Vec2::new(k * dx, 0.0), Vec2::new(k * 0.4, k * 2.0)), 0.0, label);
    }
    vignette(painter, rect, 170.0);
}
