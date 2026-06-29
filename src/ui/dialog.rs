//! Modal dialogs: text input, confirmation, progress, and messages.
//!
//! Phase 1 keeps these in one module as small state machines. Each dialog
//! consumes key events and reports a [`DialogResult`]; the app acts on
//! `Submit`/`Abort` outcomes.

use crate::ops::progress::{
    ConflictInfo, OverwriteDecision, OverwriteRule, ProgressUpdate, TaskId,
};
use crate::ui::theme::Theme;
use crate::util::bytes::{format_time, human_size};
use crate::usermenu::UserMenuEntry;
use crate::vfs::VfsPath;
use crate::vfs::remote::{Protocol, RemoteCreds};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Paragraph, Wrap,
};

/// The active modal dialog (only one at a time).
#[allow(clippy::large_enum_variant)]
pub enum Dialog {
    Input(InputDialog),
    Confirm(ConfirmDialog),
    Progress(ProgressDialog),
    /// A non-dismissible "working…" spinner shown while a blocking background
    /// operation (e.g. formatting a disk) runs.
    Busy(BusyDialog),
    Message(MessageDialog),
    Form(FormDialog),
    Select(SelectDialog),
    SearchReplace(SearchReplaceDialog),
    Find(FindDialog),
    UserMenu(UserMenuDialog),
    Overwrite(OverwriteDialog),
    Compare(CompareDialog),
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
    /// The user answered an overwrite prompt for the given task.
    Overwrite(TaskId, OverwriteDecision),
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
    /// Confirmed F2 save in the editor (no quit).
    EditorSave,
    /// Confirmed F2 save in the file-comparison view.
    DiffSave,
    /// Close the file-comparison view, saving changes first.
    DiffSaveQuit,
    /// Close the file-comparison view, discarding changes.
    DiffDiscardQuit,
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
    /// Confirmation toggles from the Confirmations dialog.
    Confirmations(ConfirmValues),
    /// Compress these (local) sources into an archive of the given name.
    Compress(Vec<VfsPath>, String),
    /// Open a remote connection on the given panel side.
    Connect(usize, RemoteCreds),
    /// Run a user-menu (F2) command template (macros expanded by the app).
    UserCommand(String),
    /// Kill a process from the process explorer (`force` ⇒ SIGKILL).
    KillProcess { pid: i32, force: bool },
    /// Compare the two panels' directories and mark the differing files.
    CompareDirs(CompareMode),
    /// Open/execute a local file with its default application (confirmed).
    OpenWith(std::path::PathBuf),
    /// Mount `device` at `path` (disk manager); the app handles create-if-missing.
    Mount { device: String, path: String },
    /// Create the (missing) mount point and then mount.
    MountCreate { device: String, path: String },
    /// A sudo password entered for a queued privileged command.
    SudoPassword(String),
    /// Prompt for a path and mount this device node.
    MountDevice(String),
    /// Open the formatter for this device node.
    FormatDevice(String),
    /// Unmount this mount point (the app confirms first if enabled).
    AskUnmount(String),
    /// Unmount this mount point now (confirmed).
    DoUnmount(String),
    /// Flush filesystem buffers for this mount point.
    SyncPath(String),
    /// A format request collected from the formatter dialog (confirm first).
    Format(crate::mount::FormatSpec),
    /// Run the (confirmed) format request.
    DoFormat(crate::mount::FormatSpec),
}

/// How the directory-comparison tool decides which files differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareMode {
    /// By name only: mark files present in one panel but not the other.
    Quick,
    /// By size: also mark the larger file when both sizes differ.
    Size,
    /// By content: mark both files when their bytes differ.
    Content,
}

/// Values collected by the settings form.
#[derive(Debug, Clone)]
pub struct SettingsValues {
    pub editor: String,
    pub viewer: String,
    pub use_internal_viewer: bool,
    pub use_internal_editor: bool,
    pub theme: String,
    pub truecolor: bool,
    pub animation: bool,
    pub system_status: bool,
}

/// Values collected by the Confirmations form (which actions need confirming).
#[derive(Debug, Clone, Copy)]
pub struct ConfirmValues {
    pub delete: bool,
    pub overwrite: bool,
    pub execute: bool,
    pub unmount: bool,
    pub exit: bool,
}

impl Dialog {
    pub fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match self {
            Dialog::Input(d) => d.handle_key(key),
            Dialog::Confirm(d) => d.handle_key(key),
            Dialog::Progress(d) => d.handle_key(key),
            Dialog::Busy(_) => DialogResult::None, // ignore keys while working
            Dialog::Message(_) => DialogResult::Cancel, // any key closes
            Dialog::Form(d) => d.handle_key(key),
            Dialog::Select(d) => d.handle_key(key),
            Dialog::SearchReplace(d) => d.handle_key(key),
            Dialog::Find(d) => d.handle_key(key),
            Dialog::UserMenu(d) => d.handle_key(key),
            Dialog::Overwrite(d) => d.handle_key(key),
            Dialog::Compare(d) => d.handle_key(key),
        }
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        match self {
            Dialog::Input(d) => d.render(f, area, theme),
            Dialog::Confirm(d) => d.render(f, area, theme),
            Dialog::Progress(d) => d.render(f, area, theme),
            Dialog::Busy(d) => d.render(f, area, theme),
            Dialog::Message(d) => d.render(f, area, theme),
            Dialog::Form(d) => d.render(f, area, theme),
            Dialog::Select(d) => d.render(f, area, theme),
            Dialog::SearchReplace(d) => d.render(f, area, theme),
            Dialog::Find(d) => d.render(f, area, theme),
            Dialog::UserMenu(d) => d.render(f, area, theme),
            Dialog::Overwrite(d) => d.render(f, area, theme),
            Dialog::Compare(d) => d.render(f, area, theme),
        }
    }

    /// Route a left-click to the active dialog. Confirmation dialogs map the
    /// last button row's left half to OK/Yes and right half to Cancel/No; the
    /// overwrite dialog hit-tests its individual buttons.
    pub fn handle_click(&mut self, area: Rect, col: u16, row: u16) -> DialogResult {
        match self {
            // Precise per-button hit-testing.
            Dialog::Overwrite(d) => return d.handle_click(col, row),
            Dialog::Compare(d) => return d.handle_click(col, row),
            Dialog::Confirm(d) => {
                let rect = d.box_rect(area);
                return d.handle_click(rect, col, row);
            }
            // Any click dismisses a message box.
            Dialog::Message(_) => return DialogResult::Cancel,
            // The progress dialog is keyboard-aborted (Esc); ignore clicks so a
            // stray click can't cancel a running operation.
            Dialog::Progress(_) => return DialogResult::None,
            // The busy spinner can't be dismissed at all.
            Dialog::Busy(_) => return DialogResult::None,
            // The connect form's history chevron/dropdown take clicks first.
            Dialog::Form(d) => {
                if let Some(res) = d.click_dropdown(col, row) {
                    return res;
                }
            }
            _ => {}
        }

        let Some(rect) = self.click_bounds(area) else {
            return DialogResult::None;
        };
        // Ignore clicks outside the dialog box.
        if col < rect.x || col >= rect.x + rect.width || row < rect.y || row >= rect.y + rect.height {
            return DialogResult::None;
        }
        // The action buttons sit on the dialog's last interior row.
        let last = rect.y + rect.height.saturating_sub(2);
        if row != last {
            return DialogResult::None;
        }
        let mid = rect.x + rect.width / 2;
        let primary = col < mid;
        // OK == Enter, Cancel == Esc for the input/form/search/find dialogs.
        let code = if primary { KeyCode::Enter } else { KeyCode::Esc };
        self.handle_key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// The centered bounding box of dialogs whose buttons live on the last row.
    /// `None` for dialogs handled specially or that ignore clicks.
    fn click_bounds(&self, area: Rect) -> Option<Rect> {
        let aw = area.width;
        let r = match self {
            Dialog::Input(_) => centered(area, 60u16.min(aw.saturating_sub(4)), 7),
            Dialog::Form(d) => {
                centered(area, 60u16.min(aw.saturating_sub(4)), d.form.field_count() as u16 + 4)
            }
            Dialog::SearchReplace(d) => {
                centered(area, 64u16.min(aw.saturating_sub(2)), if d.replace { 14 } else { 12 })
            }
            Dialog::Find(_) => centered(area, 66u16.min(aw.saturating_sub(2)), 13),
            _ => return None,
        };
        Some(r)
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
    /// Enter a mount point for `device` (disk mounter).
    MountPath(String),
    /// Enter a sudo password to run a queued privileged command.
    SudoPassword,
}

pub struct InputDialog {
    pub title: String,
    pub prompt: String,
    pub buffer: String,
    /// Caret position as a char index.
    pub cursor: usize,
    pub purpose: InputPurpose,
    /// Render the buffer masked (password entry).
    pub masked: bool,
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
            masked: false,
        }
    }

    /// A masked single-field prompt (for a password).
    pub fn password(title: impl Into<String>, prompt: impl Into<String>, purpose: InputPurpose) -> Self {
        InputDialog {
            title: title.into(),
            prompt: prompt.into(),
            buffer: String::new(),
            cursor: 0,
            purpose,
            masked: true,
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
                // A password may legitimately contain anything (and be empty); the
                // path/name fields are trimmed and must be non-empty.
                if let InputPurpose::SudoPassword = self.purpose {
                    return DialogResult::Submit(Submit::SudoPassword(self.buffer.clone()));
                }
                let text = self.buffer.trim().to_string();
                if text.is_empty() {
                    return DialogResult::Cancel;
                }
                let submit = match &self.purpose {
                    InputPurpose::MkDir => Submit::MkDir(text),
                    InputPurpose::CopyDest(s) => Submit::Copy(s.clone(), text),
                    InputPurpose::MoveDest(s) => Submit::Move(s.clone(), text),
                    InputPurpose::Compress(s) => Submit::Compress(s.clone(), text),
                    InputPurpose::MountPath(device) => Submit::Mount {
                        device: device.clone(),
                        path: text,
                    },
                    InputPurpose::SudoPassword => unreachable!(),
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
        draw_shadow(f, rect, theme);
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
        if let Some(pos) =
            draw_input_field(f, field, &self.buffer, self.cursor, true, self.masked, theme)
        {
            f.set_cursor_position(pos);
        }

        let by = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
        f.render_widget(
            Paragraph::new(ok_cancel_line(true, theme))
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(theme.dialog_bg)),
            by,
        );
    }
}

