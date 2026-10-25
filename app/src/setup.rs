//! The setup panel: shown on first run, and any time later with Ctrl+S.
//! Every change is saved immediately.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Layout, Rect, RichText, Sense,
    Stroke, StrokeKind, Vec2, emath::TSTransform,
};
use mv_audio::{Capture, Input};
use mv_core::{Device, PlaybackState};

use crate::anim::{ease_in_cubic, ease_out_cubic, glow, remap, spring};
use crate::icons;
use crate::settings::{AudioMode, ClipScreen, LyricsStyle, Settings, SharedSettings};
use crate::update::{self, SharedUpdate, Status};

/// The dot in the OZBEAT logo.
pub const BRAND_BLUE: Color32 = Color32::from_rgb(0x2D, 0x8C, 0xFF);
/// Two columns of cards, so everything fits a screen without scrolling.
const WIDTH: f32 = 920.0;
/// Between the columns.
const GAP: f32 = 14.0;
/// Corner radius of the glass panel.
const RADIUS: f32 = 28.0;
/// Corner radius of the section cards inside it.
const CARD_RADIUS: u8 = 18;
/// Dark text on light controls.
const INK: Color32 = Color32::from_rgb(0x12, 0x12, 0x16);
/// Hints and secondary text: readable on the dark glass, quieter than labels.
const QUIET: Color32 = Color32::from_rgb(0xA8, 0xA8, 0xB4);
const WARN: Color32 = Color32::from_rgb(0xE8, 0xB0, 0x4A);

/// Each section's icon (from the lyric icon font) and the color of its badge.
const SOURCES: (char, Color32) = ('\u{e340}', Color32::from_rgb(0xFF, 0x4F, 0x7B));
const AUDIO: (char, Color32) = ('\u{ea08}', Color32::from_rgb(0x9B, 0x6B, 0xFF));
const MOTION: (char, Color32) = ('\u{e2de}', Color32::from_rgb(0xFF, 0x9F, 0x2E));
const LYRICS: (char, Color32) = ('\u{e16c}', BRAND_BLUE);
const CLIPS: (char, Color32) = ('\u{e4da}', Color32::from_rgb(0xFF, 0x45, 0x45));
const LIGHT: (char, Color32) = ('\u{e2dc}', Color32::from_rgb(0xF5, 0xC5, 0x18));
const CONTROL: (char, Color32) = ('\u{e298}', Color32::from_rgb(0x2E, 0xC4, 0xD6));
const UPDATE: (char, Color32) = ('\u{e6a2}', Color32::from_rgb(0x2F, 0xC2, 0x7A));

/// Draws the panel. Returns `false` once the user confirms with "Hotovo".
/// The panel is dark frosted glass over the dimmed visuals (see `ui`), with
/// a bright rim; the options sit on cards, each with a colored icon.
///
/// `progress` (0 -> 1) animates it: opening, it grows from small on a spring
/// (like a window on a Mac), with the rim lighting up and the cards
/// appearing one after another; closing (`interactive` false, no more input)
/// it quickly shrinks and fades away.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    progress: f32,
    interactive: bool,
    settings: &SharedSettings,
    first_run: bool,
    devices: &[Device],
    audio: Option<&Capture>,
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
        .fill(Color32::from_rgba_unmultiplied(16, 16, 22, 200).gamma_multiply(opacity))
        .shadow(egui::Shadow {
            offset: [0, 24],
            blur: 80,
            spread: 0,
            color: Color32::from_black_alpha(140).gamma_multiply(opacity),
        })
        .corner_radius(RADIUS)
        .inner_margin(24.0);
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
            block(ui, reveal(0), |ui| header(ui, first_run, &mut done));
            ui.add_space(14.0);

            // Header and footer stay put; the cards scroll on small windows.
            let max_height = (ctx.content_rect().height() - 230.0).max(200.0);
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| {
                    let column = (WIDTH - GAP) / 2.0;
                    let reveal = |i| reveal(1 + i);
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = GAP;
                        ui.vertical(|ui| {
                            ui.set_width(column);
                            listening(ui, &mut s, devices, audio, &reveal);
                            // Under the effects, where the left column has room.
                            if let Some(update) = update {
                                block(ui, reveal(6), |ui| {
                                    card(ui, |ui| update_section(ui, update));
                                });
                            }
                        });
                        ui.vertical(|ui| {
                            ui.set_width(column);
                            showing(ui, &mut s, &reveal);
                        });
                    });
                });

            ui.add_space(14.0);
            block(ui, reveal(8), footer);
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
        draw_rim(
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

/// Light on the glass: a thin rim, brightest along the top edge, and
/// `shine` brightening it further.
fn draw_rim(painter: &egui::Painter, rect: Rect, opacity: f32, shine: f32) {
    use egui::{Direction, Shape};
    painter.rect_stroke(
        rect,
        RADIUS,
        Stroke::new(1.0, glow(Color32::WHITE, (0.12 + 0.5 * shine) * opacity)),
        StrokeKind::Inside,
    );
    // The light catching the top edge, fading out toward the corners so it
    // doesn't end abruptly where the rim starts to curve.
    let edge = glow(Color32::WHITE, (0.35 + 0.5 * shine) * opacity);
    let top = rect.top() + 0.5;
    let mid = rect.center().x;
    let half = (rect.width() / 2.0 - RADIUS).max(0.0);
    for (from, to, colors) in [
        (mid - half, mid, [Color32::TRANSPARENT, edge]),
        (mid, mid + half, [edge, Color32::TRANSPARENT]),
    ] {
        painter.add(Shape::gradient_rect(
            Rect::from_x_y_ranges(from..=to, top..=top + 1.0),
            Direction::LeftToRight,
            colors,
        ));
    }
}

/// The title and the "Hotovo" button.
fn header(ui: &mut egui::Ui, first_run: bool, done: &mut bool) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            let title = if first_run {
                "Vítej v OZBEAT"
            } else {
                "Nastavení"
            };
            ui.label(
                RichText::new(title)
                    .size(28.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            let hint = if first_run {
                "Pár voleb a jedeme. Kdykoli se sem vrátíš přes Ctrl+S."
            } else {
                "Všechno se ukládá hned."
            };
            ui.label(RichText::new(hint).color(QUIET));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(primary("Hotovo")).clicked() {
                *done = true;
            }
        });
    });
}

