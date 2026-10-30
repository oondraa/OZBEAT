//! Pictures the whole-screen styles are drawn with, worked out pixel by pixel
//! once, the first time they're needed: rain on a window and a CD in its
//! case. Shaded like the real things rather than drawn in flat shapes.

use std::f32::consts::TAU;

use eframe::egui::{self, ColorImage, TextureHandle, TextureId, TextureOptions, Vec2};

use super::hash;

/// The texture `name`, made by `make` the first time it's asked for.
fn cached(ctx: &egui::Context, name: &'static str, options: TextureOptions, make: fn() -> Canvas) -> TextureId {
    let id = egui::Id::new(("ozbeat-screen-texture", name));
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return texture.id();
    }
    let texture = ctx.load_texture(name, make().image(), options);
    let tid = texture.id();
    ctx.data_mut(|d| d.insert_temp(id, texture));
    tid
}

/// Drops on a window, as a tile that repeats.
pub(super) fn rain(ctx: &egui::Context) -> TextureId {
    cached(ctx, "rain", TextureOptions::LINEAR_REPEAT, make_rain)
}

/// One big drop, to run down the glass.
pub(super) fn drop(ctx: &egui::Context) -> TextureId {
    cached(ctx, "drop", TextureOptions::LINEAR, || {
        let mut c = Canvas::new(64, 64);
        c.drop(32.0, 34.0, 26.0, 28.0, false);
        c
    })
}

/// The CD case seen from the front, without its disc: the frame, the hinge
/// and the black tray with its hub. Opaque.
pub(super) fn tray(ctx: &egui::Context) -> TextureId {
    cached(ctx, "tray", TextureOptions::LINEAR, make_tray)
}

/// The case's clear lid, to lay over the disc: bevels and glare.
pub(super) fn lid(ctx: &egui::Context) -> TextureId {
    cached(ctx, "lid", TextureOptions::LINEAR, make_lid)
}

/// The light on a disc, to lay over its print: the silver rim, the clear
/// ring inside with the hole, and rainbows where the light catches.
pub(super) fn sheen(ctx: &egui::Context) -> TextureId {
    cached(ctx, "sheen", TextureOptions::LINEAR, make_sheen)
}

/// Where the hub and the disc sit in the case, against its side.
pub(super) const DISC_CENTER: Vec2 = Vec2::new(0.52, 0.5);
pub(super) const DISC_RADIUS: f32 = 0.418;
/// The print on the disc, against its radius: inside it is clear plastic.
pub(super) const PRINT_INNER: f32 = 0.33;
pub(super) const PRINT_OUTER: f32 = 0.975;

fn smooth(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Premultiplied RGBA in floats, painted over and over.
struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(width: usize, height: usize) -> Self {
        Self { width, height, pixels: vec![[0.0; 4]; width * height] }
    }

    /// `rgb` at `alpha` over what's at (`x`, `y`), wrapping round the edges if `wrap`.
    fn over(&mut self, x: i32, y: i32, rgb: [f32; 3], alpha: f32, wrap: bool) {
        let (w, h) = (self.width as i32, self.height as i32);
        let (x, y) = if wrap { (x.rem_euclid(w), y.rem_euclid(h)) } else { (x, y) };
        if alpha <= 0.0 || x < 0 || y < 0 || x >= w || y >= h {
            return;
        }
        let p = &mut self.pixels[(y * w + x) as usize];
        let a = alpha.min(1.0);
        for k in 0..3 {
            p[k] = rgb[k] * a + p[k] * (1.0 - a);
        }
        p[3] = a + p[3] * (1.0 - a);
    }

    /// A drop of water at (`cx`, `cy`), `rx` by `ry` pixels: a lens that
    /// brightens what's behind it, dark at the rim (most at the bottom),
    /// with the light bent into a crescent low inside and a highlight up left.
    fn drop(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, wrap: bool) {
        let (x0, x1) = ((cx - rx - 1.0).floor() as i32, (cx + rx + 1.0).ceil() as i32);
        let (y0, y1) = ((cy - ry - 1.0).floor() as i32, (cy + ry + 1.0).ceil() as i32);
        // Antialiasing over about a pixel, whatever the drop's size.
        let soft = (1.5 / rx.min(ry)).clamp(0.02, 0.5);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let n = Vec2::new((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - cy) / ry);
                let d = n.length();
                if d >= 1.0 {
                    continue;
                }
                let inside = 1.0 - smooth(1.0 - soft, 1.0, d);
                self.over(x, y, [1.0; 3], 0.05 * inside, wrap);
                let rim = smooth(0.5, 1.0, d) * (0.5 + 0.5 * (0.35 * n.x + 0.9 * n.y).max(0.0));
                self.over(x, y, [0.0; 3], 0.6 * rim * inside, wrap);
                let crescent = smooth(0.45, 0.75, d) * (1.0 - smooth(0.78, 0.95, d)) * n.y.max(0.0);
                self.over(x, y, [1.0; 3], 0.35 * crescent * inside, wrap);
                let h = (n - Vec2::new(-0.33, -0.4)).length();
                let spec = (1.0 - h / 0.3).max(0.0).powi(2);
                self.over(x, y, [1.0; 3], 0.85 * spec * inside, wrap);
            }
        }
    }

    fn image(&self) -> ColorImage {
        let bytes: Vec<u8> = self
            .pixels
            .iter()
            .flat_map(|p| p.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
            .collect();
        ColorImage::from_rgba_premultiplied([self.width, self.height], &bytes)
    }
}

