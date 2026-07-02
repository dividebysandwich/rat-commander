//! Pure pixel-raster primitives for the terminal-graphics layer.
//!
//! Each function builds an [`image::RgbaImage`] sized to a cell rectangle's
//! pixel dimensions; the graphics layer ([`super`]) then hands it to a terminal
//! graphics protocol. Nothing here touches the terminal, so it is unit-testable
//! and deliberately free of any `ratatui-image` dependency.
//!
//! Colors are passed in (or as closures) by the caller from the active theme, so
//! the pixel output uses exactly the same gradients as the Ratatui cell widgets
//! these graphics replace.

use image::{Rgba, RgbaImage};
use ratatui::style::Color;
use std::sync::LazyLock;

/// The bundled UI font used for all baked-in graphics text (button labels,
/// treemap labels). Ubuntu covers Latin, Cyrillic and Greek; parsed once. Text
/// is baked as anti-aliased pixels so it survives every graphics protocol —
/// terminal cell text drawn *over* a graphics image is not shown by Kitty/Sixel.
static FONT: LazyLock<fontdue::Font> = LazyLock::new(|| {
    fontdue::Font::from_bytes(
        epaint_default_fonts::UBUNTU_LIGHT,
        fontdue::FontSettings::default(),
    )
    .expect("bundled Ubuntu font parses")
});

/// An opaque RGB triple, matching the terminal's truecolor cells.
pub type Rgb = (u8, u8, u8);

/// Convert a ratatui [`Color`] to an RGB triple (themes always use `Rgb`).
pub fn rgb(c: Color) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (128, 128, 128),
    }
}

/// The process-explorer load color ramp (green → yellow → red) for a percentage
/// in `[0, 100]`. Mirrors `proc::render::load_color` so graphs match the meters.
pub fn load_rgb(pct: f64) -> Rgb {
    let p = (pct / 100.0).clamp(0.0, 1.0);
    if p < 0.5 {
        lerp3((40, 200, 140), (220, 200, 40), p / 0.5)
    } else {
        lerp3((220, 200, 40), (230, 60, 50), (p - 0.5) / 0.5)
    }
}

fn lerp3(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round().clamp(0.0, 255.0) as u8;
    (l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}

/// Alpha-composite `fg` over `bg` with opacity `a` in `[0, 1]`.
pub fn over(bg: Rgb, fg: Rgb, a: f64) -> Rgb {
    let a = a.clamp(0.0, 1.0);
    let m = |b: u8, f: u8| (b as f64 * (1.0 - a) + f as f64 * a).round().clamp(0.0, 255.0) as u8;
    (m(bg.0, fg.0), m(bg.1, fg.1), m(bg.2, fg.2))
}

/// Scale a color's brightness by `f` (for gloss / shading). `f > 1` brightens.
fn shade(c: Rgb, f: f64) -> Rgb {
    let s = |x: u8| (x as f64 * f).round().clamp(0.0, 255.0) as u8;
    (s(c.0), s(c.1), s(c.2))
}

#[inline]
fn put(img: &mut RgbaImage, x: u32, y: u32, c: Rgb) {
    if x < img.width() && y < img.height() {
        img.put_pixel(x, y, Rgba([c.0, c.1, c.2, 255]));
    }
}

/// Value of a series sampled at horizontal fraction `f` in `[0, 1]`, linearly
/// interpolated. The newest sample sits at `f = 1` (right edge). Empty → 0.
fn series_at(samples: &[f64], f: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    if samples.len() == 1 {
        return samples[0];
    }
    let pos = f.clamp(0.0, 1.0) * (samples.len() - 1) as f64;
    let i = pos.floor() as usize;
    let frac = pos - i as f64;
    if i + 1 < samples.len() {
        samples[i] * (1.0 - frac) + samples[i + 1] * frac
    } else {
        samples[samples.len() - 1]
    }
}

/// A horizontal progress bar: a rounded "pill" filled left→right to `frac`, the
/// filled part colored by `fill(t)` across its length with a soft vertical gloss,
/// the remainder `empty`, all over background `bg`.
pub fn gradient_bar(
    w: u32,
    h: u32,
    frac: f64,
    fill: impl Fn(f64) -> Rgb,
    empty: Rgb,
    bg: Rgb,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([bg.0, bg.1, bg.2, 255]));
    let (w, h) = (img.width(), img.height());
    let frac = frac.clamp(0.0, 1.0);
    let fill_px = frac * w as f64;
    let radius = (h as f64 / 2.0).min(w as f64 / 2.0);
    let cy = (h as f64 - 1.0) / 2.0;
    for y in 0..h {
        // Vertical gloss: brightest just above the middle, darker at the bottom.
        let vy = if h > 1 { y as f64 / (h - 1) as f64 } else { 0.5 };
        let gloss = 1.12 - 0.5 * (vy - 0.32).abs();
        for x in 0..w {
            // Rounded ends: skip pixels outside the pill's capsule.
            let mut inside = true;
            if (x as f64) < radius {
                let dx = radius - 0.5 - x as f64;
                let dy = y as f64 - cy;
                if dx > 0.0 && dx * dx + dy * dy > radius * radius {
                    inside = false;
                }
            } else if (x as f64) > w as f64 - radius {
                let dx = x as f64 - (w as f64 - radius - 0.5);
                let dy = y as f64 - cy;
                if dx > 0.0 && dx * dx + dy * dy > radius * radius {
                    inside = false;
                }
            }
            if !inside {
                continue; // leave the background showing through the corner
            }
            let t = if w > 1 { x as f64 / (w - 1) as f64 } else { 0.0 };
            let c = if (x as f64) < fill_px {
                shade(fill(t), gloss)
            } else {
                empty
            };
            // Anti-alias the fill's leading edge.
            let c = if (x as f64) < fill_px && fill_px - x as f64 <= 1.0 {
                over(empty, c, fill_px - x as f64)
            } else {
                c
            };
            put(&mut img, x, y, c);
        }
    }
    img
}

