//! The setup panel: shown on first run, and any time later with Ctrl+S.
//! Every change is saved immediately.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, Layout, Pos2, Rect, RichText, TextureHandle, Vec2,
    emath::TSTransform, load::SizedTexture,
};
use mv_audio::{Capture, Input};
use mv_core::{Device, PlaybackState};

use crate::anim::{ease_in_cubic, ease_out_cubic, glow, remap, spring};
use crate::settings::{AudioMode, Settings, SharedSettings};
use crate::update::{self, SharedUpdate, Status};

/// The dot in the OZBEAT logo.
pub const BRAND_BLUE: Color32 = Color32::from_rgb(0x2D, 0x8C, 0xFF);
const WIDTH: f32 = 540.0;
/// Corner radius of the glass panel.
const RADIUS: f32 = 28.0;
/// Dark text on the white glass buttons.
const INK: Color32 = Color32::from_rgb(0x12, 0x12, 0x16);
/// The wordmark inside the 1200x600 logo card (UV space).
const LOGO_UV: Rect = Rect {
    min: Pos2::new(0.2, 0.36),
    max: Pos2::new(0.78, 0.64),
};

/// Draws the panel. Returns `false` once the user confirms with "Hotovo".
/// The panel is frosted glass: a light, see-through pane over the blurred
/// background (the visuals behind are dimmed while it is open, see `ui`),
/// with a sheen from the top, a bright rim and a soft shadow.
///
/// `progress` (0 -> 1) animates it: opening, it grows from small on a spring
/// (like a window on a Mac), with the rim lighting up and the sections
/// appearing one after another; closing (`interactive` false, no more
/// input) it quickly shrinks and fades away.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    progress: f32,
    interactive: bool,
    settings: &SharedSettings,
    first_run: bool,
    devices: &[Device],
    audio: Option<&Capture>,
    logo: Option<&TextureHandle>,
    update: Option<&SharedUpdate>,
) -> bool {
    let before = settings.lock().unwrap().clone();
    let mut s = before.clone();
    let mut done = false;

    let p = progress;
    let (opacity, scale) = if interactive {
        (
            ease_out_cubic(remap(p, 0.0, 0.2)),
            egui::emath::lerp(0.3..=1.0, spring(p)),
        )
    } else {
        let gone = 1.0 - p;
        (
            1.0 - ease_in_cubic(gone),
            egui::emath::lerp(1.0..=0.6, ease_in_cubic(gone)),
        )
    };
    // Each block fades in a little after the one above it.
    let reveal = |block: usize| {
        if interactive {
            let start = 0.08 + 0.05 * block as f32;
            ease_out_cubic(remap(p, start, start + 0.3))
        } else {
            1.0
        }
    };

    let frame = egui::Frame::new()
        .fill(Color32::from_white_alpha(24).gamma_multiply(opacity))
        .shadow(egui::Shadow {
            offset: [0, 18],
            blur: 64,
            spread: 0,
            color: Color32::from_black_alpha(90).gamma_multiply(opacity),
        })
        .corner_radius(RADIUS)
        .inner_margin(28.0);
    let window = egui::Window::new("ozbeat-setup")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .interactable(interactive)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .frame(frame)
        .show(ctx, |ui| {
            ui.multiply_opacity(opacity);
            ui.set_width(WIDTH);
            if let Some(logo) = logo {
                let aspect = (LOGO_UV.height() * 600.0) / (LOGO_UV.width() * 1200.0);
                let size = Vec2::new(WIDTH * 0.32, WIDTH * 0.32 * aspect);
                block(ui, reveal(0), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add(
                            egui::Image::from_texture(SizedTexture::new(logo.id(), size))
                                .uv(LOGO_UV),
                        );
                    });
                });
            }

            block(ui, reveal(1), |ui| {
                if first_run {
                    ui.vertical_centered(|ui| {
                        ui.heading("Vítej!");
                        ui.label(
                            RichText::new("Pár voleb a jedeme. Kdykoli se sem vrátíš přes Ctrl+S.")
                                .weak(),
                        );
                    });
                } else {
                    ui.vertical_centered(|ui| ui.heading("Nastavení"));
                }
            });

            // Header and footer stay put; the options scroll on small windows.
            let max_height = (ctx.content_rect().height() - 300.0).max(160.0);
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| {
                    options(ui, &mut s, devices, audio, &|i| reveal(2 + i));
                    if let Some(update) = update {
                        block(ui, reveal(5), |ui| update_section(ui, update));
                    }
                });

            ui.add_space(16.0);
            block(ui, reveal(6), |ui| {
                footer(ui, &mut done);
            });
        });
    if let Some(window) = window {
        let rect = window.response.rect;
        // Grow from the screen center.
        let center = ctx.content_rect().center().to_vec2();
        let transform = TSTransform::from_translation(center)
            * TSTransform::from_scaling(scale)
            * TSTransform::from_translation(-center);
        ctx.set_transform_layer(window.response.layer_id, transform);

        // The rim flashes once while the panel opens, then settles.
        let shine = if interactive {
            ease_out_cubic(remap(p, 0.05, 0.3)) * (1.0 - remap(p, 0.35, 0.9))
        } else {
            0.0
        };
        draw_glass(
            &ctx.layer_painter(window.response.layer_id),
            rect,
            opacity,
            shine,
        );
    }

    if s != before {
        s.save();
        *settings.lock().unwrap() = s;
    }
    !done
}

