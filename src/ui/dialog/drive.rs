//! Drive / connection picker (Alt-F1 left, Alt-F2 right).

use super::widgets::*;
use super::{DialogResult, Submit};
use crate::vfs::remote::Protocol;

// ---------------------------------------------------------------------------
// Drive / connection picker (Alt-F1 left, Alt-F2 right)
// ---------------------------------------------------------------------------

/// One button in the picker.
#[derive(Clone, Copy)]
enum DriveItem {
    /// A local drive letter (Windows).
    Drive(char),
    /// Open a new remote connection of this protocol.
    Connect(Protocol),
    /// Return the panel to the local filesystem.
    Disconnect,
}

impl DriveItem {
    fn label(&self) -> String {
        match self {
            DriveItem::Drive(c) => format!("  {c}  "),
            DriveItem::Connect(Protocol::Sftp) => " SFTP ".to_string(),
            DriveItem::Connect(Protocol::Ftp) => " FTP ".to_string(),
            DriveItem::Connect(Protocol::Scp) => " SCP ".to_string(),
            DriveItem::Disconnect => " Disconnect ".to_string(),
        }
    }

    fn submit(&self, side: usize) -> Submit {
        match self {
            DriveItem::Drive(c) => Submit::SetDrive(side, *c),
            DriveItem::Connect(p) => Submit::OpenConnect(side, *p),
            DriveItem::Disconnect => Submit::DisconnectPanel(side),
        }
    }

    fn drive(&self) -> Option<char> {
        match self {
            DriveItem::Drive(c) => Some(*c),
            _ => None,
        }
    }
}

/// A Norton-style picker for a panel's source: drive-letter buttons (Windows)
/// on the first row(s) and remote-connection buttons (SFTP/FTP/SCP, plus
/// Disconnect when the panel is on a remote) below. Arrow keys move the
/// highlight; a drive letter or a click jumps straight to that button.
pub struct DriveDialog {
    /// Which panel this is for (0 = left, 1 = right).
    side: usize,
    items: Vec<DriveItem>,
    /// How many leading `items` are drive letters.
    drive_count: usize,
    cursor: usize,
    /// `(button_row, center_x)` per item, recorded at render for Up/Down nav.
    layout: Vec<(usize, u16)>,
    /// Clickable rects → item index, recorded at render.
    zones: Vec<(Rect, usize)>,
}

impl DriveDialog {
    pub fn new(side: usize, drives: Vec<char>, current: Option<char>, connected: bool) -> Self {
        let mut items: Vec<DriveItem> = drives.iter().map(|&c| DriveItem::Drive(c)).collect();
        let drive_count = items.len();
        items.push(DriveItem::Connect(Protocol::Sftp));
        items.push(DriveItem::Connect(Protocol::Ftp));
        items.push(DriveItem::Connect(Protocol::Scp));
        if connected {
            items.push(DriveItem::Disconnect);
        }
        // Highlight the current drive, else the first connection button.
        let cursor = current
            .and_then(|c| drives.iter().position(|&d| d == c))
            .unwrap_or(drive_count)
            .min(items.len().saturating_sub(1));
        DriveDialog { side, items, drive_count, cursor, layout: Vec::new(), zones: Vec::new() }
    }

    fn has_drives(&self) -> bool {
        self.drive_count > 0
    }

    /// Which panel this picker targets (0 = left, 1 = right), so the renderer can
    /// anchor it over that panel.
    pub(crate) fn side(&self) -> usize {
        self.side
    }

