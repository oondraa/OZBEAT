//! The share card (Ctrl+K): the line being sung, big under the song's cover
//! on its blurred colors, as a 1080×1920 picture for stories. Saved under
//! Pictures\OZBEAT and put on the clipboard, drawn on a thread of its own so
//! the visuals don't stutter meanwhile.

use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use eframe::egui::{
    self, Align, Color32, FontFamily, FontId, Galley, Mesh, Pos2, Rect, Vec2, text::LayoutJob,
};
use image::{Rgba, RgbaImage, imageops};

use crate::art::Picture;
use crate::ui::{BOLD, HEAVY, REGULAR, install_fonts};

const WIDTH: u32 = 1080;
const HEIGHT: u32 = 1920;
const MARGIN: f32 = 90.0;
/// Room kept free at the top, and the OZBEAT sign's line at the bottom.
const TOP: f32 = 130.0;
const FOOTER: f32 = 1800.0;
const LOGO: f32 = 60.0;

pub struct Card {
    pub title: String,
    pub artist: Option<String>,
    /// The line being sung, if any.
    pub line: Option<String>,
    pub cover: Option<Arc<Picture>>,
    pub backdrop: Option<Arc<Picture>>,
    /// Vivid, mid, dark (see `Art::palette`).
    pub palette: [[f32; 3]; 3],
}

/// Where the card went, and whether it is on the clipboard too.
pub type Made = Result<(PathBuf, bool), String>;

/// Draws, saves and copies `card` in the background.
pub fn spawn(card: Card) -> mpsc::Receiver<Made> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let image = draw(&card);
        let made = save(&image, &card).map(|path| (path, copy(&image)));
        let _ = tx.send(made);
    });
    rx
}

/// Where the pictures go, decided while laying out the text.
struct Placement {
    cover: Rect,
    logo: Pos2,
}

pub(crate) fn draw(card: &Card) -> RgbaImage {
    let (meshes, atlas, place) = text_layer(card);
    let mut image = backdrop(card);
    if let Some(cover) = card.cover.as_deref().and_then(to_image) {
        paste_cover(&mut image, &cover, place.cover);
    }
    if let Ok(logo) = image::load_from_memory(include_bytes!("../../assety/OZBEAT3logo.png")) {
        let side = LOGO as u32;
        let logo = imageops::resize(&logo.to_rgba8(), side, side, imageops::FilterType::Lanczos3);
        imageops::overlay(&mut image, &logo, place.logo.x as i64, place.logo.y as i64);
    }
    for mesh in &meshes {
        rasterize(&mut image, mesh, &atlas);
    }
    image
}

/// The cover's colors filling the card, blurred beyond any detail and darker
/// toward the bottom, where the lyrics sit. Without a cover, its accent.
fn backdrop(card: &Card) -> RgbaImage {
    let [vivid, _, dark] = card.palette;
    let source = card.cover.as_deref().or(card.backdrop.as_deref()).and_then(to_image);
    let mut image = match source {
        Some(source) => {
            let small = fill(&source, WIDTH / 10, HEIGHT / 10, imageops::FilterType::Triangle);
            let soft = imageops::blur(&small, 6.0);
            imageops::resize(&soft, WIDTH, HEIGHT, imageops::FilterType::Triangle)
        }
        None => RgbaImage::from_pixel(WIDTH, HEIGHT, to_pixel(vivid.map(|c| c * 0.5))),
    };
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let t = y as f32 / HEIGHT as f32;
        let shade = 0.3 + 0.45 * t * t * (3.0 - 2.0 * t);
        // A whisper of noise, so the smooth gradient doesn't band.
        let noise = (hash(x, y) - 0.5) * 1.5 / 255.0;
        let rgb: [f32; 3] = std::array::from_fn(|k| {
            let c = f32::from(pixel[k]) / 255.0 * 0.62;
            c + (dark[k] - c) * shade + noise
        });
        *pixel = to_pixel(rgb);
    }
    image
}

