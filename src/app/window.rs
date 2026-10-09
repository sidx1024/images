//! Window creation, the custom frame's hit testing, window placement, and fullscreen.

use super::shell::open_dialog;
use super::wndproc::{flush_effects, wndproc};
use super::*;

pub(crate) fn run(path: Option<PathBuf>) -> Result<()> {
    let (tx, rx) = channel();
    let notifier = Notifier {
        tx,
        hwnd: Arc::new(AtomicIsize::new(0)),
    };
    // Start decoding before the window exists; that's most of the cold-start time.
    let loader = Loader::new(notifier.clone(), 2);
    if let Some(p) = &path {
        loader.set_jobs(vec![p.clone()]);
        folder::scan_async(p.clone(), 1, notifier.clone());
    }
    let theme_pref = settings::get_u32(THEME_SETTING).unwrap_or(theme::PREF_SYSTEM);
    let theme = theme::load(theme_pref);

    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("ImagesWindow");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: CreateSolidBrush(theme.bg_colorref()),
            lpszClassName: class,
            // Icon resource 1 from app.rc, at the large and small sizes Windows asks for.
            hIcon: app_icon(instance.into(), SM_CXICON),
            hIconSm: app_icon(instance.into(), SM_CXSMICON),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        // Reopen where the window was last closed (on that monitor, at that size), else centered on
        // the primary monitor. Creating it there directly gives it the right monitor's DPI from the start.
        let saved = saved_window();
        let (x, y, w, h) = match saved {
            Some((r, _)) => (r.left, r.top, r.right - r.left, r.bottom - r.top),
            None => {
                let mut work = RECT::default();
                let _ = SystemParametersInfoW(
                    SPI_GETWORKAREA,
                    0,
                    Some(&mut work as *mut _ as _),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                );
                let (ww, wh) = (work.right - work.left, work.bottom - work.top);
                let (w, h) = (ww * 3 / 4, wh * 4 / 5);
                (work.left + (ww - w) / 2, work.top + (wh - h) / 2, w, h)
            }
        };
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("Images"),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // Re-run the frame calculation so the native caption is gone before the window is first shown.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        theme::apply_to_window(hwnd, &theme);
        theme::apply_to_menus(&theme);
        DragAcceptFiles(hwnd, true);
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;

        let app = Box::new(App {
            hwnd,
            gfx: Gfx::new(theme.text_scale)?,
            loader,
            notifier: notifier.clone(),
            rx,
            files: path.iter().cloned().collect(),
            index: 0,
            generation: 1,
            cache: HashMap::new(),
            infos: HashMap::new(),
            view: View::default(),
            view_for: None,
            drawer: settings::get_bool(DRAWER_SETTING).unwrap_or(false),
            drag: None,
            overlay: false,
            hover: Hit::None,
            toast: false,
            tracking_leave: false,
            fullscreen: None,
            dir: 1,
            scale,
            drawer_scroll: 0.0,
            drawer_content: 0.0,
            title_dirty: true,
            want_capture: false,
            want_popup: None,
            want_reveal: false,
            want_fullscreen: false,
            slider_drag: false,
            link_rect: Cell::new(None),
            want_theme: false,
            theme,
            anim: None,
            touch: Recognizer::new(scale),
            touch_target: Hit::None,
            epoch: Instant::now(),
            theme_pref,
            settings_open: false,
            caption_hover: 0,
            caption_pressed: 0,
            active: true,
            notice: None,
            pending_clip: None,
            copy_latest: 0,
            copy_busy: false,
            copy_queued: None,
            confirm_delete: settings::get_bool(CONFIRM_DELETE_SETTING).unwrap_or(true),
            delete_prompt: None,
            want_delete: None,
            delete_busy: false,
            fit_window: settings::get_bool(FIT_WINDOW_SETTING).unwrap_or(false),
            default_status: None,
            settings_scroll: 0.0,
            default_prompt: false,
            default_prompt_checked: false,
            want_default_settings: false,
        });
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(app) as isize);
        flush_effects(hwnd);

        // Workers may have finished before the handle was published; make sure their results get drained.
        notifier.hwnd.store(hwnd.0 as isize, Ordering::Release);
        let _ = PostMessageW(Some(hwnd), WM_APP_LOADED, WPARAM(0), LPARAM(0));

        let maximized = saved.is_some_and(|(_, max)| max);
        let _ = ShowWindow(
            hwnd,
            if maximized {
                SW_SHOWMAXIMIZED
            } else {
                SW_SHOWNORMAL
            },
        );
        let _ = UpdateWindow(hwnd);

        if path.is_none() {
            if let Some(p) = open_dialog(hwnd) {
                if let Some(app) = app_mut(hwnd) {
                    app.open(p);
                }
            }
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

const WINDOW_SETTINGS: [&str; 5] = [
    "WindowLeft",
    "WindowTop",
    "WindowRight",
    "WindowBottom",
    "WindowMaximized",
];

/// Remember the window's normal (restored) rectangle in screen coordinates and whether it was
/// maximized. In fullscreen, the placement saved when entering fullscreen is what counts.
pub(super) unsafe fn save_window(hwnd: HWND, before_fullscreen: Option<WINDOWPLACEMENT>) {
    let wp = match before_fullscreen {
        Some(wp) => wp,
        None => {
            let mut wp = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            if GetWindowPlacement(hwnd, &mut wp).is_err() {
                return;
            }
            wp
        }
    };
    let maximized = wp.showCmd == SW_SHOWMAXIMIZED.0 as u32
        || (wp.showCmd == SW_SHOWMINIMIZED.0 as u32
            && (wp.flags.0 & WPF_RESTORETOMAXIMIZED.0) != 0);
    // rcNormalPosition is in workspace coordinates (relative to the work area, which differs when the
    // taskbar is on the top or left); convert to screen coordinates for CreateWindowEx.
    let mut r = wp.rcNormalPosition;
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
        let (dx, dy) = (
            mi.rcWork.left - mi.rcMonitor.left,
            mi.rcWork.top - mi.rcMonitor.top,
        );
        r = RECT {
            left: r.left + dx,
            top: r.top + dy,
            right: r.right + dx,
            bottom: r.bottom + dy,
        };
    }
    for (name, v) in
        WINDOW_SETTINGS
            .iter()
            .zip([r.left, r.top, r.right, r.bottom, maximized as i32])
    {
        settings::set_u32(name, v as u32);
    }
}

