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
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "svgz")
}

/// Largest SVG (bytes, compressed or not) rendered for a thumbnail or the
/// Details preview. A vector file is cheap on disk but not to rasterize — a
/// detailed map of a few megabytes takes seconds — so the cap sits well below
/// the raster formats'.
pub const SVG_PREVIEW_MAX: usize = 4 * 1024 * 1024;
/// Most bytes a `.svgz` is inflated to.
const SVG_INFLATE_MAX: u64 = 32 * 1024 * 1024;

/// Whether `bytes` are an SVG document: gzip (an `.svgz`), or an `<svg` element
/// near the start, after any XML declaration, comments and doctype.
pub fn is_svg_bytes(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        // A gzip stream is only an SVG once inflated; peek at its head.
        use std::io::Read;
        let mut head = Vec::new();
        let _ = flate2::read::GzDecoder::new(bytes).take(4096).read_to_end(&mut head);
        return !head.starts_with(&[0x1f, 0x8b]) && is_svg_bytes(&head);
    }
    let head = &bytes[..bytes.len().min(4096)];
    memchr::memmem::find(head, b"<svg").is_some()
        && head.iter().find(|b| !b.is_ascii_whitespace()).is_some_and(|&b| b == b'<' || b == 0xEF)
}

/// Rasterize an SVG (or gzipped `.svgz`) so its longest edge is its own size
/// clamped to `min_edge..=max_edge` — a small icon is scaled up to stay crisp,
/// a huge poster down. Returns the picture and the document's own size in px.
///
/// Only `data:` images embedded in the file are drawn: an `<image href>` naming
/// a path is ignored, so viewing an SVG can't make the program read other files.
pub fn render_svg(bytes: &[u8], max_edge: u32, min_edge: u32) -> Option<(RgbaImage, (u32, u32))> {
    use resvg::{tiny_skia, usvg};
    let inflated;
    let data = if bytes.starts_with(&[0x1f, 0x8b]) {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(bytes).take(SVG_INFLATE_MAX).read_to_end(&mut out).ok()?;
        inflated = out;
        &inflated[..]
    } else {
        bytes
    };
    let opt = usvg::Options {
        resources_dir: None,
        fontdb: svg_fonts(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_data(data, &opt).ok()?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    let longest = w.max(h);
    if !(longest.is_finite() && longest > 0.0) {
        return None;
    }
    let target = longest.clamp(min_edge.max(1) as f32, max_edge.max(1) as f32);
    let scale = target / longest;
    let pw = ((w * scale).round() as u32).max(1);
    let ph = ((h * scale).round() as u32).max(1);
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    let raw: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    let img = RgbaImage::from_raw(pw, ph, raw)?;
    Some((img, (w.round() as u32, h.round() as u32)))
}

/// Composite `img` over an opaque `bg`, leaving every pixel fully opaque.
pub fn flatten_onto(img: &mut RgbaImage, bg: [u8; 3]) {
    for p in img.pixels_mut() {
        let a = p[3] as u32;
        for (c, b) in p.0.iter_mut().zip(bg) {
            *c = ((*c as u32 * a + b as u32 * (255 - a) + 127) / 255) as u8;
        }
        p[3] = 255;
    }
}

/// The fonts SVG text is drawn with: the system's, loaded once. Where there are
/// none (a minimal install), the embedded Ubuntu face stands in for every
/// generic family so text still shows.
fn svg_fonts() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
    use std::sync::{Arc, LazyLock};
    static DB: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        if db.is_empty() {
            db.load_font_data(epaint_default_fonts::UBUNTU_LIGHT.to_vec());
            db.set_serif_family("Ubuntu");
            db.set_sans_serif_family("Ubuntu");
            db.set_monospace_family("Ubuntu");
            db.set_cursive_family("Ubuntu");
            db.set_fantasy_family("Ubuntu");
        }
        Arc::new(db)
    });
    DB.clone()
}

/// Decode `bytes` and shrink to at most `max_edge` px on the longest side
/// (aspect preserved). With `prefer_embedded`, a small embedded EXIF thumbnail
/// is used when present (cheap) before falling back to a full-resolution decode.
/// `None` on any decode failure.
pub fn decode_scaled(bytes: &[u8], max_edge: u32, prefer_embedded: bool) -> Option<RgbaImage> {
    // A vector has no resolution to preserve, so it fills the box asked for.
    if is_svg_bytes(bytes) {
        if bytes.len() > SVG_PREVIEW_MAX {
            return None;
        }
        return render_svg(bytes, max_edge, max_edge).map(|(img, _)| img);
    }
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
    let exif = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)).ok()?;
    let off =
        exif.get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)?.value.get_uint(0)?
            as usize;
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

