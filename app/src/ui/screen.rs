//! Lyric styles that take the whole screen the way lyric edits do, each a
//! picture of its own: neon words behind a rainy window, a red CRT, one huge
//! word at a time, the block beside a spinning CD, and the music video under
//! a player like Spotify's, whose buttons work.

use std::f32::consts::TAU;
use std::ops::Range;
use std::time::Duration;

use eframe::egui::{
    Align2, Color32, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, TextureId,
    Vec2, emath::Rot2, epaint::Vertex,
};
use mv_core::{NowPlaying, PlaybackState, Repeat};

use super::typing::{self, Word};

mod textures;
use super::{BOLD, ClipFrame, HEAVY, REGULAR, draw_clip, fisheye, warped_mesh};

/// What a full-screen style draws from.
pub(super) struct Scene<'a> {
    pub rect: Rect,
    pub lyrics: &'a mv_enrich::Lyrics,
    /// Where the lyrics are, offset included.
    pub position: Duration,
    /// Where the song is, for the player.
    pub song_position: Option<Duration>,
    pub np: &'a NowPlaying,
    pub time: f32,
    pub palette: [[f32; 3]; 3],
    /// Jumps on a beat, then eases back down (0..1).
    pub beat: f32,
    pub cover: Option<TextureId>,
    pub clip: Option<ClipFrame>,
}

/// A button of the player.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Shuffle,
    Previous,
    PlayPause,
    Next,
    Repeat,
}

/// A number in 0..1 that looks random, the same for the same `n`.
fn hash(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x & 0xFF_FFFF) as f32 / 0x100_0000 as f32
}

fn rgb(c: [f32; 3]) -> Color32 {
    let v = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(v(c[0]), v(c[1]), v(c[2]))
}

/// `c` pushed toward full saturation and brightness, for neon.
fn neon(c: [f32; 3]) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]).max(1e-3);
    let min = c[0].min(c[1]).min(c[2]);
    c.map(|v| ((v - min) / (max - min).max(0.15)).clamp(0.0, 1.0) * 0.85 + 0.15)
}

/// The song's words, the rows of the block being sung (as ranges of them),
/// the word being sung and the block's number.
type Block = (Vec<Word>, Vec<Range<usize>>, usize, usize);

/// The block of rows the word being sung is in, as `(words, rows, current
/// word, block number)`: each line's words reflowed into rows of
/// `row_chars`, `rows` to a block, as [`typing::blocks`] does it.
fn current_block(
    lyrics: &mv_enrich::Lyrics,
    position: Duration,
    row_chars: usize,
    rows: usize,
) -> Option<Block> {
    let words = typing::song_words(lyrics, true);
    let current = words.partition_point(|w| w.at <= position).checked_sub(1)?;
    let (n, block) = typing::blocks(&words, row_chars, rows)
        .into_iter()
        .enumerate()
        .find(|(_, rows)| rows.iter().any(|r| r.contains(&current)))?;
    Some((words, block, current, n))
}

/// Px snapped to `step`, so sizes that follow the window don't fill the glyph atlas.
fn snap(px: f32, step: f32) -> f32 {
    ((px / step).floor() * step).max(step)
}

