//! A black silhouette dancing to the song on a flat neon color, a white
//! earphone cord swinging on it, and the words huge in tall narrow capitals
//! across it, turned to the background's color where they cross the dancer.

use std::f32::consts::{PI, TAU};

use eframe::egui::{Color32, FontFamily, FontId, Galley, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2, epaint::Vertex};

use super::super::live::looks::CONDENSED;
use super::{Scene, current_block, hash, neon, rgb, snap};

/// A point of the dancer, in heights: x to the right, y down, the floor at 0.
type P = Vec2;

/// Where the dancer's hands and feet go and how the body sits, for one moment.
#[derive(Clone, Copy)]
struct Pose {
    /// The pelvis, from between the feet on the floor.
    pelvis: P,
    /// Lean of the torso, radians, to the right positive.
    lean: f32,
    /// Tilt of the head on top of that.
    nod: f32,
    /// Hands, from their shoulders.
    hands: [P; 2],
    /// Feet, on the floor (y 0 or above when lifted).
    feet: [P; 2],
}

impl Pose {
    fn lerp(self, other: Pose, t: f32) -> Pose {
        let l = |a: P, b: P| a + (b - a) * t;
        let f = |a: f32, b: f32| a + (b - a) * t;
        Pose {
            pelvis: l(self.pelvis, other.pelvis),
            lean: f(self.lean, other.lean),
            nod: f(self.nod, other.nod),
            hands: [l(self.hands[0], other.hands[0]), l(self.hands[1], other.hands[1])],
            feet: [l(self.feet[0], other.feet[0]), l(self.feet[1], other.feet[1])],
        }
    }

    fn mirrored(self) -> Pose {
        let m = |p: P| Vec2::new(-p.x, p.y);
        Pose {
            pelvis: m(self.pelvis),
            lean: -self.lean,
            nod: -self.nod,
            hands: [m(self.hands[1]), m(self.hands[0])],
            feet: [m(self.feet[1]), m(self.feet[0])],
        }
    }
}

const THIGH: f32 = 0.245;
const SHIN: f32 = 0.245;
const UPPER_ARM: f32 = 0.165;
const FOREARM: f32 = 0.16;
const STAND: f32 = -0.52;

/// 0 off the beat, 1 on it: a dip landing on every beat.
fn dip(b: f32) -> f32 {
    let c = 0.5 + 0.5 * (b * TAU).cos();
    c * c
}

