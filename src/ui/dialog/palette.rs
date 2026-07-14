//! The Command palette (Ctrl-P): a fuzzy-searchable overlay listing every menu
//! action, setting, bookmark and open connection. Type to filter, `↑`/`↓` to
//! move, `Enter` to run the highlighted entry. Built by
//! [`crate::app::state::AppState::open_command_palette`], which knows the live
//! settings/sessions/bookmarks; this module only filters, renders and reports
//! the chosen entry's [`PaletteAction`].

use super::widgets::*;
use super::{DialogResult, Submit};
use crate::ui::menu::MenuAction;
use ratatui::crossterm::event::KeyModifiers;

/// Which family a palette entry belongs to — drives the right-aligned tag and
/// its colour so the four kinds read apart at a glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCategory {
    Command,
    Setting,
    Bookmark,
    Connection,
}

impl PaletteCategory {
    /// The short, localized tag drawn right-aligned on the row. Reuses existing
    /// catalog keys where possible (`Command`, `Options`) to keep translations
    /// minimal.
    fn tag(self) -> String {
        match self {
            PaletteCategory::Command => crate::l10n::tr("Command"),
            PaletteCategory::Setting => crate::l10n::tr("Options"),
            PaletteCategory::Bookmark => crate::l10n::tr("Bookmark"),
            PaletteCategory::Connection => crate::l10n::tr("Connection"),
        }
    }

    fn color(self, theme: &Theme) -> ratatui::style::Color {
        match self {
            PaletteCategory::Command => theme.dialog_fg,
            PaletteCategory::Setting => theme.doc_fg,
            PaletteCategory::Bookmark => theme.dir_fg,
            PaletteCategory::Connection => theme.archive_fg,
        }
    }
}

/// A toggleable boolean setting reachable straight from the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolSetting {
    Truecolor,
    Animation,
    SystemStatus,
    ReshapeRtl,
    InternalViewer,
    InternalEditor,
    ConfirmDelete,
    ConfirmOverwrite,
    ConfirmExecute,
    ConfirmUnmount,
    ConfirmExit,
}

/// What running a palette entry does. Most entries wrap a [`MenuAction`]; the
/// rest apply a setting, jump to a bookmark or toggle one directly.
#[derive(Debug, Clone)]
pub enum PaletteAction {
    /// Run an existing menu action (targeting the active panel where relevant).
    Menu(MenuAction),
    /// Switch to the named colour theme.
    SetTheme(String),
    /// Switch to the named UI language.
    SetLanguage(String),
    /// Set the terminal-graphics preference (`auto|off|kitty|sixel|iterm`).
    SetGraphics(String),
    /// Flip a boolean setting.
    ToggleBool(BoolSetting),
    /// Point the active panel at this (local) directory.
    JumpBookmark(String),
    /// Add or remove the active panel's directory from the bookmarks.
    ToggleBookmarkCurrent,
    /// Open the connect form for `side`, prefilled from a stored remote server.
    ConnectRemote(usize, crate::config::RemoteHistoryEntry),
}

/// One row in the palette: a label, its family, the action it runs, and an
/// optional right-aligned state hint (e.g. `on` / `off` / `✓`).
pub struct PaletteEntry {
    pub label: String,
    pub category: PaletteCategory,
    pub action: PaletteAction,
    pub hint: String,
}

impl PaletteEntry {
    pub fn new(label: impl Into<String>, category: PaletteCategory, action: PaletteAction) -> Self {
        PaletteEntry { label: label.into(), category, action, hint: String::new() }
    }

    /// Attach a right-aligned state hint (builder style).
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }
}

/// A filtered match: the source entry index plus the char positions in its label
/// that matched the query (for highlighting).
struct Hit {
    idx: usize,
    hl: Vec<usize>,
}

pub struct CommandPaletteDialog {
    entries: Vec<PaletteEntry>,
    query: String,
    qcursor: usize,
    filtered: Vec<Hit>,
    /// Selected row within `filtered`.
    sel: usize,
    /// First visible row (scroll offset), maintained by the renderer.
    offset: usize,
    /// The list's interior rect, recorded at render time for click hit-testing.
    list_area: Rect,
}

impl CommandPaletteDialog {
    pub fn new(entries: Vec<PaletteEntry>) -> Self {
        let mut d = CommandPaletteDialog {
            entries,
            query: String::new(),
            qcursor: 0,
            filtered: Vec::new(),
            sel: 0,
            offset: 0,
            list_area: Rect::new(0, 0, 0, 0),
        };
        d.refilter();
        d
    }

