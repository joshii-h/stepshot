//! Tray icon: a camera with a record dot (red when active, grey when idle).
//!
//! Drawn programmatically and smoothed via supersampling — mirrors
//! `assets/stepshot.svg`. Returns a `ksni::Icon` in ARGB32 format.

use image::{Rgba, RgbaImage};

/// Builds the tray icon in the required ARGB32 format.
pub fn tray_icon(recording: bool) -> ksni::Icon {
    let size = 48u32;
    let img = render(recording, size);
    let mut data = img.into_raw(); // RGBA
    for px in data.chunks_exact_mut(4) {
        px.rotate_right(1); // RGBA → ARGB (network byte order)
    }
    ksni::Icon {
        width: size as i32,
        height: size as i32,
        data,
    }
}

/// Debug: rendered icon as an RGBA image (for visual inspection).
pub fn debug_png(recording: bool, size: u32) -> RgbaImage {
    render(recording, size)
}

/// Renders at high resolution and scales down for anti-aliasing.
fn render(recording: bool, size: u32) -> RgbaImage {
    const SS: u32 = 4;
    let big = draw(recording, size * SS);
    image::imageops::resize(&big, size, size, image::imageops::FilterType::Lanczos3)
}

// Color palette (same as the SVG).
const OUTLINE: [u8; 3] = [28, 39, 56];
const BODY: [u8; 3] = [43, 58, 82];
const BUMP: [u8; 3] = [52, 70, 95];
const LENS: [u8; 3] = [74, 111, 165];
const GLASS: [u8; 3] = [23, 32, 51];
const HILITE: [u8; 3] = [207, 224, 245];
const DOT_RED: [u8; 3] = [230, 35, 30];
const DOT_GREY: [u8; 3] = [107, 118, 137];

fn draw(recording: bool, n: u32) -> RgbaImage {
    let mut img = RgbaImage::new(n, n);
    let f = n as f32 / 48.0; // 48-unit coordinate grid like the SVG

    // Viewfinder bump
    rrect(&mut img, 13.0, 9.0, 11.0, 7.0, 2.0, BUMP, f);
    // Body with outline
    rrect(&mut img, 3.0, 14.0, 42.0, 28.0, 5.0, OUTLINE, f);
    rrect(&mut img, 4.2, 15.2, 39.6, 25.6, 4.0, BODY, f);
    // Lens
    circle(&mut img, 23.0, 28.0, 11.0, OUTLINE, 1.0, f);
    circle(&mut img, 23.0, 28.0, 9.8, LENS, 1.0, f);
    circle(&mut img, 23.0, 28.0, 6.5, GLASS, 1.0, f);
    circle(&mut img, 20.0, 25.0, 2.4, HILITE, 0.75, f);
    // Record dot
    let dot = if recording { DOT_RED } else { DOT_GREY };
    circle(&mut img, 38.5, 20.0, 4.2, OUTLINE, 1.0, f);
    circle(&mut img, 38.5, 20.0, 3.4, dot, 1.0, f);

    img
}

/// Filled rounded rectangle (coordinates in the 48-unit grid).
#[allow(clippy::too_many_arguments)]
fn rrect(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, r: f32, color: [u8; 3], f: f32) {
    let (x0, y0, x1, y1) = (x * f, y * f, (x + w) * f, (y + h) * f);
    let r = r * f;
    let (ix0, iy0) = (x0.floor() as i32, y0.floor() as i32);
    let (ix1, iy1) = (x1.ceil() as i32, y1.ceil() as i32);
    for py in iy0..iy1 {
        for px in ix0..ix1 {
            let cx = px as f32 + 0.5;
            let cy = py as f32 + 0.5;
            // Account for the distance to the nearest corner.
            let dx = (x0 + r - cx).max(cx - (x1 - r)).max(0.0);
            let dy = (y0 + r - cy).max(cy - (y1 - r)).max(0.0);
            if dx * dx + dy * dy <= r * r {
                put(img, px, py, color, 1.0);
            }
        }
    }
}

/// Filled circle (coordinates in the 48-unit grid), `alpha` 0..1.
fn circle(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, color: [u8; 3], alpha: f32, f: f32) {
    let (cx, cy, r) = (cx * f, cy * f, r * f);
    let (ix0, iy0) = ((cx - r).floor() as i32, (cy - r).floor() as i32);
    let (ix1, iy1) = ((cx + r).ceil() as i32, (cy + r).ceil() as i32);
    for py in iy0..iy1 {
        for px in ix0..ix1 {
            let dx = px as f32 + 0.5 - cx;
            let dy = py as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= r * r {
                put(img, px, py, color, alpha);
            }
        }
    }
}

/// Blend a pixel src-over.
fn put(img: &mut RgbaImage, x: i32, y: i32, color: [u8; 3], alpha: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let dst = img.get_pixel_mut(x as u32, y as u32);
    let a = alpha.clamp(0.0, 1.0);
    let blend = |s: u8, d: u8| ((s as f32 * a) + (d as f32 * (1.0 - a))) as u8;
    let na = (a * 255.0) as u16 + (dst[3] as u16 * (1.0 - a) as u16);
    *dst = Rgba([
        blend(color[0], dst[0]),
        blend(color[1], dst[1]),
        blend(color[2], dst[2]),
        na.min(255) as u8,
    ]);
}
