//! Modal dialogs: text input, confirmation, progress, and messages.
//!
//! Phase 1 keeps these in one module as small state machines. Each dialog
//! consumes key events and reports a [`DialogResult`]; the app acts on
//! `Submit`/`Abort` outcomes.

use crate::ops::progress::{ProgressUpdate, TaskId};
use crate::ui::theme::Theme;
use crate::util::bytes::human_size;
use crate::vfs::VfsPath;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap};

/// The active modal dialog (only one at a time).
pub enum Dialog {
    Input(InputDialog),
    Confirm(ConfirmDialog),
    Progress(ProgressDialog),
    Message(MessageDialog),
    Form(FormDialog),
}

/// What the app should do after a dialog handles a key.
pub enum DialogResult {
    /// Key consumed; keep the dialog open.
    None,
    /// Close the dialog with no further action.
    Cancel,
    /// Close and perform this action.
    Submit(Submit),
    /// Abort the running task with this id (from the progress dialog).
    Abort(TaskId),
}

/// A confirmed user intent produced by a dialog.
pub enum Submit {
    MkDir(String),
    Copy(Vec<VfsPath>, String),
    Move(Vec<VfsPath>, String),
    Delete(Vec<VfsPath>),
    Quit,
    SelectGroup(String),
    UnselectGroup(String),
    Chmod(VfsPath, u32),
    Chown(VfsPath, String, String),
    Symlink {
        dir: VfsPath,
        target: String,
        name: String,
    },
    Settings(SettingsValues),
}

/// Values collected by the settings form.
#[derive(Debug, Clone)]
pub struct SettingsValues {
    pub editor: String,
    pub viewer: String,
    pub use_internal_viewer: bool,
    pub use_internal_editor: bool,
    pub confirm_delete: bool,
}

impl Dialog {
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match self {
            Dialog::Input(d) => d.handle_key(key),
            Dialog::Confirm(d) => d.handle_key(key),
            Dialog::Progress(d) => d.handle_key(key),
            Dialog::Message(_) => DialogResult::Cancel, // any key closes
            Dialog::Form(d) => d.handle_key(key),
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        match self {
            Dialog::Input(d) => d.render(f, area, theme),
            Dialog::Confirm(d) => d.render(f, area, theme),
            Dialog::Progress(d) => d.render(f, area, theme),
            Dialog::Message(d) => d.render(f, area, theme),
            Dialog::Form(d) => d.render(f, area, theme),
        }
    }
}

// ---------------------------------------------------------------------------
// Input dialog
// ---------------------------------------------------------------------------

/// What an input dialog's submitted text should be used for.
pub enum InputPurpose {
    MkDir,
    CopyDest(Vec<VfsPath>),
    MoveDest(Vec<VfsPath>),
    SelectGroup,
    UnselectGroup,
}

pub struct InputDialog {
    pub title: String,
    pub prompt: String,
    pub buffer: String,
    /// Caret position as a char index.
    pub cursor: usize,
    pub purpose: InputPurpose,
}