/// A line graph with a soft gradient fill under the curve. `samples` scale
/// against `max`; the stroke color comes from `line(t)` with `t` the horizontal
/// fraction (so an animated theme gradient sweeps across it).
pub fn line_graph(
    w: u32,
    h: u32,
    samples: &[f64],
    max: f64,
    line: impl Fn(f64) -> Rgb,
    bg: Rgb,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([bg.0, bg.1, bg.2, 255]));
    let (w, h) = (img.width(), img.height());
    let max = if max <= 0.0 { 1.0 } else { max };
    let baseline = (h - 1) as f64;
    for x in 0..w {
        let t = if w > 1 { x as f64 / (w - 1) as f64 } else { 0.0 };
        let v = (series_at(samples, t) / max).clamp(0.0, 1.0);
        let top = baseline - v * baseline; // y of the curve
        let col = line(t);
        for y in 0..h {
            let yf = y as f64;
            if yf >= top - 0.75 && yf <= top + 0.75 {
                put(&mut img, x, y, col); // ~1.5px stroke
            } else if yf > top {
                // Fill under the curve: brightest just below the line, fading
                // toward the background near the baseline (a soft glow).
                let depth = (yf - top) / (baseline - top).max(1.0);
                put(&mut img, x, y, over(bg, col, 0.48 - 0.34 * depth));
            }
        }
    }
    img
}

/// A center-axis mirrored bar graph: `up` grows above the axis, `down` below it.
/// Both scale against a shared `max`; the axis line is drawn in `axis_c`. Newest
/// samples sit at the right edge. `up_frac` is the axis position from the top.
#[allow(clippy::too_many_arguments)]
pub fn mirror_bars(
    w: u32,
    h: u32,
    up: &[f64],
    down: &[f64],
    max: f64,
    up_c: Rgb,
    down_c: Rgb,
    axis_c: Rgb,
    bg: Rgb,
    up_frac: f64,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([bg.0, bg.1, bg.2, 255]));
    let (w, h) = (img.width(), img.height());
    let max = if max <= 0.0 { 1.0 } else { max };
    let axis_y = ((h as f64) * up_frac.clamp(0.05, 0.95)).round() as u32;
    let up_span = axis_y.max(1) as f64;
    let down_span = (h - axis_y).max(1) as f64;
    for x in 0..w {
        let t = if w > 1 { x as f64 / (w - 1) as f64 } else { 0.0 };
        let uv = (series_at(up, t) / max).clamp(0.0, 1.0);
        let dv = (series_at(down, t) / max).clamp(0.0, 1.0);
        let up_h = (uv * up_span).round() as u32;
        let down_h = (dv * down_span).round() as u32;
        for k in 0..up_h {
            let y = axis_y.saturating_sub(1 + k);
            let fade = 1.0 - 0.55 * (k as f64 / up_span); // brighter near the axis
            put(&mut img, x, y, over(bg, up_c, 0.4 + 0.6 * fade));
        }
        for k in 0..down_h {
            let y = axis_y + k;
            let fade = 1.0 - 0.55 * (k as f64 / down_span);
            put(&mut img, x, y, over(bg, down_c, 0.4 + 0.6 * fade));
        }
    }
    // The horizontal axis line, drawn last so bars don't cover it.
    for x in 0..w {
        put(&mut img, x, axis_y.min(h - 1), axis_c);
    }
    img
}

