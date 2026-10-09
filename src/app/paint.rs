//! Frame composition: the image area, notices, delete dialog, and window chrome.

mod drawer;
mod settings;
mod status;
mod title;

use super::*;

impl App {
    // ---------- painting ----------

    pub(super) fn paint(&mut self) {
        if self.gfx.ensure_target(self.hwnd).is_err() {
            return;
        }
        if let Some(max) = self.gfx.max_bitmap_size() {
            self.loader.max_dim.store(max, Ordering::Relaxed);
        }
        self.sync_view();
        let animating = self.step_anim();
        self.link_rect.set(None);

        let th = self.theme.clone();
        let s = self.scale;
        let (cw, ch) = self.client_size();
        let (vw, vh) = self.viewport();
        let cur = self.current().cloned();
        let entry = cur.as_ref().and_then(|p| self.cache.get(p));
        let t = self.gfx.target.as_ref().unwrap();
        unsafe { t.rt.BeginDraw() };
        t.set_scale(1.0);
        t.clear(th.bg);

        // Image (pixels)
        if let (Some(e), true) = (entry, self.has_image()) {
            if let Some(bmp) = &e.bitmap {
                let (l, tp, r, b) = self.view.dest();
                let top = self.top_px();
                t.bitmap(bmp, rect(l, tp + top, r, b + top), self.view.scale);
            }
        }

        // Chrome over the image area (DIPs, origin at the top of the image area)
        t.set_transform(s, 0.0, self.top_px());
        let (vw_d, vh_d) = (vw / s, vh / s);
        let f = &self.gfx.fonts;

        match (cur.as_ref(), entry) {
            (None, _) => t.text(
                "Open a photo with Ctrl+O, or drop one here",
                &f.center,
                rect(0.0, 0.0, vw_d, vh_d),
                th.text_dim,
            ),
            (
                Some(p),
                Some(Entry {
                    error: Some(err), ..
                }),
            ) => {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                t.text(
                    &format!("Can't open {name}\n\n{err}"),
                    &f.center,
                    rect(24.0, 0.0, vw_d - 24.0, vh_d),
                    th.text_dim,
                );
            }
            _ => {}
        }

        if self.overlay && !self.files.is_empty() {
            let nav = |cx: f32, glyph: &str, hit: Hit| {
                let hot = self.hover == hit;
                let r = rect(cx - 20.0, vh_d / 2.0 - 20.0, cx + 20.0, vh_d / 2.0 + 20.0);
                t.fill_round(r, 20.0, if hot { th.overlay_hover } else { th.overlay });
                t.text(
                    glyph,
                    &f.icon,
                    r,
                    if hot { th.active_text } else { th.text },
                );
            };
            if self.index > 0 {
                nav(NAV_INSET, "\u{E76B}", Hit::Prev);
            }
            if self.index + 1 < self.files.len() {
                nav(vw_d - NAV_INSET, "\u{E76C}", Hit::Next);
            }
        }

        // The status bar shows the zoom level; the toast is only needed when it's hidden.
        if self.toast && self.has_image() && !self.bar_visible() {
            let pct = format!("{:.0}%", self.zoom_pct());
            let r = rect(
                vw_d / 2.0 - 40.0,
                vh_d - 64.0,
                vw_d / 2.0 + 40.0,
                vh_d - 32.0,
            );
            t.fill_round(r, 16.0, th.overlay);
            t.text(&pct, &f.center, r, th.text);
        }

        let drawer_h = ch / s - self.bar_h() - self.title_h();
        if self.drawer {
            let info = cur.as_ref().and_then(|p| self.infos.get(p));
            self.drawer_content = self.paint_drawer(cw / s, drawer_h, entry, info);
            let max = (self.drawer_content + 16.0 - drawer_h).max(0.0);
            if self.drawer_scroll > max {
                self.drawer_scroll = max;
                self.invalidate();
            }
        }
        if self.settings_open {
            self.paint_settings(cw / s, ch / s - self.title_h());
        }
        if self.default_prompt && !self.modal() {
            // First time it's actually on screen: never show it again.
            if !crate::platform::settings::get_bool(DEFAULT_PROMPT_SETTING).unwrap_or(false) {
                crate::platform::settings::set_bool(DEFAULT_PROMPT_SETTING, true);
            }
            let (c, open, dismiss) = default_prompt_layout(vw_d, vh_d);
            let tt = self.gfx.target.as_ref().unwrap();
            let f = &self.gfx.fonts;
            tt.fill_round(rect(c.0, c.1, c.2, c.3), 6.0, th.drawer);
            tt.stroke_round(
                rect(c.0 + 0.5, c.1 + 0.5, c.2 - 0.5, c.3 - 0.5),
                6.0,
                1.0,
                th.divider,
            );
            let cy = (c.1 + c.3) / 2.0;
            tt.fill_ellipse(c.0 + 26.0, cy, 10.0, th.accent);
            tt.text(
                "\u{E946}",
                &f.icon_small,
                rect(c.0 + 16.0, cy - 10.0, c.0 + 36.0, cy + 10.0),
                th.on_status,
            );
            tt.text(
                "Make Images your default photo viewer?",
                &f.bar,
                rect(c.0 + 48.0, cy - 10.0, open.0 - 8.0, cy + 12.0),
                th.text,
            );
            let open_hot = self.hover == Hit::PromptOpen;
            tt.fill_round(rect(open.0, open.1, open.2, open.3), 4.0, th.accent);
            if open_hot {
                tt.fill_round(
                    rect(open.0, open.1, open.2, open.3),
                    4.0,
                    rgba(255, 255, 255, 0.12),
                );
            }
            tt.text(
                "Open settings",
                &f.center,
                rect(open.0, open.1, open.2, open.3),
                th.on_status,
            );
            let dismiss_hot = self.hover == Hit::PromptDismiss;
            tt.fill_round(
                rect(dismiss.0, dismiss.1, dismiss.2, dismiss.3),
                4.0,
                if dismiss_hot { th.active } else { th.subtle },
            );
            tt.stroke_round(
                rect(
                    dismiss.0 + 0.5,
                    dismiss.1 + 0.5,
                    dismiss.2 - 0.5,
                    dismiss.3 - 0.5,
                ),
                4.0,
                1.0,
                th.divider,
            );
            tt.text(
                "Not now",
                &f.center,
                rect(dismiss.0, dismiss.1, dismiss.2, dismiss.3),
                if dismiss_hot { th.active_text } else { th.text },
            );
        }
        // Notices go above everything in the content area, including the settings page.
        if let Some((text, ok)) = &self.notice {
            let tt = self.gfx.target.as_ref().unwrap();
            let tw = self.gfx.text_width(text, &self.gfx.fonts.bar);
            let (w, h) = (16.0 + 20.0 + 12.0 + tw + 20.0, 48.0);
            let r = rect(
                vw_d / 2.0 - w / 2.0,
                vh_d - 24.0 - h,
                vw_d / 2.0 + w / 2.0,
                vh_d - 24.0,
            );
            tt.fill_round(r, 6.0, if *ok { th.success_bg } else { th.error_bg });
            let (cx, cy) = (r.left + 26.0, (r.top + r.bottom) / 2.0);
            tt.fill_ellipse(cx, cy, 10.0, if *ok { th.success } else { th.error });
            let glyph = if *ok { "\u{E73E}" } else { "\u{E711}" };
            tt.text(
                glyph,
                &self.gfx.fonts.icon_small,
                rect(cx - 10.0, cy - 10.0, cx + 10.0, cy + 10.0),
                th.on_status,
            );
            tt.text(
                text,
                &self.gfx.fonts.bar,
                rect(cx + 22.0, cy - 10.0, r.right - 16.0, cy + 12.0),
                th.text,
            );
        }

        // Window-level chrome (DIPs, origin at the client's top-left)
        let t = self.gfx.target.as_ref().unwrap();
        t.set_scale(s);
        if self.bar_visible() && !self.settings_open {
            self.paint_bar(cw / s, ch / s, entry);
        }
        if self.title_h() > 0.0 {
            self.paint_title(cw / s);
        }
        if self.delete_prompt.is_some() {
            self.paint_delete_dialog(cw / s, ch / s);
        }

        let t = self.gfx.target.as_ref().unwrap();
        if let Err(e) = t.end() {
            if e.code() == D2DERR_RECREATE_TARGET {
                // Device lost: bitmaps belonged to the old target, so decode again.
                self.gfx.target = None;
                self.cache.clear();
                self.view_for = None;
                self.schedule();
                self.invalidate();
            }
        }
        // EndDraw waits for vsync, so this paces the animation at the display refresh rate.
        if animating {
            self.invalidate();
        }
    }

