//! Small helpers for the UI animations: a timeline that runs forward when
//! something opens and backward when it closes, and the easing curves.

use std::time::{Duration, Instant};

use eframe::egui::Color32;

/// Progress (0 -> 1) of something opening, running back to 0 when it closes.
/// Switching mid-way continues from where it is instead of jumping.
pub struct Tween {
    on: bool,
    from: f32,
    changed: Instant,
    open: f32,
    close: f32,
}

impl Tween {
    pub fn new(open: Duration, close: Duration) -> Self {
        Self {
            on: false,
            from: 0.0,
            changed: Instant::now(),
            open: open.as_secs_f32(),
            close: close.as_secs_f32(),
        }
    }

    pub fn set(&mut self, on: bool) {
        if on != self.on {
            self.from = self.progress();
            self.on = on;
            self.changed = Instant::now();
        }
    }

    /// Linear progress; the caller picks the curves.
    pub fn progress(&self) -> f32 {
        let elapsed = self.changed.elapsed().as_secs_f32();
        if self.on {
            (self.from + elapsed / self.open).min(1.0)
        } else {
            (self.from - elapsed / self.close).max(0.0)
        }
    }
}

/// Where `t` is between `from` and `to`, clamped to 0..=1.
pub fn remap(t: f32, from: f32, to: f32) -> f32 {
    ((t - from) / (to - from)).clamp(0.0, 1.0)
}

pub fn ease_in_cubic(t: f32) -> f32 {
    t * t * t
}

pub fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

pub fn ease_in_out_cubic(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// A damped spring released at 0 and settling on 1, the way macOS and iOS
/// move windows: fast off the mark, a slight overshoot, then still by `t` = 1.
pub fn spring(t: f32) -> f32 {
    /// Damping ratio; below 1 it overshoots (here by about 4%).
    const ZETA: f32 = 0.72;
    /// Stiffness, as the natural frequency over the whole animation.
    const OMEGA: f32 = 9.0;
    if t >= 1.0 {
        return 1.0;
    }
    let damped = OMEGA * (1.0 - ZETA * ZETA).sqrt();
    let decay = (-ZETA * OMEGA * t).exp();
    1.0 - decay * ((damped * t).cos() + ZETA * OMEGA / damped * (damped * t).sin())
}

/// `color` at `amount` brightness, added onto what is below (light, not paint).
pub fn glow(color: Color32, amount: f32) -> Color32 {
    let a = amount.clamp(0.0, 1.0);
    let scale = |c: u8| (c as f32 * a) as u8;
    Color32::from_rgba_premultiplied(scale(color.r()), scale(color.g()), scale(color.b()), 0)
}