// ---------------------------------------------------------------------------
// Confirm dialog
// ---------------------------------------------------------------------------

/// One button in a [`ConfirmDialog`]. A `None` action simply cancels.
struct ConfirmButton {
    label: String,
    action: Option<Submit>,
}

pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    buttons: Vec<ConfirmButton>,
    /// Index of the currently focused button.
    focus: usize,
    /// When true, the dialog is drawn in red to flag a dangerous operation.
    danger: bool,
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
            buttons: vec![
                ConfirmButton { label: yes_label.to_string(), action: Some(submit) },
                ConfirmButton { label: no_label.to_string(), action: no_submit },
            ],
            focus: 0,
            danger: false,
        }
    }

    /// A three-button save / discard / cancel modal. Cancel resumes editing.
    fn save_discard_cancel(title: &str, message: String, save: Submit, discard: Submit) -> Self {
        ConfirmDialog {
            title: title.to_string(),
            message,
            buttons: vec![
                ConfirmButton { label: "Save".to_string(), action: Some(save) },
                ConfirmButton { label: "Discard".to_string(), action: Some(discard) },
                ConfirmButton { label: "Cancel".to_string(), action: None },
            ],
            focus: 0,
            danger: false,
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

    /// A choice dialog with arbitrary buttons (a `None` action cancels).
    fn from_buttons(title: &str, message: String, buttons: Vec<(&str, Option<Submit>)>) -> Self {
        ConfirmDialog {
            title: title.to_string(),
            message,
            buttons: buttons
                .into_iter()
                .map(|(label, action)| ConfirmButton { label: label.to_string(), action })
                .collect(),
            focus: 0,
            danger: false,
        }
    }

    /// Action menu for a block device: Mount/Format when free, Unmount when
    /// mounted.
    pub fn device_menu(name: &str, dev: &str, mountpoint: Option<&str>) -> Self {
        match mountpoint {
            Some(mp) => Self::from_buttons(
                "Device",
                format!("{name}  ({dev})  mounted at {mp}"),
                vec![
                    ("Unmount", Some(Submit::AskUnmount(mp.to_string()))),
                    ("Cancel", None),
                ],
            ),
            None => Self::from_buttons(
                "Device",
                format!("{name}  ({dev})"),
                vec![
                    ("Mount", Some(Submit::MountDevice(dev.to_string()))),
                    ("Format", Some(Submit::FormatDevice(dev.to_string()))),
                    ("Cancel", None),
                ],
            ),
        }
    }

    /// Action menu for a mount point: Unmount / Sync.
    pub fn mount_menu(mountpoint: &str) -> Self {
        Self::from_buttons(
            "Mount",
            mountpoint.to_string(),
            vec![
                ("Unmount", Some(Submit::AskUnmount(mountpoint.to_string()))),
                ("Sync", Some(Submit::SyncPath(mountpoint.to_string()))),
                ("Cancel", None),
            ],
        )
    }

    /// Confirm unmounting a mount point.
    pub fn unmount(mountpoint: &str) -> Self {
        Self::yes_no(
            "Unmount",
            format!("Unmount \"{mountpoint}\"?"),
            Submit::DoUnmount(mountpoint.to_string()),
            "Unmount",
            "Cancel",
            None,
        )
    }

    /// A loud, red warning before unmounting an essential system mount point
    /// (`/`, `/boot`, …). Defaults the focus to Cancel so a stray Enter is safe.
    pub fn unmount_danger(mountpoint: &str) -> Self {
        let mut d = Self::from_buttons(
            "DANGER",
            format!(
                "\"{mountpoint}\" is an essential system mount point. \
                 Unmounting it may make your system unusable or unbootable. \
                 Continue anyway?"
            ),
            vec![
                ("Unmount anyway", Some(Submit::DoUnmount(mountpoint.to_string()))),
                ("Cancel", None),
            ],
        );
        d.danger = true;
        d.focus = 1; // Cancel
        d
    }

    /// Final (destructive) confirmation before formatting a device.
    pub fn format(spec: crate::mount::FormatSpec) -> Self {
        let msg = format!(
            "ERASE ALL DATA on {} and create a {} filesystem?",
            spec.dev,
            spec.fs.label()
        );
        Self::from_buttons("Format", msg, vec![("Format", Some(Submit::DoFormat(spec))), ("Cancel", None)])
    }

    /// Confirm creating a missing mount point before mounting.
    pub fn create_mountpoint(device: &str, path: &str) -> Self {
        Self::yes_no(
            "Create mount point",
            format!("\"{path}\" does not exist. Create it and mount {device} there?"),
            Submit::MountCreate { device: device.to_string(), path: path.to_string() },
            "Create",
            "Cancel",
            None,
        )
    }

    /// Confirm opening/executing a file with its default application.
    pub fn execute(name: &str, path: std::path::PathBuf) -> Self {
        Self::yes_no(
            "Open file",
            format!("Open \"{name}\" with its default application?"),
            Submit::OpenWith(path),
            "Open",
            "Cancel",
            None,
        )
    }

    /// Confirm killing a process (from the process explorer).
    pub fn kill(pid: i32, name: &str, force: bool) -> Self {
        let how = if force { "Force-kill (SIGKILL)" } else { "Kill (SIGTERM)" };
        Self::yes_no(
            "Kill process",
            format!("{how} process {pid} \"{name}\"?"),
            Submit::KillProcess { pid, force },
            "Kill",
            "Cancel",
            None,
        )
    }

    /// The editor's save/discard/cancel modal. Save & quit, Discard & quit, or
    /// Cancel/Esc to resume editing.
    pub fn editor_quit(name: &str) -> Self {
        Self::save_discard_cancel(
            "File modified",
            format!("\"{name}\" has unsaved changes. Save before closing?"),
            Submit::EditorSaveQuit,
            Submit::EditorDiscardQuit,
        )
    }

    /// Confirm an explicit F2 save in the editor. Save (default) / Cancel.
    pub fn save_editor(name: &str) -> Self {
        Self::yes_no(
            "Save file",
            format!("Save changes to \"{name}\"?"),
            Submit::EditorSave,
            "Save",
            "Cancel",
            None,
        )
    }

    /// Confirm an explicit F2 save in the file-comparison view.
    pub fn save_diff() -> Self {
        Self::yes_no(
            "Save files",
            "Save the changed file(s)?".to_string(),
            Submit::DiffSave,
            "Save",
            "Cancel",
            None,
        )
    }

    /// The diff view's save/discard/cancel modal. Save & close, Discard & close,
    /// or Cancel/Esc to resume editing.
    pub fn diff_quit() -> Self {
        Self::save_discard_cancel(
            "Files modified",
            "Save changes before closing the comparison?".to_string(),
            Submit::DiffSaveQuit,
            Submit::DiffDiscardQuit,
        )
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Left => {
                let n = self.buttons.len();
                self.focus = (self.focus + n - 1) % n;
                DialogResult::None
            }
            KeyCode::Right | KeyCode::Tab => {
                self.focus = (self.focus + 1) % self.buttons.len();
                DialogResult::None
            }
            KeyCode::Enter => self.activate(self.focus),
            KeyCode::Char(c) => {
                let c = c.to_ascii_lowercase();
                // y/n alias the first two buttons; otherwise match a button's
                // leading letter (S)ave / (D)iscard / (C)ancel / (K)ill / (N)o.
                let idx = if c == 'y' {
                    Some(0)
                } else if c == 'n' && self.buttons.len() >= 2 {
                    Some(1)
                } else {
                    self.buttons.iter().position(|b| {
                        b.label.chars().next().map(|x| x.to_ascii_lowercase()) == Some(c)
                    })
                };
                match idx {
                    Some(i) => self.activate(i),
                    None => DialogResult::None,
                }
            }
            _ => DialogResult::None,
        }
    }

    fn activate(&mut self, idx: usize) -> DialogResult {
        self.focus = idx;
        match self.buttons.get_mut(idx).and_then(|b| b.action.take()) {
            Some(s) => DialogResult::Submit(s),
            None => DialogResult::Cancel,
        }
    }

    /// Hit-test a click against the centered button row. Returns `None` for
    /// clicks that miss every button.
    fn handle_click(&mut self, rect: Rect, col: u16, row: u16) -> DialogResult {
        if row != rect.y + rect.height.saturating_sub(2) {
            return DialogResult::None;
        }
        let labels = self.button_labels();
        let total: usize =
            labels.iter().map(|l| l.chars().count()).sum::<usize>() + 3 * labels.len().saturating_sub(1);
        let inner_x = rect.x + 1;
        let inner_w = rect.width.saturating_sub(2) as usize;
        let mut x = inner_x + (inner_w.saturating_sub(total) / 2) as u16;
        for (i, l) in labels.iter().enumerate() {
            let w = l.chars().count() as u16;
            if col >= x && col < x + w {
                return self.activate(i);
            }
            x += w + 3;
        }
        DialogResult::None
    }

    fn button_labels(&self) -> Vec<String> {
        self.buttons.iter().map(|b| format!("[ {} ]", b.label)).collect()
    }

    /// The centered box geometry, matching [`Self::render`], so mouse hit-testing
    /// and drawing agree (the danger variant is a touch larger).
    fn box_rect(&self, area: Rect) -> Rect {
        let (w, h) = if self.danger { (58u16, 9u16) } else { (54u16, 7u16) };
        centered(area, w.min(area.width.saturating_sub(4)), h)
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let rect = self.box_rect(area);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = if self.danger {
            danger_block(&self.title, theme)
        } else {
            dialog_block(&self.title, theme)
        };
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);

        let msg_fg = if self.danger { theme.error_fg } else { theme.dialog_fg };
        f.render_widget(
            Paragraph::new(self.message.clone())
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(msg_fg).bg(theme.dialog_bg).add_modifier(
                    if self.danger { Modifier::BOLD } else { Modifier::empty() },
                ))
                .alignment(ratatui::layout::Alignment::Center),
            rows[0],
        );

        let mut spans = Vec::new();
        for (i, label) in self.button_labels().iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("   "));
            }
            spans.push(button(label, i == self.focus, theme));
        }
        f.render_widget(
            Paragraph::new(Line::from(spans))
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
    /// Transfer-speed samples: (bytes-done, bytes/sec) for the chart.
    samples: Vec<(f64, f64)>,
    peak_speed: f64,
    last_bytes: u64,
    last_instant: Option<std::time::Instant>,
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
            samples: Vec::new(),
            peak_speed: 0.0,
            last_bytes: 0,
            last_instant: None,
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

        // Sample transfer speed (~every 100 ms) for the chart.
        let now = std::time::Instant::now();
        match self.last_instant {
            None => {
                self.last_instant = Some(now);
                self.last_bytes = u.total_done;
            }
            Some(prev) => {
                let dt = now.duration_since(prev).as_secs_f64();
                if dt >= 0.1 {
                    let speed = u.total_done.saturating_sub(self.last_bytes) as f64 / dt;
                    self.peak_speed = self.peak_speed.max(speed);
                    self.samples.push((u.total_done as f64, speed));
                    if self.samples.len() > 1024 {
                        self.samples.remove(0);
                    }
                    self.last_instant = Some(now);
                    self.last_bytes = u.total_done;
                }
            }
        }
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
        let w = 64u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 16);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(self.verb, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // file name
                Constraint::Length(1), // file gauge
                Constraint::Length(1), // total label
                Constraint::Length(1), // total gauge
                Constraint::Length(1), // chart title
                Constraint::Min(3),    // speed chart
                Constraint::Length(1), // abort
            ])
            .split(inner);

        let name = crate::util::text::ellipsize(&self.current_name, inner.width as usize);
        f.render_widget(Paragraph::new(Line::from(name)).style(base), rows[0]);

        pulse_gauge(
            f,
            rows[1],
            Self::ratio(self.file_done, self.file_total),
            &format!("{} / {}", human_size(self.file_done), human_size(self.file_total)),
            theme.exec_fg,
            theme,
        );

        f.render_widget(
            Paragraph::new(Line::from(format!(
                "Total: {} / {}  ({}/{} files)",
                human_size(self.total_done),
                human_size(self.total_total),
                self.files_done,
                self.files_total
            )))
            .style(base),
            rows[2],
        );

        let total_ratio = Self::ratio(self.total_done, self.total_total);
        pulse_gauge(
            f,
            rows[3],
            total_ratio,
            &format!("{:.0}%", total_ratio * 100.0),
            theme.panel_border_active,
            theme,
        );

        f.render_widget(
            Paragraph::new(Line::from(format!(
                "Speed: {}/s   peak {}/s",
                human_size(self.samples.last().map(|s| s.1).unwrap_or(0.0) as u64),
                human_size(self.peak_speed as u64),
            )))
            .style(base),
            rows[4],
        );
        self.render_speed_chart(f, rows[5], theme);

        f.render_widget(
            Paragraph::new(Line::from(button("[ Abort ]", true, theme)))
                .alignment(ratatui::layout::Alignment::Center)
                .style(base),
            rows[6],
        );
    }

    /// A sparkline of transfer speed over bytes transferred. Each column is a
    /// vertical bar of partial-block glyphs, colored with a gradient that
    /// brightens towards the top of the graph.
    fn render_speed_chart(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        if self.samples.len() < 2 {
            f.render_widget(Paragraph::new(Line::from("  measuring…")).style(base), area);
            return;
        }
        let (w, h) = (area.width as usize, area.height as usize);
        if w == 0 || h == 0 {
            return;
        }
        const BLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
        let y_max = (self.peak_speed * 1.15).max(1.0);
        let levels = h * 8;

        // Pick a bar color that contrasts with the dialog background: the theme
        // accent when it stands out, otherwise a green that does (e.g. MC's light
        // dialog, where the cyan accent washes out). The gradient runs from that
        // intense color at the top down toward the background near the baseline.
        let bg = theme.dialog_bg;
        let accent = theme.panel_border_active;
        let intense = if (luma(accent) - luma(bg)).abs() >= 80.0 {
            accent
        } else if luma(bg) > 140.0 {
            ratatui::style::Color::Rgb(0x1e, 0x7a, 0x1e) // dark green on light bg
        } else {
            ratatui::style::Color::Rgb(0x4c, 0xff, 0x4c) // light green on dark bg
        };
        let top = intense;
        let bottom = mix_rgb(intense, bg, 0.7);

        // Bin the peak speed by *transferred-bytes position*, so each column owns
        // a fixed byte range. Past columns therefore never change — only the
        // current (rightmost) bars move as the transfer advances — and the graph
        // grows left→right like a progress bar instead of scrolling.
        let x_max = if self.total_total > 0 {
            self.total_total as f64
        } else {
            (self.last_bytes.max(1)) as f64
        };
        let mut bars = vec![0f64; w];
        let mut seen = vec![false; w];
        for &(bytes, speed) in &self.samples {
            let col = ((bytes / x_max) * w as f64).floor().clamp(0.0, (w - 1) as f64) as usize;
            bars[col] = bars[col].max(speed);
            seen[col] = true;
        }
        // Carry the last value across empty bins inside the transferred region so
        // the area stays contiguous up to the current progress; columns beyond it
        // remain empty.
        let done_col = ((self.total_done as f64 / x_max) * w as f64).round() as usize;
        let mut last = 0.0;
        for c in 0..w {
            if seen[c] {
                last = bars[c];
            } else if c < done_col {
                bars[c] = last;
            }
        }

        let buf = f.buffer_mut();
        for (col, &bar) in bars.iter().enumerate() {
            let filled = (((bar.max(0.0) / y_max) * levels as f64).round() as usize).min(levels);
            for row in 0..h {
                let from_bottom = h - 1 - row;
                let cell = filled.saturating_sub(from_bottom * 8).min(8);
                let t = if h <= 1 { 1.0 } else { 1.0 - row as f32 / (h - 1) as f32 };
                let style = Style::default().fg(mix_rgb(bottom, top, t)).bg(theme.dialog_bg);
                buf.set_string(
                    area.x + col as u16,
                    area.y + row as u16,
                    BLOCKS[cell].to_string(),
                    style,
                );
            }
        }
    }

    /// Render an indeterminate scanning dialog (current path + sweep + count).
    fn render_indeterminate(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let w = 64u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 8);
        draw_shadow(f, rect, theme);
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
// Busy dialog (indeterminate "working…" spinner)
// ---------------------------------------------------------------------------

