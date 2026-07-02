//! Visual click marker in the screenshot.
//!
//! A translucent yellow highlight — like a marker pen: the click location is
//! obvious, but text and UI under the marker stay readable through it. An
//! amber rim keeps it visible on white/light backgrounds.

use image::{Rgba, RgbaImage};

/// Appearance of the click highlight. Defaults reproduce the built-in
/// translucent-yellow marker; overridable via the config file (`[marker]`).
#[derive(Debug, Clone, Copy)]
pub struct MarkerStyle {
    /// Fill color (the translucent highlight).
    pub fill: [u8; 3],
    /// Fill opacity, 0.0–1.0 — kept low so text under it stays readable.
    pub fill_alpha: f32,
    /// Rim color (the contrast outline).
    pub rim: [u8; 3],
    /// Rim opacity, 0.0–1.0.
    pub rim_alpha: f32,
    /// Fill radius in pixels; the rim sits just outside it.
    pub radius: f32,
}

impl Default for MarkerStyle {
    fn default() -> Self {
        Self {
            fill: [255, 225, 60], // highlighter yellow
            fill_alpha: 0.35,     // see-through
            rim: [255, 165, 0],   // amber
            rim_alpha: 0.75,      // stronger for contrast on light backgrounds
            radius: 20.0,
        }
    }
}

/// Draws the translucent highlight around (cx, cy) using `style`.
pub fn draw_click_marker(img: &mut RgbaImage, cx: i32, cy: i32, style: &MarkerStyle) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let margin = (style.radius + 4.0) as i32;
    if cx < -margin || cy < -margin || cx >= w + margin || cy >= h + margin {
        return; // entirely off-canvas → draw nothing
    }

    fill_circle(img, cx, cy, style.radius, style.fill, style.fill_alpha);
    draw_ring(
        img,
        cx,
        cy,
        style.radius - 1.0,
        style.radius + 2.0,
        style.rim,
        style.rim_alpha,
    );
}

/// Translucent filled circle.
fn fill_circle(img: &mut RgbaImage, cx: i32, cy: i32, r: f32, color: [u8; 3], alpha: f32) {
    let r2 = r * r;
    let rad = r.ceil() as i32;
    for dy in -rad..=rad {
        for dx in -rad..=rad {
            if (dx * dx + dy * dy) as f32 <= r2 {
                blend_pixel(img, cx + dx, cy + dy, color, alpha);
            }
        }
    }
}

/// Translucent ring (band between `inner` and `outer` radius).
fn draw_ring(
    img: &mut RgbaImage,
    cx: i32,
    cy: i32,
    inner: f32,
    outer: f32,
    color: [u8; 3],
    alpha: f32,
) {
    let r2i = inner * inner;
    let r2o = outer * outer;
    let rad = outer.ceil() as i32;
    for dy in -rad..=rad {
        for dx in -rad..=rad {
            let d2 = (dx * dx + dy * dy) as f32;
            if d2 >= r2i && d2 <= r2o {
                blend_pixel(img, cx + dx, cy + dy, color, alpha);
            }
        }
    }
}

/// src-over blend of one pixel (same math as the icon renderer).
fn blend_pixel(img: &mut RgbaImage, x: i32, y: i32, color: [u8; 3], alpha: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let dst = img.get_pixel_mut(x as u32, y as u32);
    let a = alpha.clamp(0.0, 1.0);
    let blend = |s: u8, d: u8| ((s as f32 * a) + (d as f32 * (1.0 - a))) as u8;
    let na = (a * 255.0 + dst[3] as f32 * (1.0 - a)) as u8;
    *dst = Rgba([
        blend(color[0], dst[0]),
        blend(color[1], dst[1]),
        blend(color[2], dst[2]),
        na,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_is_translucent_not_opaque() {
        // On a white background the fill must mix with the white below —
        // neither pure white (invisible) nor pure yellow (covering the text).
        let style = MarkerStyle::default();
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        draw_click_marker(&mut img, 50, 50, &style);
        let p = img.get_pixel(50, 50).0;
        assert_ne!(p, [255, 255, 255, 255], "marker must be visible");
        assert!(
            p[2] > style.fill[2] && p[2] < 255,
            "blue channel {} should sit between yellow ({}) and white (255)",
            p[2],
            style.fill[2]
        );
        assert_eq!(p[3], 255, "opaque background stays opaque");
    }

    #[test]
    fn rim_is_stronger_than_fill() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([255, 255, 255, 255]));
        draw_click_marker(&mut img, 50, 50, &MarkerStyle::default());
        let fill = img.get_pixel(50, 50).0;
        let rim = img.get_pixel(50 + 21, 50).0; // inside the 19..22 band
        assert!(
            rim[2] < fill[2],
            "rim (b={}) should be more saturated than the fill (b={})",
            rim[2],
            fill[2]
        );
    }

    #[test]
    fn custom_style_is_honored() {
        // A fully opaque solid box-like marker: alpha 1.0 → pure fill color.
        let style = MarkerStyle {
            fill: [10, 20, 30],
            fill_alpha: 1.0,
            radius: 8.0,
            ..MarkerStyle::default()
        };
        let mut img = RgbaImage::from_pixel(40, 40, Rgba([255, 255, 255, 255]));
        draw_click_marker(&mut img, 20, 20, &style);
        assert_eq!(img.get_pixel(20, 20).0[..3], [10, 20, 30]);
    }

    #[test]
    fn off_canvas_draws_nothing_and_does_not_panic() {
        let style = MarkerStyle::default();
        let mut img = RgbaImage::from_pixel(50, 50, Rgba([0, 0, 0, 255]));
        let before = img.clone();
        draw_click_marker(&mut img, -100, -100, &style);
        assert_eq!(img, before);
        // Partially off-canvas must not panic.
        draw_click_marker(&mut img, 0, 0, &style);
        draw_click_marker(&mut img, 49, 49, &style);
    }
}
