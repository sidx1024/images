//! Follows Windows personalization: light/dark app mode, accent color, high contrast,
//! text size, and the "Animation effects" setting.

use std::sync::OnceLock;

use windows::core::{w, BOOL, PCSTR};
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
};
use windows::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT,
    COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, SYS_COLOR_INDEX,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};
use windows::UI::ViewManagement::{UIColorType, UISettings};

use crate::render::rgba;

type C = D2D1_COLOR_F;

#[derive(Clone)]
pub struct Theme {
    pub dark: bool,
    pub high_contrast: bool,
    /// Settings > Accessibility > Visual effects > Animation effects.
    pub animations: bool,
    /// Settings > Accessibility > Text size (1.0 – 2.25).
    pub text_scale: f32,
    pub bg: C,
    pub bar: C,
    pub drawer: C,
    pub text: C,
    pub text_dim: C,
    /// Status bar details: softer than body text, stronger than `text_dim`.
    pub text_secondary: C,
    pub divider: C,
    pub hover: C,
    pub active: C,
    /// Foreground for anything drawn on `hover`/`active`/`overlay_hover` (differs in high contrast).
    pub active_text: C,
    /// Resting fill for the zoom dropdown.
    pub subtle: C,
    pub accent: C,
    pub link: C,
    pub link_hover: C,
    pub overlay: C,
    pub overlay_hover: C,
    pub track: C,
    pub thumb_ring: C,
    /// Notifications (Fluent InfoBar colors): background, icon circle, glyph on the circle.
    pub success_bg: C,
    pub success: C,
    pub error_bg: C,
    pub error: C,
    pub on_status: C,
}

fn to_colorref(c: C) -> COLORREF {
    let b = |v: f32| (v * 255.0).round() as u32;
    COLORREF(b(c.r) | b(c.g) << 8 | b(c.b) << 16)
}

impl Theme {
    pub fn bg_colorref(&self) -> COLORREF {
        to_colorref(self.bg)
    }
}

fn sys(i: SYS_COLOR_INDEX) -> C {
    let c = unsafe { GetSysColor(i) };
    rgba(
        (c & 0xff) as u8,
        ((c >> 8) & 0xff) as u8,
        ((c >> 16) & 0xff) as u8,
        1.0,
    )
}

fn high_contrast_on() -> bool {
    let mut hc = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some(&mut hc as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && (hc.dwFlags & HCF_HIGHCONTRASTON) == HCF_HIGHCONTRASTON
    }
}

fn animations_on() -> bool {
    let mut on = BOOL(1);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut _ as _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    on.as_bool()
}

/// App theme preference (Settings > App theme), stored as a number.
pub const PREF_SYSTEM: u32 = 0;
pub const PREF_LIGHT: u32 = 1;
pub const PREF_DARK: u32 = 2;