/// Every EXIF value a rename mask can reach, as `(token, value)` pairs with the
/// tokens already lower-cased — what [`crate::rename::FileMeta`] is built from.
///
/// Unlike [`exif_summary`], which composes a few display lines for the Details
/// panel, this keeps the values apart and unformatted: the capture time is
/// split so a mask can use the year alone, the ISO is a bare number, and GPS is
/// signed decimal degrees rather than the `51 deg 28' 38"` the display form
/// would give. Empty when the file carries no EXIF.
pub fn exif_fields(bytes: &[u8]) -> Vec<(String, String)> {
    let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(bytes)) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: String| {
        if !v.is_empty() {
            out.push((k.to_string(), v));
        }
    };
    let text = |tag| display_field(&exif, tag, exif::In::PRIMARY);

    // The moment the photo was taken, whole and in parts, so a mask can build
    // any arrangement of it. `DateTimeOriginal` is when the shutter fired;
    // `DateTime` is when the file was last written, which is the weaker answer.
    let when = text(exif::Tag::DateTimeOriginal).or_else(|| text(exif::Tag::DateTime));
    if let Some((d, t)) = when.as_deref().and_then(parse_exif_datetime) {
        put("ymd", format!("{:04}{:02}{:02}", d.0, d.1, d.2));
        put("hms", format!("{:02}{:02}{:02}", t.0, t.1, t.2));
        put("y", format!("{:04}", d.0));
        put("m", format!("{:02}", d.1));
        put("d", format!("{:02}", d.2));
        put("h", format!("{:02}", t.0));
        put("min", format!("{:02}", t.1));
        put("s", format!("{:02}", t.2));
    }

    if let Some(v) = text(exif::Tag::Make) {
        put("make", v);
    }
    if let Some(v) = text(exif::Tag::Model) {
        put("model", v);
    }
    if let Some(v) = text(exif::Tag::LensModel) {
        put("lens", v);
    }
    if let Some(v) = text(exif::Tag::ExposureTime) {
        put("exposure", v);
    }
    if let Some(v) = text(exif::Tag::FNumber) {
        put("fnumber", v);
    }
    if let Some(v) = text(exif::Tag::PhotographicSensitivity) {
        put("iso", v);
    }
    if let Some(v) = text(exif::Tag::FocalLength) {
        put("focallength", v);
    }
    if let Some(v) = text(exif::Tag::Orientation) {
        put("orientation", v);
    }
    // Dimensions: the EXIF pixel tags where they exist, since reading them
    // beats decoding the image to find out.
    if let Some(v) = uint_field(&exif, exif::Tag::PixelXDimension) {
        put("width", v.to_string());
    }
    if let Some(v) = uint_field(&exif, exif::Tag::PixelYDimension) {
        put("height", v.to_string());
    }
    if let Some(v) = gps_degrees(&exif, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef) {
        put("gpslat", format!("{v:.6}"));
    }
    if let Some(v) = gps_degrees(&exif, exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef) {
        put("gpslon", format!("{v:.6}"));
    }
    out
}

/// One EXIF field as its display string, trimmed of the quotes the formatter
/// puts around text values. `None` when absent or empty.
fn display_field(exif: &exif::Exif, tag: exif::Tag, ifd: exif::In) -> Option<String> {
    let f = exif.get_field(tag, ifd)?;
    let s = f.display_value().with_unit(exif).to_string();
    let s = s.trim().trim_matches('"').trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// One EXIF field as an unsigned number.
fn uint_field(exif: &exif::Exif, tag: exif::Tag) -> Option<u32> {
    exif.get_field(tag, exif::In::PRIMARY)?.value.get_uint(0)
}

/// A civil date and time read out of an EXIF timestamp: `(y, m, d)`, `(h, min, s)`.
type DateTime = ((u32, u32, u32), (u32, u32, u32));

/// An EXIF timestamp split into its parts.
///
/// Read as six numbers separated by anything, rather than by matching a layout:
/// the bytes in the file are `YYYY:MM:DD HH:MM:SS`, but this is handed the
/// *display* form, which renders the date with dashes — and a camera that
/// writes some third punctuation is then no trouble either.
///
/// A camera that has never had its clock set writes zeroes; those are not a
/// date and would rename a photo to `00000000`, so they are refused here.
fn parse_exif_datetime(s: &str) -> Option<DateTime> {
    let mut n = s
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u32>().ok());
    let (y, mo, da) = (n.next()??, n.next()??, n.next()??);
    let (h, mi, se) = (n.next()??, n.next()??, n.next()??);
    if y == 0 || mo == 0 || da == 0 || mo > 12 || da > 31 || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    Some(((y, mo, da), (h, mi, se)))
}