/// A filled area sparkline: one soft-topped bar per horizontal pixel, height from
/// the series against `max`, each column colored by `color(v)` where `v` is the
/// value's fraction of `max`. Newest sample at the right edge.
pub fn area_spark(
    w: u32,
    h: u32,
    samples: &[f64],
    max: f64,
    color: impl Fn(f64) -> Rgb,
    bg: Rgb,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([bg.0, bg.1, bg.2, 255]));
    let (w, h) = (img.width(), img.height());
    let max = if max <= 0.0 { 1.0 } else { max };
    let baseline = (h - 1) as f64;
    for x in 0..w {
        let t = if w > 1 { x as f64 / (w - 1) as f64 } else { 0.0 };
        let v = (series_at(samples, t) / max).clamp(0.0, 1.0);
        let col = color(v);
        let top = baseline - v * baseline;
        for y in 0..h {
            let yf = y as f64;
            if yf >= top {
                // Vertical fade so bars look rounded rather than flat blocks.
                let up = (yf - top) / (baseline - top).max(1.0);
                put(&mut img, x, y, over(bg, col, 0.55 + 0.45 * up));
            }
        }
    }
    img
}

/// An indeterminate progress bar: a rounded track in `track` with a bright
/// `band` block, centered at horizontal fraction `pos`, sweeping back and forth.
/// `block` is the block's width as a fraction of the bar.
pub fn sweep_bar(w: u32, h: u32, pos: f64, block: f64, band: Rgb, track: Rgb, bg: Rgb) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([bg.0, bg.1, bg.2, 255]));
    let (w, h) = (img.width(), img.height());
    let radius = (h as f64 / 2.0).min(w as f64 / 2.0);
    let cy = (h as f64 - 1.0) / 2.0;
    let center = pos.clamp(0.0, 1.0) * w as f64;
    let half = (block.clamp(0.02, 1.0) * w as f64) / 2.0;
    for y in 0..h {
        let vy = if h > 1 { y as f64 / (h - 1) as f64 } else { 0.5 };
        let gloss = 1.12 - 0.5 * (vy - 0.32).abs();
        for x in 0..w {
            let mut inside = true;
            let xf = x as f64;
            if xf < radius {
                let dx = radius - 0.5 - xf;
                let dy = y as f64 - cy;
                if dx > 0.0 && dx * dx + dy * dy > radius * radius {
                    inside = false;
                }
            } else if xf > w as f64 - radius {
                let dx = xf - (w as f64 - radius - 0.5);
                let dy = y as f64 - cy;
                if dx > 0.0 && dx * dx + dy * dy > radius * radius {
                    inside = false;
                }
            }
            if !inside {
                continue;
            }
            // Distance from the sweeping block's center → smooth bright band.
            let d = (xf - center).abs() / half.max(1.0);
            let hi = (1.0 - d).clamp(0.0, 1.0);
            let c = over(track, shade(band, gloss), hi * hi * (3.0 - 2.0 * hi));
            put(&mut img, x, y, c);
        }
    }
    img
}

/// Fill an axis-aligned rectangle with a solid color, clipped to the image.
pub fn fill_rect(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, color: Rgb) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, Rgba([color.0, color.1, color.2, 255]));
        }
    }
}

