//! Mouse input: hover, clicks, drags, and the double-click zoom.

use super::*;

impl App {
    // ---------- input ----------

    pub(in crate::app) fn on_mouse_move(&mut self, x: i32, y: i32) {
        if self.caption_hover != 0 {
            self.caption_hover = 0;
            self.caption_pressed = 0;
            self.invalidate();
        }
        if self.slider_drag {
            self.set_slider(x);
            return;
        }
        if let Some((lx, ly)) = self.drag {
            self.view.pan((x - lx) as f32, (y - ly) as f32);
            self.drag = Some((x, y));
            self.invalidate();
            return;
        }
        if !self.tracking_leave {
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            unsafe {
                let _ = TrackMouseEvent(&mut tme);
            }
            self.tracking_leave = true;
        }
        let hit = self.hit(x, y);
        if !self.overlay || hit != self.hover {
            self.overlay = true;
            self.hover = hit;
            self.invalidate();
        }
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_OVERLAY, 2000, None);
        }
    }

    pub(in crate::app) fn on_left_down(&mut self, x: i32, y: i32) {
        match self.hit(x, y) {
            Hit::Prev => self.go(-1),
            Hit::Next => self.go(1),
            Hit::Info | Hit::Close => self.toggle_drawer(),
            Hit::Link => self.want_reveal = true,
            Hit::Full => self.want_fullscreen = true,
            Hit::Fit if self.has_image() => {
                self.zoom_fit();
            }
            Hit::ZoomIn | Hit::ZoomOut => {
                let (vw, vh) = self.viewport();
                let factor = if self.hit(x, y) == Hit::ZoomIn {
                    1.25
                } else {
                    0.8
                };
                self.zoom_by(factor, vw / 2.0, vh / 2.0);
            }
            Hit::ZoomMenu => {
                let (cw, ch) = self.client_size();
                let m = bar_layout(cw / self.scale, ch / self.scale).menu;
                let min_pct = self.view.min_scale() / self.src_ratio() * 100.0;
                self.want_popup = Some(Popup::Zoom(
                    POINT {
                        x: (m.0 * self.scale) as i32,
                        y: (m.1 * self.scale) as i32,
                    },
                    min_pct,
                ));
            }
            Hit::More => {
                let m = more_rect();
                self.want_popup = Some(Popup::More(POINT {
                    x: (m.0 * self.scale) as i32,
                    y: (m.3 * self.scale) as i32,
                }));
            }
            Hit::Back => {
                self.settings_open = false;
                self.invalidate();
            }
            Hit::SetDefault => self.want_default_settings = true,
            Hit::PromptOpen => {
                self.want_default_settings = true;
                self.hide_default_prompt();
            }
            Hit::PromptDismiss => self.hide_default_prompt(),
            Hit::ConfirmToggle => {
                self.confirm_delete = !self.confirm_delete;
                settings::set_bool(CONFIRM_DELETE_SETTING, self.confirm_delete);
                self.invalidate();
            }
            Hit::DialogOk => {
                self.want_delete = self.delete_prompt.take();
                self.invalidate();
            }
            Hit::DialogCancel => {
                self.delete_prompt = None;
                self.invalidate();
            }
            Hit::ThemePick => {
                let (cw, _) = self.client_size();
                let d = settings_layout(cw / self.scale).dropdown;
                let y = (self.title_h() + d.3 - self.settings_scroll) * self.scale;
                self.want_popup = Some(Popup::Theme(POINT {
                    x: (d.0 * self.scale) as i32,
                    y: y as i32,
                }));
            }
            Hit::Slider if self.has_image() => {
                self.slider_drag = true;
                self.want_capture = true;
                self.set_slider(x);
            }
            Hit::Image if self.has_image() && self.view.can_pan() => {
                self.anim = None;
                self.drag = Some((x, y));
                self.want_capture = true;
            }
            _ => {}
        }
    }

    pub(in crate::app) fn on_double_click(&mut self, x: i32, y: i32) {
        match self.hit(x, y) {
            Hit::Image if self.has_image() => {
                if self.view.is_fit() {
                    let fit = self.view.fit_scale();
                    let actual = self.src_ratio();
                    let target = if fit < actual { actual } else { fit * 2.0 };
                    self.animate_zoom(target, x as f32, y as f32 - self.top_px());
                } else {
                    self.zoom_fit();
                }
            }
            // Fast clicking on the arrows should keep navigating.
            Hit::Prev
            | Hit::Next
            | Hit::Info
            | Hit::Close
            | Hit::ZoomIn
            | Hit::ZoomOut
            | Hit::Slider => self.on_left_down(x, y),
            _ => {}
        }
    }
}