impl InputDialog {
    pub fn new(
        title: impl Into<String>,
        prompt: impl Into<String>,
        initial: impl Into<String>,
        purpose: InputPurpose,
    ) -> Self {
        let buffer = initial.into();
        let cursor = buffer.chars().count();
        InputDialog {
            title: title.into(),
            prompt: prompt.into(),
            buffer,
            cursor,
            purpose,
        }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.buffer
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.buffer.len())
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Enter => {
                let text = self.buffer.trim().to_string();
                if text.is_empty() {
                    return DialogResult::Cancel;
                }
                let submit = match &self.purpose {
                    InputPurpose::MkDir => Submit::MkDir(text),
                    InputPurpose::CopyDest(s) => Submit::Copy(s.clone(), text),
                    InputPurpose::MoveDest(s) => Submit::Move(s.clone(), text),
                    InputPurpose::SelectGroup => Submit::SelectGroup(text),
                    InputPurpose::UnselectGroup => Submit::UnselectGroup(text),
                };
                DialogResult::Submit(submit)
            }
            KeyCode::Char(c) => {
                let b = self.byte_at(self.cursor);
                self.buffer.insert(b, c);
                self.cursor += 1;
                DialogResult::None
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let start = self.byte_at(self.cursor - 1);
                    self.buffer.remove(start);
                    self.cursor -= 1;
                }
                DialogResult::None
            }
            KeyCode::Delete => {
                let len = self.buffer.chars().count();
                if self.cursor < len {
                    let start = self.byte_at(self.cursor);
                    self.buffer.remove(start);
                }
                DialogResult::None
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Right => {
                let len = self.buffer.chars().count();
                if self.cursor < len {
                    self.cursor += 1;
                }
                DialogResult::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                DialogResult::None
            }
            KeyCode::End => {
                self.cursor = self.buffer.chars().count();
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 7);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
            .split(inner);

        f.render_widget(
            Paragraph::new(Line::from(self.prompt.clone()))
                .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)),
            rows[0],
        );

        let field = Rect {
            height: 1,
            ..rows[1]
        };
        f.render_widget(
            Paragraph::new(Line::from(self.buffer.clone())).style(
                Style::default()
                    .fg(theme.dialog_fg)
                    .bg(ratatui::style::Color::White),
            ),
            field,
        );
        // Place the real cursor in the field.
        let cx = field.x + self.cursor.min(field.width.saturating_sub(1) as usize) as u16;
        f.set_cursor_position(Position::new(cx, field.y));

        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "[ Enter=OK  Esc=Cancel ]",
                Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg),
            )))
            .alignment(ratatui::layout::Alignment::Center),
            rows[2],
        );
    }
}

// ---------------------------------------------------------------------------
// Confirm dialog
// ---------------------------------------------------------------------------

pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub focus_yes: bool,
    pub submit: Option<Submit>,
}

impl ConfirmDialog {
    pub fn delete(targets: Vec<VfsPath>) -> Self {
        let message = if targets.len() == 1 {
            format!("Delete \"{}\"?", targets[0].file_name())
        } else {
            format!("Delete {} selected items?", targets.len())
        };
        ConfirmDialog {
            title: "Delete".to_string(),
            message,
            focus_yes: true,
            submit: Some(Submit::Delete(targets)),
        }
    }

    pub fn quit() -> Self {
        ConfirmDialog {
            title: "Quit".to_string(),
            message: "Do you really want to quit rat-commander?".to_string(),
            focus_yes: true,
            submit: Some(Submit::Quit),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Char('n') | KeyCode::Char('N') => DialogResult::Cancel,
            KeyCode::Char('y') | KeyCode::Char('Y') => self.confirm(),
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                self.focus_yes = !self.focus_yes;
                DialogResult::None
            }
            KeyCode::Enter => {
                if self.focus_yes {
                    self.confirm()
                } else {
                    DialogResult::Cancel
                }
            }
            _ => DialogResult::None,
        }
    }

    fn confirm(&mut self) -> DialogResult {
        match self.submit.take() {
            Some(s) => DialogResult::Submit(s),
            None => DialogResult::Cancel,
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 50u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 7);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        f.render_widget(
            Paragraph::new(self.message.clone())
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg))
                .alignment(ratatui::layout::Alignment::Center),
            rows[0],
        );

        let yes = button("[ Yes ]", self.focus_yes, theme);
        let no = button("[ No ]", !self.focus_yes, theme);
        let buttons = Line::from(vec![yes, Span::raw("   "), no]);
        f.render_widget(
            Paragraph::new(buttons)
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(theme.dialog_bg)),
            rows[1],
        );
    }
}

// ---------------------------------------------------------------------------
// Progress dialog
// ---------------------------------------------------------------------------

pub struct ProgressDialog {
    pub id: TaskId,
    pub verb: &'static str,
    pub current_name: String,
    pub file_done: u64,
    pub file_total: u64,
    pub total_done: u64,
    pub total_total: u64,
    pub files_done: u64,
    pub files_total: u64,
}

impl ProgressDialog {
    pub fn new(id: TaskId, verb: &'static str) -> Self {
        ProgressDialog {
            id,
            verb,
            current_name: String::new(),
            file_done: 0,
            file_total: 0,
            total_done: 0,
            total_total: 0,
            files_done: 0,
            files_total: 0,
        }
    }