fn make_rain() -> Canvas {
    const N: usize = 768;
    let mut c = Canvas::new(N, N);
    let n = N as f32;
    // Mist, small drops, some bigger, and a few big ones sagging under their weight.
    let kinds: [(u32, f32, f32, f32); 4] = [(2600, 0.6, 1.3, 1.0), (700, 1.3, 3.2, 1.05), (170, 3.2, 7.0, 1.12), (28, 7.0, 13.0, 1.35)];
    let mut seed = 50_000;
    for (count, small, big, sag) in kinds {
        for _ in 0..count {
            let (x, y) = (hash(seed) * n, hash(seed + 1) * n);
            let r = small + (big - small) * hash(seed + 2).powi(2);
            let rx = r * (0.9 + 0.2 * hash(seed + 3));
            let ry = r * (sag + 0.2 * hash(seed + 4) * (sag - 1.0));
            c.drop(x, y, rx, ry, true);
            seed += 5;
        }
    }
    c
}

fn make_tray() -> Canvas {
    const N: usize = 512;
    let mut c = Canvas::new(N, N);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = ((x as f32 + 0.5) / N as f32, (y as f32 + 0.5) / N as f32);
            let edge = u.min(v).min(1.0 - u).min(1.0 - v);
            let grain = (hash((y * N + x) as u32) - 0.5) * 0.012;
            let gray = if edge < 0.035 {
                // The frame's plastic, lit from the top left along its bevel.
                let lit = if u < 0.5 && u <= v || v < 0.5 && v < u { 0.04 } else { -0.03 };
                0.8 + 0.12 * (edge / 0.035) + lit
            } else if u < 0.075 {
                // The hinge, ribbed.
                let rib = if (v * 26.0).fract() < 0.14 { -0.07 } else { 0.0 };
                0.76 + 0.08 * ((u - 0.035) / 0.04) + rib
            } else {
                let p = Vec2::new(u, v) - DISC_CENTER;
                let r = p.length();
                // Glossy black, lighter toward the top.
                let mut g = 0.06 + 0.05 * (1.0 - v);
                // The recess the disc lies in, with its lip.
                if (0.43..0.437).contains(&r) {
                    g += 0.07;
                } else if (0.437..0.45).contains(&r) {
                    g -= 0.025;
                }
                // Finger cuts at the sides of the tray.
                for side in [0.075, 0.965] {
                    let f = (Vec2::new(u, v) - Vec2::new(side, 0.5)).length();
                    if f < 0.06 {
                        g = 0.035 + if f > 0.054 { 0.05 } else { 0.0 };
                    }
                }
                // The hub that holds the disc: petals round a hole.
                if r < 0.062 {
                    let petal = ((p.y.atan2(p.x) / TAU) * 12.0).rem_euclid(1.0);
                    g = if r < 0.016 {
                        0.03
                    } else if petal < 0.15 && r > 0.026 {
                        0.025
                    } else {
                        0.2 + 0.18 * (1.0 - r / 0.062)
                    };
                    if r > 0.056 {
                        g = 0.3;
                    }
                } else if (0.075..0.09).contains(&r) {
                    g += 0.025 * (1.0 - ((r - 0.0825) / 0.0075).abs());
                }
                g
            };
            let g = gray + grain;
            c.over(x as i32, y as i32, [g; 3], 1.0, false);
        }
    }
    c
}