/// Darker toward the corners: black at `alpha` there, clear in the middle.
fn vignette(painter: &Painter, rect: Rect, alpha: f32) {
    const STEPS: u32 = 10;
    let mut mesh = Mesh::default();
    for y in 0..=STEPS {
        for x in 0..=STEPS {
            let t = Vec2::new(x as f32, y as f32) / STEPS as f32;
            let d = ((t - Vec2::splat(0.5)) * 2.0).length() / std::f32::consts::SQRT_2;
            let a = (d - 0.35).max(0.0) / 0.65;
            let a = (a * a * alpha).clamp(0.0, 255.0) as u8;
            mesh.colored_vertex(rect.min + rect.size() * t, Color32::from_black_alpha(a));
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
    painter.add(mesh);
}

/// A soft round glow of `color`, strongest at `center` and gone at `radius`.
fn glow(painter: &Painter, center: Pos2, radius: Vec2, color: Color32) {
    const SIDES: u32 = 48;
    let mut mesh = Mesh::default();
    mesh.colored_vertex(center, color);
    for k in 0..=SIDES {
        let a = k as f32 / SIDES as f32 * TAU;
        mesh.colored_vertex(center + Vec2::new(a.cos() * radius.x, a.sin() * radius.y), Color32::TRANSPARENT);
    }
    for k in 1..=SIDES {
        mesh.add_triangle(0, k, k + 1);
    }
    painter.add(mesh);
}

/// Neon words behind a rainy window: the block's rows stacked in the middle,
/// a word to a row's ends, white-hot or now and then in the cover's color,
/// blooming into the wet glass and lighting it around them, each flickering
/// on as it is sung; drops on the glass over everything and a few running down.
pub(super) fn draw_neon(painter: &Painter, s: &Scene) {
    let rect = s.rect;
    let ctx = painter.ctx();
    let scale = (rect.height() / 1080.0).max(0.5);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(8, 7, 9));
    let tint = neon(s.palette[0]);
    // The room behind the glass, out of focus: a dim haze of the light.
    glow(
        painter,
        rect.center(),
        rect.size() * Vec2::new(0.7, 0.75),
        rgb(tint.map(|v| v * 0.2)).gamma_multiply(0.8 + 0.2 * s.beat),
    );

    if let Some((words, rows, current, _)) = current_block(s.lyrics, s.position, 10, 4) {
        let side = (rect.width() * 0.78).min(rect.height() * 1.0);
        let base = snap(side * 0.16, 4.0);
        let font = |px: f32| FontId::new(px, FontFamily::Name(HEAVY.into()));
        let leading = 1.15;
        let height = base * leading * rows.len() as f32;
        let mut top = rect.center().y - height / 2.0;
        // Every lit word: its galley, where, how lit, its colors and size.
        let mut lit = Vec::new();
        for row in rows {
            let middle = top + base * leading / 2.0;
            top += base * leading;
            let texts: Vec<String> = words[row.clone()].iter().map(|w| w.text.to_uppercase()).collect();
            let layout = |px: f32| -> Vec<_> {
                texts.iter().map(|t| painter.layout_no_wrap(t.clone(), font(px), Color32::WHITE)).collect()
            };
            let mut px = base;
            let mut galleys = layout(px);
            let width: f32 = galleys.iter().map(|g| g.size().x).sum::<f32>() + px * 0.4 * (galleys.len() as f32 - 1.0);
            if width > side {
                px = snap(px * side / width, 2.0);
                galleys = layout(px);
            }
            let width: f32 = galleys.iter().map(|g| g.size().x).sum();
            // A lone word in the middle, more spread to the row's ends.
            let (mut x, gap) = match galleys.len() {
                1 => (rect.center().x - width / 2.0, 0.0),
                n => (rect.center().x - side / 2.0, (side - width) / (n - 1) as f32),
            };
            for (galley, k) in galleys.into_iter().zip(row) {
                let w = galley.size().x;
                if k <= current {
                    let since = s.position.saturating_sub(words[k].at).as_secs_f32();
                    // A tube catching: off and on a few times, then steady.
                    let on = if since < 0.22 && (since * 36.0) as u32 % 2 == 1 { 0.2 } else { 1.0 };
                    let (core, halo) = if hash(k as u32 * 7 + 3) < 0.2 {
                        (rgb(tint.map(|v| v * 0.45 + 0.55)), rgb(tint))
                    } else {
                        (Color32::from_rgb(255, 248, 242), rgb(tint.map(|v| v * 0.75 + 0.25)))
                    };
                    let pos = Pos2::new(x, middle - galley.size().y / 2.0);
                    lit.push((galley, pos, on, core, halo, px));
                }
                x += w + gap;
            }
        }
        let swell = 1.0 + 0.3 * s.beat;
        // The light the words throw on the wet glass around them.
        for (galley, pos, on, _, halo, px) in &lit {
            let size = galley.size();
            let reach = size * Vec2::new(0.8, 1.3) + Vec2::splat(*px * 0.5);
            glow(painter, *pos + size / 2.0, reach, halo.gamma_multiply(0.3 * on * swell));
        }
        // Their bloom, soft copies spreading out, and the hot tube in the middle.
        let bloom = [(0.24, 0.016), (0.16, 0.026), (0.1, 0.04), (0.06, 0.065), (0.03, 0.11), (0.012, 0.2)];
        for (galley, pos, on, core, halo, px) in lit {
            for (ring, (reach, alpha)) in bloom.into_iter().enumerate() {
                for d in 0..12 {
                    let a = (d as f32 + 0.5 * ring as f32) / 12.0 * TAU;
                    let off = Vec2::angled(a) * px * reach;
                    painter.galley_with_override_text_color(pos + off, galley.clone(), halo.gamma_multiply(alpha * on * swell));
                }
            }
            // Behind the glass even the tube is a touch soft.
            for d in 0..4 {
                let off = Vec2::angled(d as f32 / 4.0 * TAU) * (px * 0.01 + 0.6);
                painter.galley_with_override_text_color(pos + off, galley.clone(), core.gamma_multiply(0.3 * on));
            }
            painter.galley_with_override_text_color(pos, galley, core.gamma_multiply(on));
        }
    }

    // The glass: drops all over, and a few running down, leaving beads behind.
    let tile = rect.height() * 0.9;
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(rect.width() / tile, rect.height() / tile));
    painter.image(textures::rain(ctx), rect, uv, Color32::WHITE);
    let drop = textures::drop(ctx);
    let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    let h = rect.height();
    for i in 0..8u32 {
        let x = rect.left() + rect.width() * hash(1000 + i);
        let speed = (35.0 + 120.0 * hash(2000 + i)) * scale;
        let y = rect.top() + (s.time * speed + h * hash(3000 + i)) % (h * 1.5) - h * 0.25;
        let trail = h * (0.12 + 0.22 * hash(4000 + i));
        let r = (5.0 + 4.0 * hash(5000 + i)) * scale;
        let wander = |y: f32| x + ((y - rect.top()) * 0.02 + i as f32).sin() * 3.0 * scale;
        let streak: Vec<Pos2> = (0..14)
            .map(|k| {
                let ty = y - trail * k as f32 / 13.0;
                Pos2::new(wander(ty), ty)
            })
            .collect();
        painter.add(Shape::line(streak, Stroke::new(r * 0.45, Color32::from_white_alpha(8))));
        for k in 1..7u32 {
            let ty = y - trail * (k as f32 + hash(6000 + i * 10 + k)) / 7.0;
            let bead = r * (0.15 + 0.25 * hash(7000 + i * 10 + k));
            let at = Rect::from_center_size(Pos2::new(wander(ty), ty), Vec2::new(bead * 2.0, bead * 2.2));
            painter.image(drop, at, whole, Color32::WHITE);
        }
        let at = Rect::from_center_size(Pos2::new(wander(y), y), Vec2::new(r * 2.0, r * 2.3));
        painter.image(drop, at, whole, Color32::WHITE);
    }
    vignette(painter, rect, 200.0);
}