/// The cover with rounded corners, over its soft shadow.
fn paste_cover(image: &mut RgbaImage, cover: &RgbaImage, rect: Rect) {
    let side = rect.width().round() as u32;
    let radius = side as f32 * 0.035;
    let (x, y) = (rect.left().round() as i64, rect.top().round() as i64);

    let pad = 90;
    let mut shadow = RgbaImage::new(side + 2 * pad, side + 2 * pad);
    for (sx, sy, pixel) in shadow.enumerate_pixels_mut() {
        let inside = rounded(sx as f32 - pad as f32, sy as f32 - pad as f32 - 26.0, side as f32, radius);
        *pixel = Rgba([0, 0, 0, (inside * 150.0) as u8]);
    }
    let shadow = imageops::fast_blur(&shadow, 34.0);
    imageops::overlay(image, &shadow, x - pad as i64, y - pad as i64);

    let mut cover = fill(cover, side, side, imageops::FilterType::Lanczos3);
    for (cx, cy, pixel) in cover.enumerate_pixels_mut() {
        pixel[3] = (rounded(cx as f32, cy as f32, side as f32, radius) * 255.0) as u8;
    }
    imageops::overlay(image, &cover, x, y);
}

/// How much of the pixel at `x`, `y` lies inside a square of `size` with
/// corners rounded by `radius`.
fn rounded(x: f32, y: f32, size: f32, radius: f32) -> f32 {
    let half = size / 2.0;
    let q = Vec2::new((x + 0.5 - half).abs(), (y + 0.5 - half).abs()) - Vec2::splat(half - radius);
    let outside = q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - radius;
    (0.5 - outside).clamp(0.0, 1.0)
}

/// `image` scaled to cover `w`×`h` and cut to it around the middle.
fn fill(image: &RgbaImage, w: u32, h: u32, filter: imageops::FilterType) -> RgbaImage {
    let scale = (w as f32 / image.width() as f32).max(h as f32 / image.height() as f32);
    let sw = ((image.width() as f32 * scale).ceil() as u32).max(w);
    let sh = ((image.height() as f32 * scale).ceil() as u32).max(h);
    let scaled = imageops::resize(image, sw, sh, filter);
    imageops::crop_imm(&scaled, (sw - w) / 2, (sh - h) / 2, w, h).to_image()
}

fn to_image(picture: &Picture) -> Option<RgbaImage> {
    let [w, h] = picture.size;
    RgbaImage::from_raw(w as u32, h as u32, picture.rgba.clone())
}

fn to_pixel(rgb: [f32; 3]) -> Rgba<u8> {
    let [r, g, b] = rgb.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8);
    Rgba([r, g, b, 255])
}

/// 0..1, the same for the same pixel.
fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Titles, the line and the OZBEAT name, laid out and tessellated by egui
/// with the app's own fonts, for [`rasterize`].
fn text_layer(card: &Card) -> (Vec<Mesh>, Atlas, Placement) {
    let ctx = egui::Context::default();
    install_fonts(&ctx);
    let input = || egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(WIDTH as f32, HEIGHT as f32))),
        ..Default::default()
    };
    let mut atlas = Atlas::default();
    let mut place = None;
    let mut shapes = Vec::new();
    let mut pixels_per_point = 1.0;
    // The first run loads the fonts.
    for _ in 0..2 {
        let out = ctx.run_ui(input(), |ui| place = Some(lay_out(ui.painter(), card)));
        for (id, delta) in &out.textures_delta.set {
            atlas.apply(*id, delta);
        }
        shapes = out.shapes;
        pixels_per_point = out.pixels_per_point;
    }
    let meshes = ctx
        .tessellate(shapes, pixels_per_point)
        .into_iter()
        .filter_map(|p| match p.primitive {
            egui::epaint::Primitive::Mesh(mesh) => Some(mesh),
            egui::epaint::Primitive::Callback(_) => None,
        })
        .collect();
    (meshes, atlas, place.expect("laid out"))
}

