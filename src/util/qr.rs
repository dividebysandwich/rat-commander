//! QR-code generation for the "Send file over LAN" feature: encode a URL into a
//! module matrix, then render it either as a pixel image (terminal graphics) or
//! as half-block cell art (the fallback). Colors are forced black-on-white
//! regardless of the UI theme so a phone camera reads maximum contrast.

use crate::ui::graphics::raster::{self, Rgb};
use image::RgbaImage;
use qrcode::{Color as QrColor, QrCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// The light border required around a QR symbol (in modules). The spec asks for
/// four; keeping it makes phone scanners lock on reliably.
const QUIET: usize = 4;
const DARK: Rgb = (0, 0, 0);
const LIGHT: Rgb = (255, 255, 255);

/// A QR-code module matrix: a `size × size` grid of booleans (`true` = a dark
/// module). The quiet-zone border is *not* stored — the renderers add it.
pub struct Qr {
    size: usize,
    dark: Vec<bool>,
}

impl Qr {
    /// Encode `data` (e.g. a download URL) at the default error-correction level.
    /// `None` when the data is too long to fit any QR version.
    pub fn encode(data: &str) -> Option<Qr> {
        let code = QrCode::new(data.as_bytes()).ok()?;
        let size = code.width();
        let dark = code.into_colors().into_iter().map(|c| c == QrColor::Dark).collect();
        Some(Qr { size, dark })
    }

    /// Modules per side (without the quiet zone).
    pub fn size(&self) -> usize {
        self.size
    }

    /// Side length including both quiet-zone borders (in modules).
    pub fn padded(&self) -> usize {
        self.size + 2 * QUIET
    }

    #[inline]
    fn dark(&self, x: usize, y: usize) -> bool {
        self.dark.get(y * self.size + x).copied().unwrap_or(false)
    }

    /// Whether the padded-space module at `(x, y)` (quiet zone included) is dark.
    /// Anything in the quiet zone or out of range is light.
    #[inline]
    fn padded_dark(&self, x: usize, y: usize) -> bool {
        if x < QUIET || y < QUIET {
            return false;
        }
        let (x, y) = (x - QUIET, y - QUIET);
        x < self.size && y < self.size && self.dark(x, y)
    }

    /// A crisp pixel image: `module_px` square pixels per module, over a light
    /// quiet zone. Dark modules black, everything else white (forced contrast).
    pub fn to_image(&self, module_px: u32) -> RgbaImage {
        let module_px = module_px.max(1);
        let dim = self.padded() as u32 * module_px;
        let mut img = raster::canvas(dim, dim, LIGHT);
        for y in 0..self.size {
            for x in 0..self.size {
                if self.dark(x, y) {
                    let px = (x + QUIET) as u32 * module_px;
                    let py = (y + QUIET) as u32 * module_px;
                    raster::fill_rect(&mut img, px, py, module_px, module_px, DARK);
                }
            }
        }
        img
    }

    /// The largest integer-module image that fits within a `target_px` square, so
    /// the terminal-graphics scaler barely resizes it (keeping module edges sharp).
    pub fn to_image_fit(&self, target_px: u32) -> RgbaImage {
        let module_px = (target_px / self.padded() as u32).max(1);
        self.to_image(module_px)
    }

    /// Render as centered half-block cell art: each text cell packs two vertically
    /// stacked modules into `▀` (upper half = foreground, lower half = background),
    /// which makes the modules read as squares. Forced black/white including the
    /// quiet zone. Clipped if `area` is smaller than the symbol.
    pub fn render_ascii(&self, f: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let side = self.padded();
        let cols = side as u16;
        let rows = side.div_ceil(2) as u16;
        let x0 = area.x + area.width.saturating_sub(cols) / 2;
        let y0 = area.y + area.height.saturating_sub(rows) / 2;
        let color = |dark: bool| if dark { Color::Rgb(0, 0, 0) } else { Color::Rgb(255, 255, 255) };
        for cy in 0..rows {
            let y = y0 + cy;
            if y >= area.y + area.height {
                break;
            }
            let width = cols.min((area.x + area.width).saturating_sub(x0));
            let mut spans: Vec<Span> = Vec::with_capacity(width as usize);
            for cx in 0..width {
                let mx = cx as usize;
                let top = self.padded_dark(mx, cy as usize * 2);
                let bottom_row = cy as usize * 2 + 1;
                let bot = bottom_row < side && self.padded_dark(mx, bottom_row);
                spans.push(Span::styled("▀", Style::default().fg(color(top)).bg(color(bot))));
            }
            f.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect { x: x0, y, width, height: 1 },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_reports_padded_size() {
        let qr = Qr::encode("http://192.168.1.2:8080/file.zip").expect("encodes");
        // QR versions are odd-sided (21, 25, 29, …); the padded side adds 8.
        assert!(qr.size() >= 21 && qr.size() % 2 == 1);
        assert_eq!(qr.padded(), qr.size() + 8);
    }

    #[test]
    fn quiet_zone_is_light_and_symbol_has_dark_modules() {
        let qr = Qr::encode("hello world").expect("encodes");
        // Every quiet-zone module (the outer 4-module border) is light.
        for i in 0..qr.padded() {
            assert!(!qr.padded_dark(i, 0), "top border light");
            assert!(!qr.padded_dark(0, i), "left border light");
            assert!(!qr.padded_dark(i, qr.padded() - 1), "bottom border light");
        }
        // The finder patterns guarantee some dark modules exist.
        let any_dark = (0..qr.size()).any(|y| (0..qr.size()).any(|x| qr.dark(x, y)));
        assert!(any_dark, "a real symbol has dark modules");
    }

    #[test]
    fn to_image_is_square_and_sized_by_module_px() {
        let qr = Qr::encode("data").expect("encodes");
        let img = qr.to_image(3);
        assert_eq!(img.width(), img.height());
        assert_eq!(img.width(), qr.padded() as u32 * 3);
        // The very corner pixel sits in the quiet zone → white.
        assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn to_image_fit_never_exceeds_target() {
        let qr = Qr::encode("something to encode").expect("encodes");
        let img = qr.to_image_fit(400);
        assert!(img.width() <= 400 && img.width() >= 1);
        // At least one pixel per module even when the target is tiny.
        let tiny = qr.to_image_fit(1);
        assert_eq!(tiny.width(), qr.padded() as u32);
    }
}
