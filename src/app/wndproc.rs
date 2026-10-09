//! The window procedure: message dispatch, and the deferred Win32 effects run after each message.

use super::clipboard::set_clipboard;
use super::delete::recycle;
use super::popup::{more_menu, theme_menu, zoom_menu};
use super::shell::{dropped_file, open_dialog, reveal_in_explorer};
use super::window::{save_window, toggle_fullscreen};
use super::*;
use super::{
    WM_MOUSELEAVE, WM_NCMOUSELEAVE, WM_POINTERCAPTURECHANGED, WM_POINTERDOWN, WM_POINTERUP,
    WM_POINTERUPDATE,
};

fn lo(l: LPARAM) -> i32 {
    (l.0 & 0xffff) as i16 as i32
}
fn hi(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xffff) as i16 as i32
}

pub(super) extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        let r = handle(hwnd, msg, wparam, lparam);
        flush_effects(hwnd);
        r
    }
}

/// Performs deferred Win32 calls with no `&mut App` alive, since they re-enter `wndproc`.
pub(super) unsafe fn flush_effects(hwnd: HWND) {
    let Some(app) = app_mut(hwnd) else { return };
    let title = std::mem::take(&mut app.title_dirty).then(|| app.title());
    let capture = std::mem::take(&mut app.want_capture);
    let popup = app.want_popup.take();
    let clip = app.pending_clip.take();
    let delete = app.want_delete.take();
    let has_file = app.current().is_some();
    let theme_pref = app.theme_pref;
    let reveal = if std::mem::take(&mut app.want_reveal) {
        app.current().cloned()
    } else {
        None
    };
    let fullscreen = std::mem::take(&mut app.want_fullscreen);
    let default_settings = std::mem::take(&mut app.want_default_settings);
    let theme = std::mem::take(&mut app.want_theme).then(|| app.theme.clone());
    if default_settings {
        crate::platform::register::open_default_apps_settings(hwnd);
    }
    if let Some(theme) = theme {
        theme::apply_to_window(hwnd, &theme);
    }
    if capture {
        SetCapture(hwnd);
    }
    if let Some(t) = title {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(t).as_ptr()));
    }
    if fullscreen {
        toggle_fullscreen(hwnd);
    }
    if let Some(path) = delete {
        // The shell operation pumps messages; the busy flag keeps any of them from starting another delete.
        if let Some(app) = app_mut(hwnd) {
            app.delete_busy = true;
        }
        let result = recycle(hwnd, &path);
        if let Some(app) = app_mut(hwnd) {
            app.delete_busy = false;
            app.on_deleted(path, result);
        }
    }
    if let Some(data) = clip {
        let ok = set_clipboard(hwnd, data);
        if let Some(app) = app_mut(hwnd) {
            if ok {
                app.show_notice("Copied to clipboard.", true);
            } else {
                app.show_notice("Couldn't copy the image.", false);
            }
        }
    }
    if let Some(path) = reveal {
        reveal_in_explorer(&path);
    }
    match popup {
        Some(Popup::Zoom(pt, min_pct)) => {
            if let Some(id) = zoom_menu(hwnd, pt, min_pct) {
                if let Some(app) = app_mut(hwnd) {
                    app.apply_zoom_choice(id);
                }
            }
        }
        Some(Popup::More(pt)) => match more_menu(hwnd, pt, has_file) {
            Some(MENU_DELETE) => {
                if let Some(app) = app_mut(hwnd) {
                    app.request_delete();
                }
            }
            Some(MENU_OPEN) => {
                if let Some(p) = open_dialog(hwnd) {
                    if let Some(app) = app_mut(hwnd) {
                        app.open(p);
                    }
                }
            }
            Some(MENU_SETTINGS) => {
                if let Some(app) = app_mut(hwnd) {
                    // A gesture in progress keeps its original target; don't let it drive the viewer underneath.
                    app.touch.cancel();
                    app.touch_target = Hit::None;
                    app.drag = None;
                    app.settings_open = true;
                    app.settings_scroll = 0.0;
                    app.refresh_default_status();
                }
            }
            _ => {}
        },
        Some(Popup::Theme(pt)) => {
            if let Some(pref) = theme_menu(hwnd, pt, theme_pref) {
                if let Some(app) = app_mut(hwnd) {
                    app.set_theme_pref(pref);
                }
            }
        }
        None => {}
    }
}