/// Light on the glass: a sheen fading down from the top, a thin rim that is
/// brightest along the top edge, and `shine` brightening the rim further.
fn draw_glass(painter: &egui::Painter, rect: Rect, opacity: f32, shine: f32) {
    const BANDS: usize = 10;
    let sheen = rect.height().min(260.0);
    for k in 0..BANDS {
        let height = sheen * (1.0 - k as f32 / BANDS as f32);
        let band = Rect::from_min_size(rect.min, Vec2::new(rect.width(), height));
        let corners = egui::CornerRadius {
            nw: RADIUS as u8,
            ne: RADIUS as u8,
            sw: 0,
            se: 0,
        };
        painter.rect_filled(band, corners, glow(Color32::WHITE, 0.011 * opacity));
    }
    painter.rect_stroke(
        rect,
        RADIUS,
        egui::Stroke::new(1.0, glow(Color32::WHITE, (0.16 + 0.5 * shine) * opacity)),
        egui::StrokeKind::Inside,
    );
    let top = rect.top() + 1.0;
    painter.line_segment(
        [
            Pos2::new(rect.left() + RADIUS, top),
            Pos2::new(rect.right() - RADIUS, top),
        ],
        egui::Stroke::new(1.0, glow(Color32::WHITE, (0.35 + 0.5 * shine) * opacity)),
    );
}

/// A white glass pill with dark text, for the main actions.
fn pill(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).strong().color(INK))
        .fill(Color32::from_white_alpha(235))
        .stroke(egui::Stroke::NONE)
        .corner_radius(18.0)
        .min_size(Vec2::new(110.0, 36.0))
}

/// Draws `add` at `opacity`, for the blocks appearing one after another.
fn block(ui: &mut egui::Ui, opacity: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        ui.multiply_opacity(opacity);
        add(ui);
    });
}

/// Keyboard hints and the "Hotovo" button.
fn footer(ui: &mut egui::Ui, done: &mut bool) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Ctrl+S nastavení · Ctrl+D druhá obrazovka · F11 celá obrazovka")
                .small()
                .weak(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(pill("Hotovo")).clicked() {
                *done = true;
            }
        });
    });
}

/// The option sections; `reveal` gives each one's opacity (see `show`).
fn options(
    ui: &mut egui::Ui,
    s: &mut Settings,
    devices: &[Device],
    audio: Option<&Capture>,
    reveal: &dyn Fn(usize) -> f32,
) {
    block(ui, reveal(0), |ui| {
        section(
            ui,
            "Zdroje hudby",
            "Co se má zobrazovat. Nově nalezená zařízení se zapnou sama.",
        );
        if devices.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Hledám zařízení v síti…").weak());
            });
        }
        for device in devices {
            let key = device.key();
            let mut enabled = !s.disabled_sources.contains(&key);
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut enabled, RichText::new(&device.name).strong())
                    .changed()
                {
                    if enabled {
                        s.disabled_sources.remove(&key);
                    } else {
                        s.disabled_sources.insert(key);
                    }
                }
                ui.label(RichText::new(describe(device)).weak());
            });
        }
    });

    block(ui, reveal(1), |ui| {
        section(
            ui,
            "Reakce na zvuk",
            "Odkud vizuál slyší hudbu, aby se hýbal do rytmu.",
        );
        ui.radio_value(&mut s.audio, AudioMode::Auto, "Automaticky (doporučeno)");
        ui.radio_value(&mut s.audio, AudioMode::Loopback, "Zvuk z tohoto počítače");
        ui.radio_value(
            &mut s.audio,
            AudioMode::Mic,
            "Mikrofon (hudba z reproduktorů v místnosti)",
        );
        ui.radio_value(&mut s.audio, AudioMode::Off, "Vypnuto");
        ui.add_space(4.0);
        audio_meter(ui, audio);
    });

    block(ui, reveal(2), |ui| {
        section(ui, "Intenzita efektů", "Jak moc se obraz hýbe do rytmu.");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Klidné").weak());
            ui.spacing_mut().slider_width = WIDTH - 150.0;
            ui.add(egui::Slider::new(&mut s.intensity, 0.0..=1.0).show_value(false));
            ui.label(RichText::new("Výrazné").weak());
        });
    });
}