/// A themed push-button "face": a rounded (pill) body filled with `fill` under a
/// soft vertical gloss and a bevelled top/bottom rim, sitting over background
/// `bg` with a drop shadow toward the bottom-right. When `glow` is `Some` the
/// body is wrapped in a soft halo of that color (pulsing when `animated`), used
/// to mark the focused button. The label is drawn separately as crisp cell text
/// by the caller, so every script renders (the 8×8 font is ASCII-only).
#[allow(clippy::too_many_arguments)]
pub fn button(w: u32, h: u32, fill: Rgb, glow: Option<Rgb>, bg: Rgb, anim: usize, animated: bool) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([bg.0, bg.1, bg.2, 255]));
    let (w, h) = (img.width(), img.height());
    // Too small to shape a pill: fall back to a solid block.
    if w < 4 || h < 3 {
        for y in 0..h {
            for x in 0..w {
                put(&mut img, x, y, fill);
            }
        }
        return img;
    }
    let so = (h as f64 / 8.0).max(1.0); // drop-shadow offset (down + right)
    let pad = (h as f64 / 12.0).max(1.0); // inset leaving room for the glow/shadow
    let x0 = pad;
    let y0 = pad;
    let x1 = (w as f64 - 1.0 - so).max(x0 + 2.0);
    let y1 = (h as f64 - 1.0 - so).max(y0 + 2.0);
    let r = ((y1 - y0) / 2.0).max(1.0); // pill radius (capsule half-height)
    let cy = (y0 + y1) / 2.0;
    let ax = x0 + r; // capsule spine endpoints
    let bx = (x1 - r).max(ax);
    // Signed distance from `(px,py)` to the pill capsule shifted by `(dx,dy)`
    // (negative inside). Used for the body, its offset shadow, and the glow.
    let capsule = |px: f64, py: f64, dx: f64, dy: f64| -> f64 {
        let sx = (px - dx).clamp(ax, bx);
        let ex = (px - dx) - sx;
        let ey = (py - dy) - cy;
        (ex * ex + ey * ey).sqrt() - r
    };
    // The glow gently pulses on animated themes, matching the progress bars.
    let gstr = if animated {
        0.4 + 0.45 * ((anim as f64 * 0.12).sin() * 0.5 + 0.5)
    } else {
        0.6
    };
    let glow_r = r + 3.0;
    for y in 0..h {
        let py = y as f64;
        let vy = ((py - y0) / (y1 - y0).max(1.0)).clamp(0.0, 1.0);
        // Vertical gloss: brightest just above the middle, darker toward the base.
        let gloss = (1.18 - 0.55 * (vy - 0.28).abs()).clamp(0.6, 1.28);
        for x in 0..w {
            let px = x as f64;
            let mut c = bg;
            let dbody = capsule(px, py, 0.0, 0.0);
            // Outer glow halo around the body (focused buttons only).
            if let Some(gc) = glow
                && dbody > 0.0
                && dbody < glow_r
            {
                let t = 1.0 - dbody / glow_r;
                c = over(c, gc, t * t * gstr);
            }
            // Drop shadow: the body silhouette offset toward the bottom-right,
            // drawn only where the body itself won't cover it.
            let dsh = capsule(px, py, so, so);
            if dbody > 0.0 && dsh < 0.9 {
                let cover = (0.9 - dsh).clamp(0.0, 1.0);
                c = over(c, (0, 0, 0), cover * 0.42);
            }
            // Body (anti-aliased edge), with a bright top rim and dark bottom rim.
            if dbody < 0.9 {
                let cover = (0.9 - dbody).clamp(0.0, 1.0);
                let mut bc = shade(fill, gloss);
                if dbody < 0.0 {
                    if py <= y0 + 1.2 {
                        bc = over(bc, (255, 255, 255), 0.30);
                    } else if py >= y1 - 1.2 {
                        bc = over(bc, (0, 0, 0), 0.20);
                    }
                }
                c = over(c, bc, cover);
            }
            put(&mut img, x, y, c);
        }
    }
    img
}

/// A blank RGBA image filled with `bg`, for drawing multiple pillows into.
pub fn canvas(w: u32, h: u32, bg: Rgb) -> RgbaImage {
    RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([bg.0, bg.1, bg.2, 255]))
}

