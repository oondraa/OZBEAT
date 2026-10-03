//! The setup panel: shown on first run, and any time later with Ctrl+S.
//! Every change is saved immediately.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, Layout, Pos2, Rect, RichText, TextureHandle, Vec2,
    load::SizedTexture,
};
use mv_audio::{Capture, Input};
use mv_core::{Device, PlaybackState};

use crate::settings::{AudioMode, Settings, SharedSettings};
use crate::update::{self, SharedUpdate, Status};

/// The dot in the OZBEAT logo.
pub const BRAND_BLUE: Color32 = Color32::from_rgb(0x2D, 0x8C, 0xFF);
const WIDTH: f32 = 540.0;
/// The wordmark inside the 1200x600 logo card (UV space).
const LOGO_UV: Rect = Rect {
    min: Pos2::new(0.2, 0.36),
    max: Pos2::new(0.78, 0.64),
};

/// Draws the panel. Returns `false` once the user confirms with "Hotovo".
pub fn show(
    ctx: &egui::Context,
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

    let frame = egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(10, 10, 14, 242))
        .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(18)))
        .corner_radius(18.0)
        .inner_margin(28.0);
    egui::Window::new("ozbeat-setup")
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            if let Some(logo) = logo {
                let aspect = (LOGO_UV.height() * 600.0) / (LOGO_UV.width() * 1200.0);
                let size = Vec2::new(WIDTH * 0.32, WIDTH * 0.32 * aspect);
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Image::from_texture(SizedTexture::new(logo.id(), size)).uv(LOGO_UV),
                    );
                });
            }

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

            // Header and footer stay put; the options scroll on small windows.
            let max_height = (ctx.content_rect().height() - 300.0).max(160.0);
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| {
                    options(ui, &mut s, devices, audio);
                    if let Some(update) = update {
                        update_section(ui, update);
                    }
                });

            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Ctrl+S nastavení · Ctrl+D druhá obrazovka · F11 celá obrazovka")
                        .small()
                        .weak(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let button =
                        egui::Button::new(RichText::new("Hotovo").strong().color(Color32::WHITE))
                            .fill(BRAND_BLUE)
                            .corner_radius(10.0)
                            .min_size(Vec2::new(110.0, 36.0));
                    if ui.add(button).clicked() {
                        done = true;
                    }
                });
            });
        });

    if s != before {
        s.save();
        *settings.lock().unwrap() = s;
    }
    !done
}

fn options(ui: &mut egui::Ui, s: &mut Settings, devices: &[Device], audio: Option<&Capture>) {
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

    section(ui, "Intenzita efektů", "Jak moc se obraz hýbe do rytmu.");
    ui.horizontal(|ui| {
        ui.label(RichText::new("Klidné").weak());
        ui.spacing_mut().slider_width = WIDTH - 150.0;
        ui.add(egui::Slider::new(&mut s.intensity, 0.0..=1.0).show_value(false));
        ui.label(RichText::new("Výrazné").weak());
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
        let button = egui::Button::new(
            RichText::new("Aktualizovat a restartovat")
                .strong()
                .color(Color32::WHITE),
        )
        .fill(BRAND_BLUE)
        .corner_radius(10.0)
        .min_size(Vec2::new(0.0, 36.0));
        if ui.add(button).clicked() {
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
            .fill(BRAND_BLUE),
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
