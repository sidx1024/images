//! Popup menus shown from `flush_effects`, since TrackPopupMenu runs a modal loop.

use super::*;

/// Title bar ⋯ menu.
pub(super) unsafe fn more_menu(hwnd: HWND, mut pt: POINT, has_file: bool) -> Option<u32> {
    let _ = ClientToScreen(hwnd, &mut pt);
    let menu = CreatePopupMenu().ok()?;
    let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN as usize, w!("Open…\tCtrl+O"));
    let delete_flags = if has_file {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    let _ = AppendMenuW(menu, delete_flags, MENU_DELETE as usize, w!("Delete\tDel"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS as usize, w!("Settings"));
    let id = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_TOPALIGN | TPM_LEFTALIGN,
        pt.x,
        pt.y,
        None,
        hwnd,
        None,
    )
    .0;
    let _ = DestroyMenu(menu);
    (id > 0).then_some(id as u32)
}

/// App theme choices with the current one radio-checked; returns the chosen preference.
pub(super) unsafe fn theme_menu(hwnd: HWND, mut pt: POINT, current: u32) -> Option<u32> {
    const BASE: u32 = 100;
    let _ = ClientToScreen(hwnd, &mut pt);
    let menu = CreatePopupMenu().ok()?;
    let choices = [theme::PREF_LIGHT, theme::PREF_DARK, theme::PREF_SYSTEM];
    for pref in choices {
        let label = wide(theme_label(pref));
        let _ = AppendMenuW(
            menu,
            MF_STRING,
            (BASE + pref) as usize,
            PCWSTR(label.as_ptr()),
        );
    }
    let _ = CheckMenuRadioItem(menu, BASE, BASE + 2, BASE + current, MF_BYCOMMAND.0);
    let id = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_TOPALIGN | TPM_LEFTALIGN,
        pt.x,
        pt.y,
        None,
        hwnd,
        None,
    )
    .0;
    let _ = DestroyMenu(menu);
    (id as u32 >= BASE && id as u32 <= BASE + 2).then(|| id as u32 - BASE)
}

/// Zoom presets popup above the status-bar dropdown; returns the chosen id.
pub(super) unsafe fn zoom_menu(hwnd: HWND, mut pt: POINT, min_pct: f32) -> Option<u32> {
    let _ = ClientToScreen(hwnd, &mut pt);
    let menu = CreatePopupMenu().ok()?;
    let _ = AppendMenuW(menu, MF_STRING, ZOOM_FIT as usize, w!("Fit to window"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    for pct in [25u32, 50, 100, 200, 400, 800] {
        let label = if pct == 100 {
            "100% (actual size)".to_string()
        } else {
            format!("{pct}%")
        };
        let label = wide(label);
        // Below fit isn't reachable (zooming out stops at fit), so don't pretend it is.
        let flags = if (pct as f32) < min_pct - 0.5 {
            MF_STRING | MF_GRAYED
        } else {
            MF_STRING
        };
        let _ = AppendMenuW(menu, flags, pct as usize, PCWSTR(label.as_ptr()));
    }
    let id = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_BOTTOMALIGN | TPM_LEFTALIGN,
        pt.x,
        pt.y,
        None,
        hwnd,
        None,
    )
    .0;
    let _ = DestroyMenu(menu);
    (id > 0).then_some(id as u32)
}
