//! Window, app state, input handling, and frame composition.
//!
//! `window` creates the window and handles placement and fullscreen, `wndproc` dispatches messages
//! and runs deferred effects, `state` covers navigation and the decode cache, `zoom` the zoom and
//! pan animation, `input` hit testing and the mouse, touch and keyboard handlers, `paint` the frame,
//! and `popup`, `delete`, `clipboard` and `shell` the matching Win32 features. `layout` holds the
//! DIP geometry shared by hit testing and painting.

mod clipboard;
mod delete;
mod input;
mod layout;
mod paint;
mod popup;
mod shell;
mod state;
mod window;
mod wndproc;
mod zoom;

pub(crate) use window::run;

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, D2D1_ANTIALIAS_MODE_ALIASED};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, CLSCTX_INPROC_SERVER,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::CF_DIB;
use windows::Win32::UI::HiDpi::{
    AdjustWindowRectExForDpi, GetDpiForWindow, GetSystemMetricsForDpi,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Input::Pointer::GetPointerType;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use self::layout::*;
use self::zoom::ZoomAnim;

use crate::decode::{self, format_size, ClipData, ImageInfo, Loader, Msg, Notifier, WM_APP_LOADED};
use crate::platform::settings;
use crate::platform::theme::{self, Theme};
use crate::render::{rect, rgba, Gfx};
use crate::touch::{Action, Recognizer};
use crate::view::{View, MAX_SCALE};
use crate::{folder, wide};

const DRAWER_W: f32 = 320.0; // DIP
const BAR_H: f32 = 48.0; // DIP
/// Custom title bar (Photos style): ⋯ button, centered file name, caption buttons.
const TITLE_H: f32 = 48.0; // DIP
const CAPTION_W: f32 = 46.0; // DIP, Windows 11 caption button width
const THEME_SETTING: &str = "Theme";
const CONFIRM_DELETE_SETTING: &str = "ConfirmDelete";
const FIT_WINDOW_SETTING: &str = "FitToWindow";
/// The one-time "make Images your default" prompt has been shown (it never shows again).
const DEFAULT_PROMPT_SETTING: &str = "DefaultPromptShown";
const MENU_OPEN: u32 = 1;
const MENU_SETTINGS: u32 = 2;
const MENU_DELETE: u32 = 3;
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_NCMOUSELEAVE: u32 = 0x02A2;
const WM_POINTERUPDATE: u32 = 0x0245;
const WM_POINTERDOWN: u32 = 0x0246;
const WM_POINTERUP: u32 = 0x0247;
const WM_POINTERCAPTURECHANGED: u32 = 0x024C;
/// Zoom changes animate over this long with an ease-out curve (see `step_anim`).
const ZOOM_ANIM: Duration = Duration::from_millis(400);
const DRAWER_SETTING: &str = "InfoPaneOpen";
const TIMER_OVERLAY: usize = 1;
const TIMER_TOAST: usize = 2;
const TIMER_THEME: usize = 3;
const TIMER_NOTICE: usize = 4;
const TIMER_PROMPT: usize = 5;
/// Decoded images kept around the current one (±2) are also capped by this many bytes.
const CACHE_BUDGET: usize = 1024 * 1024 * 1024;

struct Entry {
    bitmap: Option<ID2D1Bitmap>,
    width: u32,
    height: u32,
    full_width: u32,
    full_height: u32,
    bytes: usize,
    dpi: f64,
    bits_per_pixel: u32,
    file_bytes: u64,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    None,
    Prev,
    Next,
    Info,
    Close,
    Drawer,
    Link,
    Image,
    // status bar
    Bar,
    Fit,
    ZoomMenu,
    ZoomOut,
    ZoomIn,
    Slider,
    Full,
    // title bar (client part) and settings page
    More,
    Back,
    TitleBar,
    ThemePick,
    ConfirmToggle,
    SetDefault,
    Page,
    // one-time "make Images your default" prompt
    PromptOpen,
    PromptDismiss,
    // delete confirmation dialog
    DialogOk,
    DialogCancel,
    DialogArea,
}

impl Hit {
    fn is_button(self) -> bool {
        !matches!(
            self,
            Hit::None
                | Hit::Drawer
                | Hit::Image
                | Hit::Bar
                | Hit::TitleBar
                | Hit::Page
                | Hit::DialogArea
        )
    }
}

/// Popup menus are shown from `flush_effects` because TrackPopupMenu runs a modal loop.
enum Popup {
    /// Anchor (client px) and the smallest reachable zoom %.
    Zoom(POINT, f32),
    More(POINT),
    Theme(POINT),
}

fn theme_label(pref: u32) -> &'static str {
    match pref {
        theme::PREF_LIGHT => "Light",
        theme::PREF_DARK => "Dark",
        _ => "Windows default",
    }
}