/// The dancer at `b` beats into move `kind`.
fn pose(kind: u32, b: f32) -> Pose {
    let d = dip(b);
    let side = if (b.floor() as i64).rem_euclid(2) == 0 { 1.0 } else { -1.0 };
    let swing = (b * PI).sin();
    match kind {
        // Bouncing on the knees, a fist pumping up on every beat, hand to hand.
        0 => {
            let up = Vec2::new(0.06, -0.33 - 0.04 * d);
            let down = Vec2::new(0.1, 0.12);
            let (l, r) = if side > 0.0 { (down * Vec2::new(-1.0, 1.0), up) } else { (Vec2::new(-up.x, up.y), down) };
            Pose {
                pelvis: Vec2::new(0.0, STAND + 0.05 * d),
                lean: 0.05 * swing,
                nod: 0.12 * d * side,
                hands: [l, r],
                feet: [Vec2::new(-0.13, 0.0), Vec2::new(0.13, 0.0)],
            }
        }
        // Stepping side to side, arms swinging against the hips.
        1 => {
            let x = 0.07 * swing;
            let lift = |s: f32| -0.06 * (s * swing).max(0.0) * (1.0 - d);
            Pose {
                pelvis: Vec2::new(x, STAND + 0.035 * d),
                lean: -0.12 * swing,
                nod: 0.1 * swing,
                hands: [Vec2::new(-0.16 - 0.08 * swing, 0.05), Vec2::new(0.16 - 0.08 * swing, 0.05)],
                feet: [Vec2::new(-0.16 + 0.6 * x, lift(-1.0)), Vec2::new(0.16 + 0.6 * x, lift(1.0))],
            }
        }
        // Knees up in turn, elbows out, the head going with the beat.
        2 => {
            let knee = |s: f32| -0.16 * (s * swing).max(0.0);
            Pose {
                pelvis: Vec2::new(0.02 * swing, STAND + 0.03 * d),
                lean: 0.08 * swing,
                nod: -0.15 * d,
                hands: [Vec2::new(-0.2, -0.02 - 0.06 * swing.max(0.0)), Vec2::new(0.2, -0.02 + 0.06 * swing.min(0.0))],
                feet: [Vec2::new(-0.1, knee(-1.0)), Vec2::new(0.1, knee(1.0))],
            }
        }
        // Wide and low, leaning back, arms thrown out.
        3 => Pose {
            pelvis: Vec2::new(0.0, STAND + 0.08 + 0.04 * d),
            lean: -0.18 + 0.1 * d,
            nod: -0.25 + 0.2 * d,
            hands: [Vec2::new(-0.3, -0.1 - 0.08 * d), Vec2::new(0.3, -0.12 + 0.05 * swing)],
            feet: [Vec2::new(-0.22, 0.0), Vec2::new(0.22, 0.0)],
        },
        // Both arms up, swaying.
        _ => Pose {
            pelvis: Vec2::new(0.05 * swing, STAND + 0.03 * d),
            lean: 0.16 * swing,
            nod: 0.18 * swing,
            hands: [Vec2::new(-0.1 + 0.08 * swing, -0.3), Vec2::new(0.1 + 0.08 * swing, -0.3 - 0.03 * d)],
            feet: [Vec2::new(-0.14, 0.0), Vec2::new(0.14, 0.0)],
        },
    }
}

const MOVES: u32 = 5;
/// Beats a move lasts.
const MOVE_BEATS: f32 = 8.0;

/// The pose at `b` beats into the song, moves changing every few bars with
/// a short blend between them.
fn dance(b: f32) -> Pose {
    let n = (b / MOVE_BEATS).floor().max(0.0) as u32;
    let kind = |n: u32| (hash(n * 31 + 7) * MOVES as f32) as u32 % MOVES;
    let at = |n: u32| {
        let p = pose(kind(n), b);
        if hash(n * 13 + 1) < 0.5 { p.mirrored() } else { p }
    };
    let into = b - n as f32 * MOVE_BEATS;
    let now = at(n);
    if n > 0 && into < 0.5 {
        let t = into / 0.5;
        at(n - 1).lerp(now, t * t * (3.0 - 2.0 * t))
    } else {
        now
    }
}

/// The middle joint of a limb from `a` reaching for `to`, bent away from `away`.
fn bend(a: P, to: P, l1: f32, l2: f32, away: f32) -> P {
    let d = (to - a).length().clamp((l1 - l2).abs() + 1e-3, l1 + l2 - 1e-3);
    let dir = (to - a).normalized();
    let angle = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0).acos();
    let options = [angle, -angle].map(|a2| a + eframe::egui::emath::Rot2::from_angle(a2) * dir * l1);
    if (options[0].x - options[1].x) * away >= 0.0 { options[0] } else { options[1] }
}

