//! Turns downloaded image files into GPU-ready pictures and a color palette.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use image::{RgbaImage, imageops};

const COVER_MAX: u32 = 1600;
const PHOTO_MAX: u32 = 512;
/// The backdrop is shown heavily blurred, so a small image is plenty (and the
/// blur is cheap at this size).
const BACKDROP_MAX: u32 = 192;
/// Deep blue/violet, used until we know the track's colors.
pub const DEFAULT_PALETTE: [[f32; 3]; 3] =
    [[0.55, 0.35, 0.95], [0.18, 0.16, 0.35], [0.03, 0.03, 0.08]];

pub struct Picture {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

pub struct Art {
    /// Unique per build, so the UI and GPU know when to re-upload.
    pub id: u64,
    pub cover: Option<Arc<Picture>>,
    pub photo: Option<Arc<Picture>>,
    pub backdrop: Option<Arc<Picture>>,
    /// Vivid, mid, dark; sRGB 0..1.
    pub palette: [[f32; 3]; 3],
}

pub fn build(cover: Option<PathBuf>, photo: Option<PathBuf>, background: Option<PathBuf>) -> Art {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    // Decoding a large JPEG is the slow part; do the three side by side.
    let [cover, photo, background] = std::thread::scope(|s| {
        [cover, photo, background]
            .map(|p| s.spawn(|| p.and_then(decode)))
            .map(|handle| handle.join().ok().flatten())
    });

    // A real wide background beats a stretched square cover.
    let backdrop = background.as_ref().or(cover.as_ref()).or(photo.as_ref());
    let palette_from = cover.as_ref().or(photo.as_ref()).or(background.as_ref());
    Art {
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        backdrop: backdrop.map(|img| Arc::new(blurred(img))),
        palette: palette_from.map_or(DEFAULT_PALETTE, palette),
        cover: cover.map(|img| Arc::new(fit(&img, COVER_MAX))),
        photo: photo.map(|img| Arc::new(fit(&img, PHOTO_MAX))),
    }
}

fn decode(path: PathBuf) -> Option<RgbaImage> {
    // Cached files have no extension; sniff the format from the bytes.
    let reader = image::ImageReader::open(&path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    match reader.decode() {
        Ok(img) => Some(img.to_rgba8()),
        Err(e) => {
            eprintln!("cannot decode {}: {e}", path.display());
            None
        }
    }
}

fn to_picture(img: &RgbaImage) -> Picture {
    Picture {
        size: [img.width() as usize, img.height() as usize],
        rgba: img.as_raw().clone(),
    }
}

fn scaled_size(img: &RgbaImage, max: u32) -> (u32, u32) {
    let (w, h) = img.dimensions();
    let scale = (max as f32 / w.max(h) as f32).min(1.0);
    (
        ((w as f32 * scale) as u32).max(1),
        ((h as f32 * scale) as u32).max(1),
    )
}

fn fit(img: &RgbaImage, max: u32) -> Picture {
    let (w, h) = scaled_size(img, max);
    if (w, h) == img.dimensions() {
        return to_picture(img);
    }
    to_picture(&imageops::resize(img, w, h, imageops::FilterType::Triangle))
}

fn blurred(img: &RgbaImage) -> Picture {
    let (w, h) = scaled_size(img, BACKDROP_MAX);
    let small = imageops::thumbnail(img, w, h);
    to_picture(&imageops::blur(&small, 3.0))
}

/// Picks a vivid accent, an average tone and a dark base from the image.
fn palette(img: &RgbaImage) -> [[f32; 3]; 3] {
    let small = imageops::thumbnail(img, 24, 24);
    let pixels: Vec<[f32; 3]> = small
        .pixels()
        .map(|p| [p[0], p[1], p[2]].map(|c| f32::from(c) / 255.0))
        .collect();
    if pixels.is_empty() {
        return DEFAULT_PALETTE;
    }

    let vividness = |c: &[f32; 3]| {
        let max = c[0].max(c[1]).max(c[2]);
        let min = c[0].min(c[1]).min(c[2]);
        max - min // saturation × value
    };
    let luma = |c: &[f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];

    let mut by_vivid = pixels.clone();
    by_vivid.sort_by(|a, b| vividness(b).total_cmp(&vividness(a)));
    let mut by_luma = pixels.clone();
    by_luma.sort_by(|a, b| luma(a).total_cmp(&luma(b)));

    let vivid = average(&by_vivid[..by_vivid.len().div_ceil(7)]);
    let dark = average(&by_luma[..by_luma.len().div_ceil(3)]);
    let mid = average(&pixels);

    // Push the extremes apart so the background has some contrast to work with.
    let vivid_peak = vivid[0].max(vivid[1]).max(vivid[2]).max(0.01);
    let vivid = vivid.map(|c| (c / vivid_peak * 0.9).min(1.0));
    let dark = dark.map(|c| c * 0.5);
    [vivid, mid, dark]
}

fn average(pixels: &[[f32; 3]]) -> [f32; 3] {
    let sum = pixels.iter().fold([0.0; 3], |acc, p| {
        [acc[0] + p[0], acc[1] + p[1], acc[2] + p[2]]
    });
    sum.map(|c| c / pixels.len().max(1) as f32)
}
