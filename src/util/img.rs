//! Shared image helpers: format detection, decoding to a thumbnail (optionally
//! via an embedded EXIF preview), an EXIF summary, and cell-based rendering
//! (centering + half-block art). Used by the Details-view preview and the F3
//! fullscreen image viewer.

use image::RgbaImage;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Whether `name`'s extension is a decodable image format.
pub fn is_image_name(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp")
}

/// Decode `bytes` and shrink to at most `max_edge` px on the longest side
/// (aspect preserved). With `prefer_embedded`, a small embedded EXIF thumbnail
/// is used when present (cheap) before falling back to a full-resolution decode.
/// `None` on any decode failure.
pub fn decode_scaled(bytes: &[u8], max_edge: u32, prefer_embedded: bool) -> Option<RgbaImage> {
    let decoded = if prefer_embedded {
        embedded_thumbnail(bytes)
            .and_then(|t| image::load_from_memory(&t).ok())
            .or_else(|| image::load_from_memory(bytes).ok())?
    } else {
        image::load_from_memory(bytes).ok()?
    };
    Some(decoded.thumbnail(max_edge, max_edge).to_rgba8())
}

/// A cheap content signature for the graphics cache.
pub fn image_sig(img: &RgbaImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    img.width().hash(&mut h);
    img.height().hash(&mut h);
    img.as_raw().hash(&mut h);
    h.finish()
}

/// The JPEG thumbnail embedded in EXIF metadata, if any (JPEG/TIFF photos carry
/// these). Using it avoids decoding the full-resolution image.
fn embedded_thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(bytes))
        .ok()?;
    let off = exif
        .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let len = exif
        .get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    exif.buf().get(off..off.checked_add(len)?).map(<[u8]>::to_vec)
}

/// A compact human-readable summary of the most useful EXIF fields (empty when
/// the image carries none). Labels are English keys the caller can localize.
pub fn exif_summary(bytes: &[u8]) -> Vec<(String, String)> {
    let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)) else {
        return Vec::new();
    };
    let field = |tag| -> Option<String> {
        let f = exif.get_field(tag, exif::In::PRIMARY)?;
        let s = f.display_value().with_unit(&exif).to_string();
        let s = s.trim().trim_matches('"').trim().to_string();
        (!s.is_empty()).then_some(s)
    };
    let mut out: Vec<(String, String)> = Vec::new();
    let camera = match (field(exif::Tag::Make), field(exif::Tag::Model)) {
        (Some(mk), Some(md)) => Some(if md.starts_with(&mk) { md } else { format!("{mk} {md}") }),
        (Some(s), None) | (None, Some(s)) => Some(s),
        (None, None) => None,
    };
    if let Some(c) = camera {
        out.push(("Camera".into(), c));
    }
    if let Some(l) = field(exif::Tag::LensModel) {
        out.push(("Lens".into(), l));
    }
    if let Some(d) = field(exif::Tag::DateTimeOriginal) {
        out.push(("Taken".into(), d));
    }
    let mut exp: Vec<String> = Vec::new();
    if let Some(t) = field(exif::Tag::ExposureTime) {
        exp.push(t);
    }
    if let Some(fnum) = field(exif::Tag::FNumber) {
        exp.push(fnum);
    }
    if let Some(iso) = field(exif::Tag::PhotographicSensitivity) {
        exp.push(format!("ISO {iso}"));
    }
    if let Some(fl) = field(exif::Tag::FocalLength) {
        exp.push(fl);
    }
    if !exp.is_empty() {
        out.push(("Exposure".into(), exp.join("  ")));
    }
    out
}