/// A red CRT: heavy capitals stacked tight, bulging as if under the tube's
/// glass, with scanlines over everything. Every other block swaps to black
/// type on a red screen ringed like the tube's moire.
pub(super) fn draw_crt(painter: &Painter, s: &Scene) {
    let rect = s.rect;
    let scale = (rect.height() / 1080.0).max(0.5);
    let red = Color32::from_rgb(208, 24, 36);
    let block = current_block(s.lyrics, s.position, 9, 4);
    let swapped = block.as_ref().is_some_and(|b| b.3 % 2 == 1);
    let (paper, ink) = if swapped { (red, Color32::from_rgb(14, 8, 10)) } else { (Color32::from_rgb(8, 6, 8), red) };
    painter.rect_filled(rect, 0.0, paper);
    let center = rect.center();
    if swapped {
        let step = 9.0 * scale;
        let far = rect.size().length() / 2.0;
        let mut r = step;
        while r < far {
            painter.add(Shape::ellipse_stroke(
                center,
                Vec2::new(r * 1.15, r),
                Stroke::new(2.0 * scale, Color32::from_black_alpha(40)),
            ));
            r += step;
        }
    }

    if let Some((words, rows, current, _)) = block {
        let side = (rect.width() * 0.62).min(rect.height() * 0.85);
        let base = snap(side * 0.2, 4.0);
        let leading = 0.92;
        let height = base * leading * rows.len() as f32;
        let bulge = fisheye(center, Vec2::new(side * 0.75, height * 0.8), 0.9);
        let atlas = Vec2::from(painter.fonts(|f| f.font_image_size()).map(|v| v as f32));
        let mut mesh = Mesh::default();
        let mut ghost = Mesh::default();
        let mut top = center.y - height / 2.0;
        for row in rows {
            let text = words[row.clone()]
                .iter()
                .enumerate()
                .map(|(n, w)| if row.start + n <= current { w.text.to_uppercase() } else { String::new() })
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            // Laid out whole for its width, so the row stays put as words come.
            let whole = words[row.clone()].iter().map(|w| w.text.to_uppercase()).collect::<Vec<_>>().join(" ");
            let font = |px: f32| FontId::new(px, FontFamily::Name(HEAVY.into()));
            let mut px = base;
            let full = painter.layout_no_wrap(whole.clone(), font(px), ink);
            if full.size().x > side {
                px = snap(px * side / full.size().x, 2.0);
            }
            let full = painter.layout_no_wrap(whole, font(px), ink);
            let galley = painter.layout_no_wrap(text, font(px), ink);
            let at = Pos2::new(center.x - full.size().x / 2.0, top + (base * leading - full.size().y) / 2.0);
            top += base * leading;
            warped_mesh(&mut mesh, &galley, atlas, |v| bulge(at + v), ink, 4);
            if !swapped {
                let split = Vec2::new(3.0 * scale * (0.5 + s.beat), 0.0);
                warped_mesh(&mut ghost, &galley, atlas, |v| bulge(at + v + split), Color32::from_rgba_unmultiplied(255, 40, 40, 70), 4);
                warped_mesh(&mut ghost, &galley, atlas, |v| bulge(at + v - split), Color32::from_rgba_unmultiplied(40, 90, 255, 45), 4);
            }
        }
        painter.add(ghost);
        painter.add(mesh);
    }

    // Scanlines, and a brighter band rolling down.
    let pitch = 4.0 * scale;
    let mut lines = Mesh::default();
    let mut y = rect.top();
    while y < rect.bottom() {
        lines.add_colored_rect(
            Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), pitch * 0.45)),
            Color32::from_black_alpha(110),
        );
        y += pitch;
    }
    painter.add(lines);
    let roll = rect.top() + (s.time * 0.12).fract() * rect.height() * 1.4 - rect.height() * 0.2;
    let band = Rect::from_center_size(Pos2::new(center.x, roll), Vec2::new(rect.width(), rect.height() * 0.12));
    painter.rect_filled(band, 0.0, Color32::from_white_alpha(6));
    // The tube breathes with the beat.
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha((26.0 * (1.0 - s.beat)) as u8));
    vignette(painter, rect, 230.0);
}

