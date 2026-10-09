//! Settings page, drawn below the title bar.

use super::*;

impl App {
    /// Settings page, drawn below the title bar over everything else. `ch` is its height in DIPs.
    pub(super) fn paint_settings(&self, cw: f32, ch: f32) {
        let th = &self.theme;
        let t = self.gfx.target.as_ref().unwrap();
        let f = &self.gfx.fonts;
        let l = settings_layout(cw);
        t.fill_rect(rect(0.0, 0.0, cw, ch), th.bg);
        // Scroll the page content; restored to the image-area transform at the end.
        let top = self.top_px();
        t.set_transform(self.scale, 0.0, top - self.settings_scroll * self.scale);

        t.text(
            "\u{E713}",
            &f.icon,
            rect(l.x, 28.0, l.x + 32.0, 64.0),
            th.text,
        );
        t.text(
            "Settings",
            &f.page_title,
            rect(l.x + 44.0, 22.0, l.x + l.w, 70.0),
            th.text,
        );
        t.text(
            "Personalisation",
            &f.section,
            rect(l.x, 94.0, l.x + l.w, 116.0),
            th.text,
        );

        let c = l.card;
        t.fill_round(rect(c.0, c.1, c.2, c.3), 6.0, th.drawer);
        t.text(
            "\u{E790}",
            &f.icon,
            rect(c.0 + 16.0, c.1, c.0 + 48.0, c.3),
            th.text,
        );
        t.text(
            "App theme",
            &f.bar,
            rect(c.0 + 64.0, c.1 + 14.0, c.2 - 200.0, c.1 + 34.0),
            th.text,
        );
        t.text(
            "Select which app theme to display",
            &f.small,
            rect(c.0 + 64.0, c.1 + 36.0, c.2 - 200.0, c.3),
            th.text_dim,
        );

        let d = l.dropdown;
        let hot = self.hover == Hit::ThemePick;
        t.fill_round(
            rect(d.0, d.1, d.2, d.3),
            4.0,
            if hot { th.active } else { th.subtle },
        );
        let dfg = if hot { th.active_text } else { th.text };
        t.text(
            theme_label(self.theme_pref),
            &f.bar,
            rect(d.0 + 12.0, d.1 + 6.0, d.2 - 28.0, d.3),
            dfg,
        );
        t.text(
            "\u{E70D}",
            &f.icon_small,
            rect(d.2 - 28.0, d.1, d.2 - 8.0, d.3),
            dfg,
        );

        let c = l.delete_card;
        t.fill_round(rect(c.0, c.1, c.2, c.3), 6.0, th.drawer);
        t.text(
            "\u{E74D}",
            &f.icon,
            rect(c.0 + 16.0, c.1, c.0 + 48.0, c.3),
            th.text,
        );
        t.text(
            "Ask for permission to delete photos",
            &f.bar,
            rect(c.0 + 64.0, c.1 + 14.0, c.2 - 140.0, c.1 + 34.0),
            th.text,
        );
        t.text(
            "Show a confirmation dialog before deleting a photo",
            &f.small,
            rect(c.0 + 64.0, c.1 + 36.0, c.2 - 140.0, c.3),
            th.text_dim,
        );
        // On/Off label + switch, Fluent style.
        let g = l.toggle;
        let on = self.confirm_delete;
        let cy = (g.1 + g.3) / 2.0;
        t.text(
            if on { "On" } else { "Off" },
            &f.bar,
            rect(g.0, cy - 10.0, g.2 - 52.0, cy + 12.0),
            th.text,
        );
        let track = rect(g.2 - 40.0, cy - 10.0, g.2, cy + 10.0);
        let hot = self.hover == Hit::ConfirmToggle;
        if on {
            t.fill_round(track, 10.0, th.accent);
            t.fill_ellipse(g.2 - 10.0, cy, if hot { 7.0 } else { 6.0 }, th.on_status);
        } else {
            t.stroke_round(
                rect(
                    track.left + 0.5,
                    track.top + 0.5,
                    track.right - 0.5,
                    track.bottom - 0.5,
                ),
                9.5,
                1.0,
                th.text_dim,
            );
            t.fill_ellipse(g.2 - 30.0, cy, if hot { 6.0 } else { 5.0 }, th.text_dim);
        }

        t.text(
            "Default app",
            &f.section,
            rect(l.x, c.3 + 28.0, l.x + l.w, c.3 + 50.0),
            th.text,
        );
        let c = l.default_card;
        t.fill_round(rect(c.0, c.1, c.2, c.3), 6.0, th.drawer);
        t.text(
            "\u{E7AC}",
            &f.icon,
            rect(c.0 + 16.0, c.1, c.0 + 48.0, c.3),
            th.text,
        );
        t.text(
            "Default photo viewer",
            &f.bar,
            rect(c.0 + 64.0, c.1 + 14.0, c.2 - 190.0, c.1 + 34.0),
            th.text,
        );
        let status = match self.default_status {
            Some(Some((_, 0))) => "Images isn't registered for any file types yet".to_string(),
            Some(Some((n, total))) if n == total => {
                format!("Images opens all {total} supported file types")
            }
            Some(Some((n, total))) => {
                format!("Images is the default for {n} of {total} supported file types")
            }
            Some(None) => "Couldn't check the current default apps".to_string(),
            None => "Choose which file types open with Images".to_string(),
        };
        t.text(
            &status,
            &f.small,
            rect(c.0 + 64.0, c.1 + 36.0, c.2 - 190.0, c.3),
            th.text_dim,
        );
        let b = l.default_button;
        let hot = self.hover == Hit::SetDefault;
        t.fill_round(
            rect(b.0, b.1, b.2, b.3),
            4.0,
            if hot { th.active } else { th.subtle },
        );
        t.stroke_round(
            rect(b.0 + 0.5, b.1 + 0.5, b.2 - 0.5, b.3 - 0.5),
            4.0,
            1.0,
            th.divider,
        );
        t.text(
            "Set as default\u{2026}",
            &f.center,
            rect(b.0, b.1, b.2, b.3),
            if hot { th.active_text } else { th.text },
        );

        t.text(
            "About",
            &f.section,
            rect(l.x, c.3 + 28.0, l.x + l.w, c.3 + 50.0),
            th.text,
        );
        let about = format!("Images {}", env!("CARGO_PKG_VERSION"));
        t.text(
            &about,
            &f.small,
            rect(l.x, c.3 + 54.0, l.x + l.w, c.3 + 74.0),
            th.text_dim,
        );
        t.set_transform(self.scale, 0.0, top);
    }
}
