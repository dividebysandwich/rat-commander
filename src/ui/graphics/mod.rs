//! Terminal graphics layer: draws true-pixel images (Kitty / Sixel / iTerm2) for
//! the data-heavy visualizations, falling back to the Ratatui cell widgets when
//! the terminal has no graphics protocol.
//!
//! [`Gfx`] wraps a `ratatui-image` [`Picker`] (which knows the protocol and the
//! terminal's cell-pixel size) plus a per-[`Slot`] cache of encoded protocols.
//! Callers build an [`image::RgbaImage`] with the [`raster`] primitives and hand
//! it to [`Gfx::draw`]; the cache re-transmits only when a slot's content
//! actually changes, so a live graph doesn't churn Kitty image IDs every frame.

pub mod raster;

use image::{DynamicImage, RgbaImage};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::{Resize, StatefulImage};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// A distinct on-screen graphics target; keys the protocol cache so each region
/// keeps its own encoded image across frames.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    TransferFileBar,
    TransferTotalBar,
    TransferSpeed,
    Indeterminate,
    DiskScanBar,
    ProcCpu,
    ProcMem,
    ProcDisk,
    ProcNet,
    ProcCore(u16),
    Treemap(u16),
    /// A dialog push-button, indexed within the currently-open dialog.
    Button(u16),
    /// The network-explorer connection-rate graph in the details popup.
    NetRate,
    /// The network-explorer overview diagram (service-card grid).
    NetDiagram,
    /// A Details-view image-thumbnail preview, per panel side.
    DetailsPreview(u16),
    /// The F3 fullscreen image viewer.
    ViewerImage,
    /// The QR code in the "Send file over LAN" dialog.
    SendQr,
}

struct Cached {
    sig: u64,
    proto: StatefulProtocol,
}

/// The active terminal-graphics capability. Absent (`None` on [`crate::app::state::AppState`])
/// when no protocol was detected or graphics are configured off.
pub struct Gfx {
    picker: Picker,
    /// The protocol chosen at startup, restored when the user picks "auto".
    detected: ProtocolType,
    enabled: bool,
    cache: HashMap<Slot, Cached>,
}

impl Gfx {
    /// Detect graphics support honoring the `graphics` config preference
    /// (`auto|off|kitty|sixel|iterm`). Must be called **after** entering the
    /// alternate screen in raw mode (the query reads terminal responses).
    /// Returns `None` when disabled or when no real graphics protocol is present.
    pub fn detect(pref: &str) -> Option<Gfx> {
        let pref = pref.trim().to_ascii_lowercase();
        if pref == "off" {
            return None;
        }
        let mut picker = Picker::from_query_stdio().ok()?;
        match forced_protocol(&pref) {
            Some(p) => picker.set_protocol_type(p),
            // "auto" (or unrecognized): trust the query, but a halfblocks-only
            // result is treated as no graphics — our tuned cell widgets read
            // better than half-block mosaics.
            None if picker.protocol_type() == ProtocolType::Halfblocks => return None,
            None => {}
        }
        let detected = picker.protocol_type();
        Some(Gfx { picker, detected, enabled: true, cache: HashMap::new() })
    }

    /// Whether graphics rendering is currently active (the runtime `Off` toggle
    /// flips this without discarding the detected capability).
    pub fn available(&self) -> bool {
        self.enabled
    }

    /// Whether graphical push-buttons should be drawn (else the caller falls back
    /// to text buttons). False under Sixel: a Sixel button is a pixel raster
    /// anchored to the text cursor, and on some terminals it lands a row below the
    /// cell it was addressed to — which pushes a dialog's bottom-row buttons onto
    /// or below the border. Kitty/iTerm2 place images in real grid cells, so those
    /// stay cell-exact and keep the graphical buttons.
    pub fn buttons_ok(&self) -> bool {
        self.enabled && self.picker.protocol_type() != ProtocolType::Sixel
    }

    /// Apply a `graphics` preference at runtime (Settings live preview): `off`
    /// disables, `auto` restores the detected protocol, and `kitty|sixel|iterm`
    /// force one. Clears the cache so images re-transmit under the new scheme.
    pub fn apply_pref(&mut self, pref: &str) {
        self.cache.clear();
        match forced_protocol(pref) {
            Some(p) => {
                self.picker.set_protocol_type(p);
                self.enabled = true;
            }
            None if pref.trim().eq_ignore_ascii_case("off") => self.enabled = false,
            None => {
                self.picker.set_protocol_type(self.detected);
                self.enabled = true;
            }
        }
    }

    /// Drop all cached encodings so images re-transmit on the next frame — called
    /// after the TUI is suspended (and the screen cleared) for a subshell or
    /// external program.
    pub fn invalidate(&mut self) {
        self.cache.clear();
    }

