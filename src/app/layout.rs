//! DIP geometry for the title bar, status bar, settings page, dialog and drawer, shared by hit
//! testing and painting.

use super::*;

/// The ⋯ / back button at the left of the title bar (DIPs).
pub(super) fn more_rect() -> Rect {
    (8.0, 6.0, 48.0, 42.0)
}

/// Settings page geometry in page-local DIPs (y = 0 is just below the title bar).
pub(super) struct SettingsLayout {
    pub(super) x: f32,
    pub(super) w: f32,
    pub(super) card: Rect,
    pub(super) dropdown: Rect,
    /// "Ask for permission to delete photos" card and its On/Off toggle (label + switch).
    pub(super) delete_card: Rect,
    pub(super) toggle: Rect,
    /// "Default app" section: status card and its "Set as default…" button.
    pub(super) default_card: Rect,
    pub(super) default_button: Rect,
}

pub(super) fn settings_layout(cw: f32) -> SettingsLayout {
    let w = (cw - 80.0).min(1000.0);
    let x = (cw - w) / 2.0;
    let card = (x, 124.0, x + w, 124.0 + 68.0);
    let cy = (card.1 + card.3) / 2.0;
    let dropdown = (card.2 - 16.0 - 160.0, cy - 16.0, card.2 - 16.0, cy + 16.0);
    let delete_card = (x, card.3 + 4.0, x + w, card.3 + 4.0 + 68.0);
    let dy = (delete_card.1 + delete_card.3) / 2.0;
    let toggle = (
        delete_card.2 - 16.0 - 100.0,
        dy - 16.0,
        delete_card.2 - 16.0,
        dy + 16.0,
    );
    let default_card = (x, delete_card.3 + 58.0, x + w, delete_card.3 + 58.0 + 68.0);
    let ddy = (default_card.1 + default_card.3) / 2.0;
    let default_button = (
        default_card.2 - 16.0 - 150.0,
        ddy - 16.0,
        default_card.2 - 16.0,
        ddy + 16.0,
    );
    SettingsLayout {
        x,
        w,
        card,
        dropdown,
        delete_card,
        toggle,
        default_card,
        default_button,
    }
}

/// One-time default-app prompt in image-area DIPs (viewport `vw` x `vh`): card, "Open settings", "Not now".
pub(super) fn default_prompt_layout(vw: f32, vh: f32) -> (Rect, Rect, Rect) {
    let (w, h) = ((vw - 32.0).min(620.0), 56.0);
    let card = (
        vw / 2.0 - w / 2.0,
        vh - 24.0 - h,
        vw / 2.0 + w / 2.0,
        vh - 24.0,
    );
    let cy = (card.1 + card.3) / 2.0;
    let dismiss = (card.2 - 12.0 - 96.0, cy - 16.0, card.2 - 12.0, cy + 16.0);
    let open = (
        dismiss.0 - 8.0 - 128.0,
        cy - 16.0,
        dismiss.0 - 8.0,
        cy + 16.0,
    );
    (card, open, dismiss)
}

/// Delete confirmation dialog, centered in the client area (DIPs): card, Delete button, Cancel button.
pub(super) fn dialog_layout(cw: f32, ch: f32) -> (Rect, Rect, Rect) {
    let (w, h) = (440.0f32.min(cw - 32.0), 188.0);
    let (x, y) = ((cw - w) / 2.0, (ch - h) / 2.0);
    let card = (x, y, x + w, y + h);
    let by = card.3 - 24.0 - 32.0;
    let bw = (w - 24.0 * 2.0 - 8.0) / 2.0;
    let ok = (x + 24.0, by, x + 24.0 + bw, by + 32.0);
    let cancel = (ok.2 + 8.0, by, ok.2 + 8.0 + bw, by + 32.0);
    (card, ok, cancel)
}

pub(super) type Rect = (f32, f32, f32, f32);

pub(super) fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

/// Status bar geometry in DIPs, shared by painting and hit testing.
pub(super) struct Bar {
    pub(super) info: Rect,
    pub(super) fit: Rect,
    pub(super) menu: Rect,
    pub(super) zoom_out: Rect,
    pub(super) slider: Rect,
    pub(super) zoom_in: Rect,
    pub(super) full: Rect,
    pub(super) sep: f32,
    pub(super) left_end: f32,
}

pub(super) fn bar_layout(cw: f32, ch: f32) -> Bar {
    let cy = ch - BAR_H / 2.0;
    let b = |l: f32, w: f32| (l, cy - 16.0, l + w, cy + 16.0);
    let full = b(cw - 12.0 - 36.0, 36.0);
    let sep = full.0 - 8.0;
    let zoom_in = b(sep - 8.0 - 36.0, 36.0);
    let slider_w = if cw >= 1000.0 { 180.0 } else { 120.0 };
    let slider = (
        zoom_in.0 - 4.0 - slider_w,
        cy - 12.0,
        zoom_in.0 - 4.0,
        cy + 12.0,
    );
    let zoom_out = b(slider.0 - 4.0 - 36.0, 36.0);
    let menu = b(zoom_out.0 - 8.0 - 92.0, 92.0);
    let fit = b(menu.0 - 8.0 - 36.0, 36.0);
    // Centered like Photos when there's room, otherwise next to the zoom controls.
    let centered = b(cw / 2.0 - 18.0, 36.0);
    let info = if centered.2 + 16.0 < fit.0 {
        centered
    } else {
        b(fit.0 - 8.0 - 36.0, 36.0)
    };
    Bar {
        info,
        fit,
        menu,
        zoom_out,
        slider,
        zoom_in,
        full,
        sep,
        left_end: info.0 - 16.0,
    }
}

/// Slider thumb travel within the slider rect.
pub(super) fn slider_track(r: Rect) -> (f32, f32) {
    (r.0 + 9.0, r.2 - 9.0)
}

/// 40×40 DIPs: the minimum comfortable touch target.
pub(super) fn close_rect(cw: f32) -> (f32, f32, f32, f32) {
    (cw - 52.0, 11.0, cw - 12.0, 51.0)
}
