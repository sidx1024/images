//! Custom title bar: the ⋯ / back button, centered file name, and caption buttons.

use super::*;

impl App {
    pub(super) fn paint_title(&self, cw: f32) {
        let th = &self.theme;
        let t = self.gfx.target.as_ref().unwrap();
        let f = &self.gfx.fonts;
        let h = TITLE_H;
        t.fill_rect(rect(0.0, 0.0, cw, h), th.bg);
        let fg = if self.active { th.text } else { th.text_dim };

        let m = more_rect();
        let hot = matches!(self.hover, Hit::More | Hit::Back);
        if hot {
            t.fill_round(rect(m.0, m.1, m.2, m.3), 4.0, th.hover);
        }
        let glyph = if self.settings_open {
            "\u{E72B}"
        } else {
            "\u{E712}"
        };
        t.text(
            glyph,
            &f.icon,
            rect(m.0, m.1, m.2, m.3),
            if hot { th.active_text } else { fg },
        );

        // File name centered in the window, kept clear of the buttons on both sides.
        let name = self
            .current()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        let half = (cw / 2.0 - CAPTION_W * 3.0 - 8.0).max(0.0);
        t.text(
            name.as_deref().unwrap_or("Images"),
            &f.title_text,
            rect(cw / 2.0 - half, 0.0, cw / 2.0 + half, h),
            fg,
        );

        let maximized = unsafe { IsZoomed(self.hwnd).as_bool() };
        let buttons = [
            (HTMINBUTTON, "\u{E921}", 3.0),
            (
                HTMAXBUTTON,
                if maximized { "\u{E923}" } else { "\u{E922}" },
                2.0,
            ),
            (HTCLOSE, "\u{E8BB}", 1.0),
        ];
        for (code, glyph, slot) in buttons {
            let r = rect(cw - CAPTION_W * slot, 0.0, cw - CAPTION_W * (slot - 1.0), h);
            let hot = self.caption_hover == code;
            let pressed = hot && self.caption_pressed == code;
            let close = code == HTCLOSE && !th.high_contrast;
            if hot {
                let fill = if close {
                    rgba(196, 43, 28, if pressed { 0.9 } else { 1.0 })
                } else if pressed {
                    th.active
                } else {
                    th.hover
                };
                t.fill_rect(r, fill);
            }
            let color = if hot && close {
                rgba(255, 255, 255, 1.0)
            } else if hot {
                th.active_text
            } else {
                fg
            };
            t.text(glyph, &f.caption, r, color);
        }
    }
}
