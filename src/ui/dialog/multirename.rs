//! Multi-file rename dialog.

use super::widgets::*;
use super::{DialogResult, Submit};
use crate::rename::{CaseMode, RenameRule};

// ---------------------------------------------------------------------------
// Multi-rename dialog
// ---------------------------------------------------------------------------

/// Like [`edit_text`] but only accepts digit input (and an optional leading
/// minus sign), for the numeric counter fields.
fn edit_number(value: &mut String, cursor: &mut usize, key: KeyEvent, allow_sign: bool) {
    if let KeyCode::Char(c) = key.code
        && !(c.is_ascii_digit() || (allow_sign && c == '-'))
    {
        return;
    }
    edit_text(value, cursor, key);
}

/// Render `label` then a turquoise input field filling the rest of `area`.
/// Returns the caret screen position when `focused`.
fn labeled_field(
    f: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    cursor: usize,
    focused: bool,
    theme: &Theme,
) -> Option<Position> {
    if area.width == 0 {
        return None;
    }
    let lw = (label.chars().count() as u16).min(area.width);
    let style = if focused {
        theme.dialog_selection
    } else {
        Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
    };
    f.render_widget(Paragraph::new(Span::styled(label.to_string(), style)), Rect { width: lw, ..area });
    let field = Rect { x: area.x + lw, width: area.width.saturating_sub(lw), ..area };
    draw_input_field(f, field, value, cursor, focused, false, theme)
}

/// Draw a fixed-width turquoise numeric field (no `[^]` history button), for the
/// narrow counter inputs. Returns the caret position when `focused`.
fn draw_num_field(f: &mut Frame, area: Rect, value: &str, cursor: usize, focused: bool, theme: &Theme) -> Option<Position> {
    let w = area.width as usize;
    if w == 0 {
        return None;
    }
    let chars: Vec<char> = value.chars().collect();
    // Scroll so the caret stays visible in the narrow field.
    let start = cursor.saturating_sub(w.saturating_sub(1)).min(chars.len());
    let mut shown: String = chars.iter().skip(start).take(w).collect();
    while shown.chars().count() < w {
        shown.push(' ');
    }
    f.render_widget(
        Paragraph::new(Span::styled(shown, Style::default().fg(theme.input_fg).bg(theme.input_bg))),
        area,
    );
    focused.then(|| Position::new(area.x + (cursor - start).min(w.saturating_sub(1)) as u16, area.y))
}

const MR_FOCUS_COUNT: usize = 8;

/// The batch-rename dialog: a mask/options area on top and two synchronized,
/// side-by-side lists below (original names | projected new names). Tab cycles
/// the option fields; ↑↓/PageUp/PageDown scroll both lists in lock-step; Enter
/// executes, Esc cancels.
pub struct MultiRenameDialog {
    sources: Vec<VfsPath>,
    originals: Vec<String>,
    mask: String,
    mask_cursor: usize,
    case: CaseMode,
    start: String,
    start_cursor: usize,
    step: String,
    step_cursor: usize,
    digits: String,
    digits_cursor: usize,
    search: String,
    search_cursor: usize,
    replace: String,
    replace_cursor: usize,
    case_sensitive: bool,
    date: String,
    time: String,
    /// Focused option: 0 mask, 1 case, 2 start, 3 step, 4 digits, 5 search,
    /// 6 replace, 7 case-sensitive.
    focus: usize,
    /// First visible list row.
    top: usize,
    /// Highlighted list row (shared by both columns).
    cursor: usize,
    /// Geometry recorded at render time for mouse handling.
    list_left: Rect,
    list_right: Rect,
    list_rows: usize,
    exec_rect: Rect,
    cancel_rect: Rect,
    /// Clickable option-field regions as `(rect, focus index)`, recorded each
    /// render so a click can focus (and, for the case chooser / checkbox, cycle
    /// or toggle) the field under the pointer.
    field_hits: Vec<(Rect, usize)>,
}

impl MultiRenameDialog {
    pub fn new(sources: Vec<VfsPath>, date: String, time: String) -> Self {
        let originals: Vec<String> = sources.iter().map(|p| p.file_name()).collect();
        MultiRenameDialog {
            sources,
            originals,
            mask: "[N].[E]".to_string(),
            mask_cursor: "[N].[E]".chars().count(),
            case: CaseMode::Unchanged,
            start: "1".to_string(),
            start_cursor: 1,
            step: "1".to_string(),
            step_cursor: 1,
            digits: "1".to_string(),
            digits_cursor: 1,
            search: String::new(),
            search_cursor: 0,
            replace: String::new(),
            replace_cursor: 0,
            case_sensitive: false,
            date,
            time,
            focus: 0,
            top: 0,
            cursor: 0,
            list_left: Rect::default(),
            list_right: Rect::default(),
            list_rows: 1,
            exec_rect: Rect::default(),
            cancel_rect: Rect::default(),
            field_hits: Vec::new(),
        }
    }