/// A small, non-dismissible modal shown while a blocking background operation
/// runs (e.g. `mkfs`). It carries no buttons and swallows all input; the app
/// replaces it when the operation reports back.
pub struct BusyDialog {
    pub title: String,
    pub message: String,
    /// Spinner frame, advanced once per UI tick.
    frame: usize,
}

impl BusyDialog {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        BusyDialog { title: title.into(), message: message.into(), frame: 0 }
    }

    /// Advance the spinner animation (called from the app's tick handler).
    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let w = 56u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 6);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let spin = SPINNER[self.frame % SPINNER.len()];
        let text = format!("{spin}  {}", self.message);
        // Vertically center the (possibly wrapped) message within the inner box.
        let iw = inner.width.max(1);
        let lines = (text.chars().count() as u16).div_ceil(iw).clamp(1, inner.height);
        let y = inner.y + inner.height.saturating_sub(lines) / 2;
        let text_area = Rect { y, height: lines, ..inner };
        f.render_widget(
            Paragraph::new(text)
                .wrap(Wrap { trim: true })
                .style(base.add_modifier(Modifier::BOLD))
                .alignment(ratatui::layout::Alignment::Center),
            text_area,
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
        draw_shadow(f, rect, theme);
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
    /// A cycle-through choice (Space / ←→ to change).
    Choice {
        label: String,
        options: Vec<String>,
        idx: usize,
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

    pub fn choice(label: &str, options: Vec<String>, selected: &str) -> Self {
        let idx = options.iter().position(|o| o == selected).unwrap_or(0);
        Field::Choice {
            label: label.to_string(),
            options,
            idx,
        }
    }

    fn as_text(&self) -> &str {
        match self {
            Field::Text { value, .. } | Field::Password { value, .. } => value,
            Field::Choice { options, idx, .. } => options.get(*idx).map(|s| s.as_str()).unwrap_or(""),
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

    /// Number of fields (used to compute the dialog height for click geometry).
    pub fn field_count(&self) -> usize {
        self.fields.len()
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
            KeyCode::Char(' ') | KeyCode::Right | KeyCode::Left
                if matches!(self.fields.get(self.focus), Some(Field::Choice { .. })) =>
            {
                let back = key.code == KeyCode::Left;
                if let Some(Field::Choice { options, idx, .. }) = self.fields.get_mut(self.focus) {
                    let n = options.len().max(1);
                    *idx = if back {
                        (*idx + n - 1) % n
                    } else {
                        (*idx + 1) % n
                    };
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

/// Set a text field's value (and place the cursor at the end).
fn set_text_field(field: &mut Field, val: &str) {
    if let Field::Text { value, cursor, .. } = field {
        *value = val.to_string();
        *cursor = value.chars().count();
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
    Confirmations,
    Chmod(VfsPath),
    Chown(VfsPath),
    /// Create a symlink inside this directory.
    Symlink(VfsPath),
    /// Open a remote connection of this protocol on the given panel side.
    Connect(Protocol, usize),
    /// Format this device node (disk manager).
    Format(String),
}

/// Connect-form history dropdown state (recent servers).
struct ConnectDropdown {
    history: Vec<crate::config::RemoteHistoryEntry>,
    open: bool,
    sel: usize,
    /// Click geometry recorded at render time: chevron, plus (rect, index) per
    /// visible dropdown entry.
    chevron: Option<Rect>,
    entries: Vec<(Rect, usize)>,
}

pub struct FormDialog {
    pub title: String,
    pub form: Form,
    pub purpose: FormPurpose,
    /// Present only for connect forms (drives the recent-servers dropdown).
    connect: Option<ConnectDropdown>,
}

impl FormDialog {
    pub fn settings(cfg: &crate::config::Config, truecolor: bool) -> Self {
        let form = Form::new(vec![
            Field::choice("Theme", crate::ui::theme::palette_names(), &cfg.theme),
            Field::check("Truecolor (gradients)", truecolor),
            Field::check("Animations", cfg.animation),
            Field::check("System status widget", cfg.system_status),
            Field::text("External editor", cfg.editor.clone()),
            Field::text("External viewer", cfg.viewer.clone()),
            Field::check("Use internal viewer", cfg.use_internal_viewer),
            Field::check("Use internal editor", cfg.use_internal_editor),
        ]);
        FormDialog {
            title: "Settings".to_string(),
            form,
            purpose: FormPurpose::Settings,
            connect: None,
        }
    }

    /// Build the Confirmations form (which actions require a confirmation).
    pub fn confirmations(cfg: &crate::config::Config) -> Self {
        let form = Form::new(vec![
            Field::check("Confirm delete", cfg.confirm_delete),
            Field::check("Confirm overwrite", cfg.confirm_overwrite),
            Field::check("Confirm execute", cfg.confirm_execute),
            Field::check("Confirm unmount", cfg.confirm_unmount),
            Field::check("Confirm exit", cfg.confirm_exit),
        ]);
        FormDialog {
            title: "Confirmations".to_string(),
            form,
            purpose: FormPurpose::Confirmations,
            connect: None,
        }
    }

    /// Build the disk formatter form for `dev`.
    pub fn format(dev: String) -> Self {
        let fs_options: Vec<String> =
            crate::mount::FsType::ALL.iter().map(|f| f.label().to_string()).collect();
        let form = Form::new(vec![
            Field::choice("Filesystem", fs_options, "FAT32"),
            Field::text("Volume label", ""),
            Field::check("Quick format (NTFS)", false),
            Field::text("Bytes/inode (ext, blank=auto)", ""),
        ]);
        FormDialog {
            title: format!("Format {dev}"),
            form,
            purpose: FormPurpose::Format(dev),
            connect: None,
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
            connect: None,
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
            connect: None,
        }
    }

    pub fn symlink(dir: VfsPath, target: String, name: String) -> Self {
        let form = Form::new(vec![
            Field::text("Points to (target)", target),
            Field::text("Link name", name),
        ]);
        FormDialog {
            title: "Create symlink".to_string(),
            form,
            purpose: FormPurpose::Symlink(dir),
            connect: None,
        }
    }

    /// The currently-selected theme name in the settings form (for live
    /// preview), or `None` if this isn't the settings form.
    pub fn theme_choice(&self) -> Option<&str> {
        if !matches!(self.purpose, FormPurpose::Settings) {
            return None;
        }
        self.form.fields.iter().find_map(|f| match f {
            Field::Choice { label, options, idx } if label == "Theme" => {
                options.get(*idx).map(|s| s.as_str())
            }
            _ => None,
        })
    }

    pub fn connect(
        protocol: Protocol,
        side: usize,
        history: Vec<crate::config::RemoteHistoryEntry>,
    ) -> Self {
        let form = Form::new(vec![
            Field::text("Host", ""),
            Field::text("Port", protocol.default_port().to_string()),
            Field::text("Username", ""),
            Field::password("Password"),
            Field::text("Remote path (blank = home)", ""),
        ]);
        // Only this protocol's recent connections.
        let history: Vec<_> = history
            .into_iter()
            .filter(|e| e.protocol == protocol.scheme_prefix())
            .collect();
        FormDialog {
            title: format!("{} connection", protocol.scheme_prefix().to_uppercase()),
            form,
            purpose: FormPurpose::Connect(protocol, side),
            connect: Some(ConnectDropdown {
                history,
                open: false,
                sel: 0,
                chevron: None,
                entries: Vec::new(),
            }),
        }
    }

    /// Fill the host/port/user/path fields from history entry `idx` and move the
    /// focus to the password field.
    fn apply_history(&mut self, idx: usize) {
        let entry = match self.connect.as_ref().and_then(|c| c.history.get(idx).cloned()) {
            Some(e) => e,
            None => return,
        };
        if let Some(c) = self.connect.as_mut() {
            c.open = false;
        }
        set_text_field(&mut self.form.fields[0], &entry.host);
        set_text_field(&mut self.form.fields[1], &entry.port.to_string());
        set_text_field(&mut self.form.fields[2], &entry.user);
        if let Some(field) = self.form.fields.get_mut(4) {
            set_text_field(field, &entry.path);
        }
        self.form.focus = 3; // password
    }

    /// Route a click for the connect dropdown. Returns `Some` if the click hit
    /// the chevron or a dropdown entry (or dismissed an open dropdown).
    fn click_dropdown(&mut self, col: u16, row: u16) -> Option<DialogResult> {
        let hit = |r: &Rect| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height;
        let cd = self.connect.as_ref()?;
        if cd.chevron.is_some_and(|r| hit(&r)) {
            let cd = self.connect.as_mut().unwrap();
            cd.open = !cd.open;
            cd.sel = 0;
            return Some(DialogResult::None);
        }
        if !cd.open {
            return None;
        }
        let hidx = cd.entries.iter().find(|(r, _)| hit(r)).map(|&(_, i)| i);
        match hidx {
            Some(i) => self.apply_history(i),
            None => self.connect.as_mut().unwrap().open = false,
        }
        Some(DialogResult::None)
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
        // Connect-form history dropdown: while open it captures navigation keys;
        // closed, pressing ↓ on the Host field opens it.
        let drop_open = self.connect.as_ref().is_some_and(|c| c.open);
        if drop_open {
            match key.code {
                KeyCode::Esc => self.connect.as_mut().unwrap().open = false,
                KeyCode::Up => {
                    let c = self.connect.as_mut().unwrap();
                    c.sel = c.sel.saturating_sub(1);
                }
                KeyCode::Down => {
                    let c = self.connect.as_mut().unwrap();
                    if c.sel + 1 < c.history.len() {
                        c.sel += 1;
                    }
                }
                KeyCode::Enter => {
                    let i = self.connect.as_ref().unwrap().sel;
                    self.apply_history(i);
                }
                _ => {}
            }
            return DialogResult::None;
        }
        if matches!(key.code, KeyCode::Down)
            && self.form.focus == 0
            && self.connect.as_ref().is_some_and(|c| !c.history.is_empty())
        {
            let c = self.connect.as_mut().unwrap();
            c.open = true;
            c.sel = 0;
            return DialogResult::None;
        }

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
                theme: fields[0].as_text().to_string(),
                truecolor: fields[1].as_bool(),
                animation: fields[2].as_bool(),
                system_status: fields[3].as_bool(),
                editor: fields[4].as_text().trim().to_string(),
                viewer: fields[5].as_text().trim().to_string(),
                use_internal_viewer: fields[6].as_bool(),
                use_internal_editor: fields[7].as_bool(),
            }),
            FormPurpose::Confirmations => Submit::Confirmations(ConfirmValues {
                delete: fields[0].as_bool(),
                overwrite: fields[1].as_bool(),
                execute: fields[2].as_bool(),
                unmount: fields[3].as_bool(),
                exit: fields[4].as_bool(),
            }),
            FormPurpose::Format(dev) => {
                let fs = crate::mount::FsType::from_label(fields[0].as_text())
                    .unwrap_or(crate::mount::FsType::Fat32);
                Submit::Format(crate::mount::FormatSpec {
                    dev: dev.clone(),
                    fs,
                    label: fields[1].as_text().trim().to_string(),
                    quick: fields[2].as_bool(),
                    inode_bytes: fields[3].as_text().trim().to_string(),
                })
            }
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

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        let n = self.form.fields.len() as u16;
        let height = n + 4;
        let w = 60u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block(&self.title, theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let focus_style = theme.dialog_selection;

        // The Host field of a connect form gets a ▼ chevron to open the history.
        let connect_host = self.connect.as_ref().is_some_and(|c| !c.history.is_empty());
        let mut host_chevron: Option<Rect> = None;

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
                    let mut field_area = Rect {
                        x: row.x + lw,
                        width: row.width.saturating_sub(lw),
                        ..row
                    };
                    // Reserve room for the chevron on the Host field.
                    if i == 0 && connect_host && field_area.width > 4 {
                        let cx = field_area.x + field_area.width - 2;
                        host_chevron = Some(Rect { x: cx, y, width: 2, height: 1 });
                        field_area.width -= 2;
                    }
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
                Field::Choice { label, options, idx } => {
                    let style = if focused { focus_style } else { base };
                    let val = options.get(*idx).map(|s| s.as_str()).unwrap_or("");
                    f.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            format!("{label}: ◂ {val} ▸"),
                            style,
                        ))),
                        row,
                    );
                }
            }
        }

        // Draw the chevron and (when open) the recent-servers dropdown.
        if let Some(chev) = host_chevron {
            let style = base.add_modifier(Modifier::BOLD);
            f.buffer_mut().set_string(chev.x, chev.y, "▼", style);
        }
        let dropdown_open = self.connect.as_ref().is_some_and(|c| c.open);
        if let Some(c) = self.connect.as_mut() {
            c.chevron = host_chevron;
            c.entries.clear();
        }
        if dropdown_open {
            self.render_dropdown(f, inner, theme);
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
                "[ OK ]  Tab/↑↓ Space toggle  [ Cancel ]{extra}"
            )))
            .style(base),
            hint,
        );

        if let Some(pos) = caret
            && !dropdown_open
        {
            f.set_cursor_position(pos);
        }
    }

    /// Render the recent-servers list under the Host field and record per-entry
    /// click rects. Scrolls so the selection stays visible.
    fn render_dropdown(&mut self, f: &mut Frame, inner: Rect, theme: &Theme) {
        let Some(c) = self.connect.as_mut() else {
            return;
        };
        if c.history.is_empty() {
            return;
        }
        // The list opens just below the Host row, capped to the dialog interior.
        let top = inner.y + 1;
        let avail = (inner.y + inner.height).saturating_sub(top) as usize;
        let visible = c.history.len().min(avail.saturating_sub(2).max(1));
        let rect = Rect {
            x: inner.x,
            y: top,
            width: inner.width,
            height: (visible + 2) as u16,
        };
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.dialog_title).bg(theme.dialog_bg))
            .title(Span::styled(
                " Recent ",
                Style::default().fg(theme.dialog_title).bg(theme.dialog_bg),
            ))
            .style(Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg));
        let list = block.inner(rect);
        f.render_widget(block, rect);

        // Scroll so the selection is on screen.
        let offset = if c.sel >= visible {
            c.sel + 1 - visible
        } else {
            0
        };
        let normal = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let sel_style = theme.dialog_selection;
        for vi in 0..visible {
            let idx = offset + vi;
            let Some(entry) = c.history.get(idx) else {
                break;
            };
            let row = Rect {
                x: list.x,
                y: list.y + vi as u16,
                width: list.width,
                height: 1,
            };
            let style = if idx == c.sel { sel_style } else { normal };
            let text = crate::util::text::ellipsize(&entry.label(), list.width as usize);
            let text = crate::util::text::pad_right(&text, list.width as usize);
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(text, style))),
                row,
            );
            c.entries.push((row, idx));
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
        draw_shadow(f, rect, theme);
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
    /// Hex mode was selected: search/replacement are hex byte strings.
    pub hex: bool,
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

    /// Like `new`, but starting in Hex mode (for the editor's hex search).
    pub fn new_hex(replace: bool, initial: String) -> Self {
        let mut d = Self::new(replace, initial);
        d.mode = 2;
        d
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
            hex: self.mode == 2,
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let title = if self.replace { "Replace" } else { "Search" };
        let height = if self.replace { 14 } else { 12 };
        let rect = centered(area, 64u16.min(area.width.saturating_sub(2)), height);
        draw_shadow(f, rect, theme);
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
        draw_shadow(f, rect, theme);
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
// User menu (F2)
// ---------------------------------------------------------------------------

pub struct UserMenuDialog {
    entries: Vec<UserMenuEntry>,
    cursor: usize,
}

impl UserMenuDialog {
    pub fn new(entries: Vec<UserMenuEntry>) -> Self {
        UserMenuDialog { entries, cursor: 0 }
    }

    fn submit_current(&self) -> DialogResult {
        match self.entries.get(self.cursor) {
            Some(e) => DialogResult::Submit(Submit::UserCommand(e.command.clone())),
            None => DialogResult::Cancel,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let max = self.entries.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::F(2) | KeyCode::F(10) => DialogResult::Cancel,
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogResult::None
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(max);
                DialogResult::None
            }
            KeyCode::Home => {
                self.cursor = 0;
                DialogResult::None
            }
            KeyCode::End => {
                self.cursor = max;
                DialogResult::None
            }
            KeyCode::Enter => self.submit_current(),
            KeyCode::Char(c) => {
                // Activate the entry whose hotkey matches (exact, then loose).
                if let Some(i) = self
                    .entries
                    .iter()
                    .position(|e| e.hotkey == c)
                    .or_else(|| {
                        self.entries
                            .iter()
                            .position(|e| e.hotkey.eq_ignore_ascii_case(&c))
                    })
                {
                    self.cursor = i;
                    return self.submit_current();
                }
                DialogResult::None
            }
            _ => DialogResult::None,
        }
    }

    fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
        let width = 64u16.min(area.width.saturating_sub(2));
        let max_h = area.height.saturating_sub(2);
        let height = (self.entries.len() as u16 + 2).min(max_h.max(3));
        let rect = centered(area, width, height);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("User menu", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let rows = inner.height as usize;
        // Window the list so the cursor stays visible.
        let first = if self.cursor < rows {
            0
        } else {
            self.cursor + 1 - rows
        };

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let hotkey_style = Style::default()
            .fg(theme.dialog_title)
            .bg(theme.dialog_bg)
            .add_modifier(Modifier::BOLD);

        let mut lines: Vec<Line> = Vec::with_capacity(rows);
        for (idx, e) in self.entries.iter().enumerate().skip(first).take(rows) {
            let title = crate::util::text::ellipsize(&e.title, inner.width.saturating_sub(6) as usize);
            if idx == self.cursor {
                let text = format!(" {}  {}", e.hotkey, title);
                let mut padded = text;
                while (padded.chars().count() as u16) < inner.width {
                    padded.push(' ');
                }
                lines.push(Line::from(Span::styled(padded, theme.button_focused)));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {} ", e.hotkey), hotkey_style),
                    Span::styled(format!(" {title}"), base),
                ]));
            }
        }
        f.render_widget(
            Paragraph::new(lines).style(Style::default().bg(theme.dialog_bg)),
            inner,
        );
    }
}