fn make_lid() -> Canvas {
    const N: usize = 512;
    let mut c = Canvas::new(N, N);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = ((x as f32 + 0.5) / N as f32, (y as f32 + 0.5) / N as f32);
            let edge = u.min(v).min(1.0 - u).min(1.0 - v);
            let mut a: f32 = 0.035;
            if edge < 0.035 {
                // The lid's bevel, catching more light at the top and left.
                let lit = if u < 0.5 && u <= v || v < 0.5 && v < u { 0.15 } else { 0.0 };
                a = 0.22 + 0.3 * (1.0 - edge / 0.035) + lit;
            } else if edge < 0.041 {
                a = 0.32;
            } else if u < 0.075 {
                a = 0.1 + if (v * 26.0).fract() < 0.14 { 0.08 } else { 0.0 };
            } else {
                let diagonal = u + v;
                a += 0.2 * (-((diagonal - 0.6) / 0.1).powi(2)).exp();
                a += 0.14 * (-((diagonal - 1.32) / 0.022).powi(2)).exp();
                // The tabs that hold the booklet.
                let tab = |top: f32| (0.925..0.952).contains(&u) && (top..top + 0.08).contains(&v);
                if tab(0.12) || tab(0.8) {
                    a = 0.22;
                }
            }
            c.over(x as i32, y as i32, [1.0; 3], a, false);
        }
    }
    c
}

fn make_sheen() -> Canvas {
    const N: usize = 512;
    let mut c = Canvas::new(N, N);
    let hue = |h: f32| {
        let h = h.rem_euclid(1.0) * 6.0;
        let f = |k: f32| ((h + k).rem_euclid(6.0) - 3.0).abs() - 1.0;
        [f(0.0), f(4.0), f(2.0)].map(|v| 0.45 + 0.55 * v.clamp(0.0, 1.0))
    };
    for y in 0..N {
        for x in 0..N {
            let p = Vec2::new((x as f32 + 0.5) / N as f32 - 0.5, (y as f32 + 0.5) / N as f32 - 0.5) * 2.0;
            let r = p.length();
            if r >= 1.0 {
                continue;
            }
            let (x, y) = (x as i32, y as i32);
            let inside = 1.0 - smooth(0.993, 1.0, r);
            let angle = p.y.atan2(p.x);
            if r > PRINT_OUTER {
                c.over(x, y, [0.86; 3], 0.9 * inside, false);
            } else if r > PRINT_INNER {
                // Rainbows in two opposite lobes, where the light meets the tracks.
                let lobe = (2.0 * angle + 0.9).cos().max(0.0).powi(5);
                c.over(x, y, hue(r * 4.0 + angle / TAU), 0.24 * lobe * (0.35 + 0.65 * r), false);
                c.over(x, y, [1.0; 3], 0.04 + 0.05 * (1.0 - r), false);
            } else if r > 0.3 {
                c.over(x, y, [0.8; 3], 0.55, false);
            } else if r > 0.13 {
                // Clear plastic with the stacking rings, the hub showing through.
                let ring = |at: f32| (1.0 - ((r - at) / 0.006).abs()).max(0.0);
                c.over(x, y, [0.88; 3], 0.28 + 0.4 * (ring(0.2) + ring(0.27)), false);
            } else if r > 0.12 {
                c.over(x, y, [0.9; 3], 0.5, false);
            }
        }
    }
    c
}
