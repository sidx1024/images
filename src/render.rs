//! Thin Direct2D / DirectWrite wrapper. The render target runs at 96 DPI so one unit = one physical pixel;
//! UI chrome is drawn in DIPs by applying a scale transform.

use windows::core::{w, Interface, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_numerics::Matrix3x2;

use crate::decode::Decoded;

pub struct Target {
    pub rt: ID2D1HwndRenderTarget,
    pub dc: Option<ID2D1DeviceContext>,
    brush: ID2D1SolidColorBrush,
}

pub struct Fonts {
    /// Drawer text; follows the Windows text-size setting.
    pub title: IDWriteTextFormat,
    pub label: IDWriteTextFormat,
    pub body: IDWriteTextFormat,
    /// Fixed-size text for the status bar and overlays, whose layout is fixed.
    pub bar: IDWriteTextFormat,
    /// Status bar details (format, dimensions, size).
    pub status: IDWriteTextFormat,
    pub center: IDWriteTextFormat,
    pub icon: IDWriteTextFormat,
    pub icon_small: IDWriteTextFormat,
    pub icon_status: IDWriteTextFormat,
    /// Window caption button glyphs (minimize/maximize/close).
    pub caption: IDWriteTextFormat,
    /// Centered, single-line, ellipsized file name in the title bar.
    pub title_text: IDWriteTextFormat,
    /// Settings page.
    pub page_title: IDWriteTextFormat,
    pub section: IDWriteTextFormat,
    pub small: IDWriteTextFormat,
}

pub struct Gfx {
    factory: ID2D1Factory1,
    dwrite: IDWriteFactory,
    pub target: Option<Target>,
    pub fonts: Fonts,
}

pub fn rgba(r: u8, g: u8, b: u8, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

pub fn rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

/// Segoe Fluent Icons ships with Windows 11; Windows 10 has Segoe MDL2 Assets with the same code points.
fn icon_family(dwrite: &IDWriteFactory) -> PCWSTR {
    unsafe {
        let mut fonts = None;
        let (mut index, mut exists) = (0u32, BOOL(0));
        if dwrite.GetSystemFontCollection(&mut fonts, false).is_ok() {
            if let Some(fonts) = fonts {
                let _ = fonts.FindFamilyName(w!("Segoe Fluent Icons"), &mut index, &mut exists);
            }
        }
        if exists.as_bool() {
            w!("Segoe Fluent Icons")
        } else {
            w!("Segoe MDL2 Assets")
        }
    }
}

fn make_fonts(dwrite: &IDWriteFactory, text_scale: f32) -> Result<Fonts> {
    unsafe {
        let fmt =
            |family: PCWSTR, weight: DWRITE_FONT_WEIGHT, size: f32| -> Result<IDWriteTextFormat> {
                let f = dwrite.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!(""),
                )?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                Ok(f)
            };
        let centered = |f: IDWriteTextFormat| -> Result<IDWriteTextFormat> {
            f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            Ok(f)
        };
        let ts = text_scale.clamp(1.0, 2.25);
        let icons = icon_family(dwrite);
        let title_text = centered(fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, 12.0)?)?;
        title_text.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        let ellipsis = dwrite.CreateEllipsisTrimmingSign(&title_text)?;
        let trimming = DWRITE_TRIMMING {
            granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
            delimiter: 0,
            delimiterCount: 0,
        };
        title_text.SetTrimming(&trimming, &ellipsis)?;
        Ok(Fonts {
            title: fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_SEMI_BOLD, 20.0 * ts)?,
            label: fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, 12.0 * ts)?,
            body: fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, 14.0 * ts)?,
            bar: fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, 14.0)?,
            status: fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, 13.0)?,
            center: centered(fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, 14.0)?)?,
            icon: centered(fmt(icons, DWRITE_FONT_WEIGHT_NORMAL, 16.0)?)?,
            icon_status: centered(fmt(icons, DWRITE_FONT_WEIGHT_NORMAL, 15.0)?)?,
            icon_small: centered(fmt(icons, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?)?,
            caption: centered(fmt(icons, DWRITE_FONT_WEIGHT_NORMAL, 10.0)?)?,
            title_text,
            page_title: fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_SEMI_BOLD, 28.0)?,
            section: fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_SEMI_BOLD, 14.0)?,
            small: fmt(w!("Segoe UI"), DWRITE_FONT_WEIGHT_NORMAL, 12.0)?,
        })
    }
}

fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

impl Gfx {
    pub fn new(text_scale: f32) -> Result<Self> {
        unsafe {
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let fonts = make_fonts(&dwrite, text_scale)?;
            Ok(Gfx {
                factory,
                dwrite,
                target: None,
                fonts,
            })
        }
    }

    /// Re-create fonts for a new Windows text-size setting.
    pub fn set_text_scale(&mut self, text_scale: f32) {
        if let Ok(fonts) = make_fonts(&self.dwrite, text_scale) {
            self.fonts = fonts;
        }
    }