// ---------------------------------------------------------------------------
// Compare-directories dialog
// ---------------------------------------------------------------------------

const COMPARE_MODES: [(&str, CompareMode); 3] = [
    ("Quick (name)", CompareMode::Quick),
    ("Size only", CompareMode::Size),
    ("Content", CompareMode::Content),
];

/// Asks how to compare the two panels' directories.
pub struct CompareDialog {
    focus: usize,
    zones: Vec<(Rect, usize)>,
}

impl CompareDialog {
    pub fn new() -> Self {
        CompareDialog { focus: 0, zones: Vec::new() }
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        match key.code {
            KeyCode::Esc => DialogResult::Cancel,
            KeyCode::Left | KeyCode::BackTab => {
                self.focus = (self.focus + COMPARE_MODES.len() - 1) % COMPARE_MODES.len();
                DialogResult::None
            }
            KeyCode::Right | KeyCode::Tab => {
                self.focus = (self.focus + 1) % COMPARE_MODES.len();
                DialogResult::None
            }
            KeyCode::Enter => DialogResult::Submit(Submit::CompareDirs(COMPARE_MODES[self.focus].1)),
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                DialogResult::Submit(Submit::CompareDirs(CompareMode::Quick))
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                DialogResult::Submit(Submit::CompareDirs(CompareMode::Size))
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                DialogResult::Submit(Submit::CompareDirs(CompareMode::Content))
            }
            _ => DialogResult::None,
        }
    }

    fn handle_click(&mut self, col: u16, row: u16) -> DialogResult {
        if let Some(&(_, i)) = self.zones.iter().find(|(r, _)| {
            col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
        }) {
            return DialogResult::Submit(Submit::CompareDirs(COMPARE_MODES[i].1));
        }
        DialogResult::None
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.zones.clear();
        let w = 52u16.min(area.width.saturating_sub(4));
        let rect = centered(area, w, 7);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = dialog_block("Compare directories", theme);
        let inner = block.inner(rect);
        f.render_widget(block, rect);

        let base = Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        f.render_widget(
            Paragraph::new("Compare the two panels by:")
                .alignment(ratatui::layout::Alignment::Center)
                .style(base),
            rows[0],
        );

        // Centered row of bracketed buttons; record click zones.
        let labels: Vec<String> = COMPARE_MODES.iter().map(|(l, _)| format!("[ {l} ]")).collect();
        let total: usize =
            labels.iter().map(|l| l.chars().count()).sum::<usize>() + labels.len().saturating_sub(1);
        let mut x = rows[1].x + (rows[1].width.saturating_sub(total as u16)) / 2;
        for (i, label) in labels.iter().enumerate() {
            let style = if i == self.focus { theme.button_focused } else { theme.button };
            f.render_widget(
                Paragraph::new(Span::styled(label.clone(), style)),
                Rect { x, y: rows[1].y, width: label.chars().count() as u16, height: 1 },
            );
            self.zones.push((
                Rect { x, y: rows[1].y, width: label.chars().count() as u16, height: 1 },
                i,
            ));
            x += label.chars().count() as u16 + 1;
        }
    }
}