    fn move_vert(&mut self, dir: isize) {
        if self.layout.len() != self.items.len() {
            return;
        }
        let (row, x) = self.layout[self.cursor];
        let target = row as isize + dir;
        if let Some((i, _)) = self
            .layout
            .iter()
            .enumerate()
            .filter(|(_, (r, _))| *r as isize == target)
            .min_by_key(|(_, (_, cx))| (*cx as i32 - x as i32).abs())
        {
            self.cursor = i;
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let n = self.items.len();
        if n == 0 {
            return DialogResult::Cancel;
        }
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => DialogResult::Submit(self.items[self.cursor].submit(self.side)),
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(n - 1);
                DialogResult::None
            }
            KeyCode::Up => {
                self.move_vert(-1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.move_vert(1);
                DialogResult::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                DialogResult::None
            }
            KeyCode::End => {
                self.cursor = n - 1;
                DialogResult::None
            }
            // A drive letter jumps straight to that drive.
            KeyCode::Char(c) => {
                let up = c.to_ascii_uppercase();
                match self.items.iter().position(|it| it.drive() == Some(up)) {
                    Some(i) => DialogResult::Submit(self.items[i].submit(self.side)),
                    None => DialogResult::None,
                }
            }
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, _area: Rect, col: u16, row: u16) -> DialogResult {
        for (r, i) in &self.zones {
            if col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height {
                return DialogResult::Submit(self.items[*i].submit(self.side));
            }
        }
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        const DCELL: usize = 5; // "  X  " drive cell
        const GAP: usize = 1;

        // Drive grid: how many letter cells fit per row.
        let avail = (area.width.saturating_sub(8) as usize).max(DCELL);
        let dcols = (((avail + GAP) / (DCELL + GAP)).max(1)).min(self.drive_count.max(1));
        let drive_rows = if self.has_drives() { self.drive_count.div_ceil(dcols) } else { 0 };
        let drive_w = if self.has_drives() {
            dcols * DCELL + dcols.saturating_sub(1) * GAP
        } else {
            0
        };

        // Connection row: variable-width word buttons laid left-to-right.
        let conn = &self.items[self.drive_count..];
        let conn_w: usize =
            conn.iter().map(|it| it.label().chars().count()).sum::<usize>() + conn.len().saturating_sub(1) * GAP;

        let noun = if self.has_drives() { "drive" } else { "connection" };
        let msg_w = "Choose right :".len() + noun.len(); // upper bound

        let content_w = drive_w.max(conn_w).max(msg_w).max(18);
        let box_w = (content_w as u16 + 6).min(area.width.saturating_sub(2));
        let gap_rows = if self.has_drives() { 1 } else { 0 };
        let box_h = (drive_rows as u16) + (gap_rows as u16) + 7; // borders+pads+msg+conn
        let rect = centered(area, box_w, box_h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let title = if self.has_drives() { "Drive Letter" } else { "Connection" };
        let block = dialog_block(title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        f.render_widget(
            Paragraph::new("").style(Style::default().bg(theme.dialog_bg)),
            inner,
        );

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let accent = base.fg(theme.hotkey_fg).add_modifier(Modifier::BOLD);
        let side_word = if self.side == 0 { "left" } else { "right" };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Choose ", base),
                Span::styled(side_word, accent),
                Span::styled(format!(" {noun}:"), base),
            ]))
            .alignment(ratatui::layout::Alignment::Center)
            .style(base),
            Rect { x: inner.x, y: inner.y + 1, width: inner.width, height: 1 },
        );

        self.zones.clear();
        self.layout = vec![(0, 0); self.items.len()];
        let grid_y = inner.y + 3;

        // Drive buttons.
        if self.has_drives() {
            let gx = inner.x + inner.width.saturating_sub(drive_w as u16) / 2;
            for i in 0..self.drive_count {
                let (col, brow) = (i % dcols, i / dcols);
                let x = gx + (col * (DCELL + GAP)) as u16;
                let cell = Rect { x, y: grid_y + brow as u16, width: DCELL as u16, height: 1 };
                self.draw_button(f, theme, i, cell, brow);
            }
        }

        // Connection row (below the drives, with a blank line between).
        let conn_row = drive_rows; // logical row index for Up/Down
        let cy = grid_y + (drive_rows + gap_rows) as u16;
        let mut cx = inner.x + inner.width.saturating_sub(conn_w as u16) / 2;
        for i in self.drive_count..self.items.len() {
            let w = self.items[i].label().chars().count() as u16;
            self.draw_button(f, theme, i, Rect { x: cx, y: cy, width: w, height: 1 }, conn_row);
            cx += w + GAP as u16;
        }
    }

    fn draw_button(&mut self, f: &mut Frame, theme: &Theme, i: usize, rect: Rect, row: usize) {
        let style = if i == self.cursor { theme.button_focused } else { theme.button };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(self.items[i].label(), style))),
            rect,
        );
        self.zones.push((rect, i));
        self.layout[i] = (row, rect.x + rect.width / 2);
    }
}