fn lay_out(painter: &egui::Painter, card: &Card) -> Placement {
    let width = WIDTH as f32 - 2.0 * MARGIN;
    let center = WIDTH as f32 / 2.0;
    let family = |name: &str| FontFamily::Name(name.into());
    let centered = |text: &str, font: FontId, color: Color32, rows: usize| {
        let mut job = LayoutJob::simple(text.to_owned(), font, color, width);
        job.halign = Align::Center;
        job.wrap.max_rows = rows;
        painter.layout_job(job)
    };

    let title = centered(&card.title, FontId::new(56.0, family(BOLD)), Color32::WHITE, 2);
    let artist = card.artist.as_deref().map(|a| {
        centered(a, FontId::new(38.0, family(REGULAR)), Color32::from_white_alpha(200), 1)
    });
    // As big as fits in four rows.
    let lyric = card.line.as_deref().map(|line| {
        let mut size = 104.0;
        loop {
            let galley = centered(line, FontId::new(size, family(HEAVY)), Color32::WHITE, 6);
            if galley.rows.len() <= 4 || size <= 56.0 {
                break galley;
            }
            size -= 8.0;
        }
    });

    let (cover_gap, artist_gap, lyric_gap) = (64.0, 16.0, 96.0);
    let text = title.size().y
        + artist.as_ref().map_or(0.0, |a| artist_gap + a.size().y)
        + lyric.as_ref().map_or(0.0, |l| lyric_gap + l.size().y);
    let room = FOOTER - 90.0 - TOP;
    let wanted: f32 = if lyric.is_some() { 640.0 } else { 820.0 };
    let side = wanted.min(room - cover_gap - text).max(360.0).round();
    // A little above the middle looks centered.
    let mut y = TOP + ((room - side - cover_gap - text) * 0.45).max(0.0);
    let cover = Rect::from_min_size(Pos2::new((center - side / 2.0).round(), y.round()), Vec2::splat(side));
    y += side + cover_gap;

    let put = |galley: Arc<Galley>, y: &mut f32| {
        let height = galley.size().y;
        let pos = Pos2::new(center - galley.rect.center().x, *y - galley.rect.top()).round();
        painter.galley(pos, galley, Color32::WHITE);
        *y += height;
    };
    put(title, &mut y);
    if let Some(artist) = artist {
        y += artist_gap;
        put(artist, &mut y);
    }
    if let Some(lyric) = lyric {
        // A short bar in the cover's color between the song and its line.
        let bar = Rect::from_center_size(Pos2::new(center, y + lyric_gap / 2.0), Vec2::new(56.0, 6.0));
        painter.rect_filled(bar, 3.0, crate::ui::color(card.palette[0]));
        y += lyric_gap;
        put(lyric, &mut y);
    }

    let name = painter.layout_no_wrap(
        "OZBEAT".to_owned(),
        FontId::new(34.0, family(BOLD)),
        Color32::from_white_alpha(235),
    );
    let gap = 18.0;
    let left = (center - (LOGO + gap + name.size().x) / 2.0).round();
    painter.galley(
        Pos2::new(left + LOGO + gap, FOOTER - name.size().y / 2.0).round(),
        name,
        Color32::WHITE,
    );
    Placement {
        cover,
        logo: Pos2::new(left, FOOTER - LOGO / 2.0).round(),
    }
}

