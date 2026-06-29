//! Active-window screenshot via `PrintWindow`.
//!
//! `PrintWindow(PW_RENDERFULLCONTENT)` asks the window to render itself into a
//! memory DC; we then pull the pixels out with `GetDIBits` as top-down 32-bit
//! BGRA and convert to RGBA. The real cursor is not captured (PrintWindow does
//! not include it) — the drawn click marker stands in for it.

use crate::platform::{Capture, WindowCapturer};
use anyhow::{Context, Result, bail};
use image::RgbaImage;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowRect, GetWindowTextW, PW_RENDERFULLCONTENT,
};

#[derive(Default)]
pub struct GdiCapturer;

impl GdiCapturer {
    pub fn new() -> Self {
        GdiCapturer
    }
}

impl WindowCapturer for GdiCapturer {
    fn capture_active_window(&self) -> Result<Capture> {
        unsafe { capture() }
    }
}

unsafe fn capture() -> Result<Capture> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        bail!("no foreground window");
    }

    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.context("GetWindowRect failed")?;
    let w = (rect.right - rect.left).max(1);
    let h = (rect.bottom - rect.top).max(1);

    // Screen DC → compatible memory DC + bitmap to print the window into.
    let screen = unsafe { GetDC(None) };
    let mem = unsafe { CreateCompatibleDC(Some(screen)) };
    let bmp = unsafe { CreateCompatibleBitmap(screen, w, h) };
    let old = unsafe { SelectObject(mem, HGDIOBJ(bmp.0)) };

    let printed = unsafe { PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)) };

    // Read the pixels back as top-down 32-bit BGRA (negative height = top-down).
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let lines = unsafe {
        GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };

    let title = window_title(hwnd);

    // Clean up GDI objects regardless of success.
    unsafe {
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }

    if !printed.as_bool() {
        bail!("PrintWindow failed");
    }
    if lines == 0 {
        bail!("GetDIBits returned no scanlines");
    }

    // BGRA → RGBA.
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255; // many windows report alpha 0; force opaque
    }

    let image = RgbaImage::from_raw(w as u32, h as u32, buf)
        .context("could not build image from window pixels")?;

    Ok(Capture {
        image,
        window_title: title,
        scale: 1.0,
    })
}

fn window_title(hwnd: HWND) -> Option<String> {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if n <= 0 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..n as usize]);
    if s.is_empty() { None } else { Some(s) }
}
