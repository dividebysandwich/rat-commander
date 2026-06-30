//! Flash target picker, image file browser, and image-save browser.

use super::widgets::*;
use super::{DialogResult, Submit};

// ---------------------------------------------------------------------------
// Flash target picker (which block device to write an image to)
// ---------------------------------------------------------------------------

/// Lists every block device + partition (reusing the disk-manager metadata) so
/// the user can pick a flash target. Devices smaller than the image are shown
/// but cannot be selected.
pub struct FlashTargetDialog {
    image_path: std::path::PathBuf,
    image_name: String,
    image_size: u64,
    devices: Vec<crate::mount::BlockDevice>,
    pub(crate) cursor: usize,
    top: usize,
    /// Inner list rect + visible row count, recorded by the renderer.
    list_area: Rect,
    list_rows: usize,
}

impl FlashTargetDialog {
    pub fn new(
        image_path: std::path::PathBuf,
        image_name: String,
        image_size: u64,
        devices: Vec<crate::mount::BlockDevice>,
        preselect: Option<&str>,
    ) -> Self {
        let cursor = preselect
            .and_then(|dev| devices.iter().position(|d| d.dev == dev))
            .unwrap_or(0);
        FlashTargetDialog {
            image_path,
            image_name,
            image_size,
            devices,
            cursor,
            top: 0,
            list_area: Rect::default(),
            list_rows: 1,
        }
    }

    /// Whether the device at `idx` is large enough to hold the image.
    fn fits(&self, idx: usize) -> bool {
        self.image_size > 0 && self.devices.get(idx).is_some_and(|d| d.size >= self.image_size)
    }

    fn spec(&self, idx: usize) -> Option<Submit> {
        let d = self.devices.get(idx)?;
        Some(Submit::FlashSelected(crate::flash::FlashSpec {
            image_path: self.image_path.clone(),
            image_name: self.image_name.clone(),
            image_size: self.image_size,
            target: crate::flash::FlashTarget::from_device(d),
        }))
    }

    fn move_cursor(&mut self, delta: isize) {
        let max = self.devices.len().saturating_sub(1) as isize;
        if max < 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
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
                self.cursor = self.devices.len().saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Enter => match self.fits(self.cursor).then(|| self.spec(self.cursor)).flatten() {
                Some(s) => DialogResult::Submit(s),
                None => DialogResult::None, // too small / empty: refuse
            },
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, _area: Rect, col: u16, row: u16) -> DialogResult {
        let a = self.list_area;
        if a.height == 0 || col < a.x || col >= a.x + a.width || row < a.y || row >= a.y + a.height {
            return DialogResult::None;
        }
        let idx = self.top + (row - a.y) as usize;
        if idx < self.devices.len() {
            self.cursor = idx;
        }
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 84u16.min(area.width.saturating_sub(4));
        let h = ((self.devices.len() as u16) + 7).min(area.height.saturating_sub(2)).max(9);
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("Flash image to device", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // image header
                Constraint::Length(1), // hint
                Constraint::Min(3),    // device list
                Constraint::Length(1), // selected-device details
                Constraint::Length(1), // footer
            ])
            .split(inner);

        let label = Style::default().fg(theme.header_fg).bg(theme.dialog_bg);
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Image: ", label),
                Span::styled(
                    format!("{}  ({})", self.image_name, human_size(self.image_size)),
                    base,
                ),
            ])),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " Pick a target — too-small devices can't be selected",
                Style::default().fg(theme.panel_border).bg(theme.dialog_bg),
            ))),
            rows[1],
        );

        // Device list.
        let list = rows[2];
        let lw = list.width as usize;
        self.list_rows = (list.height as usize).max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + self.list_rows {
            self.top = self.cursor + 1 - self.list_rows;
        }
        self.list_area = list;
        let dim = Style::default().fg(theme.panel_border).bg(theme.dialog_bg);
        let mut lines: Vec<Line> = Vec::with_capacity(self.list_rows);
        if self.devices.is_empty() {
            lines.push(Line::from(Span::styled("  (no block devices)", dim)));
        }
        for (i, d) in self.devices.iter().enumerate().skip(self.top).take(self.list_rows) {
            let name_disp = if d.parent.is_some() {
                let last = self.devices.get(i + 1).is_none_or(|n| n.parent != d.parent);
                format!("{} {}", if last { "└" } else { "├" }, d.name)
            } else {
                d.name.clone()
            };
            let info = if !d.fstype.is_empty() || !d.label.is_empty() {
                format!("{} {}", d.fstype, d.label).trim().to_string()
            } else {
                d.model.clone()
            };
            let flag = if !self.fits(i) {
                "  (too small)"
            } else if d.removable {
                "  removable"
            } else {
                "  fixed"
            };
            let size = human_size(d.size);
            let right = format!("{size:>9}{flag}");
            let name_col = pad_right(&name_disp, 14);
            let mid = lw.saturating_sub(name_col.chars().count() + right.chars().count() + 2);
            let info_col = pad_right(&ellipsize(&info, mid), mid);
            let text = format!("{name_col} {info_col} {right}");
            let style = if i == self.cursor {
                theme.menu_selection
            } else if !self.fits(i) {
                dim
            } else {
                base
            };
            lines.push(Line::from(Span::styled(pad_right(&ellipsize(&text, lw), lw), style)));
        }
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)), list);

        // Detail line for the highlighted device.
        if let Some(d) = self.devices.get(self.cursor) {
            let dash = |s: &str| if s.is_empty() { "—".to_string() } else { s.to_string() };
            let detail = format!(
                " {}   Vendor: {}   Serial: {}   Label: {}   FS: {}",
                d.dev,
                dash(&d.vendor),
                dash(&d.serial),
                dash(&d.label),
                dash(&d.fstype),
            );
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    ellipsize(&detail, inner.width as usize),
                    base,
                ))),
                rows[3],
            );
        }

        let footer = pad_right(" ↑↓ select   Enter flash   Esc cancel", inner.width as usize);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(footer, theme.fkey_label))).style(theme.fkey_label),
            rows[4],
        );
    }
}

