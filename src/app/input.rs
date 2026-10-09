//! Hit testing (DIP coordinates) and drawer scrolling.

mod keyboard;
mod mouse;
mod touch;

use super::*;

impl App {
    // ---------- hit testing (DIP coordinates) ----------

    pub(super) fn hit(&self, px: i32, py: i32) -> Hit {
        let s = self.scale;
        let (x, y) = (px as f32 / s, py as f32 / s);
        let (cw, ch) = self.client_size();
        let (cw, ch) = (cw / s, ch / s);
        if self.delete_prompt.is_some() {
            let (_, ok, cancel) = dialog_layout(cw, ch);
            return if inside(ok, x, y) {
                Hit::DialogOk
            } else if inside(cancel, x, y) {
                Hit::DialogCancel
            } else {
                Hit::DialogArea
            };
        }
        let th = self.title_h();
        if y < th {
            if inside(more_rect(), x, y) {
                return if self.settings_open {
                    Hit::Back
                } else {
                    Hit::More
                };
            }
            return Hit::TitleBar;
        }
        if self.settings_open {
            let l = settings_layout(cw);
            let y = y - th + self.settings_scroll;
            return if inside(l.dropdown, x, y) {
                Hit::ThemePick
            } else if inside(l.toggle, x, y) {
                Hit::ConfirmToggle
            } else if inside(l.default_button, x, y) {
                Hit::SetDefault
            } else {
                Hit::Page
            };
        }
        if self.bar_visible() && y >= ch - BAR_H {
            let b = bar_layout(cw, ch);
            let targets = [
                (b.info, Hit::Info),
                (b.fit, Hit::Fit),
                (b.menu, Hit::ZoomMenu),
                (b.zoom_out, Hit::ZoomOut),
                (b.slider, Hit::Slider),
                (b.zoom_in, Hit::ZoomIn),
                (b.full, Hit::Full),
            ];
            // Hit areas span the full bar height (and a little wider) so they're comfortable to tap.
            let tall = |r: Rect| (r.0 - 2.0, ch - BAR_H, r.2 + 2.0, ch);
            return targets
                .iter()
                .find(|(r, _)| inside(tall(*r), x, y))
                .map(|(_, h)| *h)
                .unwrap_or(Hit::Bar);
        }
        // Below here, y is relative to the top of the image area.
        let y = y - th;
        let vw = if self.drawer { cw - DRAWER_W } else { cw };
        if self.default_prompt {
            let (_, open, dismiss) = default_prompt_layout(vw, ch - self.bar_h() - th);
            if inside(open, x, y) {
                return Hit::PromptOpen;
            }
            if inside(dismiss, x, y) {
                return Hit::PromptDismiss;
            }
        }
        if self.drawer && x >= vw {
            if inside(close_rect(cw), x, y) {
                return Hit::Close;
            }
            if self.link_rect.get().is_some_and(|r| inside(r, x, y)) {
                return Hit::Link;
            }
            return Hit::Drawer;
        }
        let vh = ch - self.bar_h() - th;
        let within = |cx: f32| (x - cx).powi(2) + (y - vh / 2.0).powi(2) <= 24.0f32.powi(2);
        if self.index > 0 && within(NAV_INSET) {
            return Hit::Prev;
        }
        if self.index + 1 < self.files.len() && within(vw - NAV_INSET) {
            return Hit::Next;
        }
        Hit::Image
    }

    pub(super) fn scroll_drawer(&mut self, wheel_delta: f32) {
        self.scroll_drawer_by(-wheel_delta / 120.0 * 48.0);
    }

    pub(super) fn scroll_drawer_by(&mut self, dips: f32) {
        let (_, ch) = self.client_size();
        let max = (self.drawer_content + 16.0 - (ch / self.scale - self.bar_h() - self.title_h()))
            .max(0.0);
        self.drawer_scroll = (self.drawer_scroll + dips).clamp(0.0, max);
        self.invalidate();
    }
}
