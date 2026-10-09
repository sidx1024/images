//! Zoom/pan math. Everything is in physical pixels:
//! screen = image * scale + offset, within a viewport of (vw, vh).

pub const MAX_SCALE: f32 = 32.0;

#[derive(Clone, Copy, Default)]
pub struct View {
    pub scale: f32,
    pub ox: f32,
    pub oy: f32,
    img_w: f32,
    img_h: f32,
    vw: f32,
    vh: f32,
}

impl View {
    pub fn new(img_w: u32, img_h: u32, vw: f32, vh: f32) -> Self {
        let mut v = View {
            img_w: img_w as f32,
            img_h: img_h as f32,
            vw,
            vh,
            ..Default::default()
        };
        v.fit();
        v
    }

    /// Scale at which the whole image fits; small images are never upscaled.
    pub fn fit_scale(&self) -> f32 {
        if self.img_w <= 0.0 || self.img_h <= 0.0 || self.vw <= 0.0 || self.vh <= 0.0 {
            return 1.0;
        }
        (self.vw / self.img_w).min(self.vh / self.img_h).min(1.0)
    }

    /// Scale at which the image exactly fills the window, enlarging small images.
    pub fn fit_window_scale(&self) -> f32 {
        if self.img_w <= 0.0 || self.img_h <= 0.0 || self.vw <= 0.0 || self.vh <= 0.0 {
            return 1.0;
        }
        (self.vw / self.img_w).min(self.vh / self.img_h)
    }

    /// Fill the window, enlarging small images.
    pub fn fit_window(&mut self) {
        self.scale = self.fit_window_scale();
        self.clamp();
    }

    pub fn is_fit_window(&self) -> bool {
        (self.scale - self.fit_window_scale()).abs() < 1e-4
    }

    pub fn min_scale(&self) -> f32 {
        self.fit_scale().min(1.0)
    }

    pub fn is_fit(&self) -> bool {
        (self.scale - self.fit_scale()).abs() < 1e-4
    }

    pub fn fit(&mut self) {
        self.scale = self.fit_scale();
        self.clamp();
    }

    /// Zoom to an absolute scale keeping the image point under (px, py) fixed.
    pub fn zoom_to(&mut self, scale: f32, px: f32, py: f32) {
        let new = scale.clamp(self.min_scale(), MAX_SCALE);
        let k = new / self.scale;
        self.ox = px - (px - self.ox) * k;
        self.oy = py - (py - self.oy) * k;
        self.scale = new;
        self.clamp();
    }

    /// Pinch: the image point under `from` lands under `to` at `scale * factor`; clamped once at the end.
    pub fn pinch(&mut self, factor: f32, from: (f32, f32), to: (f32, f32)) {
        let new = (self.scale * factor).clamp(self.min_scale(), MAX_SCALE);
        let ix = (from.0 - self.ox) / self.scale;
        let iy = (from.1 - self.oy) / self.scale;
        self.scale = new;
        self.ox = to.0 - ix * new;
        self.oy = to.1 - iy * new;
        self.clamp();
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        self.ox += dx;
        self.oy += dy;
        self.clamp();
    }

    /// Keep the image point at the viewport center stable across a resize.
    pub fn resize(&mut self, vw: f32, vh: f32) {
        let was_fit = self.is_fit();
        let was_fit_window = self.is_fit_window() && !was_fit;
        let cx = (self.vw / 2.0 - self.ox) / self.scale;
        let cy = (self.vh / 2.0 - self.oy) / self.scale;
        self.vw = vw;
        self.vh = vh;
        if was_fit_window {
            // Keep filling the window as it resizes.
            self.scale = self.fit_window_scale();
            self.clamp();
        } else if was_fit || self.scale < self.fit_scale() {
            self.fit();
        } else {
            self.ox = vw / 2.0 - cx * self.scale;
            self.oy = vh / 2.0 - cy * self.scale;
            self.clamp();
        }
    }

    fn clamp(&mut self) {
        let (w, h) = (self.img_w * self.scale, self.img_h * self.scale);
        self.ox = if w <= self.vw {
            (self.vw - w) / 2.0
        } else {
            self.ox.clamp(self.vw - w, 0.0)
        };
        self.oy = if h <= self.vh {
            (self.vh - h) / 2.0
        } else {
            self.oy.clamp(self.vh - h, 0.0)
        };
    }

    pub fn can_pan(&self) -> bool {
        self.img_w * self.scale > self.vw + 0.5 || self.img_h * self.scale > self.vh + 0.5
    }

    /// Destination rect (l, t, r, b), snapped to whole pixels so 100% stays crisp.
    pub fn dest(&self) -> (f32, f32, f32, f32) {
        let l = self.ox.round();
        let t = self.oy.round();
        (
            l,
            t,
            l + (self.img_w * self.scale).round(),
            t + (self.img_h * self.scale).round(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinch_keeps_content_under_fingers_from_fit() {
        // 1000px image fitted into a 1000px viewport; fingers' midpoint moves (400,600) -> (400,800) at 2x.
        let mut v = View::new(1000, 1000, 1000.0, 1000.0);
        v.pinch(2.0, (400.0, 600.0), (400.0, 800.0));
        assert_eq!(v.scale, 2.0);
        assert_eq!((v.ox, v.oy), (-400.0, -400.0));
    }

    #[test]
    fn fit_window_enlarges_small_images_and_follows_resize() {
        // 500x400 image in a 1000x1000 viewport: initial fit stays at 100%, window fit is 2x.
        let mut v = View::new(500, 400, 1000.0, 1000.0);
        assert_eq!(v.scale, 1.0);
        assert_eq!(v.fit_window_scale(), 2.0);
        v.zoom_to(v.fit_window_scale(), 500.0, 500.0);
        assert!(v.is_fit_window());
        assert_eq!((v.ox, v.oy), (0.0, 100.0));
        v.resize(1500.0, 1500.0);
        assert_eq!(v.scale, 3.0);
    }

    #[test]
    fn zoom_out_stops_at_fit_and_recenters() {
        let mut v = View::new(2000, 1000, 1000.0, 1000.0);
        v.zoom_to(2.0, 0.0, 0.0);
        v.zoom_to(0.01, 0.0, 0.0);
        assert_eq!(v.scale, 0.5);
        assert_eq!((v.ox, v.oy), (0.0, 250.0));
    }
}