// ---------------------------------------------------------------------------
// File browser (pick an image to flash)
// ---------------------------------------------------------------------------

pub(crate) struct BrowseEntry {
    pub(crate) name: String,
    pub(crate) is_dir: bool,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum BrowseFocus {
    List,
    Filter,
}

/// A minimal local-filesystem browser for choosing an image file, with an
/// editable extension-glob filter. Picking a file emits its full path.
pub struct FileBrowserDialog {
    target: crate::flash::FlashTarget,
    cwd: std::path::PathBuf,
    filter: String,
    filter_cursor: usize,
    pub(crate) entries: Vec<BrowseEntry>,
    pub(crate) cursor: usize,
    top: usize,
    focus: BrowseFocus,
    list_area: Rect,
    list_rows: usize,
    filter_area: Rect,
}

impl FileBrowserDialog {
    pub fn new(target: crate::flash::FlashTarget, start_dir: std::path::PathBuf) -> Self {
        let mut d = FileBrowserDialog {
            target,
            cwd: start_dir,
            filter: crate::flash::DEFAULT_IMAGE_FILTER.to_string(),
            filter_cursor: crate::flash::DEFAULT_IMAGE_FILTER.chars().count(),
            entries: Vec::new(),
            cursor: 0,
            top: 0,
            focus: BrowseFocus::List,
            list_area: Rect::default(),
            list_rows: 1,
            filter_area: Rect::default(),
        };
        d.refresh();
        d
    }

    /// The device this browse is targeting (for the title).
    pub fn target_dev(&self) -> &str {
        &self.target.dev
    }

    fn matcher(&self) -> Option<crate::panel::selection::NameMatcher> {
        // The filter is space/`;`/`,`-separated globs; normalize to `;`.
        let pat: String = self.filter.split_whitespace().collect::<Vec<_>>().join(";");
        if pat.is_empty() {
            return None;
        }
        crate::panel::selection::NameMatcher::build(&pat, false, true).ok()
    }