/// HSV → RGB (`h` in degrees, `s`/`v` in `[0, 1]`). Used to give treemap boxes
/// distinct hues.
pub fn hsv(h: f64, s: f64, v: f64) -> Rgb {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s.clamp(0.0, 1.0);
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |z: f64| ((z + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (q(r), q(g), q(b))
}

/// Blit anti-aliased `text` into `img` with its top-left at `(x, y)`, at font
/// pixel size `px`, in color `fg` over an optional darkened `plate` (a rectangle
/// drawn behind the run for legibility). Each glyph is rasterized by the bundled
/// font and alpha-composited, so the text looks smooth at any size. Text baked
/// this way survives every graphics protocol, unlike cell text drawn over an
/// image. Off-image pixels are clipped.
pub fn draw_text(img: &mut RgbaImage, x: i32, y: i32, text: &str, fg: Rgb, plate: Option<Rgb>, px: f32) {
    let font = &*FONT;
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    let ascent = font.horizontal_line_metrics(px).map(|m| m.ascent).unwrap_or(px * 0.8);
    let baseline = y as f32 + ascent;

    if let Some(pl) = plate {
        let tw = text_width(text, px) as i32;
        let th = text_height(px) as i32;
        for py in (y - 1)..(y + th + 1) {
            for pxi in (x - 1)..(x + tw + 1) {
                if pxi >= 0 && py >= 0 && pxi < iw && py < ih {
                    let p = img.get_pixel(pxi as u32, py as u32).0;
                    let c = over((p[0], p[1], p[2]), pl, 0.6);
                    img.put_pixel(pxi as u32, py as u32, Rgba([c.0, c.1, c.2, 255]));
                }
            }
        }
    }

    let mut cursor = x as f32;
    for ch in text.chars() {
        // Skip characters the font has no glyph for, so a missing script draws a
        // gap rather than a `.notdef` "tofu" box. Still advance to keep layout.
        if !ch.is_whitespace() && !font.has_glyph(ch) {
            cursor += font.metrics(ch, px).advance_width;
            continue;
        }
        let (m, bitmap) = font.rasterize(ch, px);
        if m.width > 0 && m.height > 0 {
            let gx0 = (cursor + m.xmin as f32).round() as i32;
            // Bitmap top in screen coords: baseline − glyph-height − bottom-bearing.
            let gy0 = (baseline - m.height as f32 - m.ymin as f32).round() as i32;
            for gy in 0..m.height {
                for gx in 0..m.width {
                    let a = bitmap[gy * m.width + gx];
                    if a == 0 {
                        continue;
                    }
                    let sx = gx0 + gx as i32;
                    let sy = gy0 + gy as i32;
                    if sx >= 0 && sy >= 0 && sx < iw && sy < ih {
                        let p = img.get_pixel(sx as u32, sy as u32).0;
                        let c = over((p[0], p[1], p[2]), fg, a as f64 / 255.0);
                        img.put_pixel(sx as u32, sy as u32, Rgba([c.0, c.1, c.2, 255]));
                    }
                }
            }
        }
        cursor += m.advance_width;
    }
}

/// Pixel width of `text` rendered by [`draw_text`] at font size `px`.
pub fn text_width(text: &str, px: f32) -> u32 {
    let font = &*FONT;
    let w: f32 = text.chars().map(|c| font.metrics(c, px).advance_width).sum();
    w.ceil() as u32
}

/// Vertical extent (ascent + descent) of a line at font size `px`, for centering.
pub fn text_height(px: f32) -> u32 {
    match FONT.horizontal_line_metrics(px) {
        Some(m) => (m.ascent - m.descent).ceil().max(1.0) as u32,
        None => px.ceil() as u32,
    }
}

/// Advance width of a representative glyph at font size `px` (used to budget how
/// many characters fit in a given pixel width before ellipsizing).
pub fn char_advance(px: f32) -> f32 {
    FONT.metrics('n', px).advance_width.max(1.0)
}

/// Whether the bundled font can render every (non-space) character in `s` — i.e.
/// baking it produces real glyphs, not `.notdef` "tofu" boxes. The font covers
/// Latin, Cyrillic and Greek but not, say, Arabic or CJK; callers use this to
/// fall back to an English label rather than bake unreadable boxes.
pub fn font_can_render(s: &str) -> bool {
    let font = &*FONT;
    s.chars().all(|c| c.is_whitespace() || font.has_glyph(c))
}

/// A recessed sub-panel inside a [`pillow_box`]: a pixel rect and its fill color.
pub struct SubBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Rgb,
}

/// Draw one "pillow" box into `img` at `(ox, oy)` of size `w × h`: a cushion-
/// shaded fill in `fill` (brighter in the middle so it reads as raised), each
/// [`SubBox`] (box-local pixel coords) drawn as a semi-transparent, bevelled
/// depression so it sits *below* the surface, and an optional bright selection
/// `border`. A 1px "grout" gap is left around the box so adjacent boxes stay
/// distinct when many are drawn into a single image.
#[allow(clippy::too_many_arguments)]
pub fn pillow_into(
    img: &mut RgbaImage,
    ox: u32,
    oy: u32,
    w: u32,
    h: u32,
    fill: Rgb,
    subs: &[SubBox],
    border: Option<Rgb>,
) {
    let (iw_max, ih_max) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return;
    }
    // Interior (inside the 1px grout), clipped to the image bounds.
    let ix0 = ox + 1;
    let iy0 = oy + 1;
    let ix1 = (ox + w).saturating_sub(1).min(iw_max);
    let iy1 = (oy + h).saturating_sub(1).min(ih_max);
    if ix1 <= ix0 || iy1 <= iy0 {
        for y in oy..(oy + h).min(ih_max) {
            for x in ox..(ox + w).min(iw_max) {
                put(img, x, y, fill); // too small for grout: solid fill
            }
        }
        return;
    }
    let iw = (ix1 - ix0).max(1) as f64;
    let ih = (iy1 - iy0).max(1) as f64;
    // Cushion: a soft radial bump, brightest at the centre, dimmer toward edges.
    for y in iy0..iy1 {
        let ly = (y - iy0) as f64 / ih * 2.0 - 1.0;
        for x in ix0..ix1 {
            let lx = (x - ix0) as f64 / iw * 2.0 - 1.0;
            let cushion = (1.14 - 0.44 * (0.75 * lx * lx + ly * ly)).clamp(0.55, 1.25);
            put(img, x, y, shade(fill, cushion));
        }
    }
    // Recessed, semi-transparent sub-boxes (offset into the interior).
    for s in subs {
        let x0 = ix0 + s.x.round().max(0.0) as u32;
        let y0 = iy0 + s.y.round().max(0.0) as u32;
        let x1 = (ix0 + (s.x + s.w).round().max(0.0) as u32).min(ix1);
        let y1 = (iy0 + (s.y + s.h).round().max(0.0) as u32).min(iy1);
        if x1 < x0 + 3 || y1 < y0 + 3 {
            continue; // too small to read as a box
        }
        let (jx0, jy0, jx1, jy1) = (x0 + 1, y0 + 1, x1 - 1, y1 - 1);
        for y in jy0..jy1 {
            for x in jx0..jx1 {
                let p = img.get_pixel(x, y).0;
                let under = (p[0], p[1], p[2]);
                // A ~50%-transparent dark inset: the cushion still shows through,
                // but the panel is clearly darker so it reads as recessed *inside*
                // the pillow. A light per-box tint keeps neighbours distinct.
                let mut c = over(under, (0, 0, 0), 0.5);
                c = over(c, s.color, 0.16);
                // 2px bevel: dark shadow on the top/left, bright rim on bottom/right.
                if y <= jy0 + 1 || x <= jx0 + 1 {
                    c = over(c, (0, 0, 0), 0.42);
                } else if y + 2 >= jy1 || x + 2 >= jx1 {
                    c = over(c, (255, 255, 255), 0.24);
                }
                put(img, x, y, c);
            }
        }
    }
    // Selection border: a bright 1px rectangle just inside the grout.
    if let Some(bc) = border {
        for x in ix0..ix1 {
            put(img, x, iy0, bc);
            put(img, x, iy1 - 1, bc);
        }
        for y in iy0..iy1 {
            put(img, ix0, y, bc);
            put(img, ix1 - 1, y, bc);
        }
    }
}

