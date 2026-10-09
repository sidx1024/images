//! Navigation and the decode cache: which photo is current, what is decoded around it, folder
//! refreshes, notices, and the theme reload.

use super::*;

impl App {
    // ---------- state ----------

    pub(super) fn current(&self) -> Option<&PathBuf> {
        self.files.get(self.index)
    }

    pub(super) fn client_size(&self) -> (f32, f32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut rc);
        }
        (rc.right as f32, rc.bottom as f32)
    }

    pub(super) fn bar_visible(&self) -> bool {
        self.fullscreen.is_none()
    }

    /// Status bar height in DIPs (0 when hidden in fullscreen).
    pub(super) fn bar_h(&self) -> f32 {
        if self.bar_visible() {
            BAR_H
        } else {
            0.0
        }
    }

    /// Title bar height in DIPs (0 in fullscreen).
    pub(super) fn title_h(&self) -> f32 {
        if self.fullscreen.is_none() {
            TITLE_H
        } else {
            0.0
        }
    }

    /// Top of the image area in pixels.
    pub(super) fn top_px(&self) -> f32 {
        self.title_h() * self.scale
    }

    /// Image area in pixels (the client minus title bar, drawer and status bar). Its origin is at
    /// (0, top_px()); view coordinates are relative to that origin.
    pub(super) fn viewport(&self) -> (f32, f32) {
        let (w, h) = self.client_size();
        let drawer = if self.drawer {
            DRAWER_W * self.scale
        } else {
            0.0
        };
        (
            (w - drawer).max(1.0),
            (h - (self.bar_h() + self.title_h()) * self.scale).max(1.0),
        )
    }

    /// A page or dialog covers the viewer, so viewer shortcuts and gestures are inactive.
    pub(super) fn modal(&self) -> bool {
        self.settings_open || self.delete_prompt.is_some() || self.delete_busy
    }

    pub(super) fn set_theme_pref(&mut self, pref: u32) {
        self.theme_pref = pref;
        settings::set_u32(THEME_SETTING, pref);
        self.reload_theme();
    }

    pub(super) fn open(&mut self, path: PathBuf) {
        // Folders (e.g. dropped onto the window) aren't photos, and must never become a Delete target.
        if !path.is_file() {
            return;
        }
        self.generation += 1;
        self.files = vec![path.clone()];
        self.index = 0;
        self.cache.clear();
        self.infos.clear();
        self.view_for = None;
        self.anim = None;
        folder::scan_async(path.clone(), self.generation, self.notifier.clone());
        self.schedule();
        self.title_dirty = true;
        self.invalidate();
    }

    pub(super) fn go_to(&mut self, index: usize) {
        if self.files.is_empty() {
            return;
        }
        let index = index.min(self.files.len() - 1);
        if index == self.index {
            return;
        }
        self.dir = if index < self.index { -1 } else { 1 };
        self.index = index;
        self.drawer_scroll = 0.0;
        self.anim = None;
        self.schedule();
        self.title_dirty = true;
        self.invalidate();
    }

    pub(super) fn go(&mut self, delta: isize) {
        let i = self.index as isize + delta;
        if i >= 0 {
            self.go_to(i as usize);
        }
    }

    /// Indices kept decoded around the current one (±2). Empty when there are no files.
    fn window_range(&self) -> std::ops::Range<usize> {
        if self.files.is_empty() {
            return 0..0;
        }
        self.index.saturating_sub(2)..(self.index + 3).min(self.files.len())
    }

    /// Queue the current image first, then neighbours in the direction of travel; evict the rest.
    pub(super) fn schedule(&mut self) {
        if self.files.is_empty() {
            return;
        }
        let n = self.files.len() as isize;
        let i = self.index as isize;
        let d = self.dir.signum().max(-1);
        let d = if d == 0 { 1 } else { d };
        let wanted: Vec<PathBuf> = [0, d, -d, 2 * d]
            .iter()
            .map(|o| i + o)
            .filter(|j| (0..n).contains(j))
            .map(|j| self.files[j as usize].clone())
            .filter(|p| !self.cache.contains_key(p))
            .collect();
        self.loader.set_jobs(wanted);
        let cur = self.files[self.index].clone();
        if !self.infos.contains_key(&cur) {
            self.loader.request_info(cur);
        }
        self.evict();
    }

    fn evict(&mut self) {
        let keep: Vec<PathBuf> = self.files[self.window_range()].to_vec();
        self.cache.retain(|p, _| keep.contains(p));
        self.infos.retain(|p, _| keep.contains(p));
        let cur = self.current().cloned();
        let mut total: usize = self.cache.values().map(|e| e.bytes).sum();
        while total > CACHE_BUDGET {
            // Drop the entry farthest from the current index (never the current one).
            let victim = keep
                .iter()
                .enumerate()
                .filter(|(_, p)| Some(*p) != cur.as_ref() && self.cache.contains_key(*p))
                .max_by_key(|(k, _)| {
                    (*k as isize - (self.index - self.window_range().start) as isize).abs()
                })
                .map(|(_, p)| p.clone());
            match victim {
                Some(v) => total -= self.cache.remove(&v).map(|e| e.bytes).unwrap_or(0),
                None => break,
            }
        }
    }

    pub(super) fn drain(&mut self) {
        let mut changed = false;
        let mut reschedule = false;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Image { path, result } => {
                    if !self.files[self.window_range()].contains(&path) {
                        continue;
                    }
                    if self.gfx.ensure_target(self.hwnd).is_ok() {
                        if let Some(max) = self.gfx.max_bitmap_size() {
                            self.loader.max_dim.store(max, Ordering::Relaxed);
                        }
                    }
                    let entry = match result {
                        Ok(d) => {
                            let bitmap = self.gfx.create_bitmap(&d);
                            let max = self.gfx.max_bitmap_size().unwrap_or(u32::MAX);
                            if bitmap.is_none() && (d.width > max || d.height > max) {
                                // Decoded before the real GPU limit was known; decode again at the right size.
                                reschedule = true;
                                continue;
                            }
                            Entry {
                                error: bitmap
                                    .is_none()
                                    .then(|| "Couldn't upload image to the GPU".to_string()),
                                bitmap,
                                width: d.width,
                                height: d.height,
                                full_width: d.full_width,
                                full_height: d.full_height,
                                bytes: d.pixels.len(),
                                dpi: d.dpi,
                                bits_per_pixel: d.bits_per_pixel,
                                file_bytes: d.file_bytes,
                            }
                        }
                        Err(e) => Entry {
                            bitmap: None,
                            width: 0,
                            height: 0,
                            full_width: 0,
                            full_height: 0,
                            bytes: 0,
                            dpi: 0.0,
                            bits_per_pixel: 0,
                            file_bytes: 0,
                            error: Some(e),
                        },
                    };
                    let shown = entry.bitmap.is_some() && self.current() == Some(&path);
                    self.cache.insert(path.clone(), entry);
                    self.evict();
                    changed = true;
                    if shown {
                        self.maybe_prompt_default(&path);
                    }
                }
                Msg::Info { path, info } => {
                    if self.files[self.window_range()].contains(&path) {
                        self.infos.insert(path, info);
                        changed = true;
                    }
                }
                Msg::Clipboard { id, result } => self.on_copy_done(id, result),
                Msg::Folder {
                    generation,
                    mut files,
                } => {
                    if generation != self.generation {
                        continue;
                    }
                    let Some(cur) = self.current().cloned() else {
                        continue;
                    };
                    let key = cur.to_string_lossy().to_lowercase();
                    match files
                        .iter()
                        .position(|p| p.to_string_lossy().to_lowercase() == key)
                    {
                        Some(i) => {
                            // Keep our exact path so the cache entry still matches.
                            files[i] = cur;
                            self.index = i;
                        }
                        None => {
                            files.insert(0, cur);
                            self.index = 0;
                        }
                    }
                    self.files = files;
                    // Scheduled after the loop: decodes that already finished may still be queued behind this message.
                    reschedule = true;
                    self.title_dirty = true;
                    changed = true;
                }
            }
        }
        if reschedule {
            self.schedule();
        }
        if changed {
            self.invalidate();
        }
    }

    pub(super) fn title(&self) -> String {
        match self.current() {
            Some(p) => {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if self.files.len() > 1 {
                    format!("{name} ({}/{}) - Images", self.index + 1, self.files.len())
                } else {
                    format!("{name} - Images")
                }
            }
            None => "Images".into(),
        }
    }

    pub(super) fn invalidate(&self) {
        self.link_rect.set(None);
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Lazily create the view the first time the current image is shown.
    pub(super) fn sync_view(&mut self) {
        let Some(cur) = self.current().cloned() else {
            return;
        };
        if self.view_for.as_ref() == Some(&cur) {
            return;
        }
        if let Some(e) = self.cache.get(&cur) {
            if e.bitmap.is_some() {
                let (vw, vh) = self.viewport();
                self.view = View::new(e.width, e.height, vw, vh);
                if self.fit_window {
                    self.view.fit_window();
                }
                self.view_for = Some(cur);
                self.anim = None;
            }
        }
    }

    pub(super) fn has_image(&self) -> bool {
        self.view_for.is_some() && self.view_for.as_ref() == self.current()
    }

    pub(super) fn toggle_drawer(&mut self) {
        self.finish_anim();
        self.drawer = !self.drawer;
        settings::set_bool(DRAWER_SETTING, self.drawer);
        let (vw, vh) = self.viewport();
        self.view.resize(vw, vh);
        self.invalidate();
    }

    /// Height of the settings page content in DIPs.
    fn settings_content_h(&self) -> f32 {
        let (cw, _) = self.client_size();
        settings_layout(cw / self.scale).default_card.3 + 100.0
    }

    pub(super) fn scroll_settings_by(&mut self, dips: f32) {
        let (_, ch) = self.client_size();
        let visible = ch / self.scale - self.title_h();
        let max = (self.settings_content_h() - visible).max(0.0);
        self.settings_scroll = (self.settings_scroll + dips).clamp(0.0, max);
        self.invalidate();
    }

    /// Re-read how many file types Images is the default for (shown on the settings page).
    pub(super) fn refresh_default_status(&mut self) {
        self.default_status = Some(crate::platform::register::default_status());
        self.scroll_settings_by(0.0); // re-clamp for the current window size
        self.invalidate();
    }

    /// Offer, once ever, to make Images the default when the first photo it shows is of a type it's
    /// registered for but not the default for (Microsoft: prompt in context, respect the user's choice).
    pub(super) fn maybe_prompt_default(&mut self, path: &std::path::Path) {
        if self.default_prompt_checked {
            return;
        }
        self.default_prompt_checked = true;
        if settings::get_bool(DEFAULT_PROMPT_SETTING).unwrap_or(false) {
            return;
        }
        let Some(ext) = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        else {
            return;
        };
        // Only for types Images is registered for (so Settings can offer it), and only when Windows can
        // say for sure that it isn't already the default.
        if !crate::platform::register::is_registered_for(&ext)
            || crate::platform::register::is_default_for(&ext) != Some(false)
        {
            return;
        }
        // Recorded as shown when it's first painted (see paint), so a prompt hidden behind a page isn't lost.
        self.default_prompt = true;
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_PROMPT, 20_000, None);
        }
        self.invalidate();
    }

    pub(super) fn hide_default_prompt(&mut self) {
        self.default_prompt = false;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_PROMPT);
        }
        self.invalidate();
    }

    pub(super) fn show_notice(&mut self, text: &str, ok: bool) {
        self.notice = Some((text.to_string(), ok));
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_NOTICE, 2500, None);
        }
        self.invalidate();
    }

    pub(super) fn show_toast(&mut self) {
        self.toast = true;
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_TOAST, 1200, None);
        }
        self.invalidate();
    }

    pub(super) fn reload_theme(&mut self) {
        let old = std::mem::replace(&mut self.theme, theme::load(self.theme_pref));
        let new = &self.theme;
        if old.text_scale != new.text_scale {
            self.gfx.set_text_scale(new.text_scale);
        }
        if (old.dark, old.high_contrast) != (new.dark, new.high_contrast) {
            theme::apply_to_menus(new);
        }
        if (old.dark, old.high_contrast, old.bg) != (new.dark, new.high_contrast, new.bg) {
            self.want_theme = true;
        }
        if !new.animations {
            self.finish_anim();
        }
        self.invalidate();
    }
}