impl Default for CompareDialog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Draw a drop shadow for a dialog box: a dim band one cell below and to the
/// right of `rect`. Out-of-screen cells are clipped by the renderer.
fn draw_shadow(f: &mut Frame, rect: Rect, _theme: &Theme) {
    let shadow = Style::default().bg(ratatui::style::Color::Rgb(8, 8, 12));
    // Bottom edge (offset right by 1 so it sits under the box).
    let bottom = Rect {
        x: rect.x + 1,
        y: rect.y + rect.height,
        width: rect.width,
        height: 1,
    };
    // Right edge (offset down by 1).
    let right = Rect {
        x: rect.x + rect.width,
        y: rect.y + 1,
        width: 1,
        height: rect.height,
    };
    f.render_widget(Block::default().style(shadow), bottom);
    f.render_widget(Block::default().style(shadow), right);
}

/// A progress bar whose filled portion shows a gradient "pulse" sweeping left to
/// right (truecolor only; otherwise a solid fill). `label` is centered over it.
fn pulse_gauge(f: &mut Frame, area: Rect, ratio: f64, label: &str, base: ratatui::style::Color, theme: &Theme) {
    let w = area.width as usize;
    if w == 0 || area.height == 0 {
        return;
    }
    let filled = (ratio.clamp(0.0, 1.0) * w as f64).round() as usize;

    // Center the label over the bar.
    let label: Vec<char> = label.chars().take(w).collect();
    let lstart = (w - label.len()) / 2;

    let empty_fg = theme.panel_border;
    let buf = f.buffer_mut();
    for x in 0..w {
        let in_label = x >= lstart && x < lstart + label.len();
        let lc = if in_label { Some(label[x - lstart]) } else { None };
        if x < filled {
            let color = pulse_fill(theme, base, x, w);
            let (ch, fg, bg) = match lc {
                Some(c) => (c, theme.dialog_bg, color),
                None => ('█', color, theme.dialog_bg),
            };
            buf.set_string(area.x + x as u16, area.y, ch.to_string(), Style::default().fg(fg).bg(bg));
        } else {
            let (ch, fg) = match lc {
                Some(c) => (c, theme.dialog_fg),
                None => ('░', empty_fg),
            };
            buf.set_string(
                area.x + x as u16,
                area.y,
                ch.to_string(),
                Style::default().fg(fg).bg(theme.dialog_bg),
            );
        }
    }
}

