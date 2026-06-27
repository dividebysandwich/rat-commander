//! Modal dialogs: text input, confirmation, progress, and messages.
//!
//! Phase 1 keeps these in one module as small state machines. Each dialog
//! consumes key events and reports a [`DialogResult`]; the app acts on
//! `Submit`/`Abort` outcomes.

use crate::ops::progress::{ProgressUpdate, TaskId};
use crate::ui::theme::Theme;
use crate::util::bytes::human_size;
use crate::vfs::VfsPath;
use crate::vfs::remote::{Protocol, RemoteCreds};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap};

/// The active modal dialog (only one at a time).
#[allow(clippy::large_enum_variant)]
pub enum Dialog {
    Input(InputDialog),
    Confirm(ConfirmDialog),
    Progress(ProgressDialog),
    Message(MessageDialog),
    Form(FormDialog),
    Select(SelectDialog),
    SearchReplace(SearchReplaceDialog),
    Find(FindDialog),
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
#[allow(clippy::large_enum_variant)]
pub enum Submit {
    MkDir(String),
    Copy(Vec<VfsPath>, String),
    Move(Vec<VfsPath>, String),
    Delete(Vec<VfsPath>),
    Quit,
    EditorSaveQuit,
    EditorDiscardQuit,
    /// Select/unselect files by pattern with options.
    Select {
        select: bool,
        pattern: String,
        files_only: bool,
        case_sensitive: bool,
        shell: bool,
    },
    /// Editor search or search-and-replace.
    SearchReplace(SearchReplaceParams),
    /// Find-file request.
    Find(FindParams),
    Chmod(VfsPath, u32),
    Chown(VfsPath, String, String),
    Symlink {
        dir: VfsPath,
        target: String,
        name: String,
    },
    Settings(SettingsValues),
    /// Compress these (local) sources into an archive of the given name.
    Compress(Vec<VfsPath>, String),
    /// Open a remote connection on the given panel side.
    Connect(usize, RemoteCreds),
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
            Dialog::Select(d) => d.handle_key(key),
            Dialog::SearchReplace(d) => d.handle_key(key),
            Dialog::Find(d) => d.handle_key(key),
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        match self {
            Dialog::Input(d) => d.render(f, area, theme),
            Dialog::Confirm(d) => d.render(f, area, theme),
            Dialog::Progress(d) => d.render(f, area, theme),
            Dialog::Message(d) => d.render(f, area, theme),
            Dialog::Form(d) => d.render(f, area, theme),
            Dialog::Select(d) => d.render(f, area, theme),
            Dialog::SearchReplace(d) => d.render(f, area, theme),
            Dialog::Find(d) => d.render(f, area, theme),
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
    Compress(Vec<VfsPath>),
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
                    InputPurpose::Compress(s) => Submit::Compress(s.clone(), text),
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
        if let Some(pos) = draw_input_field(f, field, &self.buffer, self.cursor, true, false, theme) {
            f.set_cursor_position(pos);
        }

        f.render_widget(
            Paragraph::new(ok_cancel_line(true, theme))
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(theme.dialog_bg)),
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
    pub yes_label: String,
    pub no_label: String,
    pub submit: Option<Submit>,
    /// Action for the "No" button. When `None`, "No" simply cancels.
    pub no_submit: Option<Submit>,
}

impl ConfirmDialog {
    fn yes_no(
        title: &str,
        message: String,
        submit: Submit,
        yes_label: &str,
        no_label: &str,
        no_submit: Option<Submit>,
    ) -> Self {
        ConfirmDialog {
            title: title.to_string(),
            message,
            focus_yes: true,
            yes_label: yes_label.to_string(),
            no_label: no_label.to_string(),
            submit: Some(submit),
            no_submit,
        }
    }

    pub fn delete(targets: Vec<VfsPath>) -> Self {
        let message = if targets.len() == 1 {
            format!("Delete \"{}\"?", targets[0].file_name())
        } else {
            format!("Delete {} selected items?", targets.len())
        };
        Self::yes_no("Delete", message, Submit::Delete(targets), "Yes", "No", None)
    }

