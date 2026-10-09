//! Touch input: maps gestures from the recognizer onto pan, pinch, swipe and tap actions.

use super::*;
use super::{WM_POINTERDOWN, WM_POINTERUPDATE};

impl App {
    // ---------- touch ----------

    pub(in crate::app) fn on_touch(&mut self, msg: u32, id: u32, x: i32, y: i32) {
        let (fx, fy) = (x as f32, y as f32);
        if msg == WM_POINTERDOWN && !self.touch.active() {
            self.touch_target = self.hit(x, y);
            self.anim = None;
        }
        self.touch.can_pan = match self.touch_target {
            Hit::Image => self.has_image() && self.view.can_pan(),
            Hit::Drawer | Hit::Link | Hit::Close => true,
            Hit::Page | Hit::ThemePick | Hit::ConfirmToggle | Hit::SetDefault => true,
            _ => false,
        };
        let actions = match msg {
            WM_POINTERDOWN => self.touch.down(id, fx, fy),
            WM_POINTERUPDATE => {
                if self.touch_target == Hit::Slider && self.touch.finger_count() == 1 {
                    self.set_slider(x);
                }
                self.touch.update(id, fx, fy)
            }
            _ => self
                .touch
                .up(id, fx, fy, self.epoch.elapsed().as_millis() as u64),
        };
        for action in actions {
            self.on_touch_action(action);
        }
    }

    fn on_touch_action(&mut self, action: Action) {
        let in_drawer = matches!(self.touch_target, Hit::Drawer | Hit::Link | Hit::Close);
        let on_image = self.touch_target == Hit::Image && self.has_image();
        match action {
            Action::Pan(_, dy) if in_drawer => self.scroll_drawer_by(-dy / self.scale),
            Action::Pan(_, dy) if self.settings_open => self.scroll_settings_by(-dy / self.scale),
            Action::Pan(dx, dy) if on_image => {
                self.view.pan(dx, dy);
                self.invalidate();
            }
            Action::Pinch { factor, from, to } if on_image => {
                let top = self.top_px();
                self.view
                    .pinch(factor, (from.0, from.1 - top), (to.0, to.1 - top));
                self.show_toast();
            }
            Action::Swipe(dir) if self.touch_target == Hit::Image => self.go(dir as isize),
            Action::Tap(x, y) => self.on_tap(x as i32, y as i32, false),
            Action::DoubleTap(x, y) => self.on_tap(x as i32, y as i32, true),
            _ => {}
        }
    }

    fn on_tap(&mut self, x: i32, y: i32, double: bool) {
        match self.hit(x, y) {
            Hit::Image if double => self.on_double_click(x, y),
            // No hover on touch: a tap shows or hides the navigation arrows instead.
            Hit::Image => {
                self.overlay = !self.overlay;
                if self.overlay {
                    unsafe {
                        SetTimer(Some(self.hwnd), TIMER_OVERLAY, 3000, None);
                    }
                }
                self.invalidate();
            }
            Hit::Slider => self.set_slider(x),
            _ => self.on_left_down(x, y),
        }
    }
}
