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
    fn series_at_interpolates_and_puts_newest_right() {
        // Two samples: oldest 0 at the left, newest 10 at the right.
        assert_eq!(series_at(&[0.0, 10.0], 0.0), 0.0);
        assert_eq!(series_at(&[0.0, 10.0], 1.0), 10.0);
        assert_eq!(series_at(&[0.0, 10.0], 0.5), 5.0);
        assert_eq!(series_at(&[], 0.5), 0.0);
    }
}