fn save(image: &RgbaImage, card: &Card) -> Result<PathBuf, String> {
    let dir = dirs::picture_dir()
        .or_else(dirs::home_dir)
        .ok_or("Nemám kam kartu uložit")?
        .join("OZBEAT");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = match &card.artist {
        Some(artist) => format!("{artist} - {}", card.title),
        None => card.title.clone(),
    };
    let name: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .take(100)
        .collect();
    let name = name.trim().trim_end_matches('.');
    let mut path = dir.join(format!("{name}.png"));
    for n in 2.. {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("{name} ({n}).png"));
    }
    image::DynamicImage::ImageRgba8(image.clone())
        .into_rgb8()
        .save(&path)
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// Puts `image` on the clipboard, as PNG for apps that take it (browsers,
/// Discord) and as a bitmap for the rest. Whether it worked.
#[cfg(windows)]
fn copy(image: &RgbaImage) -> bool {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
    use windows_sys::Win32::System::Ole::CF_DIB;

    let mut png = Vec::new();
    if image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .is_err()
    {
        return false;
    }
    // A bottom-up 32-bit bitmap after its 40-byte header.
    let (w, h) = image.dimensions();
    let mut dib = Vec::with_capacity(40 + (w * h * 4) as usize);
    dib.extend(40u32.to_le_bytes());
    dib.extend((w as i32).to_le_bytes());
    dib.extend((h as i32).to_le_bytes());
    dib.extend(1u16.to_le_bytes());
    dib.extend(32u16.to_le_bytes());
    dib.extend(0u32.to_le_bytes()); // BI_RGB
    dib.extend((w * h * 4).to_le_bytes());
    dib.extend([0u8; 16]);
    for y in (0..h).rev() {
        for x in 0..w {
            let [r, g, b, _] = image.get_pixel(x, y).0;
            dib.extend([b, g, r, 255]);
        }
    }

    /// Hands a copy of `bytes` to the clipboard, which owns it from then on.
    unsafe fn put(format: u32, bytes: &[u8]) -> bool {
        // SAFETY: the memory is ours until SetClipboardData takes it.
        unsafe {
            let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
            if memory.is_null() {
                return false;
            }
            let target = GlobalLock(memory);
            if target.is_null() {
                GlobalFree(memory);
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
            GlobalUnlock(memory);
            if SetClipboardData(format, memory).is_null() {
                GlobalFree(memory);
                return false;
            }
            true
        }
    }

    // Owned by the main window: with no owner, emptying the clipboard
    // would make SetClipboardData fail.
    let owner = crate::frameless::main_window();
    // SAFETY: plain clipboard calls; it is closed again on every path.
    unsafe {
        // Another app may hold the clipboard for a moment.
        let mut open = false;
        for _ in 0..10 {
            if OpenClipboard(owner) != 0 {
                open = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        if !open {
            return false;
        }
        EmptyClipboard();
        let name: Vec<u16> = "PNG".encode_utf16().chain([0]).collect();
        let png_format = RegisterClipboardFormatW(name.as_ptr());
        let copied = put(CF_DIB as u32, &dib);
        if png_format != 0 {
            put(png_format, &png);
        }
        CloseClipboard();
        copied
    }
}

#[cfg(not(windows))]
fn copy(_image: &RgbaImage) -> bool {
    false
}

/// The textures egui has sent, by id: the font atlas, images.
#[derive(Default)]
pub(crate) struct Atlas(std::collections::HashMap<egui::TextureId, ColorImageBuf>);

impl Atlas {
    pub(crate) fn apply(&mut self, id: egui::TextureId, delta: &egui::epaint::ImageDelta) {
        self.0.entry(id).or_default().apply(delta);
    }
}

#[derive(Default)]
struct ColorImageBuf {
    size: [usize; 2],
    pixels: Vec<Color32>,
}

impl ColorImageBuf {
    fn apply(&mut self, delta: &egui::epaint::ImageDelta) {
        let egui::ImageData::Color(image) = &delta.image;
        let [x0, y0] = delta.pos.unwrap_or([0, 0]);
        if delta.pos.is_none() {
            self.size = image.size;
            self.pixels = vec![Color32::TRANSPARENT; image.size[0] * image.size[1]];
        }
        for y in 0..image.size[1] {
            for x in 0..image.size[0] {
                self.pixels[(y0 + y) * self.size[0] + x0 + x] = image.pixels[y * image.size[0] + x];
            }
        }
    }

    fn sample(&self, uv: Pos2) -> [f32; 4] {
        let x = ((uv.x * self.size[0] as f32) as usize).min(self.size[0] - 1);
        let y = ((uv.y * self.size[1] as f32) as usize).min(self.size[1] - 1);
        self.pixels[y * self.size[0] + x].to_array().map(|c| c as f32 / 255.0)
    }
}

/// Premultiplied triangles over `image`, without antialiasing beyond egui's own feathering.
pub(crate) fn rasterize(image: &mut RgbaImage, mesh: &Mesh, atlas: &Atlas) {
    let Some(atlas) = atlas.0.get(&mesh.texture_id) else {
        return;
    };
    for t in mesh.indices.chunks_exact(3) {
        let v = [0, 1, 2].map(|k| mesh.vertices[t[k] as usize]);
        let (a, b, c) = (v[0].pos, v[1].pos, v[2].pos);
        let area = (b - a).x * (c - a).y - (b - a).y * (c - a).x;
        if area.abs() < 1e-6 {
            continue;
        }
        let min = a.min(b).min(c);
        let max = a.max(b).max(c);
        let x_range = (min.x.floor().max(0.0) as u32)..(max.x.ceil().min(image.width() as f32) as u32);
        for y in (min.y.floor().max(0.0) as u32)..(max.y.ceil().min(image.height() as f32) as u32) {
            for x in x_range.clone() {
                let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b - p).x * (c - p).y - (b - p).y * (c - p).x) / area;
                let w1 = ((c - p).x * (a - p).y - (c - p).y * (a - p).x) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let uv = Pos2::new(
                    w0 * v[0].uv.x + w1 * v[1].uv.x + w2 * v[2].uv.x,
                    w0 * v[0].uv.y + w1 * v[1].uv.y + w2 * v[2].uv.y,
                );
                let tex = atlas.sample(uv);
                let colors = v.map(|v| v.color.to_array().map(|c| c as f32 / 255.0));
                let src: [f32; 4] = std::array::from_fn(|k| {
                    (w0 * colors[0][k] + w1 * colors[1][k] + w2 * colors[2][k]) * tex[k]
                });
                let dst = image.get_pixel_mut(x, y);
                for k in 0..4 {
                    let d = dst.0[k] as f32 / 255.0;
                    dst.0[k] = ((src[k] + d * (1.0 - src[3])) * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A card with a long line, a short one and none, to look at:
    /// `MV_TEST_WORDS=dir cargo test -p musicvisual draws_card -- --ignored`.
    #[test]
    #[ignore]
    fn draws_card() {
        let dir = PathBuf::from(std::env::var_os("MV_TEST_WORDS").expect("MV_TEST_WORDS"));
        let cover = std::env::var_os("MV_TEST_COVER")
            .and_then(|p| image::ImageReader::open(p).ok()?.with_guessed_format().ok()?.decode().ok())
            .map(|img| {
                let img = img.to_rgba8();
                Arc::new(Picture {
                    size: [img.width() as usize, img.height() as usize],
                    rgba: img.into_raw(),
                })
            });
        let lines = [
            Some("Na rohu stojím a srdce mi hoří, peníze v kapse a město nespí"),
            Some("Celou noc jedu městem"),
            None,
        ];
        for (n, line) in lines.into_iter().enumerate() {
            let card = Card {
                title: "El Camino".into(),
                artist: Some("Yzomandias".into()),
                line: line.map(str::to_owned),
                cover: cover.clone(),
                backdrop: None,
                palette: [[0.95, 0.45, 0.35], [0.4, 0.25, 0.3], [0.06, 0.04, 0.08]],
            };
            draw(&card).save(dir.join(format!("card{n}.png"))).unwrap();
        }
    }
}