/// One huge word at a time, dark on light gray, cut to the next the moment
/// it is sung.
pub(super) fn draw_giant(painter: &Painter, s: &Scene) {
    let rect = s.rect;
    painter.rect_filled(rect, 0.0, Color32::from_rgb(204, 204, 208));
    vignette(painter, rect, 90.0);
    let words = typing::song_words(s.lyrics, false);
    let Some(current) = words.partition_point(|w| w.at <= s.position).checked_sub(1) else {
        return;
    };
    let ink = Color32::from_rgb(22, 22, 24);
    let font = |px: f32| FontId::new(px, FontFamily::Name(BOLD.into()));
    let text = words[current].text.clone();
    let mut px = snap(rect.height() * 0.3, 8.0);
    let galley = painter.layout_no_wrap(text.clone(), font(px), ink);
    let room = rect.width() * 0.86;
    let galley = if galley.size().x > room {
        px = snap(px * room / galley.size().x, 8.0);
        painter.layout_no_wrap(text, font(px), ink)
    } else {
        galley
    };
    painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
}

/// The block on a white page beside a clear CD case, the cover printed on
/// the disc spinning inside it while the song plays.
pub(super) fn draw_disc(painter: &Painter, s: &Scene, size: f32) {
    use textures::{DISC_CENTER, DISC_RADIUS, PRINT_INNER, PRINT_OUTER};
    let rect = s.rect;
    let ctx = painter.ctx();
    painter.rect_filled(rect, 0.0, Color32::WHITE);
    let text = Rect::from_min_max(rect.min, Pos2::new(rect.left() + rect.width() * 0.55, rect.bottom()));
    typing::draw_block(painter, text, s.lyrics, s.position, rect.center().y, size, 1.0, 0.0);

    let side = (rect.height() * 0.55).min(rect.width() * 0.34);
    let case = Rect::from_center_size(Pos2::new(rect.left() + rect.width() * 0.73, rect.center().y), Vec2::splat(side));
    let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    // Lying on the page: a wide soft shadow and a tight one where it touches.
    for (offset, blur, alpha) in [(0.05, 0.16, 45), (0.012, 0.03, 60)] {
        let shadow = eframe::egui::Shadow {
            offset: [0, (side * offset).min(127.0) as i8],
            blur: (side * blur).min(255.0) as u8,
            spread: 0,
            color: Color32::from_black_alpha(alpha),
        };
        painter.add(shadow.as_shape(case, side * 0.01));
    }
    painter.image(textures::tray(ctx), case, whole, Color32::WHITE);

    // The disc, the cover printed on it turning: a third of a turn a second while it plays.
    let center = case.min + DISC_CENTER * side;
    let r = DISC_RADIUS * side;
    painter.circle_filled(center + Vec2::new(0.0, r * 0.025), r * 1.01, Color32::from_black_alpha(110));
    let turn = s.song_position.unwrap_or(s.position).as_secs_f32() * TAU * 0.35;
    const SIDES: u32 = 96;
    let mut print = match s.cover {
        Some(texture) => Mesh::with_texture(texture),
        None => Mesh::default(),
    };
    let ink = if s.cover.is_some() { Color32::WHITE } else { Color32::from_gray(176) };
    let hole = PRINT_INNER / PRINT_OUTER;
    for k in 0..=SIDES {
        let a = k as f32 / SIDES as f32 * TAU;
        let out = Rot2::from_angle(a + turn) * Vec2::new(r * PRINT_OUTER, 0.0);
        for (place, reach) in [(out, 0.5), (out * hole, 0.5 * hole)] {
            let uv = match s.cover {
                Some(_) => Pos2::new(0.5 + reach * a.cos(), 0.5 + reach * a.sin()),
                None => eframe::egui::epaint::WHITE_UV,
            };
            print.vertices.push(Vertex { pos: center + place, uv, color: ink });
        }
    }
    for k in 0..SIDES {
        let (a, b) = (2 * k, 2 * k + 2);
        print.add_triangle(a, b, a + 1);
        print.add_triangle(b, b + 1, a + 1);
    }
    painter.add(print);
    // The light on it stays put as it turns, and the lid closes over it all.
    painter.image(textures::sheen(ctx), Rect::from_center_size(center, Vec2::splat(r * 2.0)), whole, Color32::WHITE);
    painter.image(textures::lid(ctx), case, whole, Color32::WHITE);
}