/// A GPS coordinate as signed decimal degrees: the degrees/minutes/seconds
/// rational triple combined, negated for a southern latitude or a western
/// longitude. `None` when either the value or its hemisphere is missing.
fn gps_degrees(exif: &exif::Exif, tag: exif::Tag, ref_tag: exif::Tag) -> Option<f64> {
    let field = exif.get_field(tag, exif::In::PRIMARY)?;
    let exif::Value::Rational(parts) = &field.value else {
        return None;
    };
    let [deg, min, sec] = parts.get(..3)? else {
        return None;
    };
    let value = deg.to_f64() + min.to_f64() / 60.0 + sec.to_f64() / 3600.0;
    if !value.is_finite() {
        return None;
    }
    let hemisphere = display_field(exif, ref_tag, exif::In::PRIMARY)?;
    let south_or_west = matches!(hemisphere.trim().to_ascii_uppercase().as_str(), "S" | "W");
    Some(if south_or_west { -value } else { value })
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

/// Present `img` as an ASCII luminance ramp, one character per cell — the
/// fallback for terminals without 24-bit colour, where the half-block art would
/// be a smear of approximated colours.
///
/// The darkest character is `'.'` rather than a space on purpose: a run of
/// spaces at the end of a row is erased by the trailing-space trimmer, which
/// would punch holes in a dark part of the scene.
pub fn render_ascii_ramp(
    f: &mut Frame,
    area: Rect,
    img: &RgbaImage,
    theme: &crate::ui::theme::Theme,
) {
    use image::imageops::FilterType;
    const RAMP: [char; 10] = ['.', ':', '-', '=', '+', '*', '#', '%', '@', '█'];
    let (cols, rows) = (area.width as u32, area.height as u32);
    if cols == 0 || rows == 0 {
        return;
    }
    let small = image::imageops::resize(img, cols, rows, FilterType::Triangle);
    let style = Style::default().fg(theme.panel_fg).bg(theme.panel_bg);
    for y in 0..rows {
        let mut spans: Vec<Span> = Vec::with_capacity(cols as usize);
        for x in 0..cols {
            let p = small.get_pixel(x, y);
            // Rec. 601 luma, which tracks perceived brightness well enough for
            // a ten-step ramp.
            let lum = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0;
            let i = ((lum * RAMP.len() as f32) as usize).min(RAMP.len() - 1);
            spans.push(Span::styled(RAMP[i].to_string(), style));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect { x: area.x, y: area.y + y as u16, width: area.width, height: 1 },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_image_name_by_extension() {
        assert!(is_image_name("photo.JPG") && is_image_name("a.png") && is_image_name("x.webp"));
        assert!(
            !is_image_name("notes.txt") && !is_image_name("archive.zip") && !is_image_name("noext")
        );
    }

    const RED_SVG: &[u8] = br##"<?xml version="1.0"?>
<!-- a red square -->
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
  <rect width="10" height="10" fill="#ff0000"/>
</svg>"##;

    fn gzip(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn svg_is_an_image_by_name_and_by_content() {
        assert!(is_image_name("logo.SVG") && is_image_name("icons.svgz"));
        assert!(is_svg_bytes(RED_SVG) && is_svg_bytes(&gzip(RED_SVG)));
        assert!(!is_svg_bytes(b"plain text mentioning <svg later"));
        assert!(!is_svg_bytes(&gzip(b"not an svg at all")));
    }

    #[test]
    fn svg_renders_scaled_up_to_the_minimum_edge() {
        let (img, orig) = render_svg(RED_SVG, 2000, 64).expect("renders");
        assert_eq!(orig, (10, 10));
        assert_eq!((img.width(), img.height()), (64, 64), "a 10 px icon is drawn at 64 px");
        assert_eq!(img.get_pixel(32, 32).0, [255, 0, 0, 255]);
        let (z, _) = render_svg(&gzip(RED_SVG), 32, 32).expect("svgz renders");
        assert_eq!(z.get_pixel(5, 5).0, [255, 0, 0, 255]);
        // Thumbnails and the Details preview go through decode_scaled.
        let t = decode_scaled(RED_SVG, 40, true).expect("decode_scaled takes SVG");
        assert_eq!(t.width(), 40);
    }

    #[test]
    fn svg_does_not_load_images_from_paths() {
        let dir = std::env::temp_dir().join(format!("rc-svg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("green.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 255, 0, 255])).save(&png).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><image href="{}" width="4" height="4"/></svg>"#,
            png.display()
        );
        let (img, _) = render_svg(svg.as_bytes(), 4, 4).expect("renders");
        assert_eq!(img.get_pixel(2, 2).0[3], 0, "the file the href names is not read");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flatten_composites_over_the_background() {
        let mut img = RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0]));
        flatten_onto(&mut img, [255, 255, 255]);
        assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255]);
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

    /// A JPEG carrying nothing but an EXIF block: `FF D8`, an APP1 segment
    /// holding a little-endian TIFF with `Model` and `DateTimeOriginal`, then
    /// `FF D9`. Built here rather than committed, like the other binary
    /// fixtures in this tree.
    fn jpeg_with_exif(model: &str, taken: &str) -> Vec<u8> {
        // Offsets below are from the start of the TIFF header.
        let model_bytes: Vec<u8> = model.bytes().chain([0]).collect();
        let taken_bytes: Vec<u8> = taken.bytes().chain([0]).collect();
        const IFD0: u32 = 8;
        const IFD0_LEN: u32 = 2 + 2 * 12 + 4; // two entries
        let exif_ifd = IFD0 + IFD0_LEN;
        const EXIF_IFD_LEN: u32 = 2 + 12 + 4; // one entry
        let model_at = exif_ifd + EXIF_IFD_LEN;
        let taken_at = model_at + model_bytes.len() as u32;

        let mut t: Vec<u8> = Vec::new();
        t.extend_from_slice(b"II");
        t.extend_from_slice(&42u16.to_le_bytes());
        t.extend_from_slice(&IFD0.to_le_bytes());
        // IFD0: Model, then the pointer to the Exif IFD.
        t.extend_from_slice(&2u16.to_le_bytes());
        let entry = |tag: u16, typ: u16, count: u32, value: u32| -> Vec<u8> {
            let mut e = Vec::with_capacity(12);
            e.extend_from_slice(&tag.to_le_bytes());
            e.extend_from_slice(&typ.to_le_bytes());
            e.extend_from_slice(&count.to_le_bytes());
            e.extend_from_slice(&value.to_le_bytes());
            e
        };
        t.extend_from_slice(&entry(0x0110, 2, model_bytes.len() as u32, model_at));
        t.extend_from_slice(&entry(0x8769, 4, 1, exif_ifd));
        t.extend_from_slice(&0u32.to_le_bytes()); // no IFD1
        // The Exif IFD: when the shutter fired.
        t.extend_from_slice(&1u16.to_le_bytes());
        t.extend_from_slice(&entry(0x9003, 2, taken_bytes.len() as u32, taken_at));
        t.extend_from_slice(&0u32.to_le_bytes());
        t.extend_from_slice(&model_bytes);
        t.extend_from_slice(&taken_bytes);

        let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
        out.extend_from_slice(&((2 + 6 + t.len()) as u16).to_be_bytes());
        out.extend_from_slice(b"Exif\0\0");
        out.extend_from_slice(&t);
        out.extend_from_slice(&[0xFF, 0xD9]);
        out
    }

    #[test]
    fn exif_fields_split_the_capture_time_into_parts() {
        let jpg = jpeg_with_exif("TestCam", "2024:07:14 15:09:33");
        let got: std::collections::HashMap<String, String> =
            exif_fields(&jpg).into_iter().collect();
        assert_eq!(got.get("model").map(String::as_str), Some("TestCam"));
        assert_eq!(got.get("ymd").map(String::as_str), Some("20240714"));
        assert_eq!(got.get("hms").map(String::as_str), Some("150933"));
        // Each part is separately reachable, so a mask can arrange them freely.
        assert_eq!(got.get("y").map(String::as_str), Some("2024"));
        assert_eq!(got.get("m").map(String::as_str), Some("07"));
        assert_eq!(got.get("d").map(String::as_str), Some("14"));
        assert_eq!(got.get("h").map(String::as_str), Some("15"));
        assert_eq!(got.get("min").map(String::as_str), Some("09"));
        assert_eq!(got.get("s").map(String::as_str), Some("33"));
        // Nothing is invented for tags the file does not carry.
        assert!(!got.contains_key("gpslat") && !got.contains_key("lens"));
    }

    #[test]
    fn exif_fields_are_empty_without_exif() {
        assert!(exif_fields(b"not an image").is_empty());
        let img = RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        assert!(exif_fields(buf.get_ref()).is_empty());
    }

    #[test]
    fn a_camera_with_an_unset_clock_reports_no_date() {
        // Zeroes are what a camera that has never been set writes; renaming a
        // photo to "00000000" would be worse than leaving the token empty.
        assert!(parse_exif_datetime("0000:00:00 00:00:00").is_none());
        assert!(parse_exif_datetime("garbage").is_none());
        // Both the raw EXIF spelling and the dashed display form parse, since
        // it is the display form this is actually given.
        assert_eq!(parse_exif_datetime("2024:07:14 15:09:33"), Some(((2024, 7, 14), (15, 9, 33))));
        assert_eq!(parse_exif_datetime("2024-07-14 15:09:33"), Some(((2024, 7, 14), (15, 9, 33))));
        // Six numbers are needed; a date with no time is not a timestamp.
        assert!(parse_exif_datetime("2024-07-14").is_none());
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
