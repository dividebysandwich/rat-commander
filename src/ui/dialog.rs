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

/// The active modal dialog (only one at a time in Phase 1).
pub enum Dialog {
    Input(InputDialog),
    Confirm(ConfirmDialog),
    Progress(ProgressDialog),
    Message(MessageDialog),
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
}

impl Dialog {
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match self {
            Dialog::Input(d) => d.handle_key(key),
            Dialog::Confirm(d) => d.handle_key(key),
            Dialog::Progress(d) => d.handle_key(key),
            Dialog::Message(_) => DialogResult::Cancel, // any key closes
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        match self {
            Dialog::Input(d) => d.render(f, area, theme),
            Dialog::Confirm(d) => d.render(f, area, theme),
            Dialog::Progress(d) => d.render(f, area, theme),
            Dialog::Message(d) => d.render(f, area, theme),
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
            focus_yes: false,
            submit: Some(Submit::Delete(targets)),
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