/// The music video (or the cover's backdrop under a shade, without one) with
/// the line being sung in bold capitals over it and a player like Spotify's
/// below: title, artists, where the song is, and buttons that work. Returns
/// where each button is; `hover` lights the one under the mouse.
pub(super) fn draw_player(painter: &Painter, s: &Scene, hover: Option<Pos2>) -> Vec<(Rect, Action)> {
    let rect = s.rect;
    let u = (rect.height() / 720.0).clamp(0.6, 2.2);
    match s.clip {
        Some(clip) => draw_clip(painter, rect, clip),
        None => {
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(90));
        }
    }
    let white = Color32::WHITE;
    let center_x = rect.center().x;

    // The line, as it is sung.
    if let Some(i) = s.lyrics.line_at(s.position) {
        let line = s.lyrics.synced[i].text.trim().to_uppercase();
        let px = snap(rect.height() * 0.055, 2.0);
        let galley = painter.layout(line, FontId::new(px, FontFamily::Name(BOLD.into())), white, rect.width() * 0.8);
        let at = Pos2::new(center_x - galley.size().x / 2.0, rect.top() + rect.height() * 0.44 - galley.size().y / 2.0);
        painter.galley_with_override_text_color(at + Vec2::splat(px * 0.05), galley.clone(), Color32::from_black_alpha(170));
        painter.galley(at, galley, white);
    }

    let width = (rect.width() * 0.5).min(560.0 * u);
    let left = center_x - width / 2.0;
    let mut y = rect.top() + rect.height() * 0.66;
    let title = painter.text(
        Pos2::new(center_x, y),
        Align2::CENTER_TOP,
        &s.np.track.title,
        FontId::new(snap(26.0 * u, 2.0), FontFamily::Name(BOLD.into())),
        white,
    );
    y = title.bottom() + 4.0 * u;
    if let Some(artist) = &s.np.track.artist {
        let r = painter.text(
            Pos2::new(center_x, y),
            Align2::CENTER_TOP,
            artist,
            FontId::new(snap(12.0 * u, 2.0), FontFamily::Name(REGULAR.into())),
            Color32::from_white_alpha(170),
        );
        y = r.bottom();
    }

    // Where the song is.
    y += 22.0 * u;
    let at = s.song_position.unwrap_or_default();
    let length = s.np.duration.unwrap_or_default();
    let done = if length.is_zero() { 0.0 } else { (at.as_secs_f32() / length.as_secs_f32()).clamp(0.0, 1.0) };
    let bar = Rect::from_min_size(Pos2::new(left, y), Vec2::new(width, 3.0 * u));
    painter.rect_filled(bar, bar.height(), Color32::from_white_alpha(70));
    let mut filled = bar;
    filled.set_width(width * done);
    painter.rect_filled(filled, bar.height(), Color32::from_white_alpha(235));
    painter.circle_filled(Pos2::new(filled.right(), bar.center().y), 5.0 * u, white);
    let clock = |d: Duration| format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60);
    let small = FontId::new(snap(10.0 * u, 2.0), FontFamily::Name(REGULAR.into()));
    painter.text(Pos2::new(left, bar.bottom() + 6.0 * u), Align2::LEFT_TOP, clock(at), small.clone(), Color32::from_white_alpha(150));
    if !length.is_zero() {
        painter.text(Pos2::new(left + width, bar.bottom() + 6.0 * u), Align2::RIGHT_TOP, clock(length), small, Color32::from_white_alpha(150));
    }

    // The buttons.
    let y = bar.bottom() + 42.0 * u;
    let green = Color32::from_rgb(30, 215, 96);
    let shuffle = s.np.shuffle == Some(true);
    let repeat = s.np.repeat.unwrap_or(Repeat::Off);
    let playing = s.np.state == PlaybackState::Playing;
    let spots = [
        (Action::Shuffle, -0.4),
        (Action::Previous, -0.2),
        (Action::PlayPause, 0.0),
        (Action::Next, 0.2),
        (Action::Repeat, 0.4),
    ];
    let mut buttons = Vec::new();
    for (action, at) in spots {
        let c = Pos2::new(center_x + width * at, y);
        let hit = Rect::from_center_size(c, Vec2::splat(if action == Action::PlayPause { 58.0 } else { 40.0 } * u));
        let over = hover.is_some_and(|p| hit.contains(p));
        let ink = if over { white } else { Color32::from_white_alpha(215) };
        let k = 9.0 * u;
        let stroke = Stroke::new(1.8 * u, ink);
        match action {
            Action::PlayPause => {
                let r = 24.0 * u * if over { 1.06 } else { 1.0 };
                painter.circle_filled(c, r, white);
                let dark = Color32::from_gray(14);
                if playing {
                    for dx in [-0.45, 0.45] {
                        let bar = Rect::from_center_size(c + Vec2::new(dx * k, 0.0), Vec2::new(0.42 * k, 1.6 * k));
                        painter.rect_filled(bar, 1.0 * u, dark);
                    }
                } else {
                    let p = |x: f32, y: f32| c + Vec2::new(x, y) * k;
                    painter.add(Shape::convex_polygon(vec![p(-0.55, -0.85), p(0.95, 0.0), p(-0.55, 0.85)], dark, Stroke::NONE));
                }
            }
            Action::Previous | Action::Next => {
                let dir = if action == Action::Next { 1.0 } else { -1.0 };
                let p = |x: f32, y: f32| c + Vec2::new(x * dir, y) * k;
                painter.add(Shape::convex_polygon(vec![p(-0.7, -0.85), p(0.55, 0.0), p(-0.7, 0.85)], ink, Stroke::NONE));
                painter.rect_filled(Rect::from_two_pos(p(0.55, -0.85), p(0.85, 0.85)), 0.5 * u, ink);
            }
            Action::Shuffle => {
                let color = if shuffle { green } else { ink };
                let stroke = Stroke::new(stroke.width, color);
                let p = |x: f32, y: f32| c + Vec2::new(x, y) * k;
                painter.add(Shape::line(vec![p(-1.0, -0.6), p(-0.4, -0.6), p(0.4, 0.6), p(0.9, 0.6)], stroke));
                painter.add(Shape::line(vec![p(-1.0, 0.6), p(-0.4, 0.6), p(0.4, -0.6), p(0.9, -0.6)], stroke));
                for y in [-0.6, 0.6] {
                    painter.add(Shape::convex_polygon(vec![p(0.7, y - 0.3), p(1.1, y), p(0.7, y + 0.3)], color, Stroke::NONE));
                }
                if shuffle {
                    painter.circle_filled(c + Vec2::new(0.0, 1.6 * k), 1.8 * u, green);
                }
            }
            Action::Repeat => {
                let on = repeat != Repeat::Off;
                let color = if on { green } else { ink };
                let loop_ = Rect::from_center_size(c, Vec2::new(2.0 * k, 1.3 * k));
                painter.rect_stroke(loop_, 0.5 * k, Stroke::new(stroke.width, color), eframe::egui::StrokeKind::Middle);
                let tip = Pos2::new(loop_.center().x + 0.2 * k, loop_.top());
                painter.add(Shape::convex_polygon(
                    vec![tip + Vec2::new(-0.25, -0.35) * k, tip + Vec2::new(0.2, 0.0) * k, tip + Vec2::new(-0.25, 0.35) * k],
                    color,
                    Stroke::NONE,
                ));
                if repeat == Repeat::One {
                    painter.text(c, Align2::CENTER_CENTER, "1", FontId::new(snap(9.0 * u, 1.0), FontFamily::Name(BOLD.into())), color);
                }
                if on {
                    painter.circle_filled(c + Vec2::new(0.0, 1.6 * k), 1.8 * u, green);
                }
            }
        }
        buttons.push((hit, action));
    }
    buttons
}