    fn refresh(&mut self) {
        let matcher = self.matcher();
        let mut dirs: Vec<BrowseEntry> = Vec::new();
        let mut files: Vec<BrowseEntry> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.cwd) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue; // hide dotfiles for a tidy browser
                }
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    dirs.push(BrowseEntry { name, is_dir });
                } else if matcher.as_ref().map(|m| m.is_match(&name)).unwrap_or(true) {
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

    fn activate(&mut self) -> DialogResult {
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
            DialogResult::None
        } else {
            DialogResult::Submit(Submit::FlashBrowsePicked(
                self.cwd.join(&e.name),
                self.target.clone(),
            ))
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let max = self.entries.len().saturating_sub(1) as isize;
        if max < 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        if key.code == KeyCode::Tab {
            self.focus = if self.focus == BrowseFocus::List {
                BrowseFocus::Filter
            } else {
                BrowseFocus::List
            };
            return DialogResult::None;
        }
        if self.focus == BrowseFocus::Filter {
            match key.code {
                KeyCode::Esc => return DialogResult::Cancel,
                KeyCode::Enter => {
                    self.refresh();
                    self.focus = BrowseFocus::List;
                }
                _ => edit_text(&mut self.filter, &mut self.filter_cursor, key),
            }
            return DialogResult::None;
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
            KeyCode::Enter => self.activate(),
            _ => DialogResult::None,
        }
    }

    pub(crate) fn handle_click(&mut self, _area: Rect, col: u16, row: u16) -> DialogResult {
        if self.filter_area.height > 0 && row == self.filter_area.y {
            self.focus = BrowseFocus::Filter;
            return DialogResult::None;
        }
        let a = self.list_area;
        if a.height == 0 || col < a.x || col >= a.x + a.width || row < a.y || row >= a.y + a.height {
            return DialogResult::None;
        }
        let idx = self.top + (row - a.y) as usize;
        if idx < self.entries.len() {
            self.cursor = idx;
            self.focus = BrowseFocus::List;
        }
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 72u16.min(area.width.saturating_sub(4));
        let h = 20u16.min(area.height.saturating_sub(2)).max(8);
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&format!("Choose image → {}", self.target.dev), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // cwd
                Constraint::Length(1), // filter label
                Constraint::Length(1), // filter input
                Constraint::Min(3),    // entries
                Constraint::Length(1), // footer
            ])
            .split(inner);

        let label = Style::default().fg(theme.header_fg).bg(theme.dialog_bg);
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {}", ellipsize(&self.cwd.display().to_string(), inner.width.saturating_sub(1) as usize)),
                base,
            ))),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(" Filter (globs, e.g. *.iso *.img):", label))),
            rows[1],
        );
        self.filter_area = rows[2];
        let caret = draw_input_field(
            f,
            rows[2],
            &self.filter,
            self.filter_cursor,
            self.focus == BrowseFocus::Filter,
            false,
            theme,
        );

        // Entries.
        let list = rows[3];
        let lw = list.width as usize;
        self.list_rows = (list.height as usize).max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + self.list_rows {
            self.top = self.cursor + 1 - self.list_rows;
        }
        self.list_area = list;
        let dir_style = Style::default().fg(theme.dir_fg).bg(theme.dialog_bg);
        let mut lines: Vec<Line> = Vec::with_capacity(self.list_rows);
        if self.entries.is_empty() {
            lines.push(Line::from(Span::styled("  (no matching files)", base)));
        }
        for (i, e) in self.entries.iter().enumerate().skip(self.top).take(self.list_rows) {
            let mark = if e.is_dir { "/" } else { " " };
            let text = format!(" {}{}", e.name, mark);
            let selected = i == self.cursor && self.focus == BrowseFocus::List;
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

        let footer = pad_right(
            " ↑↓ browse   Enter open/pick   Tab filter   Esc cancel",
            inner.width as usize,
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(footer, theme.fkey_label))).style(theme.fkey_label),
            rows[4],
        );
        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

// ---------------------------------------------------------------------------
// Image save browser (where to write a device image)
// ---------------------------------------------------------------------------

#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum SaveFocus {
    List,
    Name,
}

/// A "save as" browser: navigate directories and type a file name to write a raw
/// device image to. Directories are navigable; clicking an existing file copies
/// its name into the field (to make overwriting easy).
pub struct ImageSaveDialog {
    source: crate::flash::FlashTarget,
    cwd: std::path::PathBuf,
    filename: String,
    name_cursor: usize,
    pub(crate) entries: Vec<BrowseEntry>,
    pub(crate) cursor: usize,
    top: usize,
    pub(crate) focus: SaveFocus,
    list_area: Rect,
    list_rows: usize,
    name_area: Rect,
}

impl ImageSaveDialog {
    pub fn new(source: crate::flash::FlashTarget, start_dir: std::path::PathBuf) -> Self {
        let base = source
            .dev
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("disk");
        let filename = format!("{base}.img");
        let mut d = ImageSaveDialog {
            source,
            cwd: start_dir,
            name_cursor: filename.chars().count(),
            filename,
            entries: Vec::new(),
            cursor: 0,
            top: 0,
            focus: SaveFocus::List,
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
    /// field (and focus it) to overwrite.
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
        DialogResult::Submit(Submit::ImageSave(crate::flash::ImageSpec {
            source: self.source.clone(),
            dest_path: self.cwd.join(name),
            dest_name: name.to_string(),
        }))
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
        let h = 20u16.min(area.height.saturating_sub(2)).max(9);
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("Create image of device", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // source + size
                Constraint::Length(1), // cwd
                Constraint::Min(3),    // directory / file list
                Constraint::Length(1), // name label
                Constraint::Length(1), // name input
                Constraint::Length(1), // footer
            ])
            .split(inner);

        let label = Style::default().fg(theme.header_fg).bg(theme.dialog_bg);
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" Source: ", label),
                Span::styled(
                    format!("{}  ({})", self.source.dev, human_size(self.source.size)),
                    base,
                ),
            ])),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {}", ellipsize(&self.cwd.display().to_string(), inner.width.saturating_sub(1) as usize)),
                base,
            ))),
            rows[1],
        );

        // Directory / file list.
        let list = rows[2];
        let lw = list.width as usize;
        self.list_rows = (list.height as usize).max(1);
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + self.list_rows {
            self.top = self.cursor + 1 - self.list_rows;
        }
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
            Paragraph::new(Line::from(Span::styled(" Image file name:", label))),
            rows[3],
        );
        self.name_area = rows[4];
        let caret = draw_input_field(
            f,
            rows[4],
            &self.filename,
            self.name_cursor,
            self.focus == SaveFocus::Name,
            false,
            theme,
        );

        let footer = pad_right(
            " ↑↓ browse   Enter open dir / name file   Tab name   Esc cancel",
            inner.width as usize,
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(footer, theme.fkey_label))).style(theme.fkey_label),
            rows[5],
        );
        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