    fn paint_delete_dialog(&self, cw: f32, ch: f32) {
        let th = &self.theme;
        let t = self.gfx.target.as_ref().unwrap();
        let f = &self.gfx.fonts;
        let Some(path) = &self.delete_prompt else {
            return;
        };
        // Dim everything behind the dialog, like a Fluent ContentDialog.
        t.fill_rect(
            rect(0.0, 0.0, cw, ch),
            rgba(0, 0, 0, if th.dark { 0.45 } else { 0.3 }),
        );
        let (c, ok, cancel) = dialog_layout(cw, ch);
        t.fill_round(rect(c.0, c.1, c.2, c.3), 8.0, th.drawer);
        t.stroke_round(
            rect(c.0 + 0.5, c.1 + 0.5, c.2 - 0.5, c.3 - 0.5),
            8.0,
            1.0,
            th.divider,
        );
        t.text(
            "Delete this file?",
            &f.title,
            rect(c.0 + 24.0, c.1 + 20.0, c.2 - 24.0, c.1 + 52.0),
            th.text,
        );
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let body = format!("\u{201C}{name}\u{201D} will be moved to the Recycle Bin.");
        t.text(
            &body,
            &f.bar,
            rect(c.0 + 24.0, c.1 + 62.0, c.2 - 24.0, ok.1 - 8.0),
            th.text_dim,
        );

        let ok_hot = self.hover == Hit::DialogOk;
        t.fill_round(rect(ok.0, ok.1, ok.2, ok.3), 4.0, th.accent);
        if ok_hot {
            t.fill_round(rect(ok.0, ok.1, ok.2, ok.3), 4.0, rgba(255, 255, 255, 0.12));
        }
        t.text(
            "Delete",
            &f.center,
            rect(ok.0, ok.1, ok.2, ok.3),
            th.on_status,
        );
        let cancel_hot = self.hover == Hit::DialogCancel;
        t.fill_round(
            rect(cancel.0, cancel.1, cancel.2, cancel.3),
            4.0,
            if cancel_hot { th.active } else { th.subtle },
        );
        t.stroke_round(
            rect(
                cancel.0 + 0.5,
                cancel.1 + 0.5,
                cancel.2 - 0.5,
                cancel.3 - 0.5,
            ),
            4.0,
            1.0,
            th.divider,
        );
        t.text(
            "Cancel",
            &f.center,
            rect(cancel.0, cancel.1, cancel.2, cancel.3),
            if cancel_hot { th.active_text } else { th.text },
        );
    }
}