    pub fn update(&mut self, u: &ProgressUpdate) {
        self.verb = u.verb;
        self.current_name = u.current_name.clone();
        self.file_done = u.file_done;
        self.file_total = u.file_total;
        self.total_done = u.total_done;
        self.total_total = u.total_total;
        self.files_done = u.files_done;
        self.files_total = u.files_total;
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => DialogResult::Abort(self.id),
            _ => DialogResult::None,
        }
    }

    fn ratio(done: u64, total: u64) -> f64 {
        if total == 0 {
            if done > 0 { 1.0 } else { 0.0 }
        } else {
            (done as f64 / total as f64).clamp(0.0, 1.0)
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 10);
        f.render_widget(Clear, rect);
        let block = dialog_block(self.verb, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // file name
                Constraint::Length(1), // file gauge
                Constraint::Length(1), // spacer/label
                Constraint::Length(1), // total gauge
                Constraint::Min(0),    // hint
            ])
            .split(inner);

        let name = crate::util::text::ellipsize(&self.current_name, inner.width as usize);
        f.render_widget(
            Paragraph::new(Line::from(name))
                .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)),
            rows[0],
        );

        let file_gauge = Gauge::default()
            .gauge_style(Style::default().fg(ratatui::style::Color::Green))
            .ratio(Self::ratio(self.file_done, self.file_total))
            .label(format!(
                "{} / {}",
                human_size(self.file_done),
                human_size(self.file_total)
            ));
        f.render_widget(file_gauge, rows[1]);

        f.render_widget(
            Paragraph::new(Line::from(format!(
                "Total: {} / {} files",
                self.files_done, self.files_total
            )))
            .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)),
            rows[2],
        );

        let total_gauge = Gauge::default()
            .gauge_style(Style::default().fg(ratatui::style::Color::Cyan))
            .ratio(Self::ratio(self.total_done, self.total_total))
            .label(format!(
                "{} / {}",
                human_size(self.total_done),
                human_size(self.total_total)
            ));
        f.render_widget(total_gauge, rows[3]);

        f.render_widget(
            Paragraph::new(Line::from(button("[ Abort ]", true, theme)))
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(theme.dialog_bg)),
            rows[4],
        );
    }
}

// ---------------------------------------------------------------------------
// Message dialog (errors / info)
// ---------------------------------------------------------------------------

pub struct MessageDialog {
    pub title: String,
    pub message: String,
    pub is_error: bool,
}

impl MessageDialog {
    pub fn error(message: impl Into<String>) -> Self {
        MessageDialog {
            title: "Error".to_string(),
            message: message.into(),
            is_error: true,
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 8);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let fg = if self.is_error {
            theme.error_fg
        } else {
            theme.dialog_fg
        };
        f.render_widget(
            Paragraph::new(self.message.clone())
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(fg).bg(theme.dialog_bg))
                .alignment(ratatui::layout::Alignment::Center),
            rows[0],
        );
        f.render_widget(
            Paragraph::new(Line::from(button("[ OK ]", true, theme)))
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(theme.dialog_bg)),
            rows[1],
        );
    }
}

// ---------------------------------------------------------------------------
// Form dialog (settings, chmod, chown, symlink)
// ---------------------------------------------------------------------------

/// A single editable field in a [`Form`].
pub enum Field {
    Text {
        label: String,
        value: String,
        cursor: usize,
    },
    Check {
        label: String,
        value: bool,
    },
}

impl Field {
    pub fn text(label: &str, value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor = value.chars().count();
        Field::Text {
            label: label.to_string(),
            value,
            cursor,
        }
    }

    pub fn check(label: &str, value: bool) -> Self {
        Field::Check {
            label: label.to_string(),
            value,
        }
    }

    fn as_text(&self) -> &str {
        match self {
            Field::Text { value, .. } => value,
            Field::Check { .. } => "",
        }
    }

    fn as_bool(&self) -> bool {
        matches!(self, Field::Check { value: true, .. })
    }
}

/// A vertical list of editable fields with a single focused row.
pub struct Form {
    fields: Vec<Field>,
    focus: usize,
}

impl Form {
    pub fn new(fields: Vec<Field>) -> Self {
        Form { fields, focus: 0 }
    }

    fn focus_next(&mut self) {
        if !self.fields.is_empty() {
            self.focus = (self.focus + 1) % self.fields.len();
        }
    }