pub fn load(pref: u32) -> Theme {
    let ui = UISettings::new().ok();
    let color = |t: UIColorType| {
        ui.as_ref()
            .and_then(|u| u.GetColorValue(t).ok())
            .map(|c| rgba(c.R, c.G, c.B, 1.0))
    };
    let dark = match pref {
        PREF_LIGHT => false,
        PREF_DARK => true,
        // Documented way to detect app mode: the system background color is black in dark mode.
        _ => color(UIColorType::Background)
            .map(|c| c.r < 0.5)
            .unwrap_or(true),
    };
    let text_scale = ui
        .as_ref()
        .and_then(|u| u.TextScaleFactor().ok())
        .unwrap_or(1.0) as f32;
    let animations = animations_on();

    if high_contrast_on() {
        let (window, text, highlight) = (
            sys(COLOR_WINDOW),
            sys(COLOR_WINDOWTEXT),
            sys(COLOR_HIGHLIGHT),
        );
        return Theme {
            dark,
            high_contrast: true,
            animations,
            text_scale,
            bg: window,
            bar: window,
            drawer: window,
            text,
            text_dim: text,
            text_secondary: text,
            divider: text,
            hover: highlight,
            active: highlight,
            active_text: sys(COLOR_HIGHLIGHTTEXT),
            subtle: window,
            accent: highlight,
            link: sys(COLOR_HOTLIGHT),
            link_hover: highlight,
            overlay: sys(COLOR_BTNFACE),
            overlay_hover: highlight,
            track: sys(COLOR_GRAYTEXT),
            thumb_ring: window,
            success_bg: window,
            success: highlight,
            error_bg: window,
            error: highlight,
            on_status: sys(COLOR_HIGHLIGHTTEXT),
        };
    }

    if dark {
        Theme {
            dark,
            high_contrast: false,
            animations,
            text_scale,
            bg: rgba(32, 32, 32, 1.0),
            bar: rgba(28, 28, 28, 1.0),
            drawer: rgba(43, 43, 43, 1.0),
            text: rgba(255, 255, 255, 1.0),
            text_dim: rgba(255, 255, 255, 0.6),
            text_secondary: rgba(255, 255, 255, 0.78),
            divider: rgba(255, 255, 255, 0.12),
            hover: rgba(255, 255, 255, 0.07),
            active: rgba(255, 255, 255, 0.12),
            active_text: rgba(255, 255, 255, 1.0),
            subtle: rgba(255, 255, 255, 0.05),
            // Fluent uses the lighter accent shades on dark backgrounds.
            accent: color(UIColorType::AccentLight2).unwrap_or(rgba(76, 194, 255, 1.0)),
            link: color(UIColorType::AccentLight2).unwrap_or(rgba(120, 210, 255, 1.0)),
            link_hover: color(UIColorType::AccentLight3).unwrap_or(rgba(180, 232, 255, 1.0)),
            overlay: rgba(45, 45, 45, 0.85),
            overlay_hover: rgba(70, 70, 70, 0.95),
            track: rgba(255, 255, 255, 0.35),
            thumb_ring: rgba(69, 69, 69, 1.0),
            success_bg: rgba(57, 61, 27, 1.0),
            success: rgba(108, 203, 95, 1.0),
            error_bg: rgba(68, 39, 38, 1.0),
            error: rgba(255, 153, 164, 1.0),
            on_status: rgba(0, 0, 0, 0.9),
        }
    } else {
        Theme {
            dark,
            high_contrast: false,
            animations,
            text_scale,
            bg: rgba(243, 243, 243, 1.0),
            bar: rgba(249, 249, 249, 1.0),
            drawer: rgba(251, 251, 251, 1.0),
            text: rgba(0, 0, 0, 0.9),
            text_dim: rgba(0, 0, 0, 0.6),
            text_secondary: rgba(0, 0, 0, 0.72),
            divider: rgba(0, 0, 0, 0.1),
            hover: rgba(0, 0, 0, 0.05),
            active: rgba(0, 0, 0, 0.08),
            active_text: rgba(0, 0, 0, 0.9),
            subtle: rgba(0, 0, 0, 0.04),
            // ...and the darker shades on light backgrounds.
            accent: color(UIColorType::AccentDark1).unwrap_or(rgba(0, 95, 184, 1.0)),
            link: color(UIColorType::AccentDark2).unwrap_or(rgba(0, 62, 146, 1.0)),
            link_hover: color(UIColorType::AccentDark1).unwrap_or(rgba(0, 95, 184, 1.0)),
            overlay: rgba(255, 255, 255, 0.85),
            overlay_hover: rgba(235, 235, 235, 0.95),
            track: rgba(0, 0, 0, 0.3),
            thumb_ring: rgba(255, 255, 255, 1.0),
            success_bg: rgba(223, 246, 221, 1.0),
            success: rgba(15, 123, 15, 1.0),
            error_bg: rgba(253, 231, 233, 1.0),
            error: rgba(196, 43, 28, 1.0),
            on_status: rgba(255, 255, 255, 1.0),
        }
    }
}

/// Title bar to match: dark/light frame and our background as the caption color.
/// High contrast keeps the system's own caption colors.
pub fn apply_to_window(hwnd: HWND, theme: &Theme) {
    unsafe {
        let dark = BOOL((theme.dark && !theme.high_contrast) as i32);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const _ as _,
            4,
        );
        const DWMWA_COLOR_DEFAULT: u32 = 0xFFFF_FFFF;
        let caption = if theme.high_contrast {
            COLORREF(DWMWA_COLOR_DEFAULT)
        } else {
            theme.bg_colorref()
        };
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    }
}

/// Windows build number (e.g. 22631), read from the registry since GetVersionEx is shimmed.
fn windows_build() -> u32 {
    let mut buf = [0u16; 16];
    let mut size = std::mem::size_of_val(&buf) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as _),
            Some(&mut size),
        )
    };
    if r.is_err() {
        return 0;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).parse().unwrap_or(0)
}

type SetPreferredAppMode = extern "system" fn(i32) -> i32;
type FlushMenuThemes = extern "system" fn();

/// uxtheme's ordinal 135 (SetPreferredAppMode) and 136 (FlushMenuThemes), resolved once.
/// Only on 1903+ (build 18362): before that, ordinal 135 had a different signature.
fn menu_theme_fns() -> Option<(SetPreferredAppMode, FlushMenuThemes)> {
    static FNS: OnceLock<Option<(usize, usize)>> = OnceLock::new();
    let fns = FNS.get_or_init(|| unsafe {
        if windows_build() < 18362 {
            return None;
        }
        let uxtheme = LoadLibraryW(w!("uxtheme.dll")).ok()?;
        let set = GetProcAddress(uxtheme, PCSTR(135 as *const u8))?;
        let flush = GetProcAddress(uxtheme, PCSTR(136 as *const u8))?;
        Some((set as usize, flush as usize))
    });
    fns.map(|(set, flush)| unsafe {
        (
            std::mem::transmute::<usize, SetPreferredAppMode>(set),
            std::mem::transmute::<usize, FlushMenuThemes>(flush),
        )
    })
}

/// Make Win32 popup menus follow the theme. There is no documented API for this; these uxtheme
/// ordinals are what Windows' own Win32 apps use. Silently does nothing where unavailable.
pub fn apply_to_menus(theme: &Theme) {
    let Some((set_mode, flush)) = menu_theme_fns() else {
        return;
    };
    // 0 = default, 2 = force dark, 3 = force light.
    set_mode(if theme.high_contrast {
        0
    } else if theme.dark {
        2
    } else {
        3
    });
    flush();
}
