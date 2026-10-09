//! Keyboard shortcuts for the viewer and the delete dialog.

use super::*;

impl App {
    /// Returns true if handled.
    pub(in crate::app) fn on_key(&mut self, vk: VIRTUAL_KEY) -> bool {
        if self.delete_prompt.is_some() {
            // The dialog is modal: Enter deletes, Esc cancels, everything else is ignored.
            match vk {
                VK_RETURN => self.want_delete = self.delete_prompt.take(),
                VK_ESCAPE => self.delete_prompt = None,
                _ => {}
            }
            self.invalidate();
            return true;
        }
        if self.settings_open {
            // The settings page swallows viewer shortcuts; Esc goes back.
            if vk == VK_ESCAPE {
                self.settings_open = false;
                self.invalidate();
            }
            return true;
        }
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let (vw, vh) = self.viewport();
        match vk {
            VK_LEFT => self.go(-1),
            VK_RIGHT => self.go(1),
            VK_HOME => self.go_to(0),
            VK_END => self.go_to(usize::MAX),
            VK_I => self.toggle_drawer(),
            VK_DELETE => self.request_delete(),
            VK_ESCAPE if self.drawer && self.fullscreen.is_none() => self.toggle_drawer(),
            VK_OEM_PLUS | VK_ADD => self.zoom_by(1.25, vw / 2.0, vh / 2.0),
            VK_OEM_MINUS | VK_SUBTRACT => self.zoom_by(0.8, vw / 2.0, vh / 2.0),
            VK_0 | VK_NUMPAD0 if ctrl && self.has_image() => self.set_fit_window(true),
            VK_1 | VK_NUMPAD1 if ctrl && self.has_image() => {
                self.animate_zoom(self.src_ratio(), vw / 2.0, vh / 2.0)
            }
            _ => return false,
        }
        true
    }
}
