//! Confirmation dialog (yes/no, multi-button menus, danger prompts).

use super::widgets::*;
use super::{DialogResult, Submit};
use crate::ops::progress::TaskId;

// ---------------------------------------------------------------------------
// Confirm dialog
// ---------------------------------------------------------------------------

/// One button in a [`ConfirmDialog`]. A `None` action simply cancels.
pub(crate) struct ConfirmButton {
    label: String,
    action: Option<Submit>,
}

pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub(crate) buttons: Vec<ConfirmButton>,
    /// Index of the currently focused button.
    pub(crate) focus: usize,
    /// When true, the dialog is drawn in red to flag a dangerous operation.
    pub(crate) danger: bool,
    /// Optional box-width override (for menus with many/long buttons).
    width: Option<u16>,
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
            width: None,
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
            width: None,
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
            width: None,
        }
    }

    /// Action menu for a block device: Mount/Format/Flash/Create image when free,
    /// Unmount/Flash/Create image when mounted.
    pub fn device_menu(d: &crate::mount::BlockDevice) -> Self {
        let target = crate::flash::FlashTarget::from_device(d);
        let flash = ("Flash image", Some(Submit::FlashBrowse(target.clone())));
        let create = ("Create image", Some(Submit::ImageBrowse(target)));
        let mut m = match d.mountpoint.as_deref() {
            Some(mp) => Self::from_buttons(
                "Device",
                format!("{}  ({})  mounted at {mp}", d.name, d.dev),
                vec![
                    ("Unmount", Some(Submit::AskUnmount(mp.to_string()))),
                    flash,
                    create,
                    ("Cancel", None),
                ],
            ),
            None => Self::from_buttons(
                "Device",
                format!("{}  ({})", d.name, d.dev),
                vec![
                    ("Mount", Some(Submit::MountDevice(d.dev.clone()))),
                    ("Format", Some(Submit::FormatDevice(d.dev.clone()))),
                    flash,
                    create,
                    ("Cancel", None),
                ],
            ),
        };
        m.width = Some(78); // room for the longer action buttons on one row
        m
    }

    /// Final (destructive) confirmation before flashing an image to a device.
    pub fn flash_confirm(spec: crate::flash::FlashSpec) -> Self {
        let msg = format!(
            "ERASE ALL DATA on {} and write \"{}\" ({})?",
            spec.target.describe(),
            spec.image_name,
            human_size(spec.image_size),
        );
        Self::from_buttons("Flash image", msg, vec![("Flash", Some(Submit::DoFlash(spec))), ("Cancel", None)])
    }

    /// Loud red warning before flashing a *non-removable* device (likely a fixed
    /// or system disk). Defaults focus to Cancel.
    pub fn flash_danger(spec: crate::flash::FlashSpec) -> Self {
        let dev = spec.target.dev.clone();
        let mut d = Self::from_buttons(
            "DANGER",
            format!(
                "{dev} is NOT a removable device — it may be a system or data disk. \
                 Flashing will destroy everything on it and can make your system \
                 unbootable. Continue anyway?"
            ),
            vec![("Continue", Some(Submit::FlashConfirm(spec))), ("Cancel", None)],
        );
        d.danger = true;
        d.focus = 1; // Cancel
        d
    }

    /// Final confirmation before overwriting an existing image file.
    pub fn image_overwrite(spec: crate::flash::ImageSpec) -> Self {
        Self::yes_no(
            "Overwrite",
            format!("\"{}\" already exists. Overwrite it?", spec.dest_name),
            Submit::DoImage(spec),
            "Overwrite",
            "Cancel",
            None,
        )
    }

    /// Asked when the user hits Abort during a flash/imaging task: resume, or
    /// really abort (discarding whatever has been written so far).
    pub fn abort_flash(id: TaskId) -> Self {
        let mut d = Self::from_buttons(
            "Abort?",
            "The operation is still in progress. Resume, or abort and discard \
             what has been written so far?"
                .to_string(),
            vec![
                ("Resume", Some(Submit::FlashResume)),
                ("Abort", Some(Submit::FlashAbort(id))),
            ],
        );
        d.danger = true;
        d.focus = 0; // Resume (the safe choice)
        d
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

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> DialogResult {
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
    pub(crate) fn handle_click(&mut self, rect: Rect, col: u16, row: u16) -> DialogResult {
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
    pub(crate) fn box_rect(&self, area: Rect) -> Rect {
        let (w, h) = if self.danger { (58u16, 9u16) } else { (54u16, 7u16) };
        let w = self.width.unwrap_or(w);
        centered(area, w.min(area.width.saturating_sub(4)), h)
    }

    pub(crate) fn render(&self, f: &mut Frame, area: Rect, theme: &Theme) {
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

