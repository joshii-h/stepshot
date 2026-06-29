//! Visual click marker in the screenshot.
//!
//! KWin already renders the real mouse cursor into the image via `include-cursor`;
//! on top of that we draw a conspicuous ring at the click location.

use image::{Rgba, RgbaImage};

/// Draws a red ring (with a light contrast edge) around (cx, cy).
pub fn draw_click_marker(img: &mut RgbaImage, cx: i32, cy: i32) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    if cx < -40 || cy < -40 || cx >= w + 40 || cy >= h + 40 {
        return; // entirely off-canvas → draw nothing
    }

    let red = Rgba([235, 30, 30, 255]);
    let white = Rgba([255, 255, 255, 255]);

    // Two rings: white outer edge for contrast, red ring on top.
    draw_ring(img, cx, cy, 20.0, 22.0, white);
    draw_ring(img, cx, cy, 15.0, 20.0, red);
    draw_ring(img, cx, cy, 13.0, 15.0, white);
}

fn draw_ring(img: &mut RgbaImage, cx: i32, cy: i32, inner: f32, outer: f32, color: Rgba<u8>) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let r2i = inner * inner;
    let r2o = outer * outer;
    let rad = outer.ceil() as i32;
    for dy in -rad..=rad {
        for dx in -rad..=rad {
            let (px, py) = (cx + dx, cy + dy);
            if px < 0 || py < 0 || px >= w || py >= h {
                continue;
            }
            let d2 = (dx * dx + dy * dy) as f32;
            if d2 >= r2i && d2 <= r2o {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}