    pub fn quit() -> Self {
        Self::yes_no(
            "Quit",
            "Do you really want to quit rat-commander?".to_string(),
            Submit::Quit,
            "Yes",
            "No",
            None,
        )
    }

    /// The editor's save/discard/cancel modal. Yes = save & quit, No = discard
    /// & quit, Esc = cancel (stay in the editor).
    pub fn editor_quit(name: &str) -> Self {
        Self::yes_no(
            "File modified",
            format!("\"{name}\" has unsaved changes. Save before closing?"),
            Submit::EditorSaveQuit,
            "Save",
            "Discard",
            Some(Submit::EditorDiscardQuit),
        )
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Char('n') | KeyCode::Char('N') => self.no_action(),
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('s') | KeyCode::Char('S') => {
                self.confirm()
            }
            KeyCode::Char('d') | KeyCode::Char('D') => self.no_action(),
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                self.focus_yes = !self.focus_yes;
                DialogResult::None
            }
            KeyCode::Enter => {
                if self.focus_yes {
                    self.confirm()
                } else {
                    self.no_action()
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

    fn no_action(&mut self) -> DialogResult {
        match self.no_submit.take() {
            Some(s) => DialogResult::Submit(s),
            None => DialogResult::Cancel,
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 54u16.min(area.width.saturating_sub(4));
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

        let yes = button(&format!("[ {} ]", self.yes_label), self.focus_yes, theme);
        let no = button(&format!("[ {} ]", self.no_label), !self.focus_yes, theme);
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
    /// When true, render an indeterminate sweep (e.g. find-file scanning).
    pub indeterminate: bool,
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
            indeterminate: false,
        }
    }

    /// An indeterminate progress dialog for find-file scanning.
    pub fn find(id: TaskId) -> Self {
        let mut d = Self::new(id, "Searching");
        d.indeterminate = true;
        d
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
        if self.indeterminate {
            return self.render_indeterminate(f, area, theme);
        }
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

    /// Render an indeterminate scanning dialog (current path + sweep + count).
    fn render_indeterminate(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 64u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 8);
        f.render_widget(Clear, rect);
        let block = dialog_block(self.verb, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let line_at = |yy: u16| Rect { x: inner.x, y: yy, width: inner.width, height: 1 };

        f.render_widget(
            Paragraph::new(Line::from(format!("{} files found", self.files_done))).style(base),
            line_at(inner.y),
        );
        let name = crate::util::text::ellipsize(&self.current_name, inner.width as usize);
        f.render_widget(Paragraph::new(Line::from(name)).style(base), line_at(inner.y + 1));

        // A bouncing block sweeps based on the update counter (files_done).
        let bar_w = inner.width as usize;
        let block_w = (bar_w / 5).max(1);
        let span = bar_w.saturating_sub(block_w).max(1);
        let phase = (self.files_done as usize) % (2 * span);
        let pos = if phase < span { phase } else { 2 * span - phase };
        let mut bar = String::with_capacity(bar_w);
        for i in 0..bar_w {
            bar.push(if i >= pos && i < pos + block_w { '█' } else { '░' });
        }
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                bar,
                Style::default().fg(theme.input_bg).bg(theme.dialog_bg),
            ))),
            line_at(inner.y + 3),
        );