    /// Move the caret of the field identified by `focus` to the end of its value
    /// (used when a click focuses a text/number field).
    fn caret_to_end(&mut self, focus: usize) {
        let (val, cur) = match focus {
            0 => (&self.mask, &mut self.mask_cursor),
            2 => (&self.start, &mut self.start_cursor),
            3 => (&self.step, &mut self.step_cursor),
            4 => (&self.digits, &mut self.digits_cursor),
            5 => (&self.search, &mut self.search_cursor),
            6 => (&self.replace, &mut self.replace_cursor),
            _ => return,
        };
        *cur = val.chars().count();
    }

    fn rule(&self) -> RenameRule {
        RenameRule {
            mask: self.mask.clone(),
            case: self.case,
            counter_start: self.start.trim().parse().unwrap_or(1),
            counter_step: self.step.trim().parse().unwrap_or(1),
            counter_digits: self.digits.trim().parse().unwrap_or(0),
            search: self.search.clone(),
            replace: self.replace.clone(),
            search_case_sensitive: self.case_sensitive,
            date: self.date.clone(),
            time: self.time.clone(),
        }
    }

    /// `(source, new name)` for every file, in list order.
    fn plan(&self) -> Vec<(VfsPath, String)> {
        let rule = self.rule();
        self.sources
            .iter()
            .zip(&self.originals)
            .enumerate()
            .map(|(i, (src, orig))| (src.clone(), rule.apply(orig, i)))
            .collect()
    }

    fn cycle_case(&mut self, dir: isize) {
        let all = CaseMode::ALL;
        let i = all.iter().position(|c| *c == self.case).unwrap_or(0) as isize;
        self.case = all[(i + dir).rem_euclid(all.len() as isize) as usize];
    }