/// A filled blue pill with white text, for the main actions.
fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).strong().color(Color32::WHITE))
        .fill(BRAND_BLUE)
        .stroke(Stroke::NONE)
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

/// A section on its own card; it starts with a [`heading`].
fn card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(9))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(16)))
        .corner_radius(CARD_RADIUS)
        .inner_margin(18.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 10.0;
            add(ui);
        });
    ui.add_space(12.0);
}

/// A rounded square in `tint` with `icon` on it in white.
fn badge(ui: &mut egui::Ui, icon: char, tint: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 10.0, tint);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        icon,
        FontId::new(20.0, FontFamily::Name(icons::FONT.into())),
        Color32::WHITE,
    );
}

/// A card's colored icon, title and what it is for.
fn heading(ui: &mut egui::Ui, (icon, tint): (char, Color32), title: &str, hint: &str) {
    ui.horizontal_top(|ui| {
        badge(ui, icon, tint);
        ui.add_space(2.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                RichText::new(title)
                    .size(17.0)
                    .strong()
                    .color(Color32::WHITE),
            );
            ui.label(RichText::new(hint).size(14.0).color(QUIET));
        });
    });
    ui.add_space(2.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(13.5).color(QUIET));
}

/// An on/off switch like on a phone, with `label` (and `detail` under it)
/// on the left; the whole row toggles it.
fn switch(ui: &mut egui::Ui, on: &mut bool, label: &str, detail: Option<&str>) -> bool {
    let row = ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(RichText::new(label).strong().color(Color32::WHITE));
            if let Some(detail) = detail {
                ui.label(RichText::new(detail).size(13.5).color(QUIET));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let size = Vec2::new(44.0, 26.0);
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let t = ui.ctx().animate_bool_responsive(ui.id().with(label), *on);
            let track = Color32::from_white_alpha(40).lerp_to_gamma(BRAND_BLUE, t);
            ui.painter().rect_filled(rect, 13.0, track);
            let knob = egui::emath::lerp(rect.left() + 13.0..=rect.right() - 13.0, t);
            let center = egui::pos2(knob, rect.center().y);
            ui.painter().circle_filled(
                center + Vec2::new(0.0, 1.0),
                11.0,
                Color32::from_black_alpha(50),
            );
            ui.painter().circle_filled(center, 11.0, Color32::WHITE);
        });
    });
    let response = ui.interact(
        row.response.rect,
        ui.id().with(("switch", label)),
        Sense::click(),
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        *on = !*on;
        return true;
    }
    false
}

