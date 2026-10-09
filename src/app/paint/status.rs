//! Status bar: file details, the action buttons, and the zoom controls.

use super::*;

impl App {
    pub(super) fn paint_bar(&self, cw: f32, ch: f32, entry: Option<&Entry>) {
        let th = &self.theme;
        let t = self.gfx.target.as_ref().unwrap();
        let f = &self.gfx.fonts;
        let b = bar_layout(cw, ch);
        let top = ch - BAR_H;
        let cy = ch - BAR_H / 2.0;
        t.fill_rect(rect(0.0, top, cw, ch), th.bar);
        t.fill_rect(rect(0.0, top, cw, top + 1.0), th.divider);

        // Left: format | dimensions | file size. Items that don't fully fit are dropped, not clipped.
        if let (Some(path), Some(e)) = (self.current(), entry.filter(|e| e.full_width > 0)) {
            let ext = path
                .extension()
                .map(|x| x.to_string_lossy().to_uppercase())
                .unwrap_or_default();
            let dims = format!("{} x {}", e.full_width, e.full_height);
            let size = format_size(e.file_bytes);
            let mut x = 16.0;
            for (i, (icon, label)) in [
                (None, ext.as_str()),
                (Some("\u{E7A8}"), dims.as_str()),
                (Some("\u{E74E}"), size.as_str()),
            ]
            .into_iter()
            .enumerate()
            {
                let w = self.gfx.text_width(label, &f.status);
                let lead = if i > 0 { 13.0 } else { 0.0 } + if icon.is_some() { 25.0 } else { 0.0 };
                if x + lead + w > b.left_end {
                    break;
                }
                if i > 0 {
                    t.fill_rect(rect(x, cy - 8.0, x + 1.0, cy + 8.0), th.divider);
                    x += 13.0;
                }
                if let Some(icon) = icon {
                    t.text(
                        icon,
                        &f.icon_status,
                        rect(x, cy - 12.0, x + 20.0, cy + 12.0),
                        th.text_secondary,
                    );
                    x += 25.0;
                }
                t.text(
                    label,
                    &f.status,
                    rect(x, cy - 9.5, x + w + 1.0, cy + 12.0),
                    th.text_secondary,
                );
                x += w + 12.0;
            }
        }

        let button = |r: Rect, glyph: &str, hit: Hit, active: bool| {
            let hot = active || self.hover == hit;
            if hot {
                t.fill_round(
                    rect(r.0, r.1, r.2, r.3),
                    4.0,
                    if active { th.active } else { th.hover },
                );
            }
            t.text(
                glyph,
                &f.icon,
                rect(r.0, r.1, r.2, r.3),
                if hot { th.active_text } else { th.text },
            );
        };
        button(b.info, "\u{E946}", Hit::Info, self.drawer);
        if self.fit_active() {
            // Active: a rounded frame with a filled rectangle inside, as in Photos.
            let r = b.fit;
            t.fill_round(rect(r.0, r.1, r.2, r.3), 4.0, th.active);
            let (cx, cy) = ((r.0 + r.2) / 2.0, (r.1 + r.3) / 2.0);
            t.stroke_round(
                rect(cx - 8.5, cy - 6.5, cx + 8.5, cy + 6.5),
                2.5,
                1.2,
                th.active_text,
            );
            t.fill_round(
                rect(cx - 5.0, cy - 3.0, cx + 5.0, cy + 3.0),
                1.2,
                th.active_text,
            );
        } else {
            button(b.fit, "\u{E9A6}", Hit::Fit, false);
        }

        // Zoom percentage dropdown
        let m = b.menu;
        let hot = self.hover == Hit::ZoomMenu;
        t.fill_round(
            rect(m.0, m.1, m.2, m.3),
            4.0,
            if hot { th.active } else { th.subtle },
        );
        let fg = if hot { th.active_text } else { th.text };
        let pct = if self.has_image() {
            format!("{:.0}%", self.zoom_pct())
        } else {
            "—".into()
        };
        t.text(&pct, &f.center, rect(m.0, m.1, m.2 - 20.0, m.3), fg);
        t.text(
            "\u{E70D}",
            &f.icon_small,
            rect(m.2 - 26.0, m.1, m.2 - 6.0, m.3),
            fg,
        );

        button(b.zoom_out, "\u{E71F}", Hit::ZoomOut, false);
        button(b.zoom_in, "\u{E8A3}", Hit::ZoomIn, false);

        // Zoom slider (logarithmic between fit and max zoom)
        let (l, r) = slider_track(b.slider);
        let tx = l + (r - l) * self.slider_pos();
        t.fill_round(rect(l, cy - 2.0, r, cy + 2.0), 2.0, th.track);
        t.fill_round(rect(l, cy - 2.0, tx, cy + 2.0), 2.0, th.accent);
        t.fill_ellipse(tx, cy, 10.0, th.thumb_ring);
        let inner = if self.hover == Hit::Slider || self.slider_drag {
            7.0
        } else {
            5.0
        };
        t.fill_ellipse(tx, cy, inner, th.accent);

        t.fill_rect(rect(b.sep, cy - 12.0, b.sep + 1.0, cy + 12.0), th.divider);
        button(b.full, "\u{E740}", Hit::Full, false);
    }
}