    pub fn ensure_target(&mut self, hwnd: HWND) -> Result<()> {
        if self.target.is_some() {
            return Ok(());
        }
        unsafe {
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            };
            let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U {
                    width: rc.right.max(1) as u32,
                    height: rc.bottom.max(1) as u32,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let rt = self.factory.CreateHwndRenderTarget(&props, &hprops)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&rgba(255, 255, 255, 1.0), None)?;
            let dc = rt.cast::<ID2D1DeviceContext>().ok();
            self.target = Some(Target { rt, dc, brush });
        }
        Ok(())
    }

    pub fn max_bitmap_size(&self) -> Option<u32> {
        self.target
            .as_ref()
            .map(|t| unsafe { t.rt.GetMaximumBitmapSize() })
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if let Some(t) = &self.target {
            // A failed resize means the target is unusable; recreate it on the next paint.
            if unsafe {
                t.rt.Resize(&D2D_SIZE_U {
                    width: w.max(1),
                    height: h.max(1),
                })
            }
            .is_err()
            {
                self.target = None;
            }
        }
    }

    pub fn create_bitmap(&self, d: &Decoded) -> Option<ID2D1Bitmap> {
        let t = self.target.as_ref()?;
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe {
            t.rt.CreateBitmap(
                D2D_SIZE_U {
                    width: d.width,
                    height: d.height,
                },
                Some(d.pixels.as_ptr() as _),
                d.width * 4,
                &props,
            )
            .ok()
        }
    }

    pub fn text_width(&self, s: &str, fmt: &IDWriteTextFormat) -> f32 {
        unsafe {
            let Ok(layout) = self
                .dwrite
                .CreateTextLayout(&utf16(s), fmt, 10_000.0, 10_000.0)
            else {
                return 0.0;
            };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            m.widthIncludingTrailingWhitespace
        }
    }

    pub fn text_height(&self, s: &str, fmt: &IDWriteTextFormat, width: f32) -> f32 {
        unsafe {
            let Ok(layout) = self
                .dwrite
                .CreateTextLayout(&utf16(s), fmt, width, 10_000.0)
            else {
                return 0.0;
            };
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            m.height
        }
    }
}

/// Drawing helpers valid between BeginDraw/EndDraw.
impl Target {
    pub fn set_scale(&self, s: f32) {
        self.set_transform(s, 0.0, 0.0);
    }

    /// Scale by `s`, then translate by (dx, dy) pixels.
    pub fn set_transform(&self, s: f32, dx: f32, dy: f32) {
        let m = Matrix3x2 {
            M11: s,
            M12: 0.0,
            M21: 0.0,
            M22: s,
            M31: dx,
            M32: dy,
        };
        unsafe { self.rt.SetTransform(&m) };
    }

    pub fn clear(&self, c: D2D1_COLOR_F) {
        unsafe { self.rt.Clear(Some(&c)) };
    }

    pub fn fill_rect(&self, r: D2D_RECT_F, c: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&c);
            self.rt.FillRectangle(&r, &self.brush);
        }
    }

    pub fn fill_ellipse(&self, cx: f32, cy: f32, r: f32, c: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&c);
            let e = D2D1_ELLIPSE {
                point: windows_numerics::Vector2 { X: cx, Y: cy },
                radiusX: r,
                radiusY: r,
            };
            self.rt.FillEllipse(&e, &self.brush);
        }
    }

    pub fn fill_round(&self, r: D2D_RECT_F, radius: f32, c: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&c);
            let rr = D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            };
            self.rt.FillRoundedRectangle(&rr, &self.brush);
        }
    }

    pub fn stroke_round(&self, r: D2D_RECT_F, radius: f32, width: f32, c: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&c);
            let rr = D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            };
            self.rt.DrawRoundedRectangle(&rr, &self.brush, width, None);
        }
    }

    pub fn text(&self, s: &str, fmt: &IDWriteTextFormat, r: D2D_RECT_F, c: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&c);
            self.rt.DrawText(
                &utf16(s),
                fmt,
                &r,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_NONE,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    pub fn bitmap(&self, bmp: &ID2D1Bitmap, dest: D2D_RECT_F, scale: f32) {
        unsafe {
            if let Some(dc) = &self.dc {
                // High-quality cubic avoids shimmering when large photos are shown scaled down.
                let mode = if scale < 1.0 {
                    D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC
                } else {
                    D2D1_INTERPOLATION_MODE_LINEAR
                };
                dc.DrawBitmap(bmp, Some(&dest), 1.0, mode, None, None);
            } else {
                self.rt.DrawBitmap(
                    bmp,
                    Some(&dest),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
        }
    }

    pub fn end(&self) -> Result<()> {
        unsafe { self.rt.EndDraw(None, None) }
    }
}