/// The saved window rectangle (screen coordinates) and maximized flag, if it is still usable:
/// a sane size and mostly on a monitor that is connected now.
unsafe fn saved_window() -> Option<(RECT, bool)> {
    let v: Vec<i32> = WINDOW_SETTINGS
        .iter()
        .map(|n| settings::get_u32(n).map(|v| v as i32))
        .collect::<Option<_>>()?;
    let r = RECT {
        left: v[0],
        top: v[1],
        right: v[2],
        bottom: v[3],
    };
    if r.right - r.left < 200 || r.bottom - r.top < 150 {
        return None;
    }
    // The title bar area must land on some monitor, or the window couldn't be grabbed.
    let title = RECT {
        left: r.left + 40,
        top: r.top,
        right: r.right - 40,
        bottom: r.top + 40,
    };
    let monitor = MonitorFromRect(&title, MONITOR_DEFAULTTONULL);
    if monitor.is_invalid() {
        return None;
    }
    // Shrink to the work area if the monitor got smaller (e.g. a resolution change).
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let r = if GetMonitorInfoW(monitor, &mut mi).as_bool() {
        let wa = mi.rcWork;
        let w = (r.right - r.left).min(wa.right - wa.left);
        let h = (r.bottom - r.top).min(wa.bottom - wa.top);
        let left = r.left.clamp(wa.left, wa.right - w);
        let top = r.top.clamp(wa.top, wa.bottom - h);
        RECT {
            left,
            top,
            right: left + w,
            bottom: top + h,
        }
    } else {
        r
    };
    Some((r, v[4] != 0))
}

unsafe fn app_icon(instance: HINSTANCE, metric: SYSTEM_METRICS_INDEX) -> HICON {
    let size = GetSystemMetrics(metric);
    LoadImageW(
        Some(instance),
        PCWSTR(1 as _),
        IMAGE_ICON,
        size,
        size,
        LR_DEFAULTCOLOR,
    )
    .map(|h| HICON(h.0))
    .unwrap_or_default()
}

impl App {
    /// WM_NCHITTEST for the custom title bar: caption buttons, drag area, and the top resize edge.
    pub(super) fn nc_hit_test(&self, sx: i32, sy: i32) -> u32 {
        if self.fullscreen.is_some() {
            return HTCLIENT;
        }
        let mut pt = POINT { x: sx, y: sy };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut pt);
        }
        let (cw_px, _) = self.client_size();
        let s = self.scale;
        let (x, y, cw) = (pt.x as f32 / s, pt.y as f32 / s, cw_px / s);
        let frame = unsafe {
            let dpi = GetDpiForWindow(self.hwnd);
            GetSystemMetricsForDpi(SM_CYFRAME, dpi) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        };
        let maximized = unsafe { IsZoomed(self.hwnd).as_bool() };
        if !maximized && pt.y < frame {
            return if pt.x < frame * 2 {
                HTTOPLEFT
            } else if pt.x as f32 > cw_px - (frame * 2) as f32 {
                HTTOPRIGHT
            } else {
                HTTOP
            };
        }
        if y >= TITLE_H {
            return HTCLIENT;
        }
        if x >= cw - CAPTION_W {
            HTCLOSE
        } else if x >= cw - CAPTION_W * 2.0 {
            // Returning HTMAXBUTTON is what makes Windows 11 show Snap Layouts on hover.
            HTMAXBUTTON
        } else if x >= cw - CAPTION_W * 3.0 {
            HTMINBUTTON
        } else if inside(more_rect(), x, y) {
            HTCLIENT
        } else {
            HTCAPTION
        }
    }
}

/// Borderless fullscreen on the current monitor (Raymond Chen's approach). Runs with no App borrow held
/// because SetWindowPos re-enters the window procedure.
pub(super) unsafe fn toggle_fullscreen(hwnd: HWND) {
    let restore = app_mut(hwnd).and_then(|a| a.fullscreen.take());
    if let Some((wp, style)) = restore {
        SetWindowLongPtrW(hwnd, GWL_STYLE, style);
        let _ = SetWindowPlacement(hwnd, &wp);
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
        );
        return;
    }
    let mut wp = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetWindowPlacement(hwnd, &mut wp).is_err()
        || !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut mi).as_bool()
    {
        return;
    }
    let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    if let Some(a) = app_mut(hwnd) {
        a.fullscreen = Some((wp, style));
    }
    SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_OVERLAPPEDWINDOW.0 as isize));
    let m = mi.rcMonitor;
    let _ = SetWindowPos(
        hwnd,
        Some(HWND_TOP),
        m.left,
        m.top,
        m.right - m.left,
        m.bottom - m.top,
        SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
    );
}
