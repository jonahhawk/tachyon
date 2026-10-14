//! Direct2D/DirectWrite resources shared by all popup windows. Everything is
//! created once and reused so a popup paints within a frame of the click.

use crate::icons;
use crate::platform::wide;
use std::collections::HashMap;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum IconKey {
    Svg(String, u32, u32),
    /// path, size, is_dir
    Shell(String, u32, bool),
}

pub struct Gfx {
    pub d2d: ID2D1Factory,
    pub dw: IDWriteFactory,
    pub rt: ID2D1DCRenderTarget,
    pub brush: ID2D1SolidColorBrush,
    fmt_key: (String, u32),
    pub text: IDWriteTextFormat,
    pub small: IDWriteTextFormat,
    icons: HashMap<IconKey, Option<ID2D1Bitmap>>,
}

pub fn color(rgb: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 0xFF) as f32 / 255.0,
        g: ((rgb >> 8) & 0xFF) as f32 / 255.0,
        b: (rgb & 0xFF) as f32 / 255.0,
        a,
    }
}

pub fn rectf(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: l, top: t, right: r, bottom: b }
}

fn make_rt(d2d: &ID2D1Factory) -> Result<ID2D1DCRenderTarget> {
    let props = D2D1_RENDER_TARGET_PROPERTIES {
        // Software rasterization: no GPU device spin-up, consistently ~1ms for menu-sized surfaces.
        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
        dpiX: 96.0,
        dpiY: 96.0,
        usage: D2D1_RENDER_TARGET_USAGE_NONE,
        minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
    };
    unsafe { d2d.CreateDCRenderTarget(&props) }
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let rt = make_rt(&d2d)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let brush = rt.CreateSolidColorBrush(&color(0, 1.0), None)?;
            let text = Self::format(&dw, "Segoe UI", 13.0)?;
            let small = Self::format(&dw, "Segoe UI", 11.0)?;
            Ok(Gfx { d2d, dw, rt, brush, fmt_key: (String::new(), 0), text, small, icons: HashMap::new() })
        }
    }

    fn format(dw: &IDWriteFactory, family: &str, px: f32) -> Result<IDWriteTextFormat> {
        unsafe {
            let fam = wide(family);
            let f = dw.CreateTextFormat(
                PCWSTR(fam.as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                px,
                w!("en-us"),
            )?;
            f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            let sign = dw.CreateEllipsisTrimmingSign(&f)?;
            let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
            f.SetTrimming(&trim, &sign)?;
            Ok(f)
        }
    }

    /// Recreate text formats when family/size/DPI change.
    pub fn ensure_fonts(&mut self, family: &str, px: f32) {
        let key = (family.to_string(), (px * 100.0) as u32);
        if self.fmt_key == key {
            return;
        }
        if let (Ok(t), Ok(s)) = (Self::format(&self.dw, family, px), Self::format(&self.dw, family, (px * 0.85).max(9.0))) {
            self.text = t;
            self.small = s;
        }
        self.fmt_key = key;
    }

    pub fn measure(&self, s: &str, fmt: &IDWriteTextFormat) -> f32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            match self.dw.CreateTextLayout(&w, fmt, 10000.0, 100.0) {
                Ok(l) => {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    l.GetMetrics(&mut m).map(|_| m.widthIncludingTrailingWhitespace).unwrap_or(0.0)
                }
                Err(_) => 0.0,
            }
        }
    }

    /// x offset of the caret before char index `idx` (UTF-16 based).
    pub fn caret_x(&self, s: &str, idx: usize, fmt: &IDWriteTextFormat) -> f32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        let pos: usize = s.chars().take(idx).map(|c| c.len_utf16()).sum();
        unsafe {
            let Ok(l) = self.dw.CreateTextLayout(&w, fmt, 10000.0, 100.0) else { return 0.0 };
            let (mut x, mut y) = (0.0f32, 0.0f32);
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            if pos >= w.len() {
                let mut tm = DWRITE_TEXT_METRICS::default();
                return l.GetMetrics(&mut tm).map(|_| tm.widthIncludingTrailingWhitespace).unwrap_or(0.0);
            }
            let _ = l.HitTestTextPosition(pos as u32, false, &mut x, &mut y, &mut m);
            x
        }
    }

    pub fn fill(&self, r: D2D_RECT_F, rgb: u32, a: f32, radius: f32) {
        unsafe {
            self.brush.SetColor(&color(rgb, a));
            if radius > 0.0 {
                self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, &self.brush);
            } else {
                self.rt.FillRectangle(&r, &self.brush);
            }
        }
    }

    pub fn stroke(&self, r: D2D_RECT_F, rgb: u32, a: f32, radius: f32, width: f32) {
        unsafe {
            self.brush.SetColor(&color(rgb, a));
            self.rt.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius },
                &self.brush,
                width,
                None,
            );
        }
    }

    pub fn text(&self, s: &str, r: D2D_RECT_F, rgb: u32, a: f32, small: bool) {
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            self.brush.SetColor(&color(rgb, a));
            self.rt.DrawText(
                &w,
                if small { &self.small } else { &self.text },
                &r,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    pub fn icon(&mut self, key: &IconKey, x: f32, y: f32, opacity: f32) {
        if !self.icons.contains_key(key) {
            let bmp = self.load_icon(key);
            if self.icons.len() > 2048 {
                self.icons.clear();
            }
            self.icons.insert(key.clone(), bmp);
        }
        if let Some(Some(b)) = self.icons.get(key) {
            let size = match key {
                IconKey::Svg(_, s, _) | IconKey::Shell(_, s, _) => *s as f32,
            };
            unsafe {
                self.rt.DrawBitmap(
                    b,
                    Some(&rectf(x, y, x + size, y + size)),
                    opacity,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
        }
    }

    fn load_icon(&self, key: &IconKey) -> Option<ID2D1Bitmap> {
        let (mut px, size) = match key {
            IconKey::Svg(r, size, tint) => (icons::render(r, *size, *tint)?, *size),
            IconKey::Shell(p, size, dir) => (shell_icon(p, *size, *dir)?, *size),
        };
        if matches!(key, IconKey::Svg(..)) {
            // tiny-skia: premultiplied RGBA -> D2D: premultiplied BGRA.
            for c in px.chunks_exact_mut(4) {
                c.swap(0, 2);
            }
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        unsafe {
            self.rt
                .CreateBitmap(D2D_SIZE_U { width: size, height: size }, Some(px.as_ptr() as *const _), size * 4, &props)
                .ok()
        }
    }

    /// Drop every cached device resource (after D2DERR_RECREATE_TARGET).
    pub fn reset(&mut self) {
        if let Ok(rt) = make_rt(&self.d2d) {
            unsafe {
                rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                if let Ok(b) = rt.CreateSolidColorBrush(&color(0, 1.0), None) {
                    self.brush = b;
                }
            }
            self.rt = rt;
        }
        self.icons.clear();
    }

    pub fn clear_icons(&mut self) {
        self.icons.clear();
    }
}

/// Native shell icon as premultiplied BGRA. Network paths use attribute-only
/// lookup so an offline share can never stall the menu.
pub fn shell_icon(path: &str, size: u32, is_dir: bool) -> Option<Vec<u8>> {
    unsafe {
        let p = wide(path);
        let mut sfi = SHFILEINFOW::default();
        let mut flags = SHGFI_ICON | if size <= 20 { SHGFI_SMALLICON } else { SHGFI_LARGEICON };
        let attrs = if is_dir { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
        if path.starts_with("\\\\") || path.is_empty() {
            flags |= SHGFI_USEFILEATTRIBUTES;
        }
        let cb = std::mem::size_of::<SHFILEINFOW>() as u32;
        if SHGetFileInfoW(PCWSTR(p.as_ptr()), attrs, Some(&mut sfi), cb, flags) == 0 || sfi.hIcon.is_invalid() {
            // Missing/offline path: generic icon for its type.
            flags |= SHGFI_USEFILEATTRIBUTES;
            if SHGetFileInfoW(PCWSTR(p.as_ptr()), attrs, Some(&mut sfi), cb, flags) == 0 || sfi.hIcon.is_invalid() {
                return None;
            }
        }
        let out = hicon_to_bgra(sfi.hIcon, size);
        let _ = DestroyIcon(sfi.hIcon);
        out
    }
}

pub fn hicon_to_bgra(icon: HICON, size: u32) -> Option<Vec<u8>> {
    unsafe {
        let dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        let old = SelectObject(dc, bmp.into());
        let _ = DrawIconEx(dc, 0, 0, icon, size as i32, size as i32, 0, None, DI_NORMAL);
        let n = (size * size * 4) as usize;
        let mut px = std::slice::from_raw_parts(bits as *const u8, n).to_vec();
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        // Legacy icons without an alpha channel: treat drawn pixels as opaque.
        if px.chunks_exact(4).all(|c| c[3] == 0) {
            for c in px.chunks_exact_mut(4) {
                if c[0] | c[1] | c[2] != 0 {
                    c[3] = 255;
                }
            }
        }
        Some(px)
    }
}

/// A 32-bpp top-down DIB section a popup renders into before UpdateLayeredWindow.
pub struct Surface {
    pub dc: HDC,
    bmp: HBITMAP,
    #[cfg_attr(not(test), allow(dead_code))]
    pub bits: *const u8,
    old: HGDIOBJ,
    pub w: i32,
    pub h: i32,
}

impl Surface {
    pub fn new(w: i32, h: i32) -> Option<Surface> {
        unsafe {
            let dc = CreateCompatibleDC(None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w.max(1),
                    biHeight: -h.max(1),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let old = SelectObject(dc, bmp.into());
            Some(Surface { dc, bmp, old, w, h, bits: bits as *const u8 })
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