    /// Recompute the filtered/ranked list for the current query and reset the
    /// selection to the top.
    fn refilter(&mut self) {
        let q = self.query.trim();
        let mut hits: Vec<(i32, usize, Vec<usize>)> = Vec::new();
        for (idx, e) in self.entries.iter().enumerate() {
            if q.is_empty() {
                hits.push((0, idx, Vec::new()));
            } else if let Some((score, pos)) = fuzzy(q, &e.label) {
                // Label matches rank above tag-only matches.
                hits.push((score + 1000, idx, pos));
            } else if fuzzy(q, &e.category.tag()).is_some() {
                hits.push((0, idx, Vec::new()));
            }
        }
        // Best score first; ties keep the original build order (stable).
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.filtered = hits.into_iter().map(|(_, idx, hl)| Hit { idx, hl }).collect();
        self.sel = 0;
        self.offset = 0;
    }

    fn move_sel(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            self.sel = 0;
            return;
        }
        let max = self.filtered.len() as isize - 1;
        self.sel = (self.sel as isize + delta).clamp(0, max) as usize;
    }

    fn submit_current(&self) -> DialogResult {
        match self.filtered.get(self.sel) {
            Some(hit) => DialogResult::Submit(Submit::Palette(self.entries[hit.idx].action.clone())),
            None => DialogResult::None,
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            // Ctrl-P again toggles the palette back off, mirroring Ctrl-H.
            KeyCode::Char('p') if ctrl => DialogResult::Cancel,
            KeyCode::Enter => self.submit_current(),
            // Vertical keys navigate the results; the query line keeps the rest.
            KeyCode::Up => {
                self.move_sel(-1);
                DialogResult::None
            }
            KeyCode::Down | KeyCode::Tab => {
                self.move_sel(1);
                DialogResult::None
            }
            KeyCode::PageUp => {
                self.move_sel(-(self.list_area.height.max(1) as isize));
                DialogResult::None
            }
            KeyCode::PageDown => {
                self.move_sel(self.list_area.height.max(1) as isize);
                DialogResult::None
            }
            // Everything else edits the query (readline chords included), then we
            // re-filter if the text actually changed.
            _ => {
                let before = self.query.clone();
                edit_text(&mut self.query, &mut self.qcursor, key);
                if self.query != before {
                    self.refilter();
                }
                DialogResult::None
            }
        }
    }

    /// A left click on a result row selects and runs it. Geometry mirrors
    /// `render` via the `list_area`/`offset` recorded there.
    pub(crate) fn handle_click(&mut self, _area: Rect, col: u16, row: u16) -> DialogResult {
        let a = self.list_area;
        if col < a.x || col >= a.x + a.width || row < a.y || row >= a.y + a.height {
            return DialogResult::None;
        }
        let idx = self.offset + (row - a.y) as usize;
        if idx < self.filtered.len() {
            self.sel = idx;
            return self.submit_current();
        }
        DialogResult::None
    }

    pub(crate) fn handle_scroll(&mut self, delta: isize) -> DialogResult {
        self.move_sel(delta);
        DialogResult::None
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let width = 78u16.min(area.width.saturating_sub(4)).max(24);
        // Reserve rows: 2 borders + query line + separator, then as many results
        // as fit (capped so the box stays a comfortable overlay height).
        let max_body = area.height.saturating_sub(4 + 2).clamp(1, 18);
        let rows = (self.filtered.len() as u16).clamp(1, max_body);
        let height = rows + 4; // borders(2) + query(1) + separator(1) + rows
        let rect = centered(area, width, height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let title = format!(
            "{} ({})",
            crate::l10n::trd("Command palette"),
            self.filtered.len()
        );
        let block = dialog_block(&title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        if inner.width < 4 || inner.height < 3 {
            return;
        }

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let accent = Style::default().fg(theme.dialog_title).bg(theme.input_bg);

        // --- Query line (row 0) ---
        let qrow = Rect { height: 1, ..inner };
        let iw = inner.width as usize;
        let prompt = "  ";
        let field_style = Style::default().fg(theme.input_fg).bg(theme.input_bg);
        // Horizontal scroll so the caret stays visible.
        let avail = iw.saturating_sub(prompt.chars().count() + 1);
        let start = self.qcursor.saturating_sub(avail.saturating_sub(1));
        let shown: String = self.query.chars().skip(start).take(avail).collect();
        let shown_len = shown.chars().count();
        let pad: String = " ".repeat(iw.saturating_sub(prompt.chars().count() + shown_len));
        let qline = Line::from(vec![
            Span::styled(prompt.to_string(), accent),
            Span::styled(shown, field_style),
            Span::styled(pad, field_style),
        ]);
        f.render_widget(Paragraph::new(qline), qrow);
        // Place the caret within the query field.
        let cx = qrow.x + (prompt.chars().count() + (self.qcursor - start)).min(iw.saturating_sub(1)) as u16;
        f.set_cursor_position(Position::new(cx, qrow.y));

        // --- Separator (row 1) ---
        let sep = Rect { y: inner.y + 1, height: 1, ..inner };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(iw),
                Style::default().fg(theme.panel_border).bg(theme.dialog_bg),
            ))),
            sep,
        );

        // --- Results list (rows 2..) ---
        let list = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        };
        self.list_area = list;
        let visible = list.height as usize;
        // Keep the selection within the visible window.
        if self.sel < self.offset {
            self.offset = self.sel;
        } else if self.sel >= self.offset + visible {
            self.offset = self.sel + 1 - visible;
        }

        let mut lines: Vec<Line> = Vec::with_capacity(visible);
        for (row, hit) in self.filtered.iter().enumerate().skip(self.offset).take(visible) {
            let e = &self.entries[hit.idx];
            let selected = row == self.sel;
            let row_base = if selected { theme.button_focused } else { base };
            let hot = if selected {
                theme.button_focused.fg(theme.hotkey_fg).add_modifier(Modifier::BOLD)
            } else {
                base.fg(theme.hotkey_fg).add_modifier(Modifier::BOLD)
            };
            // Right side: the category tag, plus any state hint after it.
            let tag = e.category.tag();
            let right: String = if e.hint.is_empty() {
                tag.clone()
            } else {
                format!("{}  {}", tag, e.hint)
            };
            let right_len = right.chars().count();
            let max_label = iw.saturating_sub(right_len + 3);
            let label = ellipsize(&e.label, max_label);
            let label_len = label.chars().count();

            let mut spans: Vec<Span> = Vec::new();
            spans.push(Span::styled(" ", row_base));
            spans.extend(highlight_spans(&label, &hit.hl, row_base, hot));
            let used = 1 + label_len + right_len;
            let mid = " ".repeat(iw.saturating_sub(used));
            spans.push(Span::styled(mid, row_base));
            // Tag (dim/category-coloured) and hint (accent), but on a selected row
            // keep the single highlight style so it reads as one bar.
            if selected {
                spans.push(Span::styled(right, row_base));
            } else {
                let tag_style = base.fg(e.category.color(theme));
                if e.hint.is_empty() {
                    spans.push(Span::styled(tag, tag_style));
                } else {
                    spans.push(Span::styled(tag, tag_style));
                    spans.push(Span::styled(
                        format!("  {}", e.hint),
                        base.fg(theme.dialog_title),
                    ));
                }
            }
            lines.push(Line::from(spans));
        }
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)), list);
    }
}