    /// Cell size in pixels (width, height).
    pub fn cell(&self) -> (u32, u32) {
        let fs = self.picker.font_size();
        (fs.width.max(1) as u32, fs.height.max(1) as u32)
    }

    /// Pixel dimensions of a cell `area`.
    pub fn px_size(&self, area: Rect) -> (u32, u32) {
        let (cw, ch) = self.cell();
        (area.width as u32 * cw, area.height as u32 * ch)
    }

    /// Draw `img` into the cell `area` for `slot`, reusing the cached encoding
    /// when the content is unchanged (so Kitty doesn't re-transmit every frame).
    pub fn draw(&mut self, f: &mut Frame, area: Rect, slot: Slot, img: RgbaImage) {
        if !self.enabled || area.width == 0 || area.height == 0 {
            return;
        }
        let sig = sig_of(&img);
        let fresh = match self.cache.get(&slot) {
            Some(c) => c.sig != sig,
            None => true,
        };
        if fresh {
            let proto = self.picker.new_resize_protocol(DynamicImage::ImageRgba8(img));
            self.cache.insert(slot, Cached { sig, proto });
        }
        if let Some(c) = self.cache.get_mut(&slot) {
            f.render_stateful_widget(StatefulImage::default().resize(Resize::Fit(None)), area, &mut c.proto);
        }
    }

    /// Like [`draw`], but the caller supplies a cheap content signature and a
    /// closure that builds the image only when that signature changes. Use this
    /// for large, expensive-to-build images that stay static across most frames
    /// (e.g. the disk-explorer treemap): a cached slot skips both building and
    /// re-encoding, so a burst of redraws costs almost nothing. `sig` must capture
    /// every input the image depends on (size, colors, data, selection).
    ///
    /// [`draw`]: Gfx::draw
    pub fn draw_cached(
        &mut self,
        f: &mut Frame,
        area: Rect,
        slot: Slot,
        sig: u64,
        build: impl FnOnce() -> RgbaImage,
    ) {
        if !self.enabled || area.width == 0 || area.height == 0 {
            return;
        }
        let fresh = match self.cache.get(&slot) {
            Some(c) => c.sig != sig,
            None => true,
        };
        if fresh {
            let proto = self.picker.new_resize_protocol(DynamicImage::ImageRgba8(build()));
            self.cache.insert(slot, Cached { sig, proto });
        }
        if let Some(c) = self.cache.get_mut(&slot) {
            f.render_stateful_widget(StatefulImage::default().resize(Resize::Fit(None)), area, &mut c.proto);
        }
    }
}

#[cfg(test)]
impl Gfx {
    /// A test-only context using the halfblocks protocol at a fixed cell size, so
    /// the graphics render paths can be exercised without a real terminal.
    pub fn test_halfblocks() -> Gfx {
        #[allow(deprecated)]
        let mut picker = Picker::from_fontsize((8u16, 16u16).into());
        picker.set_protocol_type(ProtocolType::Halfblocks);
        Gfx { picker, detected: ProtocolType::Halfblocks, enabled: true, cache: HashMap::new() }
    }
}

fn sig_of(img: &RgbaImage) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    img.width().hash(&mut h);
    img.height().hash(&mut h);
    img.as_raw().hash(&mut h);
    h.finish()
}

/// Map a `graphics` config string to a forced protocol, or `None` for auto.
pub fn forced_protocol(pref: &str) -> Option<ProtocolType> {
    match pref.trim().to_ascii_lowercase().as_str() {
        "kitty" => Some(ProtocolType::Kitty),
        "sixel" => Some(ProtocolType::Sixel),
        "iterm" | "iterm2" => Some(ProtocolType::Iterm2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn draw_cached_builds_only_when_signature_changes() {
        let mut gfx = Gfx::test_halfblocks();
        let mut term = Terminal::new(TestBackend::new(20, 10)).unwrap();
        let area = Rect::new(0, 0, 10, 4);
        let mut builds = 0usize;
        let mut render = |gfx: &mut Gfx, sig: u64, builds: &mut usize| {
            term.draw(|f| {
                gfx.draw_cached(f, area, Slot::Treemap(0), sig, || {
                    *builds += 1;
                    RgbaImage::from_pixel(16, 16, Rgba([10, 20, 30, 255]))
                });
            })
            .unwrap();
        };
        // Same signature across three frames → the image is built once (a burst of
        // redraws, e.g. on terminal refocus, no longer rebuilds the treemap).
        render(&mut gfx, 1, &mut builds);
        render(&mut gfx, 1, &mut builds);
        render(&mut gfx, 1, &mut builds);
        assert_eq!(builds, 1, "unchanged signature must not rebuild");
        // A new signature (selection/size/data changed) rebuilds once more.
        render(&mut gfx, 2, &mut builds);
        assert_eq!(builds, 2, "changed signature rebuilds");
    }
}