/// A row of choices where one is picked: the picked one is a light pill.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, choices: &[(T, &str)]) {
    ui.spacing_mut().button_padding = Vec2::new(12.0, 4.0);
    egui::Frame::new()
        .fill(Color32::from_white_alpha(12))
        .corner_radius(12)
        .inner_margin(3.0)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.horizontal(|ui| {
                for &(choice, label) in choices {
                    let picked = *value == choice;
                    let text =
                        RichText::new(label).color(if picked { INK } else { Color32::WHITE });
                    let fill = if picked {
                        Color32::from_white_alpha(235)
                    } else {
                        Color32::TRANSPARENT
                    };
                    let button = egui::Button::new(if picked { text.strong() } else { text })
                        .fill(fill)
                        .stroke(Stroke::NONE)
                        .corner_radius(9.0)
                        .min_size(Vec2::new(0.0, 32.0));
                    if ui.add(button).clicked() {
                        *value = choice;
                    }
                }
            });
        });
}

/// The shortcuts, as key caps.
fn footer(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for (keys, what) in [
            ("Ctrl+S", "nastavení"),
            ("Ctrl+D", "druhá obrazovka"),
            ("Ctrl+L", "styl textů"),
            ("F11", "celá obrazovka"),
        ] {
            egui::Frame::new()
                .stroke(Stroke::new(1.0, Color32::from_white_alpha(40)))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(6, 2))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(keys)
                            .size(12.5)
                            .strong()
                            .color(Color32::WHITE),
                    );
                });
            ui.label(RichText::new(what).size(13.0).color(QUIET));
            ui.add_space(8.0);
        }
    });
}

/// The left column: where the music comes from and how the picture moves
/// with it. `reveal` gives each card's opacity (see `show`).
fn listening(
    ui: &mut egui::Ui,
    s: &mut Settings,
    devices: &[Device],
    audio: Option<&Capture>,
    reveal: &dyn Fn(usize) -> f32,
) {
    block(ui, reveal(0), |ui| {
        card(ui, |ui| {
            heading(
                ui,
                SOURCES,
                "Zdroje hudby",
                "Co se má zobrazovat. Nově nalezená zařízení se zapnou sama.",
            );
            if devices.is_empty() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    hint(ui, "Hledám zařízení v síti…");
                });
            }
            for device in devices {
                let key = device.key();
                let mut enabled = !s.disabled_sources.contains(&key);
                if switch(ui, &mut enabled, &device.name, Some(&describe(device))) {
                    if enabled {
                        s.disabled_sources.remove(&key);
                    } else {
                        s.disabled_sources.insert(key);
                    }
                }
            }
        });
    });

    block(ui, reveal(1), |ui| {
        card(ui, |ui| {
            heading(
                ui,
                AUDIO,
                "Reakce na zvuk",
                "Odkud vizuál slyší hudbu, aby se hýbal do rytmu.",
            );
            segmented(
                ui,
                &mut s.audio,
                &[
                    (AudioMode::Auto, "Automaticky"),
                    (AudioMode::Loopback, "Počítač"),
                    (AudioMode::Mic, "Mikrofon"),
                    (AudioMode::Off, "Vypnuto"),
                ],
            );
            hint(
                ui,
                match s.audio {
                    AudioMode::Auto => {
                        "Zvuk z počítače, když hraje on, a mikrofon pro reproduktory v síti."
                    }
                    AudioMode::Loopback => "Co právě hraje tento počítač.",
                    AudioMode::Mic => "Hudba z reproduktorů v místnosti.",
                    AudioMode::Off => "Obraz se hýbe jen podle tempa skladby.",
                },
            );
            if s.audio != AudioMode::Off {
                audio_meter(ui, audio);
            }
        });
    });

    block(ui, reveal(2), |ui| {
        card(ui, |ui| {
            heading(
                ui,
                MOTION,
                "Intenzita efektů",
                "Jak moc se obraz hýbe do rytmu.",
            );
            ui.horizontal(|ui| {
                hint(ui, "Klidné");
                ui.spacing_mut().slider_width = ui.available_width() - 70.0;
                ui.spacing_mut().slider_rail_height = 6.0;
                ui.add(egui::Slider::new(&mut s.intensity, 0.0..=1.0).show_value(false));
                hint(ui, "Výrazné");
            });
        });
    });

    block(ui, reveal(3), |ui| {
        card(ui, |ui| {
            heading(
                ui,
                CONTROL,
                "Ovládání",
                "Hlasitost, pauza a přeskakování toho, co hraje.",
            );
            switch(
                ui,
                &mut s.buttons,
                "Tlačítka na obrazovce",
                Some("Ukážou se vpravo dole, když pohneš myší."),
            );
            switch(
                ui,
                &mut s.playback_keys,
                "Klávesové zkratky",
                Some("Ctrl+↑ / ↓ hlasitost, Ctrl+← / → přeskočit, Ctrl+mezerník pauza."),
            );
        });
    });
}

