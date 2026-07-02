//! Active-window screenshot via `PrintWindow`.
//!
//! `PrintWindow(PW_RENDERFULLCONTENT)` asks the window to render itself into a
//! memory DC; we then pull the pixels out with `GetDIBits` as top-down 32-bit
//! BGRA and convert to RGBA. The real cursor is not captured (PrintWindow does
//! not include it) — the drawn click marker stands in for it.

use crate::platform::{Capture, CursorInfo, WindowCapturer};
use anyhow::{Context, Result, bail};
use image::RgbaImage;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HBITMAP, HDC,
    HGDIOBJ, ROP_CODE, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowRect, GetWindowTextW, PW_RENDERFULLCONTENT,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
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

    /// `BitBlt` of the whole virtual screen — `PrintWindow` can't see the
    /// taskbar or popup menus. A blit does not include the cursor, so the
    /// click marker is drawn in here instead.
    fn capture_screen_under_cursor(&self, ci: Option<&CursorInfo>) -> Result<Capture> {
        let (mut cap, origin) = unsafe { capture_screen() }?;
        if let Some(c) = ci {
            crate::annotate::draw_click_marker(&mut cap.image, c.x - origin.0, c.y - origin.1);
        }
        Ok(cap)
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

    let pixels = unsafe { read_dib(mem, bmp, w, h) };
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
    let buf = pixels?;

    let image = RgbaImage::from_raw(w as u32, h as u32, buf)
        .context("could not build image from window pixels")?;

    Ok(Capture {
        image,
        window_title: title,
        scale: 1.0,
        is_screen: false,
    })
}

/// Full capture of the virtual screen (all monitors) via `BitBlt`. Returns the
/// capture plus the virtual-screen origin, so screen coordinates can be mapped
/// into the image.
unsafe fn capture_screen() -> Result<(Capture, (i32, i32))> {
    let vx = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let vy = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let w = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(1);
    let h = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(1);

    let screen = unsafe { GetDC(None) };
    let mem = unsafe { CreateCompatibleDC(Some(screen)) };
    let bmp = unsafe { CreateCompatibleBitmap(screen, w, h) };
    let old = unsafe { SelectObject(mem, HGDIOBJ(bmp.0)) };

    // CAPTUREBLT includes layered windows (menus, tooltips) in the blit.
    let rop = ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0);
    let blitted = unsafe { BitBlt(mem, 0, 0, w, h, Some(screen), vx, vy, rop) };

    let pixels = unsafe { read_dib(mem, bmp, w, h) };

    unsafe {
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
    }

    blitted.context("BitBlt failed")?;
    let buf = pixels?;

    let image = RgbaImage::from_raw(w as u32, h as u32, buf)
        .context("could not build image from screen pixels")?;

    Ok((
        Capture {
            image,
            window_title: None,
            scale: 1.0,
            is_screen: true,
        },
        (vx, vy),
    ))
}

/// Read the pixels of `bmp` back as top-down RGBA (negative height = top-down
/// BGRA, then swapped in place).
unsafe fn read_dib(mem: HDC, bmp: HBITMAP, w: i32, h: i32) -> Result<Vec<u8>> {
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
    if lines == 0 {
        bail!("GetDIBits returned no scanlines");
    }

    // BGRA → RGBA.
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255; // many windows report alpha 0; force opaque
    }
    Ok(buf)
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
