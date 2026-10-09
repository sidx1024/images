//! Info drawer: file name, date, size, camera and exposure details, and the location link.

use super::*;

impl App {
    /// Returns the bottom of the laid-out content (DIPs, unscrolled) for scroll clamping.
    pub(super) fn paint_drawer(
        &self,
        cw: f32,
        ch: f32,
        entry: Option<&Entry>,
        info: Option<&ImageInfo>,
    ) -> f32 {
        let th = &self.theme;
        let t = self.gfx.target.as_ref().unwrap();
        let f = &self.gfx.fonts;
        let x0 = cw - DRAWER_W;
        t.fill_rect(rect(x0, 0.0, cw, ch), th.drawer);
        t.fill_rect(rect(x0, 0.0, x0 + 1.0, ch), th.divider);

        // Header heights follow the Windows text-size setting.
        let title_h = self.gfx.text_height("Info", &f.title, DRAWER_W - 80.0);
        t.text(
            "Info",
            &f.title,
            rect(x0 + 20.0, 14.0, cw - 60.0, 14.0 + title_h),
            th.text,
        );
        let (l, tp, r, b) = close_rect(cw);
        let hot = self.hover == Hit::Close;
        if hot {
            t.fill_round(rect(l, tp, r, b), 4.0, th.hover);
        }
        t.text(
            "\u{E711}",
            &f.icon,
            rect(l, tp, r, b),
            if hot { th.active_text } else { th.text },
        );

        let Some(path) = self.current() else {
            return 0.0;
        };
        let width = DRAWER_W - 40.0;
        let x = x0 + 20.0;
        let top = (14.0 + title_h + 10.0).max(60.0);
        let mut y = top + 8.0 - self.drawer_scroll;
        unsafe {
            t.rt.PushAxisAlignedClip(&rect(x0, top, cw, ch), D2D1_ANTIALIAS_MODE_ALIASED)
        };
        let label = |y: &mut f32, text: &str| {
            let h = self.gfx.text_height(text, &f.label, width);
            t.text(text, &f.label, rect(x, *y, x + width, *y + h), th.text_dim);
            *y += h + 2.0;
        };
        let field = |y: &mut f32, name: &str, value: &str| {
            label(y, name);
            let h = self.gfx.text_height(value, &f.body, width);
            t.text(value, &f.body, rect(x, *y, x + width, *y + h), th.text);
            *y += h + 16.0;
        };

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        field(&mut y, "File name", &name);

        match info {
            Some(i) if i.date_taken.is_some() => {
                field(&mut y, "Date taken", i.date_taken.as_deref().unwrap())
            }
            Some(i) if i.date_modified.is_some() => {
                field(&mut y, "Date modified", i.date_modified.as_deref().unwrap())
            }
            _ => {}
        }

        if let Some(e) = entry.filter(|e| e.full_width > 0) {
            let mp = e.full_width as f64 * e.full_height as f64 / 1e6;
            let mut extra = vec![format_size(e.file_bytes)];
            if e.dpi > 0.0 {
                extra.push(format!("{:.0} dpi", e.dpi));
            }
            if e.bits_per_pixel > 0 {
                extra.push(format!("{} bit", e.bits_per_pixel));
            }
            field(
                &mut y,
                "Size",
                &format!(
                    "{} × {} ({:.1} MP)\n{}",
                    e.full_width,
                    e.full_height,
                    mp,
                    extra.join("   ")
                ),
            );
        }

        if let Some(i) = info {
            if let Some(v) = &i.camera {
                field(&mut y, "Camera", v);
            }
            if let Some(v) = &i.lens {
                field(&mut y, "Lens", v);
            }
            let exposure: Vec<&str> = [&i.f_number, &i.exposure, &i.focal_length, &i.iso]
                .iter()
                .filter_map(|v| v.as_deref())
                .collect();
            if !exposure.is_empty() {
                field(&mut y, "Exposure", &exposure.join("   "));
            }
            if let Some(v) = &i.flash {
                field(&mut y, "Flash", v);
            }
            if let Some(v) = &i.author {
                field(&mut y, "Author", v);
            }
            if let Some(v) = &i.program {
                field(&mut y, "Software", v);
            }
        }

        // Full path as a link that reveals the file in Explorer.
        let full = path.to_string_lossy();
        label(&mut y, "Location");
        let h = self.gfx.text_height(&full, &f.body, width);
        let link = if self.hover == Hit::Link {
            th.link_hover
        } else {
            th.link
        };
        t.text(&full, &f.body, rect(x, y, x + width, y + h), link);
        if y + h > top && y < ch {
            self.link_rect
                .set(Some((x, y.max(top), x + width, (y + h).min(ch))));
        }
        y += h + 16.0;

        unsafe { t.rt.PopAxisAlignedClip() };
        y + self.drawer_scroll
    }
}