/// Linearly blend two RGB colors: `t`=0 → `a`, `t`=1 → `b`. Non-RGB inputs
/// fall back to `b`.
fn mix_rgb(a: ratatui::style::Color, b: ratatui::style::Color, t: f32) -> ratatui::style::Color {
    use ratatui::style::Color;
    match (a, b) {
        (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) => {
            let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
            Color::Rgb(f(ar, br), f(ag, bg), f(ab, bb))
        }
        _ => b,
    }
}

/// Color of filled cell `x` (of `w`) in an animated pulse bar over `base`: a
/// bright band sweeps left→right as `theme.anim` advances (truecolor only;
/// otherwise the solid `base`). Shared by the copy gauges and the disk scan bar
/// so they pulse identically.
pub(crate) fn pulse_fill(
    theme: &Theme,
    base: ratatui::style::Color,
    x: usize,
    w: usize,
) -> ratatui::style::Color {
    if !theme.truecolor {
        return base;
    }
    let band = (w as f64 * 0.33).max(5.0);
    let period = w as f64 + band;
    let pos = (theme.anim as f64 * 3.2) % period;
    let t = (1.0 - (x as f64 - pos).abs() / (band * 0.5)).clamp(0.0, 1.0);
    pulse_color(base, t)
}

/// Perceived brightness (Rec. 601 luma, 0..255) of an RGB color; 128 for
/// non-RGB colors.
fn luma(c: ratatui::style::Color) -> f32 {
    if let ratatui::style::Color::Rgb(r, g, b) = c {
        0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
    } else {
        128.0
    }
}

/// Brighten `base` toward a white-hot highlight by pulse intensity `t` (0..1).
fn pulse_color(base: ratatui::style::Color, t: f64) -> ratatui::style::Color {
    if let ratatui::style::Color::Rgb(r, g, b) = base {
        let bright = 0.5 + 0.5 * t; // 0.5×..1.0× brightness
        let hl = t * t * 110.0; // white highlight near the pulse center
        let mix = |c: u8| ((c as f64 * bright) + hl).min(255.0) as u8;
        ratatui::style::Color::Rgb(mix(r), mix(g), mix(b))
    } else {
        base
    }
}

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
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.dialog_title).bg(theme.dialog_bg))
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

/// Like [`dialog_block`] but drawn in a loud red to flag a dangerous action.
fn danger_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(theme.error_fg).bg(theme.dialog_bg))
        .title(Span::styled(
            format!(" ⚠ {title} ⚠ "),
            Style::default()
                .fg(theme.error_fg)
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
        theme.dialog_selection
    } else {
        Style::default().fg(theme.dialog_fg).bg(theme.dialog_bg)
    };
    Span::styled(format!("{mark}{label}"), style)
}

/// A `[x] Label` / `[ ] Label` checkbox span.
pub(crate) fn check_span(label: &str, checked: bool, focused: bool, theme: &Theme) -> Span<'static> {
    let mark = if checked { "[x] " } else { "[ ] " };
    let style = if focused {
        theme.dialog_selection
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

// ---------------------------------------------------------------------------
// Overwrite-confirmation dialog (shown mid-copy when a destination exists)
// ---------------------------------------------------------------------------

/// The interactive controls of the overwrite dialog, in focus order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwControl {
    Yes,
    No,
    Append,
    SkipEmpty,
    All,
    Older,
    NoneRule,
    Smaller,
    SizeDiffers,
    Abort,
}

const OW_ORDER: [OwControl; 10] = [
    OwControl::Yes,
    OwControl::No,
    OwControl::Append,
    OwControl::SkipEmpty,
    OwControl::All,
    OwControl::Older,
    OwControl::NoneRule,
    OwControl::Smaller,
    OwControl::SizeDiffers,
    OwControl::Abort,
];

/// A red "File exists" prompt offering per-file (Yes/No/Append) and global
/// (All/Older/None/Smaller/Size differs) overwrite choices, plus Abort.
pub struct OverwriteDialog {
    info: ConflictInfo,
    focus: usize,
    skip_empty: bool,
    /// Clickable control regions, recorded during render.
    zones: Vec<(Rect, OwControl)>,
}

