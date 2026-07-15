//! Compare-directories dialog.

use super::widgets::*;
use super::{CompareMode, DialogResult, Submit};

// ---------------------------------------------------------------------------
// Compare-directories dialog
// ---------------------------------------------------------------------------

/// What a button does. The three comparison modes only *mark* the differing
/// files; "Synchronize" carries the same question — how do these two directories
/// differ? — through to actually reconciling them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Mode(CompareMode),
    Sync,
}

/// `(label, choice, accelerator)`, in button order.
const CHOICES: [(&str, Choice, char); 4] = [
    ("Quick (name)", Choice::Mode(CompareMode::Quick), 'q'),
    ("Size only", Choice::Mode(CompareMode::Size), 's'),
    ("Content", Choice::Mode(CompareMode::Content), 'c'),
    ("Synchronize...", Choice::Sync, 'y'),
];

/// Asks how to compare the two panels' directories — or to sync them instead.
pub struct CompareDialog {
    focus: usize,
    zones: Vec<(Rect, usize)>,
}

impl CompareDialog {
    pub fn new() -> Self {
        CompareDialog { focus: 0, zones: Vec::new() }
    }

    fn submit(i: usize) -> DialogResult {
        match CHOICES[i].1 {
            Choice::Mode(m) => DialogResult::Submit(Submit::CompareDirs(m)),
            Choice::Sync => DialogResult::Submit(Submit::OpenSync),
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Left | KeyCode::BackTab => {
                self.focus = (self.focus + CHOICES.len() - 1) % CHOICES.len();
                DialogResult::None
            }
            KeyCode::Right | KeyCode::Tab => {
                self.focus = (self.focus + 1) % CHOICES.len();
                DialogResult::None
            }
            KeyCode::Enter => Self::submit(self.focus),
            KeyCode::Char(c) => {
                let lc = c.to_ascii_lowercase();
                match CHOICES.iter().position(|(_, _, hk)| *hk == lc) {
                    Some(i) => Self::submit(i),
                    None => DialogResult::None,
                }
            }
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, col: u16, row: u16) -> DialogResult {
        if let Some(&(_, i)) = self.zones.iter().find(|(r, _)| {
            col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
        }) {
            return Self::submit(i);
        }
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        self.zones.clear();
        let mut gfx = gfx;
        let w = 68u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 7);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd("Compare directories"), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        f.render_widget(
            Paragraph::new(format!("{}:", crate::l10n::trd("Compare the two panels by")))
                .alignment(ratatui::layout::Alignment::Center)
                .style(base),
            rows[0],
        );

        // Centered row of bracketed buttons; record click zones.
        let texts: Vec<String> = CHOICES.iter().map(|(l, _, _)| crate::l10n::trd(l)).collect();
        let labels: Vec<String> = texts.iter().map(|l| format!("[ {l} ]")).collect();
        let total: usize =
            labels.iter().map(|l| l.chars().count()).sum::<usize>() + labels.len().saturating_sub(1);
        let mut x = rows[1].x + (rows[1].width.saturating_sub(total as u16)) / 2;
        for (i, label) in labels.iter().enumerate() {
            let rect = Rect { x, y: rows[1].y, width: label.chars().count() as u16, height: 1 };
            if !gfx_button(f, gfx.as_deref_mut(), Slot::Button(i as u16), rect, &texts[i], i == self.focus, theme) {
                let style = if i == self.focus { theme.button_focused } else { theme.button };
                f.render_widget(Paragraph::new(Span::styled(label.clone(), style)), rect);
            }
            self.zones.push((rect, i));
            x += label.chars().count() as u16 + 1;
        }
    }
}

impl Default for CompareDialog {
    fn default() -> Self {
        Self::new()
    }
}