/// The right column: lyrics, videos and the light.
fn showing(ui: &mut egui::Ui, s: &mut Settings, reveal: &dyn Fn(usize) -> f32) {
    block(ui, reveal(3), |ui| {
        card(ui, |ui| {
            heading(
                ui,
                LYRICS,
                "Texty",
                "Jak se ukazují texty písniček, které mají časování.",
            );
            segmented(
                ui,
                &mut s.lyrics,
                &[
                    (LyricsStyle::Words, "Po slovech"),
                    (LyricsStyle::Drum, "Válec"),
                    (LyricsStyle::Lens, "Čočka"),
                ],
            );
        });
    });

    block(ui, reveal(4), |ui| {
        card(ui, |ui| {
            heading(
                ui,
                CLIPS,
                "Videoklipy",
                "Oficiální klip z YouTube hraje spolu se skladbou, bez zvuku.",
            );
            switch(ui, &mut s.youtube, "Přehrávat videoklipy", None);
            if s.youtube {
                hint(ui, "Se dvěma obrazovkami (Ctrl+D) je klip:");
                segmented(
                    ui,
                    &mut s.clip_screen,
                    &[
                        (ClipScreen::Lyrics, "Pod textem"),
                        (ClipScreen::Main, "Na hlavní"),
                        (ClipScreen::Both, "Na obou"),
                    ],
                );
                hint(
                    ui,
                    "Klip se stáhne, až skladba hraje 20 s, a pak se pamatuje. \
                     Mezi stahováními je pauza, aby YouTube nic nezablokoval.",
                );
            }
        });
    });

    block(ui, reveal(5), |ui| {
        card(ui, |ui| {
            heading(
                ui,
                LIGHT,
                "Světlo WLED",
                "Svítí do rytmu a v barvách obalu. Nech prázdné, když žádné nemáš.",
            );
            ui.add(
                egui::TextEdit::singleline(&mut s.wled)
                    .hint_text("IP adresa, třeba 192.168.1.50")
                    .margin(egui::Margin::symmetric(10, 8))
                    .desired_width(280.0),
            );
            if !s.wled.trim().is_empty() {
                hint(
                    ui,
                    "Ve WLED zapni AudioReactive a v něm Sync → Receive, \
                     pak vyber některý ze zvukových efektů (♪).",
                );
            }
        });
    });
}

/// The installed version and, when there is a newer one, the update button.
fn update_section(ui: &mut egui::Ui, shared: &SharedUpdate) {
    let current = update::CURRENT;
    heading(ui, UPDATE, "Aktualizace", &format!("Máš verzi {current}."));
    let status = shared.lock().unwrap().clone();
    let release = match status {
        Status::Checking => {
            ui.horizontal(|ui| {
                ui.spinner();
                hint(ui, "Zjišťuji, jestli je nová verze…");
            });
            None
        }
        Status::UpToDate => {
            hint(ui, "Máš nejnovější verzi.");
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
                    .size(13.5)
                    .color(WARN),
            );
            Some(release)
        }
    };
    if let Some(release) = release {
        if ui.add(primary("Aktualizovat a restartovat")).clicked() {
            update::install_in_background(Arc::clone(shared), release, ui.ctx().clone());
        }
        hint(ui, "Stáhne novou verzi, nahradí tuhle a spustí ji znovu.");
    }
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
        hint(ui, "Vstup se zapne, až začne něco hrát.");
        return;
    };
    let levels = capture.levels();
    // -60 dBFS .. 0 dBFS
    let meter = ((20.0 * levels.rms.max(1e-6).log10() + 60.0) / 60.0).clamp(0.0, 1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 6.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), Color32::from_white_alpha(24));
    let level = Rect::from_min_size(rect.min, Vec2::new(rect.width() * meter, rect.height()));
    painter.rect_filled(
        level,
        CornerRadius::same(3),
        Color32::from_rgb(0x2F, 0xC2, 0x7A),
    );
    hint(ui, &format!("Poslouchám: {}", capture.device()));
    if !levels.signal {
        let text = match capture.input() {
            Input::Loopback => {
                "Zatím ticho. Hraje hudba opravdu z tohoto počítače? Když hraje z reproduktoru \
                 (třeba přes Spotify Connect), zvol Mikrofon."
            }
            Input::Microphone => {
                "Zatím ticho. Pokud hudba hraje, zkontroluj, že mají aplikace přístup k mikrofonu."
            }
        };
        ui.label(RichText::new(text).size(13.5).color(WARN));
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let cut: String = text.chars().take(max_chars - 1).collect();
    format!("{cut}…")
}