/// Split `label` into styled spans, painting the chars at `hl` positions with
/// `hot` and the rest with `base`.
fn highlight_spans(label: &str, hl: &[usize], base: Style, hot: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span> = Vec::new();
    let mut cur = String::new();
    let mut cur_hot = false;
    for (i, ch) in label.chars().enumerate() {
        let is_hot = hl.contains(&i);
        if is_hot != cur_hot && !cur.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut cur), if cur_hot { hot } else { base }));
        }
        cur_hot = is_hot;
        cur.push(ch);
    }
    if !cur.is_empty() {
        spans.push(Span::styled(cur, if cur_hot { hot } else { base }));
    }
    spans
}

/// Case-insensitive subsequence fuzzy match. Returns `(score, matched char
/// positions)` when every char of `query` appears in `text` in order, or `None`
/// otherwise. Consecutive matches and word-start matches score higher, so the
/// tightest match ranks first.
fn fuzzy(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let qchars: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    if qchars.is_empty() {
        return Some((0, Vec::new()));
    }
    let tchars: Vec<char> = text.chars().collect();
    let tlower: Vec<char> = text.chars().flat_map(|c| c.to_lowercase()).collect();
    // `to_lowercase` can change length; fall back to a simple char map to keep
    // positions aligned with `tchars` (true for the ASCII-heavy labels here).
    let tlower: Vec<char> = if tlower.len() == tchars.len() {
        tlower
    } else {
        tchars.iter().map(|c| c.to_ascii_lowercase()).collect()
    };

    let mut qi = 0usize;
    let mut score = 0i32;
    let mut positions = Vec::with_capacity(qchars.len());
    let mut prev: Option<usize> = None;
    for (ti, &lc) in tlower.iter().enumerate() {
        if qi >= qchars.len() {
            break;
        }
        if lc == qchars[qi] {
            let mut bonus = 2;
            if prev == Some(ti.wrapping_sub(1)) {
                bonus += 6; // consecutive run
            }
            if ti == 0 {
                bonus += 10; // matches the very start
            } else {
                let before = tchars[ti - 1];
                if matches!(before, ' ' | '/' | '-' | '_' | ':' | '.') {
                    bonus += 7; // word boundary
                }
            }
            score += bonus;
            positions.push(ti);
            prev = Some(ti);
            qi += 1;
        }
    }
    if qi == qchars.len() {
        // Prefer shorter targets slightly so exact-ish labels outrank long ones.
        score -= tchars.len() as i32 / 16;
        Some((score, positions))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyModifiers;

    fn entry(label: &str, cat: PaletteCategory) -> PaletteEntry {
        PaletteEntry::new(label, cat, PaletteAction::ToggleBookmarkCurrent)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn fuzzy_matches_subsequence_and_ranks_word_starts() {
        // "cf" matches "Compare files" (C…f) and "Checksum".
        assert!(fuzzy("cf", "Compare files").is_some());
        assert!(fuzzy("xyz", "Compare files").is_none());
        // A start-anchored match beats the same run buried mid-word.
        let (tight, _) = fuzzy("com", "Compare").unwrap();
        let (loose, _) = fuzzy("com", "Recompare").unwrap();
        assert!(tight > loose, "start match should outrank a mid-word one");
    }

    #[test]
    fn typing_filters_and_enter_runs_the_top_hit() {
        let entries = vec![
            entry("Copy", PaletteCategory::Command),
            entry("Compare files", PaletteCategory::Command),
            entry("Find file", PaletteCategory::Command),
        ];
        let mut d = CommandPaletteDialog::new(entries);
        assert_eq!(d.filtered.len(), 3, "empty query shows everything");
        d.handle_key(ch('f'));
        d.handle_key(ch('i'));
        // "fi" matches "Find file" and "Compare files"; both survive.
        assert!(!d.filtered.is_empty());
        // The selection is a valid index and Enter submits it.
        assert!(matches!(d.submit_current(), DialogResult::Submit(Submit::Palette(_))));
    }

    #[test]
    fn no_match_leaves_nothing_selectable() {
        let mut d = CommandPaletteDialog::new(vec![entry("Copy", PaletteCategory::Command)]);
        for c in "zzzz".chars() {
            d.handle_key(ch(c));
        }
        assert!(d.filtered.is_empty());
        assert!(matches!(d.submit_current(), DialogResult::None));
    }

    #[test]
    fn navigation_clamps_and_esc_cancels() {
        let mut d = CommandPaletteDialog::new(vec![
            entry("a", PaletteCategory::Command),
            entry("b", PaletteCategory::Command),
        ]);
        d.handle_key(key(KeyCode::Up)); // clamped at top
        assert_eq!(d.sel, 0);
        d.handle_key(key(KeyCode::Down));
        assert_eq!(d.sel, 1);
        d.handle_key(key(KeyCode::Down)); // clamped at bottom
        assert_eq!(d.sel, 1);
        assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
    }

    #[test]
    fn tag_search_matches_by_category() {
        // Typing the category tag ("Bookmark") surfaces bookmark entries even
        // when the label doesn't contain those letters.
        let mut d = CommandPaletteDialog::new(vec![
            entry("/home/user/project", PaletteCategory::Bookmark),
            entry("Copy", PaletteCategory::Command),
        ]);
        for c in "bookmark".chars() {
            d.handle_key(ch(c));
        }
        assert_eq!(d.filtered.len(), 1);
        assert_eq!(d.entries[d.filtered[0].idx].category, PaletteCategory::Bookmark);
    }
}