    fn focus_prev(&mut self) {
        if !self.fields.is_empty() {
            self.focus = (self.focus + self.fields.len() - 1) % self.fields.len();
        }
    }

    /// Handle a key for the focused field. Returns true if Enter (submit) was
    /// pressed.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter => return true,
            KeyCode::Tab | KeyCode::Down => self.focus_next(),
            KeyCode::BackTab | KeyCode::Up => self.focus_prev(),
            KeyCode::Char(' ') if matches!(self.fields.get(self.focus), Some(Field::Check { .. })) => {
                if let Some(Field::Check { value, .. }) = self.fields.get_mut(self.focus) {
                    *value = !*value;
                }
            }
            _ => {
                if let Some(Field::Text { value, cursor, .. }) = self.fields.get_mut(self.focus) {
                    edit_text(value, cursor, key);
                }
            }
        }
        false
    }
}

/// Apply a single editing key to a text buffer + char cursor.
fn edit_text(value: &mut String, cursor: &mut usize, key: KeyEvent) {
    let byte_at = |s: &str, idx: usize| {
        s.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(s.len())
    };
    match key.code {
        KeyCode::Char(c) => {
            let b = byte_at(value, *cursor);
            value.insert(b, c);
            *cursor += 1;
        }
        KeyCode::Backspace => {
            if *cursor > 0 {
                let b = byte_at(value, *cursor - 1);
                value.remove(b);
                *cursor -= 1;
            }
        }
        KeyCode::Delete => {
            if *cursor < value.chars().count() {
                let b = byte_at(value, *cursor);
                value.remove(b);
            }
        }
        KeyCode::Left => *cursor = cursor.saturating_sub(1),
        KeyCode::Right => {
            if *cursor < value.chars().count() {
                *cursor += 1;
            }
        }
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = value.chars().count(),
        _ => {}
    }
}

/// What a form's values should become on submit.
pub enum FormPurpose {
    Settings,
    Chmod(VfsPath),
    Chown(VfsPath),
    /// Create a symlink inside this directory.
    Symlink(VfsPath),
}

pub struct FormDialog {
    pub title: String,
    pub form: Form,
    pub purpose: FormPurpose,
}

impl FormDialog {
    pub fn settings(cfg: &crate::config::Config) -> Self {
        let form = Form::new(vec![
            Field::text("External editor", cfg.editor.clone()),
            Field::text("External viewer", cfg.viewer.clone()),
            Field::check("Use internal viewer", cfg.use_internal_viewer),
            Field::check("Use internal editor", cfg.use_internal_editor),
            Field::check("Confirm before delete", cfg.confirm_delete),
        ]);
        FormDialog {
            title: "Settings".to_string(),
            form,
            purpose: FormPurpose::Settings,
        }
    }

    /// Build a chmod form from the current mode bits.
    pub fn chmod(path: VfsPath, mode: u32) -> Self {
        let bit = |m: u32| mode & m != 0;
        let form = Form::new(vec![
            Field::check("Owner read    (400)", bit(0o400)),
            Field::check("Owner write   (200)", bit(0o200)),
            Field::check("Owner exec    (100)", bit(0o100)),
            Field::check("Group read    (040)", bit(0o040)),
            Field::check("Group write   (020)", bit(0o020)),
            Field::check("Group exec    (010)", bit(0o010)),
            Field::check("Other read    (004)", bit(0o004)),
            Field::check("Other write   (002)", bit(0o002)),
            Field::check("Other exec    (001)", bit(0o001)),
        ]);
        FormDialog {
            title: format!("Chmod: {}", path.file_name()),
            form,
            purpose: FormPurpose::Chmod(path),
        }
    }

    pub fn chown(path: VfsPath, owner: String, group: String) -> Self {
        let form = Form::new(vec![
            Field::text("Owner (name or uid)", owner),
            Field::text("Group (name or gid)", group),
        ]);
        FormDialog {
            title: format!("Chown: {}", path.file_name()),
            form,
            purpose: FormPurpose::Chown(path),
        }
    }

    pub fn symlink(dir: VfsPath) -> Self {
        let form = Form::new(vec![
            Field::text("Points to (target)", ""),
            Field::text("Link name", ""),
        ]);
        FormDialog {
            title: "Create symlink".to_string(),
            form,
            purpose: FormPurpose::Symlink(dir),
        }
    }

