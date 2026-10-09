//! Zoom and pan: animated zoom, the "Fit to window" mode, and the zoom slider.

use super::*;

pub(super) struct ZoomAnim {
    from: f32,
    to: f32,
    ax: f32,
    ay: f32,
    start: Instant,
}

impl App {
    /// Original pixels per decoded pixel (> 1 when the decode was downscaled for GPU/memory limits).
    pub(super) fn src_ratio(&self) -> f32 {
        match self.current().and_then(|p| self.cache.get(p)) {
            Some(e) if e.width > 0 => e.full_width as f32 / e.width as f32,
            _ => 1.0,
        }
    }

    /// Zoom relative to the original image (what "100%" means to the user).
    pub(super) fn zoom_pct(&self) -> f32 {
        self.view.scale / self.src_ratio() * 100.0
    }

    /// Animate to `target` scale keeping (ax, ay) fixed; instant if Windows animations are off.
    pub(super) fn animate_zoom(&mut self, target: f32, ax: f32, ay: f32) {
        if !self.has_image() {
            return;
        }
        let target = target.clamp(self.view.min_scale(), MAX_SCALE);
        if self.theme.animations {
            self.anim = Some(ZoomAnim {
                from: self.view.scale,
                to: target,
                ax,
                ay,
                start: Instant::now(),
            });
        } else {
            self.anim = None;
            self.view.zoom_to(target, ax, ay);
        }
        self.show_toast();
    }

    pub(super) fn zoom_by(&mut self, factor: f32, x: f32, y: f32) {
        // Chain onto a running animation so quick wheel notches accumulate instead of restarting.
        let base = self.anim.as_ref().map_or(self.view.scale, |a| a.to);
        self.animate_zoom(base * factor, x, y);
    }

    /// The "Fit to window" button shows as active while the mode is on and the photo is fitted
    /// (or animating to fit); zooming by hand makes it inactive without changing the saved mode.
    pub(super) fn fit_active(&self) -> bool {
        if !self.fit_window || !self.has_image() {
            return false;
        }
        let target = self.anim.as_ref().map_or(self.view.scale, |a| a.to);
        (target - self.view.fit_window_scale()).abs() < 1e-3
    }

    /// Turn the global "Fit to window" mode on (fit and enlarge) or off (back to at most 100%).
    pub(super) fn set_fit_window(&mut self, on: bool) {
        self.fit_window = on;
        settings::set_bool(FIT_WINDOW_SETTING, on);
        let (vw, vh) = self.viewport();
        let target = if on {
            self.view.fit_window_scale()
        } else {
            self.view.fit_scale()
        };
        self.animate_zoom(target, vw / 2.0, vh / 2.0);
        self.invalidate();
    }

    /// Status bar button: toggle the mode (re-fits if the mode is on but the photo was zoomed by hand).
    pub(super) fn zoom_fit(&mut self) {
        let on = !self.fit_active();
        self.set_fit_window(on);
    }

    /// Jump a running animation to its end (before the viewport changes, or when animations are turned off).
    pub(super) fn finish_anim(&mut self) {
        if let Some(a) = self.anim.take() {
            self.view.zoom_to(a.to, a.ax, a.ay);
        }
    }

    /// Advance the zoom animation; returns true while it's still running.
    pub(super) fn step_anim(&mut self) -> bool {
        let Some(a) = &self.anim else { return false };
        let t = (a.start.elapsed().as_secs_f32() / ZOOM_ANIM.as_secs_f32()).min(1.0);
        // Ease-out quadratic: decelerates into the target without a long creeping tail or overshoot.
        let eased = 1.0 - (1.0 - t).powi(2);
        let (scale, ax, ay) = (a.from + (a.to - a.from) * eased, a.ax, a.ay);
        self.view.zoom_to(scale, ax, ay);
        if t >= 1.0 {
            self.anim = None;
        }
        self.anim.is_some()
    }

    /// Slider position 0..=1 for the current zoom.
    pub(super) fn slider_pos(&self) -> f32 {
        if !self.has_image() {
            return 0.0;
        }
        let (lo, hi) = (self.view.min_scale(), MAX_SCALE);
        if hi <= lo {
            return 0.0;
        }
        ((self.view.scale / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0)
    }

    pub(super) fn set_slider(&mut self, px: i32) {
        if !self.has_image() {
            return;
        }
        let (cw, ch) = self.client_size();
        let b = bar_layout(cw / self.scale, ch / self.scale);
        let (l, r) = slider_track(b.slider);
        let pos = ((px as f32 / self.scale - l) / (r - l)).clamp(0.0, 1.0);
        let (lo, hi) = (self.view.min_scale(), MAX_SCALE);
        let (vw, vh) = self.viewport();
        self.anim = None;
        self.view
            .zoom_to(lo * (hi / lo).powf(pos), vw / 2.0, vh / 2.0);
        self.invalidate();
    }

    pub(super) fn apply_zoom_choice(&mut self, id: u32) {
        if !self.has_image() {
            return;
        }
        let (vw, vh) = self.viewport();
        match id {
            ZOOM_FIT => self.set_fit_window(true),
            pct => self.animate_zoom(pct as f32 / 100.0 * self.src_ratio(), vw / 2.0, vh / 2.0),
        }
    }
}