/// The convex hull of circles, as a polygon.
fn hull(circles: &[(P, f32)]) -> Vec<P> {
    let mut points: Vec<P> = circles
        .iter()
        .flat_map(|&(c, r)| (0..20).map(move |k| c + Vec2::angled(k as f32 / 20.0 * TAU) * r))
        .collect();
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let cross = |o: P, a: P, b: P| (a - o).x * (b - o).y - (a - o).y * (b - o).x;
    let mut lower: Vec<P> = Vec::new();
    for &p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<P> = Vec::new();
    for &p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The dancer as convex pieces, and where the earphone cord runs, in heights.
fn body(pose: Pose) -> (Vec<Vec<P>>, Vec<Vec<P>>) {
    let rot = |a: f32, v: Vec2| eframe::egui::emath::Rot2::from_angle(a) * v;
    let p = pose.pelvis;
    let chest = p + rot(pose.lean, Vec2::new(0.0, -0.29));
    let neck = chest + rot(pose.lean, Vec2::new(0.0, -0.035));
    let head = neck + rot(pose.lean + pose.nod, Vec2::new(0.0, -0.07));
    let shoulders = [-1.0, 1.0].map(|s| chest + rot(pose.lean, Vec2::new(0.085 * s, 0.03)));
    let waist = [-1.0, 1.0].map(|s| p + rot(pose.lean * 0.5, Vec2::new(0.06 * s, -0.1)));
    let hips = [-1.0, 1.0].map(|s| p + rot(pose.lean * 0.3, Vec2::new(0.062 * s, 0.0)));
    let mut pieces = Vec::new();
    // The head in a cap, its brim to the side it tilts to.
    let face = if pose.nod + pose.lean >= 0.0 { 1.0 } else { -1.0 };
    let up = rot(pose.lean + pose.nod, Vec2::new(0.0, -1.0));
    let across = rot(pose.lean + pose.nod, Vec2::new(face, 0.0));
    pieces.push(hull(&[(head, 0.066), (head - up * 0.03 + across * 0.012, 0.05)]));
    pieces.push(hull(&[(head + up * 0.014, 0.07), (head + up * 0.024 - across * 0.012, 0.064)]));
    pieces.push(hull(&[(head + up * 0.036 + across * 0.05, 0.014), (head + up * 0.03 + across * 0.13, 0.01)]));
    // A hoodie: the hood bunched round the neck, roomy down to the waist.
    pieces.push(hull(&[(neck, 0.05), (chest + rot(pose.lean, Vec2::new(0.0, -0.01)), 0.07)]));
    pieces.push(hull(&[(shoulders[0], 0.06), (shoulders[1], 0.06), (chest + rot(pose.lean, Vec2::new(0.0, 0.08)), 0.085), (waist[0], 0.055), (waist[1], 0.055)]));
    pieces.push(hull(&[(waist[0], 0.055), (waist[1], 0.055), (hips[0], 0.064), (hips[1], 0.064)]));
    for s in 0..2 {
        let away = if s == 0 { -1.0 } else { 1.0 };
        let hand = shoulders[s] + pose.hands[s];
        let elbow = bend(shoulders[s], hand, UPPER_ARM, FOREARM, away);
        pieces.push(hull(&[(shoulders[s], 0.05), (elbow, 0.04)]));
        pieces.push(hull(&[(elbow, 0.04), (hand - (hand - elbow).normalized() * 0.03, 0.034)]));
        pieces.push(hull(&[(hand, 0.03), (hand + (hand - elbow).normalized() * 0.012, 0.026)]));
        let foot = pose.feet[s];
        let ankle = foot + Vec2::new(0.0, -0.04);
        let knee = bend(hips[s], ankle, THIGH, SHIN, away);
        // Baggy jeans, wide over the sneakers.
        pieces.push(hull(&[(hips[s], 0.066), (knee, 0.056)]));
        pieces.push(hull(&[(knee, 0.056), (ankle + Vec2::new(0.0, -0.01), 0.058)]));
        pieces.push(hull(&[(ankle, 0.042), (foot + Vec2::new(0.1 * away, -0.016), 0.03), (foot + Vec2::new(-0.03 * away, -0.02), 0.028)]));
    }
    // The cord: from both ears down to a split on the chest, then to the pocket.
    let ears = [-1.0, 1.0].map(|s| head + rot(pose.lean + pose.nod, Vec2::new(0.055 * s, 0.015)));
    let split = chest + rot(pose.lean, Vec2::new(0.02, 0.06));
    let pocket = hips[1] + rot(pose.lean * 0.3, Vec2::new(-0.01, -0.04));
    let sag = |a: P, b: P, droop: f32| -> Vec<P> {
        (0..=12)
            .map(|k| {
                let t = k as f32 / 12.0;
                a + (b - a) * t + Vec2::new(0.0, droop * (t * PI).sin())
            })
            .collect()
    };
    let cords = vec![sag(ears[0], split, 0.02), sag(ears[1], split, 0.02), sag(split, pocket, 0.03)];
    (pieces, cords)
}

/// `poly` (convex) cut to the part inside `clip` (convex).
fn clip_convex(poly: &[Pos2], clip: &[Pos2]) -> Vec<Pos2> {
    let area: f32 = clip.iter().zip(clip.iter().cycle().skip(1)).map(|(a, b)| a.x * b.y - b.x * a.y).sum();
    let sign = area.signum();
    let mut out = poly.to_vec();
    for (i, &a) in clip.iter().enumerate() {
        let b = clip[(i + 1) % clip.len()];
        let inside = |p: Pos2| ((b - a).x * (p - a).y - (b - a).y * (p - a).x) * sign >= 0.0;
        let input = std::mem::take(&mut out);
        for (j, &p) in input.iter().enumerate() {
            let q = input[(j + 1) % input.len()];
            let (pi, qi) = (inside(p), inside(q));
            if pi {
                out.push(p);
            }
            if pi != qi {
                let n = Vec2::new(-(b - a).y, (b - a).x);
                let t = n.dot(a - p) / n.dot(q - p);
                out.push(p + (q - p) * t);
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
}

/// The glyphs of `galley` at `at`, only where they fall inside the pieces,
/// in `color`.
fn glyphs_inside(galley: &Galley, at: Pos2, pieces: &[Vec<Pos2>], atlas: Vec2, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let bounds: Vec<Rect> = pieces
        .iter()
        .map(|p| p.iter().fold(Rect::NOTHING, |r, &q| r.union(Rect::from_min_max(q, q))))
        .collect();
    for row in &galley.rows {
        for quad in row.visuals.mesh.vertices.chunks_exact(4) {
            let fold = |f: fn(&Vertex) -> Pos2| quad.iter().map(f).fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(p, p)));
            let pos = fold(|v| v.pos).translate(at.to_vec2() + row.pos.to_vec2());
            let uv = fold(|v| v.uv);
            if pos.width() <= 0.0 || pos.height() <= 0.0 {
                continue;
            }
            let corners = [pos.left_top(), pos.right_top(), pos.right_bottom(), pos.left_bottom()];
            for (piece, b) in pieces.iter().zip(&bounds) {
                if !b.intersects(pos) {
                    continue;
                }
                let cut = clip_convex(&corners, piece);
                if cut.len() < 3 {
                    continue;
                }
                let base = mesh.vertices.len() as u32;
                for p in &cut {
                    let t = (*p - pos.min) / pos.size();
                    let uv = uv.min + uv.size() * t;
                    mesh.vertices.push(Vertex { pos: *p, uv: Pos2::new(uv.x / atlas.x, uv.y / atlas.y), color });
                }
                for k in 1..cut.len() as u32 - 1 {
                    mesh.add_triangle(base, base + k, base + k + 1);
                }
            }
        }
    }
    mesh
}

/// Loud colors for the background, the cover's own among them.
const COLORS: [Color32; 9] = [
    Color32::from_rgb(57, 255, 20),
    Color32::from_rgb(40, 200, 255),
    Color32::from_rgb(255, 46, 147),
    Color32::from_rgb(255, 230, 0),
    Color32::from_rgb(255, 106, 0),
    Color32::from_rgb(160, 80, 255),
    Color32::from_rgb(255, 40, 40),
    Color32::from_rgb(0, 240, 190),
    Color32::from_rgb(245, 245, 240),
];

/// The background and the dancer's colors for block `n`: a new loud color
/// each block, never the last one again, and now and then turned round to a
/// colored dancer on black.
fn colors(s: &Scene, n: usize) -> (Color32, Color32) {
    let black = Color32::from_rgb(8, 8, 10);
    let cover = {
        let c = neon(s.palette[0]);
        let max = c[0].max(c[1]).max(c[2]).max(1e-3);
        rgb(c.map(|v| (v / max).powf(1.4) * 0.95 + 0.03))
    };
    let count = COLORS.len() + 1;
    let pick = |n: usize| (hash(n as u32 * 97 + 11) * count as f32) as usize % count;
    let mut k = pick(n);
    if n > 0 && k == pick(n - 1) {
        k = (k + 1 + (hash(n as u32 * 5 + 3) * (count - 1) as f32) as usize) % count;
    }
    let color = if k == COLORS.len() { cover } else { COLORS[k] };
    let white = color == COLORS[8];
    if !white && hash(n as u32 * 41 + 29) < 0.3 { (black, color) } else { (color, black) }
}

/// The dancing silhouettes.
pub(in crate::ui) fn draw_dance(painter: &Painter, s: &Scene) {
    let rect = s.rect;
    let block = current_block(s.lyrics, s.position, 14, 1);
    let (back, figure) = colors(s, block.as_ref().map_or(0, |b| b.3));
    painter.rect_filled(rect, 0.0, back);
    let ink = figure;

    // The dancer, a little taller than the screen, cut off at the shins.
    let bpm = s.bpm.unwrap_or(112.0).clamp(60.0, 180.0);
    let bpm = if bpm > 140.0 { bpm / 2.0 } else { bpm };
    let beats = s.song_position.unwrap_or(s.position).as_secs_f32() * bpm / 60.0;
    let height = rect.height() * 1.12;
    let feet = Pos2::new(rect.center().x, rect.bottom() + rect.height() * 0.2);
    let place = |p: P| feet + p * height;
    let (pieces, cords) = body(dance(beats));
    let pieces: Vec<Vec<Pos2>> = pieces.into_iter().map(|p| p.into_iter().map(place).collect()).collect();
    for piece in &pieces {
        painter.add(Shape::convex_polygon(piece.clone(), ink, Stroke::NONE));
    }
    let cord = Stroke::new((height * 0.0045).max(1.5), Color32::from_rgb(244, 244, 240));
    for c in cords {
        painter.add(Shape::line(c.into_iter().map(place).collect(), cord));
    }

    // The words, huge across the dancer.
    let Some((words, rows, current, _)) = block else {
        return;
    };
    let row = &rows[0];
    let whole = words[row.clone()].iter().map(|w| w.text.trim().to_owned()).collect::<Vec<_>>().join(" ");
    let sung = words[row.start..=current.min(row.end - 1)].iter().map(|w| w.text.trim().to_owned()).collect::<Vec<_>>().join(" ");
    let font = |px: f32| FontId::new(px, FontFamily::Name(CONDENSED.into()));
    let mut px = snap(rect.height() * 0.42, 8.0);
    let full = painter.layout_no_wrap(whole.clone(), font(px), ink);
    let room = rect.width() * 0.9;
    if full.size().x > room {
        px = snap(px * room / full.size().x, 4.0);
    }
    let full = painter.layout_no_wrap(whole, font(px), ink);
    let galley = painter.layout_no_wrap(sung, font(px), ink);
    let at = Pos2::new(rect.center().x - galley.size().x / 2.0, rect.top() + rect.height() * 0.5 - full.size().y / 2.0);
    painter.galley(at, galley.clone(), ink);
    let atlas = Vec2::from(painter.fonts(|f| f.font_image_size()).map(|v| v as f32));
    painter.add(glyphs_inside(&galley, at, &pieces, atlas, back));
}
