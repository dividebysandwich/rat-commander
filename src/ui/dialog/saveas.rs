//! A generic "Save as" browser: navigate directories and type a file name.
//! Used by the editor (Shift-F2 / Ctrl-F2) and as the automatic fallback when a
//! normal save fails, so the user can pick a different path.

use super::flash::{BrowseEntry, SaveFocus};
use super::widgets::*;
use super::{DialogResult, Submit};

pub struct SaveAsDialog {
    cwd: std::path::PathBuf,
    filename: String,
    name_cursor: usize,
    /// Optional reason a previous save failed, shown in red at the top.
    error: Option<String>,
    pub(crate) entries: Vec<BrowseEntry>,
    pub(crate) cursor: usize,
    top: usize,
    pub(crate) focus: SaveFocus,
    list_area: Rect,
    list_rows: usize,
    name_area: Rect,
}

impl SaveAsDialog {
    pub fn new(start_dir: std::path::PathBuf, filename: String, error: Option<String>) -> Self {
        let mut d = SaveAsDialog {
            cwd: start_dir,
            name_cursor: filename.chars().count(),
            filename,
            error,
            entries: Vec::new(),
            cursor: 0,
            top: 0,
            // Start on the name field: it is prefilled, so Enter saves at once.
            focus: SaveFocus::Name,
            list_area: Rect::default(),
            list_rows: 1,
            name_area: Rect::default(),
        };
        d.refresh();
        d
    }

    fn refresh(&mut self) {
        let mut dirs: Vec<BrowseEntry> = Vec::new();
        let mut files: Vec<BrowseEntry> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.cwd) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    dirs.push(BrowseEntry { name, is_dir });
                } else {
                    files.push(BrowseEntry { name, is_dir });
                }
            }
        }
        dirs.sort_by_key(|e| e.name.to_lowercase());
        files.sort_by_key(|e| e.name.to_lowercase());
        let mut entries = Vec::with_capacity(dirs.len() + files.len() + 1);
        if self.cwd.parent().is_some() {
            entries.push(BrowseEntry { name: "..".to_string(), is_dir: true });
        }
        entries.extend(dirs);
        entries.extend(files);
        self.entries = entries;
        self.cursor = self.cursor.min(self.entries.len().saturating_sub(1));
        self.top = 0;
    }

    fn move_cursor(&mut self, delta: isize) {
        let max = self.entries.len().saturating_sub(1) as isize;
        if max < 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
    }

    /// Enter on a list row: descend a directory, or copy a file's name into the
    /// field (and focus it) so an existing file is easy to overwrite.
    fn activate_list(&mut self) -> DialogResult {
        let Some(e) = self.entries.get(self.cursor) else {
            return DialogResult::None;
        };
        if e.is_dir {
            if e.name == ".." {
                if let Some(p) = self.cwd.parent() {
                    self.cwd = p.to_path_buf();
                }
            } else {
                self.cwd.push(&e.name);
            }
            self.cursor = 0;
            self.refresh();
        } else {
            self.filename = e.name.clone();
            self.name_cursor = self.filename.chars().count();
            self.focus = SaveFocus::Name;
        }
        DialogResult::None
    }

    fn confirm(&self) -> DialogResult {
        let name = self.filename.trim();
        if name.is_empty() {
            return DialogResult::None;
        }
        DialogResult::Submit(Submit::EditorSaveAs(self.cwd.join(name)))
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        if key.code == KeyCode::Tab {
            self.focus = if self.focus == SaveFocus::List {
                SaveFocus::Name
            } else {
                SaveFocus::List
            };
            return DialogResult::None;
        }
        if self.focus == SaveFocus::Name {
            return match key.code {
                KeyCode::Esc => DialogResult::Cancel,
                KeyCode::Enter => self.confirm(),
                KeyCode::Down => {
                    self.focus = SaveFocus::List;
                    DialogResult::None
                }
                _ => {
                    edit_text(&mut self.filename, &mut self.name_cursor, key);
                    DialogResult::None
                }
            };
        }
        let page = self.list_rows.max(1) as isize;
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Up => {
                self.move_cursor(-1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.move_cursor(1);
                DialogResult::None
            }
            KeyCode::PageUp => {
                self.move_cursor(-page);
                DialogResult::None
            }
            KeyCode::PageDown => {
                self.move_cursor(page);
                DialogResult::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                DialogResult::None
            }
            KeyCode::End => {
                self.cursor = self.entries.len().saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Enter => self.activate_list(),
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, _area: Rect, col: u16, row: u16) -> DialogResult {
        if self.name_area.height > 0 && row == self.name_area.y {
            self.focus = SaveFocus::Name;
            return DialogResult::None;
        }
        let a = self.list_area;
        if a.height == 0 || col < a.x || col >= a.x + a.width || row < a.y || row >= a.y + a.height {
            return DialogResult::None;
        }
        let idx = self.top + (row - a.y) as usize;
        if idx < self.entries.len() {
            self.cursor = idx;
            self.focus = SaveFocus::List;
        }
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 72u16.min(area.width.saturating_sub(4));
        let extra = u16::from(self.error.is_some());
        let h = (19 + extra).min(area.height.saturating_sub(2)).max(9);
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("Save as", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let mut constraints = Vec::new();
        if self.error.is_some() {
            constraints.push(Constraint::Length(1)); // error line
        }
        constraints.extend([
            Constraint::Length(1), // cwd
            Constraint::Min(3),    // directory / file list
            Constraint::Length(1), // name label
            Constraint::Length(1), // name input
            Constraint::Length(1), // footer
        ]);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);
        let mut ri = 0;

        let label = Style::default().fg(theme.header_fg).bg(theme.dialog_bg);
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);

        if let Some(err) = &self.error {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!(" {}", ellipsize(err, inner.width.saturating_sub(1) as usize)),
                    Style::default().fg(theme.error_fg).bg(theme.dialog_bg),
                ))),
                rows[ri],
            );
            ri += 1;
        }

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {}", ellipsize(&self.cwd.display().to_string(), inner.width.saturating_sub(1) as usize)),
                base,
            ))),
            rows[ri],
        );
        ri += 1;

        // Directory / file list.
        let list = rows[ri];
        ri += 1;
        let lw = list.width as usize;
        self.list_rows = (list.height as usize).max(1);
        self.top = crate::util::scroll::scroll_to_visible(self.top, self.cursor, self.list_rows);
        self.list_area = list;
        let dir_style = Style::default().fg(theme.dir_fg).bg(theme.dialog_bg);
        let mut lines: Vec<Line> = Vec::with_capacity(self.list_rows);
        for (i, e) in self.entries.iter().enumerate().skip(self.top).take(self.list_rows) {
            let mark = if e.is_dir { "/" } else { " " };
            let text = format!(" {}{}", e.name, mark);
            let selected = i == self.cursor && self.focus == SaveFocus::List;
            let style = if selected {
                theme.menu_selection
            } else if e.is_dir {
                dir_style
            } else {
                base
            };
            lines.push(Line::from(Span::styled(pad_right(&ellipsize(&text, lw), lw), style)));
        }
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)), list);

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(" File name:", label))),
            rows[ri],
        );
        ri += 1;
        self.name_area = rows[ri];
        let caret = draw_input_field(
            f,
            rows[ri],
            &self.filename,
            self.name_cursor,
            self.focus == SaveFocus::Name,
            false,
            theme,
        );
        ri += 1;

        let footer = pad_right(
            " ↑↓ browse   Enter open dir / save   Tab switch   Esc cancel",
            inner.width as usize,
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(footer, theme.fkey_label))).style(theme.fkey_label),
            rows[ri],
        );

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}