/// The largest cell rect within `area` that keeps the image's aspect ratio,
/// centred both ways. `cell` is the terminal's (pixel-width, height) per cell,
/// so the target reflects true pixel proportions rather than the ~1:2 cell shape.
pub fn center_rect(area: Rect, iw: u32, ih: u32, cell: (u32, u32)) -> Rect {
    let (cw, ch) = (cell.0.max(1), cell.1.max(1));
    let (iw, ih) = (iw.max(1), ih.max(1));
    let (avail_w, avail_h) = (area.width as u32 * cw, area.height as u32 * ch);
    let scale = (avail_w as f64 / iw as f64).min(avail_h as f64 / ih as f64);
    let pw = (iw as f64 * scale).round().max(1.0) as u32;
    let ph = (ih as f64 * scale).round().max(1.0) as u32;
    let tw = (pw.div_ceil(cw) as u16).clamp(1, area.width);
    let th = (ph.div_ceil(ch) as u16).clamp(1, area.height);
    Rect {
        x: area.x + (area.width - tw) / 2,
        y: area.y + (area.height - th) / 2,
        width: tw,
        height: th,
    }
}

/// Render an image as centred half-block cell art (each cell is two vertically
/// stacked pixels via `▀`, upper = foreground, lower = background) — the fallback
/// when no pixel-graphics protocol is available. `bg` fills the letterbox and the
/// lower half of a final odd row.
pub fn render_halfblocks(f: &mut Frame, area: Rect, img: &RgbaImage, bg: Color) {
    use image::imageops::FilterType;
    let (cols, rows) = (area.width as u32, area.height as u32);
    if cols == 0 || rows == 0 {
        return;
    }
    // Fit within cols × (2·rows) pixels, preserving aspect ratio.
    let (iw, ih) = (img.width().max(1), img.height().max(1));
    let scale = (cols as f64 / iw as f64).min((rows * 2) as f64 / ih as f64);
    let tw = ((iw as f64 * scale).round() as u32).clamp(1, cols);
    let th = ((ih as f64 * scale).round() as u32).clamp(1, rows * 2);
    let small = image::imageops::resize(img, tw, th, FilterType::Triangle);
    let cell_rows = th.div_ceil(2);
    let x0 = area.x + ((cols - tw) / 2) as u16;
    let y0 = area.y + ((rows - cell_rows) / 2) as u16;
    for cy in 0..cell_rows {
        let y = y0 + cy as u16;
        if y >= area.y + area.height {
            break;
        }
        let mut spans: Vec<Span> = Vec::with_capacity(tw as usize);
        for cx in 0..tw {
            let top = small.get_pixel(cx, cy * 2);
            let top_c = Color::Rgb(top[0], top[1], top[2]);
            let by = cy * 2 + 1;
            let bot_c = if by < th {
                let b = small.get_pixel(cx, by);
                Color::Rgb(b[0], b[1], b[2])
            } else {
                bg
            };
            spans.push(Span::styled("▀".to_string(), Style::default().fg(top_c).bg(bot_c)));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect { x: x0, y, width: tw as u16, height: 1 },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_image_name_by_extension() {
        assert!(is_image_name("photo.JPG") && is_image_name("a.png") && is_image_name("x.webp"));
        assert!(!is_image_name("notes.txt") && !is_image_name("archive.zip") && !is_image_name("noext"));
    }

    #[test]
    fn center_rect_preserves_aspect_and_centers() {
        // 20×10 cells at 10×20 px/cell → a 200×200 px canvas.
        let area = Rect::new(0, 0, 20, 10);
        let cell = (10, 20);
        // A square image fills the canvas exactly at the origin.
        let r = center_rect(area, 100, 100, cell);
        assert_eq!((r.x, r.y, r.width, r.height), (0, 0, 20, 10));
        // A wide image letterboxes vertically, centred.
        let r = center_rect(area, 200, 50, cell);
        assert_eq!(r.width, 20);
        assert!(r.height < 10 && r.y > 0);
        assert_eq!(r.y, (area.height - r.height) / 2);
        // A tall image letterboxes horizontally, centred.
        let r = center_rect(area, 50, 200, cell);
        assert!(r.width < 20 && r.x > 0);
    }

    #[test]
    fn decode_scaled_shrinks_within_max_edge() {
        // Encode a small PNG in memory, then decode+scale it.
        let img = RgbaImage::from_pixel(40, 20, image::Rgba([9, 200, 30, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let out = decode_scaled(buf.get_ref(), 10, false).expect("decodes");
        assert!(out.width() <= 10 && out.height() <= 10, "fits within max edge");
        assert!(decode_scaled(b"not an image", 10, false).is_none());
    }
}