/// The installed version and, when there is a newer one, the update button.
fn update_section(ui: &mut egui::Ui, shared: &SharedUpdate) {
    let current = update::CURRENT;
    section(ui, "Aktualizace", &format!("Máš verzi {current}."));
    let status = shared.lock().unwrap().clone();
    let release = match status {
        Status::Checking => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Zjišťuji, jestli je nová verze…").weak());
            });
            None
        }
        Status::UpToDate => {
            ui.label(RichText::new("Máš nejnovější verzi.").weak());
            None
        }
        Status::Available(release) => {
            ui.label(format!("Je dostupná verze {}.", release.version));
            Some(release)
        }
        Status::Installing => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Stahuji a instaluji novou verzi…");
            });
            None
        }
        Status::Restarting => {
            ui.label("Hotovo, spouštím novou verzi…");
            None
        }
        Status::Failed(error, release) => {
            ui.label(
                RichText::new(format!("Aktualizace se nepovedla: {error}"))
                    .small()
                    .color(Color32::from_rgb(0xE8, 0xB0, 0x4A)),
            );
            Some(release)
        }
    };
    if let Some(release) = release {
        if ui.add(pill("Aktualizovat a restartovat")).clicked() {
            update::install_in_background(Arc::clone(shared), release, ui.ctx().clone());
        }
        ui.label(
            RichText::new("Stáhne novou verzi, nahradí tuhle a spustí ji znovu.")
                .small()
                .weak(),
        );
    }
}

fn section(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.add_space(12.0);
    ui.label(RichText::new(title).size(17.0).strong());
    ui.label(RichText::new(hint).small().weak());
    ui.add_space(2.0);
}

/// "Bluesound · hraje Artist – Title"
fn describe(device: &Device) -> String {
    let kind = match device.id.kind {
        "bluos" => "Bluesound",
        "smtc" => "tento počítač",
        other => other,
    };
    let state = match &device.now_playing {
        Some(np) if np.state == PlaybackState::Playing => {
            let track = match &np.track.artist {
                Some(artist) => format!("{artist} – {}", np.track.title),
                None => np.track.title.clone(),
            };
            format!("hraje {}", truncate(&track, 38))
        }
        Some(np) if np.state == PlaybackState::Paused => "pozastaveno".to_owned(),
        _ => "nehraje".to_owned(),
    };
    format!("{kind} · {state}")
}

/// Live input level, so it is obvious whether the chosen input hears anything.
fn audio_meter(ui: &mut egui::Ui, audio: Option<&Capture>) {
    let Some(capture) = audio else {
        ui.label(
            RichText::new("Vstup se zapne, až začne něco hrát.")
                .small()
                .weak(),
        );
        return;
    };
    let levels = capture.levels();
    // -60 dBFS .. 0 dBFS
    let meter = ((20.0 * levels.rms.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
    ui.add(
        egui::ProgressBar::new(meter)
            .desired_height(8.0)
            .fill(Color32::from_white_alpha(210)),
    );
    ui.label(
        RichText::new(format!("Poslouchám: {}", capture.device()))
            .small()
            .weak(),
    );
    if !levels.signal {
        let hint = match capture.input() {
            Input::Loopback => {
                "Zatím ticho. Hraje hudba opravdu z tohoto počítače? Když hraje z reproduktoru \
                 (třeba přes Spotify Connect), zvol Mikrofon."
            }
            Input::Microphone => {
                "Zatím ticho. Pokud hudba hraje, zkontroluj, že mají aplikace přístup k mikrofonu."
            }
        };
        ui.label(
            RichText::new(hint)
                .small()
                .color(Color32::from_rgb(0xE8, 0xB0, 0x4A)),
        );
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max_chars - 1).collect();
    format!("{cut}…")
}