    fn chmod_mode(&self) -> u32 {
        const BITS: [u32; 9] = [
            0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001,
        ];
        let mut mode = 0;
        for (i, f) in self.form.fields.iter().enumerate() {
            if f.as_bool() {
                mode |= BITS[i];
            }
        }
        mode
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        if let KeyCode::Esc = key.code {
            return DialogResult::Cancel;
        }
        if !self.form.handle_key(key) {
            return DialogResult::None;
        }
        // Enter pressed → build the submit payload.
        let fields = &self.form.fields;
        let submit = match &self.purpose {
            FormPurpose::Settings => Submit::Settings(SettingsValues {
                editor: fields[0].as_text().trim().to_string(),
                viewer: fields[1].as_text().trim().to_string(),
                use_internal_viewer: fields[2].as_bool(),
                use_internal_editor: fields[3].as_bool(),
                confirm_delete: fields[4].as_bool(),
            }),
            FormPurpose::Chmod(p) => Submit::Chmod(p.clone(), self.chmod_mode()),
            FormPurpose::Chown(p) => Submit::Chown(
                p.clone(),
                fields[0].as_text().trim().to_string(),
                fields[1].as_text().trim().to_string(),
            ),
            FormPurpose::Symlink(dir) => {
                let target = fields[0].as_text().trim().to_string();
                let name = fields[1].as_text().trim().to_string();
                if target.is_empty() || name.is_empty() {
                    return DialogResult::Cancel;
                }
                Submit::Symlink {
                    dir: dir.clone(),
                    target,
                    name,
                }
            }
        };
        DialogResult::Submit(submit)
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let n = self.form.fields.len() as u16;
        let height = n + 4;
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, height);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let focus_style = Style::default()
            .fg(theme.dialog_fg)
            .bg(ratatui::style::Color::Cyan)
            .add_modifier(Modifier::BOLD);

        let mut caret: Option<Position> = None;
        for (i, field) in self.form.fields.iter().enumerate() {
            let y = inner.y + i as u16;
            if y >= inner.y + inner.height.saturating_sub(1) {
                break;
            }
            let row = Rect {
                y,
                height: 1,
                ..inner
            };
            let focused = i == self.form.focus;
            match field {
                Field::Text {
                    label,
                    value,
                    cursor,
                } => {
                    let label_str = format!("{label}: ");
                    let style = if focused { focus_style } else { base };
                    let line = Line::from(vec![
                        Span::styled(label_str.clone(), style),
                        Span::styled(
                            value.clone(),
                            Style::default()
                                .fg(theme.dialog_fg)
                                .bg(ratatui::style::Color::White),
                        ),
                    ]);
                    f.render_widget(Paragraph::new(line), row);
                    if focused {
                        let cx = row.x
                            + label_str.chars().count() as u16
                            + (*cursor).min(value.chars().count()) as u16;
                        caret = Some(Position::new(cx.min(row.x + row.width - 1), row.y));
                    }
                }
                Field::Check { label, value } => {
                    let mark = if *value { "[x]" } else { "[ ]" };
                    let style = if focused { focus_style } else { base };
                    f.render_widget(
                        Paragraph::new(Line::from(Span::styled(format!("{mark} {label}"), style))),
                        row,
                    );
                }
            }
        }

        let hint = Rect {
            y: inner.y + inner.height.saturating_sub(1),
            height: 1,
            ..inner
        };
        let extra = match &self.purpose {
            FormPurpose::Chmod(_) => format!("  octal {:03o}", self.chmod_mode()),
            _ => String::new(),
        };
        f.render_widget(
            Paragraph::new(Line::from(format!(
                "Tab/↑↓ move  Space toggle  Enter OK  Esc Cancel{extra}"
            )))
            .style(base),
            hint,
        );

        if let Some(pos) = caret {
            f.set_cursor_position(pos);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// A rectangle of fixed size centered within `area`.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn dialog_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.dialog_fg)
                .bg(theme.dialog_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg))
}

fn button(text: &str, focused: bool, theme: &Theme) -> Span<'static> {
    let style = if focused {
        theme.button_focused
    } else {
        theme.button
    };
    Span::styled(text.to_string(), style)
}