        f.render_widget(
            Paragraph::new(Line::from(button("[ Abort ]", true, theme)))
                .alignment(ratatui::layout::Alignment::Center)
                .style(base),
            line_at(inner.y + inner.height - 1),
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
    Password {
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

    pub fn password(label: &str) -> Self {
        Field::Password {
            label: label.to_string(),
            value: String::new(),
            cursor: 0,
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
            Field::Text { value, .. } | Field::Password { value, .. } => value,
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
            _ => match self.fields.get_mut(self.focus) {
                Some(Field::Text { value, cursor, .. })
                | Some(Field::Password { value, cursor, .. }) => edit_text(value, cursor, key),
                _ => {}
            },
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
    /// Open a remote connection of this protocol on the given panel side.
    Connect(Protocol, usize),
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

    pub fn connect(protocol: Protocol, side: usize) -> Self {
        let form = Form::new(vec![
            Field::text("Host", ""),
            Field::text("Port", protocol.default_port().to_string()),
            Field::text("Username", ""),
            Field::password("Password"),
            Field::text("Remote path (blank = home)", ""),
        ]);
        FormDialog {
            title: format!("{} connection", protocol.scheme_prefix().to_uppercase()),
            form,
            purpose: FormPurpose::Connect(protocol, side),
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
            FormPurpose::Connect(protocol, side) => {
                let host = fields[0].as_text().trim().to_string();
                if host.is_empty() {
                    return DialogResult::Cancel;
                }
                let port = fields[1]
                    .as_text()
                    .trim()
                    .parse::<u16>()
                    .unwrap_or(protocol.default_port());
                Submit::Connect(
                    *side,
                    RemoteCreds {
                        protocol: *protocol,
                        host,
                        port,
                        user: fields[2].as_text().trim().to_string(),
                        password: fields[3].as_text().to_string(),
                        path: fields[4].as_text().trim().to_string(),
                    },
                )
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
                }
                | Field::Password {
                    label,
                    value,
                    cursor,
                } => {
                    let masked = matches!(field, Field::Password { .. });
                    let label_str = format!("{label}: ");
                    let lw = (label_str.chars().count() as u16).min(row.width);
                    let style = if focused { focus_style } else { base };
                    f.render_widget(
                        Paragraph::new(Span::styled(label_str, style)),
                        Rect { width: lw, ..row },
                    );
                    let field_area = Rect {
                        x: row.x + lw,
                        width: row.width.saturating_sub(lw),
                        ..row
                    };
                    if let Some(pos) =
                        draw_input_field(f, field_area, value, *cursor, focused, masked, theme)
                    {
                        caret = Some(pos);
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
// Select / unselect-group dialog
// ---------------------------------------------------------------------------

pub struct SelectDialog {
    select: bool,
    pattern: String,
    cursor: usize,
    files_only: bool,
    case_sensitive: bool,
    shell: bool,
    focus: usize, // 0 pattern, 1 files_only, 2 case, 3 shell
}

impl SelectDialog {
    pub fn new(select: bool) -> Self {
        SelectDialog {
            select,
            pattern: "*".to_string(),
            cursor: 1,
            files_only: false,
            case_sensitive: true,
            shell: true,
            focus: 0,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => {
                if self.pattern.trim().is_empty() {
                    return DialogResult::Cancel;
                }
                return DialogResult::Submit(Submit::Select {
                    select: self.select,
                    pattern: self.pattern.clone(),
                    files_only: self.files_only,
                    case_sensitive: self.case_sensitive,
                    shell: self.shell,
                });
            }
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % 4,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + 3) % 4,
            KeyCode::Char(' ') if self.focus > 0 => match self.focus {
                1 => self.files_only = !self.files_only,
                2 => self.case_sensitive = !self.case_sensitive,
                3 => self.shell = !self.shell,
                _ => {}
            },
            _ if self.focus == 0 => edit_text(&mut self.pattern, &mut self.cursor, key),
            _ => {}
        }
        DialogResult::None
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = if self.select { "Select" } else { "Unselect" };
        let rect = centered(area, 54u16.min(area.width.saturating_sub(2)), 7);
        f.render_widget(Clear, rect);
        let block = dialog_block(title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let mut caret = None;
        let field = Rect { height: 1, ..inner };
        if let Some(p) =
            draw_input_field(f, field, &self.pattern, self.cursor, self.focus == 0, false, theme)
        {
            caret = Some(p);
        }

        let half = inner.width / 2;
        let r1 = Rect { y: inner.y + 2, height: 1, ..inner };
        f.render_widget(
            Paragraph::new(Line::from(check_span("Files only", self.files_only, self.focus == 1, theme)))
                .style(Style::default().bg(theme.dialog_bg)),
            Rect { width: half, ..r1 },
        );
        f.render_widget(
            Paragraph::new(Line::from(check_span(
                "Case sensitive",
                self.case_sensitive,
                self.focus == 2,
                theme,
            )))
            .style(Style::default().bg(theme.dialog_bg)),
            Rect { x: inner.x + half, width: inner.width - half, ..r1 },
        );
        let r2 = Rect { y: inner.y + 3, height: 1, ..inner };
        f.render_widget(
            Paragraph::new(Line::from(check_span(
                "Using shell patterns",
                self.shell,
                self.focus == 3,
                theme,
            )))
            .style(Style::default().bg(theme.dialog_bg)),
            r2,
        );

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

// ---------------------------------------------------------------------------
// Search / replace dialog (editor)
// ---------------------------------------------------------------------------

/// Result of the editor search/replace dialog.
#[derive(Debug, Clone)]
pub struct SearchReplaceParams {
    pub replace: bool,
    pub search: String,
    pub replacement: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_words: bool,
    pub backwards: bool,
}

pub struct SearchReplaceDialog {
    replace: bool,
    search: String,
    search_cursor: usize,
    replacement: String,
    repl_cursor: usize,
    mode: usize, // 0 Normal, 1 Regex, 2 Hex, 3 Wildcard
    case_sensitive: bool,
    backwards: bool,
    in_selection: bool,
    whole_words: bool,
    all_charsets: bool,
    focus: usize,
}

#[derive(Clone, Copy)]
enum SrFocus {
    Search,
    Repl,
    Mode(usize),
    Check(usize),
}

impl SearchReplaceDialog {
    pub fn new(replace: bool, initial: String) -> Self {
        let search_cursor = initial.chars().count();
        SearchReplaceDialog {
            replace,
            search: initial,
            search_cursor,
            replacement: String::new(),
            repl_cursor: 0,
            mode: 0,
            case_sensitive: false,
            backwards: false,
            in_selection: false,
            whole_words: false,
            all_charsets: false,
            focus: 0,
        }
    }

    fn items(&self) -> Vec<SrFocus> {
        let mut v = vec![SrFocus::Search];
        if self.replace {
            v.push(SrFocus::Repl);
        }
        v.extend([SrFocus::Mode(0), SrFocus::Mode(1), SrFocus::Mode(2), SrFocus::Mode(3)]);
        v.extend((0..5).map(SrFocus::Check));
        v
    }

    fn cur(&self) -> SrFocus {
        let items = self.items();
        items[self.focus.min(items.len() - 1)]
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let len = self.items().len();
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => {
                if self.search.trim().is_empty() {
                    return DialogResult::Cancel;
                }
                return DialogResult::Submit(Submit::SearchReplace(self.params()));
            }
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % len,
            KeyCode::BackTab | KeyCode::Up => self.focus = (self.focus + len - 1) % len,
            KeyCode::Char(' ') if !matches!(self.cur(), SrFocus::Search | SrFocus::Repl) => {
                match self.cur() {
                    SrFocus::Mode(m) => self.mode = m,
                    SrFocus::Check(c) => self.toggle_check(c),
                    _ => {}
                }
            }
            _ => match self.cur() {
                SrFocus::Search => edit_text(&mut self.search, &mut self.search_cursor, key),
                SrFocus::Repl => edit_text(&mut self.replacement, &mut self.repl_cursor, key),
                _ => {}
            },
        }
        DialogResult::None
    }

    fn toggle_check(&mut self, c: usize) {
        match c {
            0 => self.case_sensitive = !self.case_sensitive,
            1 => self.backwards = !self.backwards,
            2 => self.in_selection = !self.in_selection,
            3 => self.whole_words = !self.whole_words,
            4 => self.all_charsets = !self.all_charsets,
            _ => {}
        }
    }

    fn params(&self) -> SearchReplaceParams {
        // Map the search mode to a regex flag, converting wildcards.
        let (search, regex) = match self.mode {
            1 => (self.search.clone(), true),                // Regular expression
            3 => (wildcard_to_regex(&self.search), true),    // Wildcard search
            _ => (self.search.clone(), false),               // Normal / Hex (literal)
        };
        SearchReplaceParams {
            replace: self.replace,
            search,
            replacement: self.replacement.clone(),
            regex,
            case_sensitive: self.case_sensitive,
            whole_words: self.whole_words,
            backwards: self.backwards,
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = if self.replace { "Replace" } else { "Search" };
        let height = if self.replace { 14 } else { 12 };
        let rect = centered(area, 64u16.min(area.width.saturating_sub(2)), height);
        f.render_widget(Clear, rect);
        let block = dialog_block(title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let mut y = inner.y;
        let mut caret = None;
        let line_at = |yy: u16| Rect { x: inner.x, y: yy, width: inner.width, height: 1 };

        f.render_widget(Paragraph::new(Span::styled("Enter search string:", base)), line_at(y));
        y += 1;
        if let Some(p) = draw_input_field(
            f, line_at(y), &self.search, self.search_cursor,
            matches!(self.cur(), SrFocus::Search), false, theme,
        ) {
            caret = Some(p);
        }
        y += 1;
        if self.replace {
            f.render_widget(
                Paragraph::new(Span::styled("Enter replacement string:", base)),
                line_at(y),
            );
            y += 1;
            if let Some(p) = draw_input_field(
                f, line_at(y), &self.replacement, self.repl_cursor,
                matches!(self.cur(), SrFocus::Repl), false, theme,
            ) {
                caret = Some(p);
            }
            y += 1;
        }
        y += 1; // spacer

        // Options: radios (left) + checkboxes (right).
        let radios = ["Normal", "Regular expression", "Hexadecimal", "Wildcard search"];
        let checks = ["Case sensitive", "Backwards", "In selection", "Whole words", "All charsets"];
        let check_vals = [
            self.case_sensitive, self.backwards, self.in_selection, self.whole_words, self.all_charsets,
        ];
        let half = inner.width / 2;
        for row in 0..5u16 {
            let ry = y + row;
            if ry >= inner.y + inner.height - 1 {
                break;
            }
            if (row as usize) < radios.len() {
                let focused = matches!(self.cur(), SrFocus::Mode(m) if m == row as usize);
                f.render_widget(
                    Paragraph::new(Line::from(radio_span(
                        radios[row as usize], self.mode == row as usize, focused, theme,
                    )))
                    .style(base),
                    Rect { x: inner.x, y: ry, width: half, height: 1 },
                );
            }
            let focused = matches!(self.cur(), SrFocus::Check(c) if c == row as usize);
            f.render_widget(
                Paragraph::new(Line::from(check_span(
                    checks[row as usize], check_vals[row as usize], focused, theme,
                )))
                .style(base),
                Rect { x: inner.x + half, y: ry, width: inner.width - half, height: 1 },
            );
        }

        let by = inner.y + inner.height - 1;
        f.render_widget(
            Paragraph::new(ok_cancel_line(true, theme))
                .alignment(ratatui::layout::Alignment::Center)
                .style(base),
            line_at(by),
        );

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

// ---------------------------------------------------------------------------
// Find-file dialog
// ---------------------------------------------------------------------------

/// Result of the find-file dialog.
#[derive(Debug, Clone)]
pub struct FindParams {
    pub start_at: String,
    pub file_name: String,
    pub content: String,
    pub recursive: bool,
    pub case_sensitive: bool,
    pub skip_hidden: bool,
    pub shell: bool,
}

pub struct FindDialog {
    start_at: String,
    start_cursor: usize,
    file_name: String,
    name_cursor: usize,
    content: String,
    content_cursor: usize,
    recursive: bool,
    case_sensitive: bool,
    skip_hidden: bool,
    shell: bool,
    focus: usize, // 0 start, 1 name, 2 content, 3..6 checks
}

impl FindDialog {
    pub fn new(start_at: String) -> Self {
        let start_cursor = start_at.chars().count();
        FindDialog {
            start_at,
            start_cursor,
            file_name: "*".to_string(),
            name_cursor: 1,
            content: String::new(),
            content_cursor: 0,
            recursive: true,
            case_sensitive: false,
            skip_hidden: true,
            shell: true,
            focus: 1,
        }
    }

    const FOCUS_COUNT: usize = 7;

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => return DialogResult::Cancel,
            KeyCode::Enter => {
                if self.file_name.trim().is_empty() {
                    return DialogResult::Cancel;
                }
                return DialogResult::Submit(Submit::Find(FindParams {
                    start_at: self.start_at.clone(),
                    file_name: self.file_name.clone(),
                    content: self.content.clone(),
                    recursive: self.recursive,
                    case_sensitive: self.case_sensitive,
                    skip_hidden: self.skip_hidden,
                    shell: self.shell,
                }));
            }
            KeyCode::Tab | KeyCode::Down => self.focus = (self.focus + 1) % Self::FOCUS_COUNT,
            KeyCode::BackTab | KeyCode::Up => {
                self.focus = (self.focus + Self::FOCUS_COUNT - 1) % Self::FOCUS_COUNT
            }
            KeyCode::Char(' ') if self.focus >= 3 => match self.focus {
                3 => self.recursive = !self.recursive,
                4 => self.case_sensitive = !self.case_sensitive,
                5 => self.skip_hidden = !self.skip_hidden,
                6 => self.shell = !self.shell,
                _ => {}
            },
            _ => match self.focus {
                0 => edit_text(&mut self.start_at, &mut self.start_cursor, key),
                1 => edit_text(&mut self.file_name, &mut self.name_cursor, key),
                2 => edit_text(&mut self.content, &mut self.content_cursor, key),
                _ => {}
            },
        }
        DialogResult::None
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let rect = centered(area, 66u16.min(area.width.saturating_sub(2)), 13);
        f.render_widget(Clear, rect);
        let block = dialog_block("Find File", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let line_at = |yy: u16| Rect { x: inner.x, y: yy, width: inner.width, height: 1 };
        let mut caret = None;
        let mut y = inner.y;

        f.render_widget(Paragraph::new(Span::styled("Start at:", base)), line_at(y));
        y += 1;
        if let Some(p) = draw_input_field(
            f, line_at(y), &self.start_at, self.start_cursor, self.focus == 0, false, theme,
        ) {
            caret = Some(p);
        }
        y += 2;

        f.render_widget(Paragraph::new(Span::styled("File name:", base)), line_at(y));
        y += 1;
        if let Some(p) = draw_input_field(
            f, line_at(y), &self.file_name, self.name_cursor, self.focus == 1, false, theme,
        ) {
            caret = Some(p);
        }
        y += 1;
        f.render_widget(Paragraph::new(Span::styled("Content:", base)), line_at(y));
        y += 1;
        if let Some(p) = draw_input_field(
            f, line_at(y), &self.content, self.content_cursor, self.focus == 2, false, theme,
        ) {
            caret = Some(p);
        }
        y += 2;

        // Checkboxes in two columns.
        let half = inner.width / 2;
        f.render_widget(
            Paragraph::new(Line::from(check_span("Find recursively", self.recursive, self.focus == 3, theme))).style(base),
            Rect { x: inner.x, y, width: half, height: 1 },
        );
        f.render_widget(
            Paragraph::new(Line::from(check_span("Case sensitive", self.case_sensitive, self.focus == 4, theme))).style(base),
            Rect { x: inner.x + half, y, width: inner.width - half, height: 1 },
        );
        f.render_widget(
            Paragraph::new(Line::from(check_span("Skip hidden", self.skip_hidden, self.focus == 5, theme))).style(base),
            Rect { x: inner.x, y: y + 1, width: half, height: 1 },
        );
        f.render_widget(
            Paragraph::new(Line::from(check_span("Using shell patterns", self.shell, self.focus == 6, theme))).style(base),
            Rect { x: inner.x + half, y: y + 1, width: inner.width - half, height: 1 },
        );

        let by = inner.y + inner.height - 1;
        f.render_widget(
            Paragraph::new(ok_cancel_line(true, theme))
                .alignment(ratatui::layout::Alignment::Center)
                .style(base),
            line_at(by),
        );

        if let Some(p) = caret {
            f.set_cursor_position(p);
        }
    }
}

/// Convert a shell wildcard to an (unanchored) regular expression.
fn wildcard_to_regex(pattern: &str) -> String {
    let mut out = String::new();
    for ch in pattern.chars() {
        match ch {
            '*' => out.push_str(".*"),
            '?' => out.push('.'),
            c if ".+()|[]{}^$\\".contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
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
        .border_style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.dialog_title)
                .bg(theme.dialog_bg)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(ratatui::layout::Alignment::Center)
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

// --- Reusable styled widgets matching the mc dialog look -------------------

/// Draw a turquoise input field with a trailing `[^]` history button. Returns
/// the caret screen position when `focused`.
pub(crate) fn draw_input_field(
    f: &mut Frame,
    area: Rect,
    value: &str,
    cursor: usize,
    focused: bool,
    masked: bool,
    theme: &Theme,
) -> Option<Position> {
    let total = area.width as usize;
    if total < 4 {
        return None;
    }
    let inner_w = total - 3; // leave room for "[^]"
    let field_style = Style::default().fg(theme.input_fg).bg(theme.input_bg);

    // Horizontal scroll so the caret stays visible.
    let char_count = value.chars().count();
    let start = cursor.saturating_sub(inner_w.saturating_sub(1));
    let shown: String = if masked {
        "*".repeat(char_count)
    } else {
        value.chars().collect()
    };
    let shown: String = shown.chars().skip(start).take(inner_w).collect();
    let mut padded = shown.clone();
    while padded.chars().count() < inner_w {
        padded.push(' ');
    }
    let line = Line::from(vec![
        Span::styled(padded, field_style),
        Span::styled(
            "[^]",
            Style::default().fg(theme.dialog_title).bg(theme.input_bg),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);

    if focused {
        let cx = area.x + (cursor - start).min(inner_w.saturating_sub(1)) as u16;
        Some(Position::new(cx, area.y))
    } else {
        None
    }
}

/// A `(*) Label` / `( ) Label` radio span.
pub(crate) fn radio_span(label: &str, selected: bool, focused: bool, theme: &Theme) -> Span<'static> {
    let mark = if selected { "(*) " } else { "( ) " };
    let style = if focused {
        Style::default()
            .fg(theme.dialog_fg)
            .bg(ratatui::style::Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
    };
    Span::styled(format!("{mark}{label}"), style)
}

/// A `[x] Label` / `[ ] Label` checkbox span.
pub(crate) fn check_span(label: &str, checked: bool, focused: bool, theme: &Theme) -> Span<'static> {
    let mark = if checked { "[x] " } else { "[ ] " };
    let style = if focused {
        Style::default()
            .fg(theme.dialog_fg)
            .bg(ratatui::style::Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
    };
    Span::styled(format!("{mark}{label}"), style)
}

/// The `[< OK >]   [ Cancel ]` button row.
pub(crate) fn ok_cancel_line(focus_ok: bool, theme: &Theme) -> Line<'static> {
    let ok = if focus_ok {
        Span::styled("[< OK >]", theme.button_focused)
    } else {
        Span::styled("[  OK  ]", theme.button)
    };
    let cancel = if focus_ok {
        Span::styled("[ Cancel ]", theme.button)
    } else {
        Span::styled("[< Cancel >]", theme.button_focused)
    };
    Line::from(vec![ok, Span::styled("   ", Style::default().bg(theme.dialog_bg)), cancel])
}