impl OverwriteDialog {
    pub fn new(info: ConflictInfo) -> Self {
        OverwriteDialog {
            info,
            focus: 0,
            skip_empty: false,
            zones: Vec::new(),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
        let len = OW_ORDER.len();
        match key.code {
            KeyCode::Esc => self.activate(OwControl::Abort),
            KeyCode::Enter => self.activate(OW_ORDER[self.focus]),
            KeyCode::Char(' ') => {
                if OW_ORDER[self.focus] == OwControl::SkipEmpty {
                    self.skip_empty = !self.skip_empty;
                }
                DialogResult::None
            }
            KeyCode::Left | KeyCode::Up | KeyCode::BackTab => {
                self.focus = (self.focus + len - 1) % len;
                DialogResult::None
            }
            KeyCode::Right | KeyCode::Down | KeyCode::Tab => {
                self.focus = (self.focus + 1) % len;
                DialogResult::None
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => self.activate(OwControl::Yes),
            KeyCode::Char('n') | KeyCode::Char('N') => self.activate(OwControl::No),
            KeyCode::Char('p') | KeyCode::Char('P') => self.activate(OwControl::Append),
            _ => DialogResult::None,
        }
    }

    /// Hit-test a mouse click against the recorded control zones.
    pub fn handle_click(&mut self, col: u16, row: u16) -> DialogResult {
        if let Some(&(_, ctrl)) = self
            .zones
            .iter()
            .find(|(r, _)| col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height)
        {
            // Move focus to the clicked control, then activate it.
            if let Some(i) = OW_ORDER.iter().position(|c| *c == ctrl) {
                self.focus = i;
            }
            return self.activate(ctrl);
        }
        DialogResult::None
    }

    fn activate(&mut self, ctrl: OwControl) -> DialogResult {
        let id = self.info.id;
        let decision = |d: OverwriteDecision| DialogResult::Overwrite(id, d);
        let policy = |rule: OverwriteRule, skip_empty: bool| {
            DialogResult::Overwrite(id, OverwriteDecision::Policy { rule, skip_empty })
        };
        match ctrl {
            OwControl::Yes => decision(OverwriteDecision::OverwriteOnce),
            OwControl::No => decision(OverwriteDecision::SkipOnce),
            OwControl::Append => decision(OverwriteDecision::AppendOnce),
            OwControl::SkipEmpty => {
                self.skip_empty = !self.skip_empty;
                DialogResult::None
            }
            OwControl::All => policy(OverwriteRule::All, self.skip_empty),
            OwControl::Older => policy(OverwriteRule::Older, self.skip_empty),
            OwControl::NoneRule => policy(OverwriteRule::None, self.skip_empty),
            OwControl::Smaller => policy(OverwriteRule::Smaller, self.skip_empty),
            OwControl::SizeDiffers => policy(OverwriteRule::SizeDiffers, self.skip_empty),
            OwControl::Abort => decision(OverwriteDecision::Abort),
        }
    }

    fn render(&mut self, f: &mut Frame, area: Rect, theme: &Theme) {
        self.zones.clear();

        // A red (warning) box: white text on the theme's error color.
        let bg = theme.error_fg;
        let fg = ratatui::style::Color::White;
        let base = Style::default().fg(fg).bg(bg);

        let w = 60u16.min(area.width.saturating_sub(2));
        let h = 15u16.min(area.height.saturating_sub(2));
        let rect = centered(area, w, h);
        draw_shadow(f, rect, theme);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(base.add_modifier(Modifier::BOLD))
            .title(Span::styled(
                " File exists ",
                base.add_modifier(Modifier::BOLD),
            ))
            .title_alignment(ratatui::layout::Alignment::Center)
            .style(base);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        if inner.width < 10 || inner.height < 10 {
            return;
        }

        let mut y = inner.y;
        let name_w = inner.width as usize;
        ow_meta_line(f, inner, y, &format!("New     : {}", crate::util::text::ellipsize(&self.info.new_path, name_w.saturating_sub(10))), base);
        y += 1;
        ow_meta_line(f, inner, y, &ow_meta(self.info.new_size, self.info.new_mtime), base);
        y += 1;
        ow_meta_line(f, inner, y, &format!("Existing: {}", crate::util::text::ellipsize(&self.info.old_path, name_w.saturating_sub(10))), base);
        y += 1;
        ow_meta_line(f, inner, y, &ow_meta(self.info.old_size, self.info.old_mtime), base);
        y += 1;
        self.rule(f, inner, y, bg);
        y += 1;

        // Per-file row.
        ow_center(f, inner, y, "Overwrite this file?", base.add_modifier(Modifier::BOLD));
        y += 1;
        self.button_row(
            f,
            inner,
            y,
            &[
                (" Yes ", OwControl::Yes),
                (" No ", OwControl::No),
                (" Append ", OwControl::Append),
            ],
            theme,
        );
        y += 1;
        self.rule(f, inner, y, bg);
        y += 1;

        // Global row.
        ow_center(f, inner, y, "Overwrite all files?", base.add_modifier(Modifier::BOLD));
        y += 1;
        self.checkbox_row(f, inner, y, "Don't overwrite with zero length file", theme);
        y += 1;
        self.button_row(
            f,
            inner,
            y,
            &[
                (" All ", OwControl::All),
                (" Older ", OwControl::Older),
                (" None ", OwControl::NoneRule),
                (" Smaller ", OwControl::Smaller),
                (" Size differs ", OwControl::SizeDiffers),
            ],
            theme,
        );
        y += 1;
        self.rule(f, inner, y, bg);
        y += 1;

        self.button_row(f, inner, y, &[(" Abort ", OwControl::Abort)], theme);
    }

    fn rule(&self, f: &mut Frame, inner: Rect, y: u16, bg: ratatui::style::Color) {
        if y >= inner.y + inner.height {
            return;
        }
        let style = Style::default().fg(ratatui::style::Color::White).bg(bg);
        f.buffer_mut()
            .set_string(inner.x, y, "─".repeat(inner.width as usize), style);
    }

    /// Render a centered row of bracketed buttons and record their click zones.
    fn button_row(
        &mut self,
        f: &mut Frame,
        inner: Rect,
        y: u16,
        buttons: &[(&str, OwControl)],
        theme: &Theme,
    ) {
        if y >= inner.y + inner.height {
            return;
        }
        let bg = theme.error_fg;
        // Each label is wrapped as "[label]"; buttons separated by one space.
        let labels: Vec<String> = buttons.iter().map(|(l, _)| format!("[{l}]")).collect();
        let total: usize = labels.iter().map(|l| l.chars().count()).sum::<usize>() + labels.len().saturating_sub(1);
        let mut x = inner.x + (inner.width.saturating_sub(total as u16)) / 2;
        for (label, (_, ctrl)) in labels.iter().zip(buttons.iter()) {
            let focused = OW_ORDER[self.focus] == *ctrl;
            let style = if focused {
                Style::default()
                    .fg(bg)
                    .bg(ratatui::style::Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(ratatui::style::Color::White)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            };
            let wlen = label.chars().count() as u16;
            f.buffer_mut().set_string(x, y, label, style);
            self.zones.push((Rect { x, y, width: wlen, height: 1 }, *ctrl));
            x += wlen + 1;
        }
    }

    fn checkbox_row(&mut self, f: &mut Frame, inner: Rect, y: u16, label: &str, theme: &Theme) {
        if y >= inner.y + inner.height {
            return;
        }
        let bg = theme.error_fg;
        let focused = OW_ORDER[self.focus] == OwControl::SkipEmpty;
        let mark = if self.skip_empty { "[x] " } else { "[ ] " };
        let text = format!("{mark}{label}");
        let style = if focused {
            Style::default().fg(bg).bg(ratatui::style::Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(ratatui::style::Color::White).bg(bg)
        };
        let wlen = text.chars().count() as u16;
        let x = inner.x + (inner.width.saturating_sub(wlen)) / 2;
        f.buffer_mut().set_string(x, y, &text, style);
        self.zones.push((Rect { x, y, width: wlen, height: 1 }, OwControl::SkipEmpty));
    }
}

/// Format a "size + date" detail line for the overwrite dialog.
fn ow_meta(size: u64, mtime: Option<std::time::SystemTime>) -> String {
    let date = mtime.map(format_time).unwrap_or_default();
    format!("{size:>14}      {date}")
}

/// Render a left-aligned detail line within the dialog interior.
fn ow_meta_line(f: &mut Frame, inner: Rect, y: u16, text: &str, style: Style) {
    if y >= inner.y + inner.height {
        return;
    }
    let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
    f.render_widget(Paragraph::new(Span::styled(text.to_string(), style)), row);
}

/// Render a centered label line within the dialog interior.
fn ow_center(f: &mut Frame, inner: Rect, y: u16, text: &str, style: Style) {
    if y >= inner.y + inner.height {
        return;
    }
    let row = Rect { x: inner.x, y, width: inner.width, height: 1 };
    f.render_widget(
        Paragraph::new(Span::styled(text.to_string(), style))
            .alignment(ratatui::layout::Alignment::Center),
        row,
    );
}





#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteHistoryEntry;
    use crate::vfs::remote::Protocol;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn mount_path_and_password_inputs_submit() {
        // The mount-path input yields a Mount submit with the device + typed path.
        let mut d = InputDialog::new("Mount", "at:", "/mnt/x", InputPurpose::MountPath("/dev/sdb1".into()));
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::Mount { device, path }) => {
                assert_eq!(device, "/dev/sdb1");
                assert_eq!(path, "/mnt/x");
            }
            _ => panic!("expected Mount submit"),
        }
        // The password input is masked and submits the raw buffer (even empty).
        let mut d = InputDialog::password("Auth", "pw:", InputPurpose::SudoPassword);
        assert!(d.masked);
        d.handle_key(key(KeyCode::Char('s')));
        d.handle_key(key(KeyCode::Char('3')));
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::SudoPassword(pw)) => assert_eq!(pw, "s3"),
            _ => panic!("expected SudoPassword submit"),
        }
    }

    #[test]
    fn device_and_mount_action_menus() {
        // Unmounted device: the focused "Mount" button → MountDevice.
        let mut d = ConfirmDialog::device_menu("sdb1", "/dev/sdb1", None);
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::MountDevice(dev)) => assert_eq!(dev, "/dev/sdb1"),
            _ => panic!("expected MountDevice"),
        }
        // Mounted device: the only action is Unmount.
        let mut d = ConfirmDialog::device_menu("sdb1", "/dev/sdb1", Some("/mnt/x"));
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::AskUnmount(mp)) => assert_eq!(mp, "/mnt/x"),
            _ => panic!("expected AskUnmount"),
        }
        // Mount menu: second button is Sync.
        let mut d = ConfirmDialog::mount_menu("/mnt/x");
        d.handle_key(key(KeyCode::Right)); // focus "Sync"
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::SyncPath(mp)) => assert_eq!(mp, "/mnt/x"),
            _ => panic!("expected SyncPath"),
        }
    }

    #[test]
    fn unmount_danger_defaults_to_cancel_and_confirms_explicitly() {
        // The red essential-mount warning defaults focus to Cancel, so a stray
        // Enter is harmless.
        let mut d = ConfirmDialog::unmount_danger("/");
        assert!(d.danger, "dialog flagged dangerous");
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Cancel => {}
            _ => panic!("default focus must be Cancel"),
        }
        // Choosing "Unmount anyway" still goes through to DoUnmount.
        let mut d = ConfirmDialog::unmount_danger("/boot");
        d.handle_key(key(KeyCode::Left)); // move focus to "Unmount anyway"
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::DoUnmount(mp)) => assert_eq!(mp, "/boot"),
            _ => panic!("expected DoUnmount"),
        }
    }

    #[test]
    fn formatter_collects_a_format_spec() {
        let mut d = FormDialog::format("/dev/sdb1".into());
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::Format(spec)) => {
                assert_eq!(spec.dev, "/dev/sdb1");
                assert_eq!(spec.fs, crate::mount::FsType::Fat32); // default choice
            }
            _ => panic!("expected Format submit"),
        }
    }

    #[test]
    fn create_mountpoint_confirm_yields_mount_create() {
        let mut d = ConfirmDialog::create_mountpoint("/dev/sdb1", "/mnt/new");
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::MountCreate { device, path }) => {
                assert_eq!(device, "/dev/sdb1");
                assert_eq!(path, "/mnt/new");
            }
            _ => panic!("expected MountCreate submit"),
        }
    }

    #[test]
    fn confirmations_form_collects_toggles() {
        let cfg = crate::config::Config::default(); // delete=T, overwrite=T, execute=F, exit=T
        // Submitting the defaults reflects the config.
        let mut d = FormDialog::confirmations(&cfg);
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::Confirmations(v)) => {
                assert!(v.delete && v.overwrite && !v.execute && v.exit);
            }
            _ => panic!("expected Confirmations submit"),
        }
        // Space toggles the focused field (Confirm delete); Enter then submits.
        let mut d = FormDialog::confirmations(&cfg);
        d.handle_key(key(KeyCode::Char(' ')));
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::Confirmations(v)) => assert!(!v.delete),
            _ => panic!("expected Confirmations submit"),
        }
    }

    #[test]
    fn mix_rgb_blends_endpoints() {
        use ratatui::style::Color;
        let a = Color::Rgb(0, 0, 0);
        let b = Color::Rgb(100, 200, 50);
        assert_eq!(mix_rgb(a, b, 0.0), a);
        assert_eq!(mix_rgb(a, b, 1.0), b);
        assert_eq!(mix_rgb(a, b, 0.5), Color::Rgb(50, 100, 25));
    }

    #[test]
    fn save_discard_cancel_has_three_buttons() {
        // Save.
        let mut d = ConfirmDialog::editor_quit("notes.txt");
        assert_eq!(d.buttons.len(), 3);
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::EditorSaveQuit)
        ));

        // Discard via its hotkey.
        let mut d = ConfirmDialog::editor_quit("notes.txt");
        assert!(matches!(
            d.handle_key(key(KeyCode::Char('d'))),
            DialogResult::Submit(Submit::EditorDiscardQuit)
        ));

        // Cancel via its hotkey resumes editing (no submit).
        let mut d = ConfirmDialog::editor_quit("notes.txt");
        assert!(matches!(d.handle_key(key(KeyCode::Char('c'))), DialogResult::Cancel));

        // Esc still cancels.
        let mut d = ConfirmDialog::diff_quit();
        assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));

        // Focus the third button with Tab×2, then Enter cancels.
        let mut d = ConfirmDialog::diff_quit();
        d.handle_key(key(KeyCode::Tab));
        d.handle_key(key(KeyCode::Tab));
        assert_eq!(d.focus, 2);
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
    }

    #[test]
    fn two_button_confirm_still_works() {
        let mut d = ConfirmDialog::quit();
        assert_eq!(d.buttons.len(), 2);
        assert!(matches!(d.handle_key(key(KeyCode::Char('n'))), DialogResult::Cancel));
        let mut d = ConfirmDialog::quit();
        assert!(matches!(
            d.handle_key(key(KeyCode::Char('y'))),
            DialogResult::Submit(Submit::Quit)
        ));
    }

    #[test]
    fn connect_history_dropdown_fills_fields() {
        let history = vec![
            RemoteHistoryEntry {
                protocol: "sftp".into(),
                host: "a.example".into(),
                port: 2222,
                user: "alice".into(),
                path: "/srv".into(),
            },
            // A different protocol must be filtered out of the dropdown.
            RemoteHistoryEntry {
                protocol: "ftp".into(),
                host: "nope".into(),
                port: 21,
                user: String::new(),
                path: String::new(),
            },
        ];
        let mut d = FormDialog::connect(Protocol::Sftp, 1, history);

        // ↓ on the Host field opens the dropdown; Enter selects the only entry.
        assert!(matches!(d.handle_key(key(KeyCode::Down)), DialogResult::None));
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::None));

        // Submitting now yields the filled-in connection.
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::Connect(side, creds)) => {
                assert_eq!(side, 1);
                assert_eq!(creds.host, "a.example");
                assert_eq!(creds.port, 2222);
                assert_eq!(creds.user, "alice");
                assert_eq!(creds.path, "/srv");
            }
            _ => panic!("expected a Connect submit"),
        }
    }

    #[test]
    fn down_does_not_open_dropdown_without_history() {
        let mut d = FormDialog::connect(Protocol::Scp, 0, vec![]);
        // With no history, ↓ just moves focus to the next field (no dropdown).
        d.handle_key(key(KeyCode::Down));
        assert!(d.connect.as_ref().is_some_and(|c| !c.open));
        assert_eq!(d.form.focus, 1);
    }

    #[test]
    fn connect_dialog_renders_chevron_and_dropdown() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let history = vec![RemoteHistoryEntry {
            protocol: "sftp".into(),
            host: "host.example".into(),
            port: 22,
            user: "bob".into(),
            path: "/home".into(),
        }];
        let mut d = FormDialog::connect(Protocol::Sftp, 0, history);
        let theme = crate::ui::theme::Theme::mc();
        let mut t = Terminal::new(TestBackend::new(80, 20)).unwrap();

        let dump = |t: &Terminal<TestBackend>| {
            let b = t.backend().buffer();
            let mut s = String::new();
            for y in 0..b.area.height {
                for x in 0..b.area.width {
                    s.push_str(b[(x, y)].symbol());
                }
            }
            s
        };

        t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
        assert!(dump(&t).contains('▼'), "chevron shown on the host field");

        d.handle_key(key(KeyCode::Down)); // open the dropdown
        t.draw(|f| d.render(f, f.area(), &theme)).unwrap();
        let s = dump(&t);
        assert!(s.contains("Recent"), "dropdown box title");
        assert!(s.contains("bob@host.example:22"), "history entry label");
    }

    #[test]
    fn compare_dialog_selects_mode() {
        // Default focus is Quick; Enter submits it.
        let mut d = CompareDialog::new();
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::CompareDirs(CompareMode::Quick))
        ));
        // Hotkeys pick a mode directly.
        assert!(matches!(
            d.handle_key(key(KeyCode::Char('s'))),
            DialogResult::Submit(Submit::CompareDirs(CompareMode::Size))
        ));
        assert!(matches!(
            d.handle_key(key(KeyCode::Char('c'))),
            DialogResult::Submit(Submit::CompareDirs(CompareMode::Content))
        ));
        // Arrow navigation then Enter.
        let mut d = CompareDialog::new();
        d.handle_key(key(KeyCode::Right));
        d.handle_key(key(KeyCode::Right));
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::CompareDirs(CompareMode::Content))
        ));
        assert!(matches!(d.handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
    }
}