    fn scroll(&mut self, delta: isize) {
        let n = self.originals.len();
        if n == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, n as isize - 1) as usize;
    }

    fn edit_focused(&mut self, key: KeyEvent) {
        match self.focus {
            0 => edit_text(&mut self.mask, &mut self.mask_cursor, key),
            2 => edit_number(&mut self.start, &mut self.start_cursor, key, true),
            3 => edit_number(&mut self.step, &mut self.step_cursor, key, true),
            4 => edit_number(&mut self.digits, &mut self.digits_cursor, key, false),
            5 => edit_text(&mut self.search, &mut self.search_cursor, key),
            6 => edit_text(&mut self.replace, &mut self.replace_cursor, key),
            _ => {}
        }
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => return DialogResult::Submit(Submit::MultiRename(self.plan())),
            KeyCode::Tab => self.focus = (self.focus + 1) % MR_FOCUS_COUNT,
            KeyCode::BackTab => self.focus = (self.focus + MR_FOCUS_COUNT - 1) % MR_FOCUS_COUNT,
            KeyCode::Up => self.scroll(-1),
            KeyCode::Down => self.scroll(1),
            KeyCode::PageUp => self.scroll(-(self.list_rows as isize)),
            KeyCode::PageDown => self.scroll(self.list_rows as isize),
            KeyCode::Char(' ') if self.focus == 1 => self.cycle_case(1),
            KeyCode::Char(' ') if self.focus == 7 => self.case_sensitive = !self.case_sensitive,
            KeyCode::Left if self.focus == 1 => self.cycle_case(-1),
            KeyCode::Right if self.focus == 1 => self.cycle_case(1),
            _ => self.edit_focused(key),
        }
        DialogResult::None
    }

    pub(crate) fn handle_click(&mut self, _area: Rect, col: u16, row: u16) -> DialogResult {
        let hit = |r: Rect| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height;
        if hit(self.exec_rect) {
            return DialogResult::Submit(Submit::MultiRename(self.plan()));
        }
        if hit(self.cancel_rect) {
            return DialogResult::Cancel;
        }
        // Click an option field to focus it; the case chooser cycles and the
        // case-sensitive checkbox toggles on click.
        for (rect, focus) in self.field_hits.clone() {
            if hit(rect) {
                self.focus = focus;
                match focus {
                    1 => self.cycle_case(1),
                    7 => self.case_sensitive = !self.case_sensitive,
                    _ => self.caret_to_end(focus),
                }
                return DialogResult::None;
            }
        }
        for list in [self.list_left, self.list_right] {
            if hit(list) && row >= list.y {
                let idx = self.top + (row - list.y) as usize;
                if idx < self.originals.len() {
                    self.cursor = idx;
                }
            }
        }
        DialogResult::None
    }

    /// Mouse-wheel over the dialog scrolls the file lists (three rows per notch,
    /// matching the viewer).
    pub(crate) fn handle_scroll(&mut self, delta: isize) {
        self.scroll(delta);
    }

    pub(crate) fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme, gfx: Option<&mut Gfx>) {
        let w = area.width.saturating_sub(4).max(40);
        let h = area.height.saturating_sub(2).max(14);
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&crate::l10n::trd("Multi rename"), theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let dim = Style::default().fg(theme.panel_border).bg(theme.dialog_bg);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // mask
                Constraint::Length(1), // placeholder hint
                Constraint::Length(1), // case + counter
                Constraint::Length(1), // search + replace
                Constraint::Length(1), // separator
                Constraint::Length(1), // column headers
                Constraint::Min(1),    // lists
                Constraint::Length(1), // footer / buttons
            ])
            .split(inner);

        self.field_hits.clear();

        // -- Mask --
        let caret = labeled_field(
            f, rows[0], &format!("{}: ", crate::l10n::trd("Rename mask")), &self.mask,
            self.mask_cursor, self.focus == 0, theme,
        );
        self.field_hits.push((rows[0], 0));

        // -- Placeholder hint --
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  [N] name  [E] ext  [C] counter  [YMD] date  [hms] time  [N1-3]/[E1-2] part",
                dim,
            ))),
            rows[1],
        );

        // -- Case chooser (left) --
        let crow = rows[2];
        let case_style = if self.focus == 1 { theme.dialog_selection } else { base };
        let case_text =
            format!("{}: ◂ {} ▸", crate::l10n::trd("Case"), crate::l10n::trd(self.case.label()));
        let case_w = (case_text.chars().count() as u16).min(crow.width);
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(case_text, case_style))),
            crow,
        );
        self.field_hits.push((Rect { width: case_w, ..crow }, 1));

        // -- Counter fields (right-aligned group): Counter Start / Step / Digits --
        let cs_label = format!("{}: ", crate::l10n::trd("Counter Start"));
        let step_label = format!("{}: ", crate::l10n::trd("Step"));
        let digits_label = format!("{}: ", crate::l10n::trd("Digits"));
        let counter: [(&str, usize, &str, usize, u16); 3] = [
            (&cs_label, 2, self.start.as_str(), self.start_cursor, 5),
            (&step_label, 3, self.step.as_str(), self.step_cursor, 5),
            (&digits_label, 4, self.digits.as_str(), self.digits_cursor, 3),
        ];
        const CGAP: u16 = 2;
        let group_w: u16 = counter
            .iter()
            .map(|(label, _, _, _, fw)| label.chars().count() as u16 + fw)
            .sum::<u16>()
            + CGAP * (counter.len() as u16 - 1);
        let mut cx = crow.x + crow.width.saturating_sub(group_w);
        let mut caret_counter = None;
        for (i, (label, focus_idx, val, cur, fw)) in counter.iter().enumerate() {
            if i > 0 {
                cx += CGAP;
            }
            let focused = self.focus == *focus_idx;
            let lstyle = if focused { theme.dialog_selection } else { base };
            let lw = label.chars().count() as u16;
            let group_x = cx;
            f.render_widget(
                Paragraph::new(Span::styled(label.to_string(), lstyle)),
                Rect { x: cx, y: crow.y, width: lw, height: 1 },
            );
            cx += lw;
            let field = Rect { x: cx, y: crow.y, width: *fw, height: 1 };
            if let Some(p) = draw_num_field(f, field, val, *cur, focused, theme) {
                caret_counter = Some(p);
            }
            cx += fw;
            // The label + field together are clickable to focus this counter.
            self.field_hits.push((
                Rect { x: group_x, y: crow.y, width: lw + fw, height: 1 },
                *focus_idx,
            ));
        }

        // -- Search & replace --
        let sr = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(38),
                Constraint::Percentage(38),
                Constraint::Min(10),
            ])
            .split(rows[3]);
        let s1 = labeled_field(f, sr[0], &format!("{}: ", crate::l10n::trd("Search")), &self.search, self.search_cursor, self.focus == 5, theme);
        let s2 = labeled_field(f, sr[1], &format!("{}: ", crate::l10n::trd("Replace")), &self.replace, self.replace_cursor, self.focus == 6, theme);
        f.render_widget(
            Paragraph::new(Line::from(check_span(&crate::l10n::trd("Case sensitive"), self.case_sensitive, self.focus == 7, theme)))
                .style(Style::default().bg(theme.dialog_bg)),
            sr[2],
        );
        self.field_hits.push((sr[0], 5));
        self.field_hits.push((sr[1], 6));
        self.field_hits.push((sr[2], 7));

        // -- List geometry: two columns split by a vertical divider. --
        let list = rows[6];
        let half = list.width.saturating_sub(1) / 2;
        let left = Rect { width: half, ..list };
        let divider_x = list.x + half;
        let right = Rect {
            x: list.x + half + 1,
            width: list.width.saturating_sub(half + 1),
            ..list
        };

        // -- Separator between the settings and the file tables. --
        let sep = rows[4];
        f.render_widget(
            Paragraph::new(Line::from(Span::styled("─".repeat(sep.width as usize), dim))),
            sep,
        );
        f.buffer_mut().set_string(divider_x, sep.y, "┬", dim);

        // -- Column headers (with the divider). --
        let hdr = rows[5];
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&crate::l10n::trd("Original name"), left.width as usize),
                base.add_modifier(Modifier::BOLD),
            ))),
            Rect { width: left.width, ..hdr },
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pad_right(&crate::l10n::trd("New name"), right.width as usize),
                base.add_modifier(Modifier::BOLD),
            ))),
            Rect { x: right.x, width: right.width, ..hdr },
        );
        f.buffer_mut().set_string(divider_x, hdr.y, "│", dim);

        self.list_rows = (list.height as usize).max(1);
        self.top = crate::util::scroll::scroll_to_visible(self.top, self.cursor, self.list_rows);
        self.list_left = left;
        self.list_right = right;

        let rule = self.rule();
        let changed = Style::default().fg(theme.exec_fg).bg(theme.dialog_bg);
        for vi in 0..self.list_rows {
            let idx = self.top + vi;
            if idx >= self.originals.len() {
                break;
            }
            let y = list.y + vi as u16;
            let orig = &self.originals[idx];
            let newname = rule.apply(orig, idx);
            let selected = idx == self.cursor;
            let lstyle = if selected { theme.dialog_selection } else { base };
            let rstyle = if selected {
                theme.dialog_selection
            } else if newname != *orig {
                changed
            } else {
                base
            };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    pad_right(&ellipsize(orig, left.width as usize), left.width as usize),
                    lstyle,
                ))),
                Rect { x: left.x, y, width: left.width, height: 1 },
            );
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    pad_right(&ellipsize(&newname, right.width as usize), right.width as usize),
                    rstyle,
                ))),
                Rect { x: right.x, y, width: right.width, height: 1 },
            );
            f.buffer_mut().set_string(divider_x, y, "│", dim);
        }

        // -- Footer: file count (left) + Execute / Cancel (right). --
        let footer = rows[7];
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {} {}   Tab: field   ↑↓: scroll", self.originals.len(), crate::l10n::trd("file(s)")),
                dim,
            ))),
            footer,
        );
        let exec_label = crate::l10n::trd("Execute");
        let cancel_label = crate::l10n::trd("Cancel");
        let exec = format!("[ {exec_label} ]");
        let cancel = format!("[ {cancel_label} ]");
        let total = exec.chars().count() + 3 + cancel.chars().count();
        let bx = footer.x + footer.width.saturating_sub(total as u16 + 1);
        let exec_rect = Rect { x: bx, y: footer.y, width: exec.chars().count() as u16, height: 1 };
        let cancel_rect = Rect {
            x: bx + exec.chars().count() as u16 + 3,
            y: footer.y,
            width: cancel.chars().count() as u16,
            height: 1,
        };
        let mut gfx = gfx;
        if gfx.as_deref().is_some_and(|g| g.buttons_ok()) {
            gfx_button(f, gfx.as_deref_mut(), Slot::Button(0), exec_rect, &exec_label, true, theme);
            gfx_button(f, gfx, Slot::Button(1), cancel_rect, &cancel_label, false, theme);
        } else {
            f.render_widget(Paragraph::new(Line::from(button(&exec, true, theme))), exec_rect);
            f.render_widget(Paragraph::new(Line::from(button(&cancel, false, theme))), cancel_rect);
        }
        self.exec_rect = exec_rect;
        self.cancel_rect = cancel_rect;

        // Place the terminal caret on whichever text field is focused (only the
        // focused field returns a position).
        for c in [caret, caret_counter, s1, s2].into_iter().flatten() {
            f.set_cursor_position(c);
        }
    }
}

