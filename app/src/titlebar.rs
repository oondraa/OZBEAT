//! Our own title bar, since the main window has no system frame: the name
//! on the left, minimize / maximize / close on the right. It fades in
//! when the mouse moves and out again when it rests, so the visuals keep the
//! whole window. Without a frame the edges are resized here as well.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, FontFamily, FontId, Pos2, Rect, ResizeDirection, Sense,
    Stroke, Vec2, ViewportCommand,
};

const HEIGHT: f32 = 36.0;
const BUTTON_WIDTH: f32 = 46.0;
/// How long the bar stays after the mouse stops moving.
const LINGER: Duration = Duration::from_millis(1800);
/// Fade speed, in alpha per second.
const FADE_SPEED: f32 = 6.0;
/// Width of the invisible strip along each edge that resizes the window.
const RESIZE_MARGIN: f32 = 6.0;
/// Windows' own close-button red.
const CLOSE_RED: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

pub struct TitleBar {
    last_activity: Instant,
    alpha: f32,
}

impl TitleBar {
    pub fn new() -> Self {
        Self {
            last_activity: Instant::now(),
            alpha: 1.0,
        }
    }

    /// Whether the pointer is on the bar, so a double-click there maximizes
    /// instead of toggling fullscreen.
    pub fn under_pointer(&self, ctx: &egui::Context) -> bool {
        let top = ctx.content_rect().top();
        self.alpha > 0.0
            && ctx
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| p.y < top + HEIGHT)
    }

    /// Draws the bar and handles dragging and resizing. `pinned` keeps it
    /// visible regardless of the mouse (e.g. while the settings are open).
    pub fn show(&mut self, ctx: &egui::Context, pinned: bool) {
        let (fullscreen, maximized, hover, moved, dt) = ctx.input(|i| {
            (
                i.viewport().fullscreen.unwrap_or(false),
                i.viewport().maximized.unwrap_or(false),
                i.pointer.hover_pos(),
                i.pointer.delta() != Vec2::ZERO || i.pointer.any_pressed(),
                i.stable_dt.min(0.1),
            )
        });
        // Fullscreen has nothing to drag or resize, and F11/Esc to get out.
        if fullscreen {
            self.alpha = 0.0;
            return;
        }
        let screen = ctx.content_rect();
        if !maximized {
            resize_edges(ctx, screen, hover);
        }

        let bar = Rect::from_min_size(screen.min, Vec2::new(screen.width(), HEIGHT));
        if moved {
            self.last_activity = Instant::now();
        }
        let over_bar = hover.is_some_and(|p| bar.contains(p));
        let wanted =
            pinned || over_bar || (hover.is_some() && self.last_activity.elapsed() < LINGER);
        let target = if wanted { 1.0 } else { 0.0 };
        let step = FADE_SPEED * dt;
        self.alpha = if self.alpha < target {
            (self.alpha + step).min(target)
        } else {
            (self.alpha - step).max(target)
        };
        if self.alpha <= 0.0 {
            return;
        }

        egui::Area::new(egui::Id::new("ozbeat-titlebar"))
            .order(egui::Order::Foreground)
            .fixed_pos(bar.min)
            .show(ctx, |ui| {
                self.draw(ui, bar, maximized);
            });
    }

    fn draw(&self, ui: &mut egui::Ui, bar: Rect, maximized: bool) {
        let alpha = self.alpha;
        let painter = ui.painter().clone();
        // A soft shade so the buttons read over a bright cover.
        let shade = egui::Mesh {
            indices: vec![0, 1, 2, 0, 2, 3],
            vertices: [
                (bar.left_top(), 150.0),
                (bar.right_top(), 150.0),
                (bar.right_bottom() + Vec2::new(0.0, 24.0), 0.0),
                (bar.left_bottom() + Vec2::new(0.0, 24.0), 0.0),
            ]
            .into_iter()
            .map(|(pos, a)| egui::epaint::Vertex {
                pos,
                uv: egui::epaint::WHITE_UV,
                color: Color32::from_black_alpha((a * alpha) as u8),
            })
            .collect(),
            ..Default::default()
        };
        painter.add(shade);

        let buttons = BUTTON_WIDTH * 3.0;
        let drag_rect = Rect::from_min_max(bar.min, Pos2::new(bar.right() - buttons, bar.bottom()));
        let drag = ui.interact(drag_rect, ui.id().with("drag"), Sense::click_and_drag());
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        painter.text(
            Pos2::new(bar.left() + 14.0, bar.center().y),
            Align2::LEFT_CENTER,
            "OZBEAT",
            FontId::new(13.0, FontFamily::Name(crate::ui::BOLD.into())),
            Color32::from_white_alpha((220.0 * alpha) as u8),
        );

        let ctx = ui.ctx().clone();
        let mut left = bar.right() - buttons;
        for kind in [Button::Minimize, Button::Maximize, Button::Close] {
            let rect =
                Rect::from_min_size(Pos2::new(left, bar.top()), Vec2::new(BUTTON_WIDTH, HEIGHT));
            left += BUTTON_WIDTH;
            let response = ui.interact(rect, ui.id().with(kind as u8), Sense::click());
            if response.hovered() {
                let fill = match kind {
                    Button::Close => CLOSE_RED,
                    _ => Color32::from_white_alpha(28),
                };
                painter.rect_filled(rect, 0.0, fill.gamma_multiply(alpha));
            }
            let stroke = Stroke::new(1.0, Color32::from_white_alpha((235.0 * alpha) as u8));
            draw_glyph(&painter, rect.center(), kind, maximized, stroke);
            if response.clicked() {
                let command = match kind {
                    Button::Minimize => ViewportCommand::Minimized(true),
                    Button::Maximize => ViewportCommand::Maximized(!maximized),
                    Button::Close => ViewportCommand::Close,
                };
                ctx.send_viewport_cmd(command);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Button {
    Minimize,
    Maximize,
    Close,
}

/// The same thin glyphs as Windows 11's caption buttons.
fn draw_glyph(painter: &egui::Painter, c: Pos2, kind: Button, maximized: bool, stroke: Stroke) {
    let h = 5.0;
    match kind {
        Button::Minimize => {
            painter.line_segment([c + Vec2::new(-h, 0.0), c + Vec2::new(h, 0.0)], stroke);
        }
        Button::Maximize if maximized => {
            // Restore: two overlapping squares.
            let back = Rect::from_center_size(c + Vec2::new(1.5, -1.5), Vec2::splat(2.0 * h - 2.0));
            let front =
                Rect::from_center_size(c + Vec2::new(-1.5, 1.5), Vec2::splat(2.0 * h - 2.0));
            painter.line_segment([back.left_top(), back.right_top()], stroke);
            painter.line_segment([back.right_top(), back.right_bottom()], stroke);
            painter.rect_stroke(front, 1.0, stroke, egui::StrokeKind::Middle);
        }
        Button::Maximize => {
            let rect = Rect::from_center_size(c, Vec2::splat(2.0 * h));
            painter.rect_stroke(rect, 1.0, stroke, egui::StrokeKind::Middle);
        }
        Button::Close => {
            painter.line_segment([c + Vec2::new(-h, -h), c + Vec2::new(h, h)], stroke);
            painter.line_segment([c + Vec2::new(-h, h), c + Vec2::new(h, -h)], stroke);
        }
    }
}

/// Resize cursor along the edges, and a native resize on press there.
fn resize_edges(ctx: &egui::Context, screen: Rect, hover: Option<Pos2>) {
    let Some(pos) = hover else {
        return;
    };
    let near = |a: f32, b: f32| (a - b).abs() <= RESIZE_MARGIN;
    let (n, s) = (near(pos.y, screen.top()), near(pos.y, screen.bottom()));
    let (w, e) = (near(pos.x, screen.left()), near(pos.x, screen.right()));
    let (direction, cursor) = match (n, s, w, e) {
        (true, _, true, _) => (ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
        (true, _, _, true) => (ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
        (_, true, true, _) => (ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
        (true, ..) => (ResizeDirection::North, CursorIcon::ResizeNorth),
        (_, true, ..) => (ResizeDirection::South, CursorIcon::ResizeSouth),
        (_, _, true, _) => (ResizeDirection::West, CursorIcon::ResizeWest),
        (_, _, _, true) => (ResizeDirection::East, CursorIcon::ResizeEast),
        _ => return,
    };
    ctx.set_cursor_icon(cursor);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}