pub(super) unsafe fn handle(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Remove the standard caption but keep the resize borders; the title bar is drawn by us.
    // Handled before the App exists so the very first frame calculation already uses it.
    if msg == WM_NCCALCSIZE {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 {
            // wParam TRUE passes NCCALCSIZE_PARAMS, FALSE (sent at creation) a bare RECT; both start with the rect.
            let r = if wparam.0 != 0 {
                &mut (*(lparam.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0]
            } else {
                &mut *(lparam.0 as *mut RECT)
            };
            let dpi = GetDpiForWindow(hwnd);
            let pad = GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi);
            let fx = GetSystemMetricsForDpi(SM_CXFRAME, dpi) + pad;
            let fy = GetSystemMetricsForDpi(SM_CYFRAME, dpi) + pad;
            r.left += fx;
            r.right -= fx;
            r.bottom -= fy;
            // Maximized windows extend past the monitor by the frame size; keep the title bar on screen.
            if IsZoomed(hwnd).as_bool() {
                r.top += fy;
            }
            return LRESULT(0);
        }
    }
    {
        let Some(app) = app_mut(hwnd) else {
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        };
        match msg {
            WM_PAINT => {
                // Validate first: a device-loss recovery inside paint() re-invalidates and must not be cancelled.
                let _ = ValidateRect(Some(hwnd), None);
                app.paint();
                LRESULT(0)
            }
            WM_ERASEBKGND if app.gfx.target.is_some() => LRESULT(1),
            WM_SIZE => {
                let (w, h) = (
                    (lparam.0 & 0xffff) as u32,
                    ((lparam.0 >> 16) & 0xffff) as u32,
                );
                app.gfx.resize(w, h);
                app.finish_anim();
                let (vw, vh) = app.viewport();
                app.view.resize(vw, vh);
                app.invalidate();
                LRESULT(0)
            }
            WM_APP_LOADED => {
                app.drain();
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                app.on_mouse_move(lo(lparam), hi(lparam));
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                app.tracking_leave = false;
                if app.drag.is_none() {
                    app.overlay = false;
                    app.hover = Hit::None;
                    app.invalidate();
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                app.on_left_down(lo(lparam), hi(lparam));
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                app.on_double_click(lo(lparam), hi(lparam));
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                app.caption_pressed = 0;
                let slider = std::mem::take(&mut app.slider_drag);
                if app.drag.take().is_some() || slider {
                    let _ = ReleaseCapture();
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                app.drag = None;
                app.slider_drag = false;
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let mut pt = POINT {
                    x: lo(lparam),
                    y: hi(lparam),
                };
                let _ = ScreenToClient(hwnd, &mut pt);
                let delta = ((wparam.0 >> 16) & 0xffff) as i16 as f32;
                if matches!(app.hit(pt.x, pt.y), Hit::Drawer | Hit::Close | Hit::Link) {
                    app.scroll_drawer(delta);
                } else if app.settings_open && app.delete_prompt.is_none() {
                    app.scroll_settings_by(-delta / 120.0 * 48.0);
                } else {
                    if !app.modal() {
                        app.zoom_by(
                            1.2f32.powf(delta / 120.0),
                            pt.x as f32,
                            pt.y as f32 - app.top_px(),
                        );
                    }
                }
                LRESULT(0)
            }
            WM_NCHITTEST => {
                let d = DefWindowProcW(hwnd, msg, wparam, lparam);
                if d.0 != HTCLIENT as isize {
                    return d; // side and bottom resize borders
                }
                LRESULT(app.nc_hit_test(lo(lparam), hi(lparam)) as isize)
            }
            WM_NCMOUSEMOVE => {
                let code = wparam.0 as u32;
                let code = if matches!(code, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) {
                    code
                } else {
                    0
                };
                if code != app.caption_hover {
                    app.caption_hover = code;
                    app.invalidate();
                }
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE | TME_NONCLIENT,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_NCMOUSELEAVE => {
                if app.caption_hover != 0 || app.caption_pressed != 0 {
                    app.caption_hover = 0;
                    app.caption_pressed = 0;
                    app.invalidate();
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            // Our caption buttons: swallow the press (DefWindowProc would draw classic buttons) and act on release.
            WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK
                if matches!(wparam.0 as u32, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) =>
            {
                app.caption_pressed = wparam.0 as u32;
                app.invalidate();
                LRESULT(0)
            }
            WM_NCLBUTTONUP => {
                // Any release ends a caption press, even over the title text; act only on the same button.
                let code = wparam.0 as u32;
                let pressed = std::mem::take(&mut app.caption_pressed);
                if pressed != 0 {
                    app.invalidate();
                }
                if !matches!(code, HTMINBUTTON | HTMAXBUTTON | HTCLOSE) {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                if pressed == code {
                    let cmd = match code {
                        HTMINBUTTON => SC_MINIMIZE,
                        HTMAXBUTTON if IsZoomed(hwnd).as_bool() => SC_RESTORE,
                        HTMAXBUTTON => SC_MAXIMIZE,
                        _ => SC_CLOSE,
                    };
                    // Posted, so the resulting size/close messages don't re-enter this handler.
                    let _ =
                        PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(cmd as usize), LPARAM(0));
                }
                LRESULT(0)
            }
            WM_ACTIVATE => {
                app.active = (wparam.0 & 0xffff) as u32 != WA_INACTIVE;
                if !app.active {
                    app.caption_pressed = 0;
                    app.caption_hover = 0;
                } else if app.settings_open {
                    // Back from Windows Settings: show the updated default-app status.
                    app.refresh_default_status();
                }
                app.invalidate();
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP => {
                // Touch gets our own gestures; pen and mouse fall through and arrive as mouse messages.
                let id = (wparam.0 & 0xffff) as u32;
                let mut kind = POINTER_INPUT_TYPE::default();
                if GetPointerType(id, &mut kind).is_err() || kind != PT_TOUCH {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                // POINTER_MESSAGE_FLAG_CANCELED: Windows aborted this contact (e.g. an edge swipe took it).
                if ((wparam.0 >> 16) & 0x8000) != 0 {
                    app.touch.cancel();
                    return LRESULT(0);
                }
                let mut pt = POINT {
                    x: lo(lparam),
                    y: hi(lparam),
                };
                let _ = ScreenToClient(hwnd, &mut pt);
                app.on_touch(msg, id, pt.x, pt.y);
                LRESULT(0)
            }
            WM_POINTERCAPTURECHANGED => {
                app.touch.cancel();
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
                // Light/dark, accent, high contrast, text size and animation settings can all change live.
                // These arrive in bursts, so reload once after they settle.
                SetTimer(Some(hwnd), TIMER_THEME, 150, None);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_XBUTTONDOWN => {
                let button = ((wparam.0 >> 16) & 0xffff) as u16;
                if !app.modal() {
                    app.go(if button == XBUTTON1 { -1 } else { 1 });
                }
                LRESULT(1)
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wparam.0 as u16);
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                // Viewer shortcuts stay inactive behind the settings page (fullscreen would hide its Back button).
                let viewer = !app.modal();
                if viewer
                    && (vk == VK_F11
                        || (vk == VK_F && !ctrl)
                        || (vk == VK_ESCAPE && app.fullscreen.is_some()))
                {
                    toggle_fullscreen(hwnd);
                } else if vk == VK_O && ctrl && viewer {
                    if let Some(p) = open_dialog(hwnd) {
                        if let Some(app) = app_mut(hwnd) {
                            app.open(p);
                        }
                    }
                } else if vk == VK_DELETE && lparam.0 & (1 << 30) != 0 {
                    // Held key: one delete per press, never a run through the folder.
                } else if vk == VK_C && ctrl && viewer {
                    if lparam.0 & (1 << 30) == 0 {
                        app.copy_image();
                    }
                } else if vk == VK_W && ctrl {
                    let _ = DestroyWindow(hwnd);
                } else if !app.on_key(vk) {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                }
                LRESULT(0)
            }
            WM_SYSKEYDOWN if wparam.0 as u16 == VK_RETURN.0 && !app.modal() => {
                app.toggle_drawer(); // Alt+Enter, as in Photos
                LRESULT(0)
            }
            WM_TIMER => {
                match wparam.0 {
                    TIMER_OVERLAY
                        if !matches!(app.hover, Hit::Prev | Hit::Next) && app.drag.is_none() =>
                    {
                        let _ = KillTimer(Some(hwnd), TIMER_OVERLAY);
                        app.overlay = false;
                        app.invalidate();
                    }
                    TIMER_PROMPT => app.hide_default_prompt(),
                    TIMER_NOTICE => {
                        let _ = KillTimer(Some(hwnd), TIMER_NOTICE);
                        app.notice = None;
                        app.invalidate();
                    }
                    TIMER_THEME => {
                        let _ = KillTimer(Some(hwnd), TIMER_THEME);
                        app.reload_theme();
                    }
                    TIMER_TOAST => {
                        let _ = KillTimer(Some(hwnd), TIMER_TOAST);
                        app.toast = false;
                        app.invalidate();
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            WM_SETCURSOR if (lparam.0 & 0xffff) as u32 == HTCLIENT => {
                let id = if app.drag.is_some() {
                    IDC_SIZEALL
                } else if app.hover.is_button() {
                    IDC_HAND
                } else {
                    IDC_ARROW
                };
                if let Ok(c) = LoadCursorW(None, id) {
                    SetCursor(Some(c));
                }
                LRESULT(1)
            }
            WM_DROPFILES => {
                // Always release the drop; ignore it while a dialog or delete is active.
                let dropped = dropped_file(HDROP(wparam.0 as _));
                if let (Some(p), false) = (dropped, app.modal()) {
                    app.open(p);
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                app.scale = ((wparam.0 >> 16) & 0xffff) as f32 / 96.0;
                app.touch.set_scale(app.scale);
                let fullscreen = app.fullscreen.is_some();
                app.invalidate();
                // Fullscreen already covers the monitor; the suggested rect is for normal windows.
                if !fullscreen {
                    let r = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let mmi = &mut *(lparam.0 as *mut MINMAXINFO);
                let s = app.scale;
                let mut r = RECT {
                    left: 0,
                    top: 0,
                    right: (720.0 * s) as i32,
                    bottom: (360.0 * s) as i32,
                };
                let style = WINDOW_STYLE(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32);
                let _ = AdjustWindowRectExForDpi(
                    &mut r,
                    style,
                    false,
                    WINDOW_EX_STYLE::default(),
                    (s * 96.0) as u32,
                );
                mmi.ptMinTrackSize = POINT {
                    x: r.right - r.left,
                    y: r.bottom - r.top,
                };
                LRESULT(0)
            }
            WM_DESTROY => {
                save_window(hwnd, app.fullscreen.map(|(wp, _)| wp));
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