/// A standalone "pillow" box image (see [`pillow_into`]).
pub fn pillow_box(w: u32, h: u32, fill: Rgb, subs: &[SubBox], bg: Rgb) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(w.max(1), h.max(1), Rgba([bg.0, bg.1, bg.2, 255]));
    pillow_into(&mut img, 0, 0, w.max(1), h.max(1), fill, subs, None);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_bg(img: &RgbaImage, x: u32, y: u32, bg: Rgb) -> bool {
        let p = img.get_pixel(x, y).0;
        (p[0], p[1], p[2]) == bg
    }

    #[test]
    fn over_blends_between_endpoints() {
        assert_eq!(over((0, 0, 0), (255, 255, 255), 0.0), (0, 0, 0));
        assert_eq!(over((0, 0, 0), (255, 255, 255), 1.0), (255, 255, 255));
        assert_eq!(over((0, 0, 0), (200, 100, 40), 0.5), (100, 50, 20));
    }

    #[test]
    fn load_rgb_runs_green_to_red() {
        assert_eq!(load_rgb(0.0), (40, 200, 140)); // idle → green
        assert_eq!(load_rgb(50.0), (220, 200, 40)); // mid → yellow
        assert_eq!(load_rgb(100.0), (230, 60, 50)); // busy → red
    }

    #[test]
    fn gradient_bar_fills_left_to_right_to_frac() {
        let bg = (10, 10, 10);
        let img = gradient_bar(100, 8, 0.5, |_| (0, 200, 0), (30, 30, 30), bg);
        let midy = img.height() / 2; // avoid the rounded corner rows
        // Deep in the filled half: not background.
        assert!(!is_bg(&img, 25, midy, bg), "left half should be filled");
        // Past the fill fraction: the empty-track color, not the fill color.
        let p = img.get_pixel(80, midy).0;
        assert_eq!((p[0], p[1], p[2]), (30, 30, 30), "right of frac is the empty track");
    }

    #[test]
    fn gradient_bar_zero_and_full() {
        let bg = (0, 0, 0);
        let empty = (20, 20, 20);
        let none = gradient_bar(40, 6, 0.0, |_| (0, 255, 0), empty, bg);
        let midy = none.height() / 2;
        assert_eq!(none.get_pixel(20, midy).0[1], 20, "0% shows only the empty track");
        let full = gradient_bar(40, 6, 1.0, |_| (0, 255, 0), empty, bg);
        assert!(full.get_pixel(20, midy).0[1] > 100, "100% is filled green");
    }

    #[test]
    fn mirror_bars_draws_axis_and_splits_up_down() {
        let bg = (0, 0, 0);
        let up = vec![1.0; 8];
        let down = vec![1.0; 8];
        let img = mirror_bars(20, 30, &up, &down, 1.0, (0, 0, 255), (255, 0, 0), (80, 80, 80), bg, 0.5);
        let axis_y = 15u32; // h * 0.5
        // The axis line is present.
        assert_eq!(img.get_pixel(10, axis_y).0[0], 80);
        // Above the axis is the "up" (blue) color; below is "down" (red).
        assert!(img.get_pixel(10, axis_y - 3).0[2] > 100, "above axis is blue-ish");
        assert!(img.get_pixel(10, axis_y + 3).0[0] > 100, "below axis is red-ish");
    }

    #[test]
    fn area_spark_taller_sample_fills_higher() {
        let bg = (0, 0, 0);
        let low = area_spark(4, 20, &[0.1], 1.0, |_| (0, 200, 0), bg);
        let high = area_spark(4, 20, &[0.9], 1.0, |_| (0, 200, 0), bg);
        let filled = |img: &RgbaImage, x: u32| (0..img.height()).filter(|&y| !is_bg(img, x, y, bg)).count();
        assert!(filled(&high, 2) > filled(&low, 2), "higher value → more filled pixels");
    }

    #[test]
    fn pillow_box_cushions_and_recesses_subboxes() {
        let fill = (120, 120, 120);
        let bg = (0, 0, 0);
        // One sub-box in the middle of a 40x40 pillow.
        let subs = vec![SubBox { x: 10.0, y: 10.0, w: 20.0, h: 20.0, color: (60, 60, 60) }];
        let img = pillow_box(40, 40, fill, &subs, bg);
        let lum = |x: u32, y: u32| {
            let p = img.get_pixel(x, y).0;
            p[0] as u32 + p[1] as u32 + p[2] as u32
        };
        // Cushion: the centre is brighter than a corner (raised look).
        assert!(lum(20, 20) > lum(1, 1), "cushion centre should be brighter than the edge");
        // The sub-box interior is drawn (its top/left bevel is darker than its
        // own centre → a recessed, bevelled look).
        assert!(lum(12, 12) < lum(20, 20), "sub-box shadow edge darker than pillow centre");
    }

    #[test]
    fn button_has_body_shadow_and_focus_glow() {
        let fill = (40, 90, 200);
        let bg = (10, 10, 10);
        // Unfocused: a filled body over the background, with a bottom-right shadow.
        let plain = button(80, 24, fill, None, bg, 0, false);
        let (cx, cy) = (plain.width() / 2, plain.height() / 2);
        assert!(!is_bg(&plain, cx, cy, bg), "the button body is filled");
        // A pixel just inside the bottom-right corner is darkened by the shadow
        // (neither the flat body color nor the untouched background).
        let sx = plain.width() - 2;
        let sy = plain.height() - 2;
        let sp = plain.get_pixel(sx, sy).0;
        let lum = |p: [u8; 4]| p[0] as u32 + p[1] as u32 + p[2] as u32;
        assert!(lum(sp) < lum([bg.0, bg.1, bg.2, 255]) + 60, "drop shadow darkens the corner");

        // Focused: a glow tint appears in the margin around the body that is plain
        // background when unfocused.
        let glow = (120, 200, 255);
        let lit = button(80, 24, fill, Some(glow), bg, 0, false);
        // Sample a column at the far left edge, above/below the body center where
        // the halo bleeds into the padding.
        let edge_glows = (0..lit.height())
            .any(|y| !is_bg(&lit, 1, y, bg) && is_bg(&plain, 1, y, bg));
        assert!(edge_glows, "focus glow tints the padding around the body");
    }

    #[test]
    fn hsv_primaries_are_correct() {
        assert_eq!(hsv(0.0, 1.0, 1.0), (255, 0, 0)); // red
        assert_eq!(hsv(120.0, 1.0, 1.0), (0, 255, 0)); // green
        assert_eq!(hsv(240.0, 1.0, 1.0), (0, 0, 255)); // blue
        assert_eq!(hsv(0.0, 0.0, 0.5), (128, 128, 128)); // desaturated grey
    }

    #[test]
    fn draw_text_sets_glyph_pixels() {
        let mut img = RgbaImage::from_pixel(64, 24, Rgba([0, 0, 0, 255]));
        draw_text(&mut img, 1, 1, "A", (255, 255, 255), None, 18.0);
        // Some pixels within the glyph box are now lit; the far corner is not.
        let lit = |x: u32, y: u32| img.get_pixel(x, y).0[0] > 0;
        let any = (1..18).flat_map(|x| (1..22).map(move |y| (x, y))).any(|(x, y)| lit(x, y));
        assert!(any, "glyph 'A' should light some pixels");
        assert!(!lit(60, 22), "pixels outside the text stay background");
        // Anti-aliasing produces intermediate (grey) intensities, not just on/off.
        let has_partial =
            (0..img.width()).flat_map(|x| (0..img.height()).map(move |y| (x, y))).any(|(x, y)| {
                let v = img.get_pixel(x, y).0[0];
                v > 0 && v < 255
            });
        assert!(has_partial, "anti-aliased glyph edges should have partial intensities");
    }

    #[test]
    fn text_metrics_scale_with_size() {
        // Wider text and taller lines at a larger font size.
        assert!(text_width("Cancel", 24.0) > text_width("Cancel", 12.0));
        assert!(text_height(24.0) > text_height(12.0));
        assert!(char_advance(20.0) > 1.0);
    }

    #[test]
    fn font_can_render_covers_latin_cyrillic_greek_not_arabic_cjk() {
        assert!(font_can_render("OK"));
        assert!(font_can_render("Cancel 123"));
        assert!(font_can_render("Отмена")); // Cyrillic
        assert!(font_can_render("Ελληνικά")); // Greek
        assert!(!font_can_render("موافق")); // Arabic
        assert!(!font_can_render("取消")); // CJK
        // A single unsupported character makes the whole string unrenderable.
        assert!(!font_can_render("OK取"));
    }

    #[test]
    fn draw_text_skips_unrenderable_chars_instead_of_tofu() {
        // An unsupported script bakes no pixels at all (no ".notdef" tofu boxes).
        let mut a = RgbaImage::from_pixel(80, 24, Rgba([0, 0, 0, 255]));
        draw_text(&mut a, 1, 1, "取消", (255, 255, 255), None, 18.0);
        assert_eq!(a.pixels().filter(|p| p.0[0] > 0).count(), 0, "no tofu baked");
        // Renderable text still bakes glyph pixels.
        let mut b = RgbaImage::from_pixel(80, 24, Rgba([0, 0, 0, 255]));
        draw_text(&mut b, 1, 1, "OK", (255, 255, 255), None, 18.0);
        assert!(b.pixels().any(|p| p.0[0] > 0), "renderable text is drawn");
    }

    #[test]
    fn series_at_interpolates_and_puts_newest_right() {
        // Two samples: oldest 0 at the left, newest 10 at the right.
        assert_eq!(series_at(&[0.0, 10.0], 0.0), 0.0);
        assert_eq!(series_at(&[0.0, 10.0], 1.0), 10.0);
        assert_eq!(series_at(&[0.0, 10.0], 0.5), 5.0);
        assert_eq!(series_at(&[], 0.5), 0.0);
    }
}
