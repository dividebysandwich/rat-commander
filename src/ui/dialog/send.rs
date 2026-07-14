//! The "Send file over LAN" dialog: shows a QR code for the ephemeral download
//! URL (pixel graphics when available, half-block cell art otherwise), the URL
//! itself, the file being shared, and a live count of completed downloads. It
//! stays open — serving the file — until dismissed; closing it stops the server.

use super::widgets::*;
use super::DialogResult;
use crate::util::qr::Qr;
use ratatui::style::Color;

pub struct SendFileDialog {
    /// The advertised download URL (also encoded in the QR).
    pub url: String,
    /// The file name offered to the downloader (the shared file, or the zip).
    pub filename: String,
    /// Size of the served file in bytes.
    pub size: u64,
    /// The encoded QR matrix.
    qr: Qr,
    /// Number of completed downloads reported by the server so far.
    pub downloads: usize,
}

impl SendFileDialog {
    /// Build the dialog for `url`. `None` if the URL is somehow too long to
    /// encode as a QR (the caller then falls back to showing just the URL).
    pub fn new(url: String, filename: String, size: u64) -> Option<Self> {
        let qr = Qr::encode(&url)?;
        Some(SendFileDialog { url, filename, size, qr, downloads: 0 })
    }

    /// A device finished downloading the file; bump the counter shown in the box.
    pub fn record_download(&mut self) {
        self.downloads = self.downloads.saturating_add(1);
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => DialogResult::Cancel,
            _ => DialogResult::None,
        }
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, mut gfx: Option<&mut Gfx>) {
        // ASCII footprint in cells: one module per column, two modules per row
        // (half-blocks stack two vertically), which keeps the modules square.
        let ascii_cols = self.qr.padded() as u16;
        let ascii_rows = self.qr.padded().div_ceil(2) as u16;

        // Below the QR: URL, "name · size", the download status, and the button.
        let text_rows = 4u16;
        let avail_qr_rows = area.height.saturating_sub(text_rows + 2).max(1);
        let have_gfx = gfx.as_deref().map(Gfx::available).unwrap_or(false);
        // A pixel QR scales to any area, so it can use a small square-on-screen
        // block (cells are ~1:2, hence twice as many columns as rows); the
        // half-block fallback needs its full footprint and only shrinks if the
        // terminal is too short.
        let (qr_cols, qr_rows) = if have_gfx {
            let rows = ascii_rows.min(13).min(avail_qr_rows);
            (rows * 2, rows)
        } else {
            (ascii_cols, ascii_rows.min(avail_qr_rows))
        };

        let inner_w = qr_cols
            .max(self.url.chars().count() as u16)
            .max(30)
            .min(area.width.saturating_sub(4));
        let box_w = (inner_w + 4).min(area.width);
        let box_h = (qr_rows + text_rows + 2).min(area.height);
        let rect = centered(area, box_w, box_h);

        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd("Send file over LAN"), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(qr_rows), // QR code
                Constraint::Length(1),       // URL
                Constraint::Length(1),       // file name · size
                Constraint::Length(1),       // downloads count
                Constraint::Min(1),          // OK button
            ])
            .split(inner);

        // A white plate behind the code so the graphics letterbox (or any ASCII
        // margin) stays light — QR readers want a quiet white border.
        f.render_widget(
            Block::default().style(Style::default().bg(Color::Rgb(255, 255, 255))),
            rows[0],
        );
        let drawn = gfx
            .as_deref_mut()
            .map(|g| {
                if g.available() {
                    let (pw, ph) = g.px_size(rows[0]);
                    let img = self.qr.to_image_fit(pw.min(ph));
                    g.draw(f, rows[0], Slot::SendQr, img);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
        if !drawn {
            self.qr.render_ascii(f, rows[0]);
        }

        let centered_line = |s: String, style: Style| {
            Paragraph::new(Line::from(s)).alignment(ratatui::layout::Alignment::Center).style(style)
        };

        f.render_widget(
            centered_line(
                ellipsize(&self.url, inner.width as usize),
                base.fg(theme.dialog_title).add_modifier(Modifier::BOLD),
            ),
            rows[1],
        );
        f.render_widget(
            centered_line(
                ellipsize(&format!("{}  ·  {}", self.filename, human_size(self.size)), inner.width as usize),
                base,
            ),
            rows[2],
        );
        let status = if self.downloads == 0 {
            "Waiting for a device to download…".to_string()
        } else if self.downloads == 1 {
            "Downloaded 1 time".to_string()
        } else {
            format!("Downloaded {} times", self.downloads)
        };
        f.render_widget(centered_line(status, base.fg(theme.exec_fg)), rows[3]);

        let ok = center_button_rect(rows[4], 10);
        if !gfx_button(f, gfx, Slot::Button(0), ok, "OK", true, theme) {
            f.render_widget(
                Paragraph::new(Line::from(button("[ OK ]", true, theme)))
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(base),
                rows[4],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::graphics::Gfx;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;

    fn screen(d: &mut SendFileDialog, w: u16, h: u16, gfx: Option<&mut Gfx>) -> String {
        let theme = crate::ui::theme::Theme::default();
        let area = Rect::new(0, 0, w, h);
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| d.render(f, area, &theme, gfx)).unwrap();
        let buf = t.backend().buffer();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn shows_url_size_and_download_status() {
        let mut d =
            SendFileDialog::new("http://192.168.1.5:8000/gift.zip".into(), "gift.zip".into(), 2048)
                .expect("encodes");
        let s = screen(&mut d, 70, 30, None);
        assert!(s.contains("192.168.1.5:8000"), "URL is shown");
        assert!(s.contains("gift.zip"), "file name is shown");
        assert!(s.contains("Waiting"), "shows the waiting-for-download status");

        d.record_download();
        let s = screen(&mut d, 70, 30, None);
        assert!(s.contains("Downloaded 1 time"), "counts a single download");
        d.record_download();
        let s = screen(&mut d, 70, 30, None);
        assert!(s.contains("Downloaded 2 times"), "counts further downloads");
    }

    #[test]
    fn renders_in_both_ascii_and_graphics_without_panicking() {
        let mut d = SendFileDialog::new(
            "http://10.0.0.9:54321/photos.zip".into(),
            "photos.zip".into(),
            9_999_999,
        )
        .expect("encodes");
        // Half-block cell art fallback.
        let _ = screen(&mut d, 80, 24, None);
        // Pixel path via the test graphics context (compact block).
        let mut gfx = Gfx::test_halfblocks();
        let _ = screen(&mut d, 80, 24, Some(&mut gfx));
    }

    #[test]
    fn key_closes_only_on_dismiss_keys() {
        let mut d = SendFileDialog::new("http://x/y".into(), "y".into(), 1).expect("encodes");
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let k = |c| KeyEvent::new(c, KeyModifiers::NONE);
        // A stray letter keeps it open; Esc / Enter / q close it.
        assert!(matches!(d.handle_key(k(KeyCode::Char('x'))), DialogResult::None));
        assert!(matches!(d.handle_key(k(KeyCode::Esc)), DialogResult::Cancel));
        assert!(matches!(d.handle_key(k(KeyCode::Enter)), DialogResult::Cancel));
    }
}
