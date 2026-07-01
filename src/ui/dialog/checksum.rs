//! The checksum-result dialog.
//!
//! Shown after a `File → Checksum` task finishes: it presents the file name, the
//! algorithm, the computed digest, and — when the user supplied a comparison
//! checksum — a colored pass/fail verdict. Any key (or click) dismisses it. The
//! options/input phase reuses the generic [`FormDialog`](super::form::FormDialog)
//! and the calculation phase reuses the determinate [`ProgressDialog`].
//!
//! [`ProgressDialog`]: super::progress::ProgressDialog

use super::widgets::*;
use super::DialogResult;
use crate::util::checksum::ChecksumReport;

pub struct ChecksumResultDialog {
    report: ChecksumReport,
    /// The OK button's screen rect, recorded at render time for click hit-testing.
    ok_rect: Rect,
}

impl ChecksumResultDialog {
    pub fn new(report: ChecksumReport) -> Self {
        ChecksumResultDialog { report, ok_rect: Rect::default() }
    }

    /// The dialog is dismissed only via its OK button: Enter (or Esc) closes it;
    /// other keys are ignored.
    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => DialogResult::Cancel,
            _ => DialogResult::None,
        }
    }

    /// Close only when the OK button itself is clicked; clicks elsewhere are
    /// ignored (so a stray click can't dismiss the result).
    pub(crate) fn handle_click(&self, col: u16, row: u16) -> DialogResult {
        let r = self.ok_rect;
        let hit = r.width > 0
            && col >= r.x
            && col < r.x + r.width
            && row >= r.y
            && row < r.y + r.height;
        if hit {
            DialogResult::Cancel
        } else {
            DialogResult::None
        }
    }

    /// Split `s` into consecutive `width`-character rows (for wrapping a long
    /// hex digest without breaking the layout).
    fn chunks(s: &str, width: usize) -> Vec<String> {
        if width == 0 {
            return vec![s.to_string()];
        }
        let chars: Vec<char> = s.chars().collect();
        chars.chunks(width).map(|c| c.iter().collect()).collect()
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let w = 72u16.min(area.width.saturating_sub(4));
        let iw = w.saturating_sub(2) as usize; // interior width (inside the border)
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);

        // Assemble the body first, so the box height fits the content exactly
        // (SHA-512 wraps onto two lines; a mismatch adds the expected value).
        let mut body: Vec<Line> = Vec::new();
        body.push(Line::from(Span::styled(
            format!("File:  {}", ellipsize(&self.report.name, iw.saturating_sub(7))),
            base,
        )));
        body.push(Line::from(Span::styled(
            format!("Type:  {}", self.report.kind.label()),
            base,
        )));
        body.push(Line::from(""));
        body.push(Line::from(Span::styled("Checksum:", base)));
        for chunk in Self::chunks(&self.report.digest, iw) {
            body.push(Line::from(Span::styled(chunk, base.add_modifier(Modifier::BOLD))));
        }
        if let Some(matched) = self.report.verdict() {
            body.push(Line::from(""));
            let (text, color) = if matched {
                ("✓ MATCH — checksums are identical", theme.exec_fg)
            } else {
                ("✗ MISMATCH — checksums differ", theme.error_fg)
            };
            body.push(Line::from(Span::styled(
                text,
                Style::default().fg(color).bg(theme.dialog_bg).add_modifier(Modifier::BOLD),
            )));
            // On a mismatch, show what was expected so the difference is visible.
            if !matched
                && let Some(expected) = &self.report.expected
            {
                body.push(Line::from(Span::styled("Expected:", base)));
                for chunk in Self::chunks(expected, iw) {
                    body.push(Line::from(Span::styled(chunk, base)));
                }
            }
        }

        // Height: borders (2) + body + spacer (1) + button row (1).
        let height = (body.len() as u16).saturating_add(4).min(area.height.saturating_sub(2));
        let rect = centered(area, w, height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd("Checksum"), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        for (i, line) in body.into_iter().enumerate() {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height.saturating_sub(1) {
                break; // leave the last interior row for the OK button
            }
            f.render_widget(Paragraph::new(line).style(base), Rect { y, height: 1, ..inner });
        }

        // Centered, focused OK button on the last interior row; record its rect
        // so a click can be hit-tested against it (nothing else dismisses).
        let label = "[ OK ]";
        let bw = label.chars().count() as u16;
        let ok = Rect {
            x: inner.x + inner.width.saturating_sub(bw) / 2,
            y: inner.y + inner.height.saturating_sub(1),
            width: bw,
            height: 1,
        };
        if !gfx_button(f, gfx, Slot::Button(0), ok, "OK", true, theme) {
            f.render_widget(Paragraph::new(Line::from(button(label, true, theme))).style(base), ok);
        }
        self.ok_rect = ok;
    }
}