struct App {
    hwnd: HWND,
    gfx: Gfx,
    loader: Loader,
    notifier: Notifier,
    rx: Receiver<Msg>,
    files: Vec<PathBuf>,
    index: usize,
    generation: u64,
    cache: HashMap<PathBuf, Entry>,
    infos: HashMap<PathBuf, ImageInfo>,
    view: View,
    view_for: Option<PathBuf>,
    drawer: bool,
    drag: Option<(i32, i32)>,
    overlay: bool,
    hover: Hit,
    toast: bool,
    tracking_leave: bool,
    fullscreen: Option<(WINDOWPLACEMENT, isize)>,
    dir: isize,
    scale: f32,
    drawer_scroll: f32,
    drawer_content: f32,
    // Win32 calls that synchronously re-enter the window procedure are deferred until the
    // current handler has released its `&mut App` (see `flush_effects`).
    title_dirty: bool,
    want_capture: bool,
    want_popup: Option<Popup>,
    want_reveal: bool,
    want_fullscreen: bool,
    slider_drag: bool,
    /// Clickable file-path link in the drawer, in DIPs; set during paint.
    link_rect: Cell<Option<Rect>>,
    want_theme: bool,
    theme: Theme,
    anim: Option<ZoomAnim>,
    touch: Recognizer,
    /// What the first finger of the current touch gesture landed on.
    touch_target: Hit,
    epoch: Instant,
    theme_pref: u32,
    settings_open: bool,
    /// Caption button under the mouse / being pressed, as an HT* code (0 = none).
    caption_hover: u32,
    caption_pressed: u32,
    active: bool,
    /// Bottom-center notification: text and whether it's a success.
    notice: Option<(String, bool)>,
    /// Clipboard contents ready to publish (done in flush_effects: EmptyClipboard re-enters wndproc).
    pending_clip: Option<ClipData>,
    /// Latest copy request id; results from older requests are dropped.
    copy_latest: u64,
    /// A copy is decoding; further requests wait in `copy_queued` (only the newest is kept).
    copy_busy: bool,
    copy_queued: Option<PathBuf>,
    /// Settings > Ask for permission to delete photos.
    confirm_delete: bool,
    /// Delete confirmation dialog is open for this file.
    delete_prompt: Option<PathBuf>,
    /// Confirmed deletion, performed from flush_effects (the shell file operation pumps messages).
    want_delete: Option<PathBuf>,
    /// A deletion is running; no other may start until it finishes.
    delete_busy: bool,
    /// Global "Fit to window" mode (status bar button): photos open fitted to the window, enlarging small ones.
    fit_window: bool,
    /// Settings > Default app: (file types Images is the default for, file types it's registered for);
    /// outer None = not checked yet, inner None = Windows couldn't tell for some type.
    default_status: Option<Option<(usize, usize)>>,
    /// Settings page scroll offset (DIPs), for windows too short to show the whole page.
    settings_scroll: f32,
    /// The one-time default-app prompt is showing.
    default_prompt: bool,
    /// The first displayed photo has been checked for the prompt (only that one is).
    default_prompt_checked: bool,
    /// Open Images' page in Settings > Default apps (from flush_effects: ShellExecute).
    want_default_settings: bool,
}

unsafe fn app_mut<'a>(hwnd: HWND) -> Option<&'a mut App> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App).as_mut()
}

const NAV_INSET: f32 = 36.0;
const ZOOM_FIT: u32 = 1;
