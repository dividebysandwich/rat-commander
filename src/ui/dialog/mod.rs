//! Modal dialogs: text input, confirmation, progress, and messages.
//!
//! Phase 1 keeps these in one module as small state machines. Each dialog
//! consumes key events and reports a [`DialogResult`]; the app acts on
//! `Submit`/`Abort` outcomes.

mod widgets;

mod compare;
mod confirm;
mod drive;
mod find;
mod flash;
mod form;
mod goto;
mod input;
mod message;
mod multirename;
mod overwrite;
mod progress;
mod search;
mod select;
mod usermenu;

// Shared widget helpers used by `src/disk/render.rs` and kept accessible at the
// dialog module root.
pub(crate) use widgets::pulse_fill;
pub use widgets::centered;

// Re-exported so the in-module test suite (`mod tests`) can reach these via
// `use super::*`.
#[cfg(test)]
pub(crate) use flash::SaveFocus;
#[cfg(test)]
pub(crate) use widgets::mix_rgb;

pub use compare::CompareDialog;
pub use confirm::ConfirmDialog;
pub use drive::DriveDialog;
pub use find::{FindDialog, FindParams};
pub use flash::{FileBrowserDialog, FlashTargetDialog, ImageSaveDialog};
pub use form::FormDialog;
pub use goto::GotoDialog;
pub use input::{InputDialog, InputPurpose};
pub use message::MessageDialog;
pub use multirename::MultiRenameDialog;
pub use overwrite::OverwriteDialog;
pub use progress::{BusyDialog, ProgressDialog};
pub use search::{SearchReplaceDialog, SearchReplaceParams};
pub use select::SelectDialog;
pub use usermenu::UserMenuDialog;

use crate::ops::progress::{OverwriteDecision, TaskId};
use crate::ui::theme::Theme;
use crate::vfs::VfsPath;
use crate::vfs::remote::{Protocol, RemoteCreds};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

/// The active modal dialog (only one at a time).
#[allow(clippy::large_enum_variant)]
pub enum Dialog {
    Input(InputDialog),
    Confirm(ConfirmDialog),
    Progress(ProgressDialog),
    /// A non-dismissible "working…" spinner shown while a blocking background
    /// operation (e.g. formatting a disk) runs.
    Busy(BusyDialog),
    /// The viewer's "Goto" dialog (line / percent / byte offset).
    Goto(GotoDialog),
    /// Pick which block device to flash an image onto.
    FlashTarget(FlashTargetDialog),
    /// A small file browser for choosing an image to flash.
    FileBrowser(FileBrowserDialog),
    /// A "save as" browser for choosing where to write a device image.
    ImageSave(ImageSaveDialog),
    /// The Windows drive-letter picker.
    Drive(DriveDialog),
    /// The multi-file rename dialog.
    MultiRename(MultiRenameDialog),
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
    /// Jump the viewer to a position (value + how to interpret it).
    ViewerGoto(String, crate::viewer::GotoMode),
    /// A device was chosen in the flash target picker → proceed to confirmation.
    FlashSelected(crate::flash::FlashSpec),
    /// The non-removable danger warning was accepted → final flash confirmation.
    FlashConfirm(crate::flash::FlashSpec),
    /// The flash was confirmed → start writing.
    DoFlash(crate::flash::FlashSpec),
    /// Browse for an image to flash onto this device (disk-manager entry point).
    FlashBrowse(crate::flash::FlashTarget),
    /// An image file was picked in the flash file browser (path + target device).
    FlashBrowsePicked(std::path::PathBuf, crate::flash::FlashTarget),
    /// A sudo password was entered to start a flash.
    FlashPassword(String),
    /// Resume a flash whose abort prompt was dismissed.
    FlashResume,
    /// Really abort the running flash task.
    FlashAbort(TaskId),
    /// Browse for a destination to read this device out to (disk-manager entry).
    ImageBrowse(crate::flash::FlashTarget),
    /// A destination file was chosen in the save dialog → confirm / start.
    ImageSave(crate::flash::ImageSpec),
    /// Imaging was confirmed (e.g. overwrite accepted) → start reading.
    DoImage(crate::flash::ImageSpec),
    /// A sudo password was entered to start imaging.
    ImagePassword(String),
    /// Switch a panel (`side`) to a drive letter (Windows drive picker).
    SetDrive(usize, char),
    /// Open the connect form for `side` with the given protocol (drive picker).
    OpenConnect(usize, Protocol),
    /// Return a panel (`side`) to the local filesystem (drive picker).
    DisconnectPanel(usize),
    /// Apply a batch rename: each `(source, new name)` pair renames the source
    /// to the new name in its own directory.
    MultiRename(Vec<(VfsPath, String)>),
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
            Dialog::Goto(d) => d.handle_key(key),
            Dialog::FlashTarget(d) => d.handle_key(key),
            Dialog::FileBrowser(d) => d.handle_key(key),
            Dialog::ImageSave(d) => d.handle_key(key),
            Dialog::Drive(d) => d.handle_key(key),
            Dialog::MultiRename(d) => d.handle_key(key),
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
            Dialog::Goto(d) => d.render(f, area, theme),
            Dialog::FlashTarget(d) => d.render(f, area, theme),
            Dialog::FileBrowser(d) => d.render(f, area, theme),
            Dialog::ImageSave(d) => d.render(f, area, theme),
            Dialog::Drive(d) => d.render(f, area, theme),
            Dialog::MultiRename(d) => d.render(f, area, theme),
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
            // The Goto dialog hit-tests its radios and OK/Cancel buttons.
            Dialog::Goto(d) => return d.handle_click(area, col, row),
            // The flash dialogs hit-test their scrolling lists.
            Dialog::FlashTarget(d) => return d.handle_click(area, col, row),
            Dialog::FileBrowser(d) => return d.handle_click(area, col, row),
            Dialog::ImageSave(d) => return d.handle_click(area, col, row),
            Dialog::Drive(d) => return d.handle_click(area, col, row),
            Dialog::MultiRename(d) => return d.handle_click(area, col, row),
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
    fn goto_dialog_collects_value_and_mode_by_keyboard() {
        let mut d = GotoDialog::new();
        for c in "0x1f".chars() {
            d.handle_key(key(KeyCode::Char(c)));
        }
        // Move the radio selection to "Hexadecimal offset" (mode 3).
        for _ in 0..3 {
            d.handle_key(key(KeyCode::Down));
        }
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::ViewerGoto(v, m)) => {
                assert_eq!(v, "0x1f");
                assert_eq!(m, crate::viewer::GotoMode::HexOffset);
            }
            _ => panic!("expected a ViewerGoto submit"),
        }
        // An empty value (or Esc) cancels rather than submitting.
        assert!(matches!(GotoDialog::new().handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
        assert!(matches!(GotoDialog::new().handle_key(key(KeyCode::Esc)), DialogResult::Cancel));
    }

    #[test]
    fn goto_dialog_mouse_selects_radio_and_buttons() {
        // 80x24 → the box is centered at {20,7,40,9}; inner {21,8,38,7}.
        let area = Rect::new(0, 0, 80, 24);
        let mut d = GotoDialog::new();
        for c in "12".chars() {
            d.handle_key(key(KeyCode::Char(c)));
        }
        // Radio rows are inner.y+1.. → row 11 is "Decimal offset" (index 2).
        assert!(matches!(d.handle_click(area, 25, 11), DialogResult::None));
        assert_eq!(d.mode, 2);
        // The button row is the last interior row (y=14); left half is OK.
        match d.handle_click(area, 25, 14) {
            DialogResult::Submit(Submit::ViewerGoto(v, m)) => {
                assert_eq!(v, "12");
                assert_eq!(m, crate::viewer::GotoMode::DecimalOffset);
            }
            _ => panic!("clicking OK submits"),
        }
        // The right half of the button row cancels.
        assert!(matches!(d.handle_click(area, 55, 14), DialogResult::Cancel));
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
        let dev = |mp: Option<&str>| crate::mount::BlockDevice {
            name: "sdb1".into(),
            dev: "/dev/sdb1".into(),
            mountpoint: mp.map(str::to_string),
            ..Default::default()
        };
        // Unmounted device: the focused "Mount" button → MountDevice.
        let mut d = ConfirmDialog::device_menu(&dev(None));
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::MountDevice(dev)) => assert_eq!(dev, "/dev/sdb1"),
            _ => panic!("expected MountDevice"),
        }
        // Mounted device: the only action is Unmount.
        let mut d = ConfirmDialog::device_menu(&dev(Some("/mnt/x")));
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

    fn bdev(name: &str, dev: &str, size: u64, removable: bool) -> crate::mount::BlockDevice {
        crate::mount::BlockDevice {
            name: name.into(),
            dev: dev.into(),
            size,
            removable,
            ..Default::default()
        }
    }

    #[test]
    fn device_menu_offers_flash_and_create_image() {
        // Free device: [Mount, Format, Flash image, Create image, Cancel].
        let mut menu = ConfirmDialog::device_menu(&bdev("sdb", "/dev/sdb", 100, true));
        menu.handle_key(key(KeyCode::Right)); // Format
        menu.handle_key(key(KeyCode::Right)); // Flash image
        match menu.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::FlashBrowse(t)) => assert_eq!(t.dev, "/dev/sdb"),
            _ => panic!("expected FlashBrowse"),
        }
        let mut menu = ConfirmDialog::device_menu(&bdev("sdb", "/dev/sdb", 100, true));
        for _ in 0..3 {
            menu.handle_key(key(KeyCode::Right)); // → Create image
        }
        match menu.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::ImageBrowse(t)) => assert_eq!(t.dev, "/dev/sdb"),
            _ => panic!("expected ImageBrowse"),
        }
    }

    #[test]
    fn image_save_dialog_builds_a_spec() {
        let src = crate::flash::FlashTarget { dev: "/dev/sdb".into(), size: 4096, ..Default::default() };
        // Start in an existing dir (the temp dir) and confirm with the default name.
        let mut d = ImageSaveDialog::new(src, std::env::temp_dir());
        d.focus = SaveFocus::Name; // jump to the name field
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::ImageSave(spec)) => {
                assert_eq!(spec.source.dev, "/dev/sdb");
                assert_eq!(spec.dest_name, "sdb.img", "default name from the device");
                assert_eq!(spec.dest_path, std::env::temp_dir().join("sdb.img"));
            }
            _ => panic!("expected ImageSave submit"),
        }
        // The overwrite confirm routes to DoImage.
        let spec = crate::flash::ImageSpec {
            source: crate::flash::FlashTarget { dev: "/dev/sdb".into(), size: 10, ..Default::default() },
            dest_path: "/tmp/x.img".into(),
            dest_name: "x.img".into(),
        };
        let mut ov = ConfirmDialog::image_overwrite(spec);
        assert!(matches!(ov.handle_key(key(KeyCode::Enter)), DialogResult::Submit(Submit::DoImage(_))));
    }

    #[test]
    fn drive_dialog_connection_buttons() {
        // No drives (Linux/macOS): only SFTP/FTP/SCP; default cursor on SFTP.
        let mut d = DriveDialog::new(0, vec![], None, false);
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::OpenConnect(0, Protocol::Sftp)) => {}
            _ => panic!("expected SFTP OpenConnect"),
        }
        // Right, Right → SCP.
        let mut d = DriveDialog::new(1, vec![], None, false);
        d.handle_key(key(KeyCode::Right));
        d.handle_key(key(KeyCode::Right));
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::OpenConnect(1, Protocol::Scp))
        ));
    }

    #[test]
    fn drive_dialog_disconnect_only_when_connected() {
        // Connected → a trailing Disconnect button (End lands on it).
        let mut d = DriveDialog::new(0, vec![], None, true);
        d.handle_key(key(KeyCode::End));
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::DisconnectPanel(0))
        ));
        // Not connected → End lands on the last connection (SCP), no Disconnect.
        let mut d = DriveDialog::new(0, vec![], None, false);
        d.handle_key(key(KeyCode::End));
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::OpenConnect(0, Protocol::Scp))
        ));
    }

    #[test]
    fn drive_dialog_letter_jumps_and_highlights_current() {
        // Windows-style: drive letters present, current drive highlighted.
        let mut d = DriveDialog::new(0, vec!['A', 'C', 'D', 'Z'], Some('C'), false);
        // A drive letter jumps straight to that drive.
        match d.handle_key(key(KeyCode::Char('z'))) {
            DialogResult::Submit(Submit::SetDrive(0, c)) => assert_eq!(c, 'Z'),
            _ => panic!("expected SetDrive Z"),
        }
        // Enter activates the highlighted (current) drive C.
        let mut d = DriveDialog::new(0, vec!['A', 'C', 'D', 'Z'], Some('C'), false);
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::SetDrive(0, 'C'))
        ));
    }

    #[test]
    fn flash_target_picker_enforces_size() {
        let devs = vec![
            bdev("sda", "/dev/sda", 1_000, false),  // too small
            bdev("sdb", "/dev/sdb", 10_000, true),  // fits
        ];
        let img = std::path::PathBuf::from("/img/x.iso");
        // Preselect the big device → Enter flashes it.
        let mut d = FlashTargetDialog::new(img.clone(), "x.iso".into(), 5_000, devs.clone(), Some("/dev/sdb"));
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::FlashSelected(spec)) => {
                assert_eq!(spec.target.dev, "/dev/sdb");
                assert_eq!(spec.image_size, 5_000);
            }
            _ => panic!("expected FlashSelected"),
        }
        // The default (first) device is too small → Enter is refused.
        let mut d = FlashTargetDialog::new(img, "x.iso".into(), 5_000, devs, None);
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::None));
    }

    #[test]
    fn flash_confirmations_emit_expected_submits() {
        let spec = |removable: bool| crate::flash::FlashSpec {
            image_path: "/x.iso".into(),
            image_name: "x.iso".into(),
            image_size: 10,
            target: crate::flash::FlashTarget {
                dev: "/dev/sdb".into(),
                size: 100,
                removable,
                ..Default::default()
            },
        };
        // Removable → straight to the destructive confirm; "Flash" → DoFlash.
        let mut d = ConfirmDialog::flash_confirm(spec(true));
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::DoFlash(s)) => assert_eq!(s.target.dev, "/dev/sdb"),
            _ => panic!("expected DoFlash"),
        }
        // Non-removable danger defaults to Cancel; "Continue" → FlashConfirm.
        let mut d = ConfirmDialog::flash_danger(spec(false));
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Cancel));
        let mut d = ConfirmDialog::flash_danger(spec(false));
        d.handle_key(key(KeyCode::Left)); // focus "Continue"
        assert!(matches!(
            d.handle_key(key(KeyCode::Enter)),
            DialogResult::Submit(Submit::FlashConfirm(_))
        ));
        // Abort prompt: Resume (default) vs really-abort.
        let mut d = ConfirmDialog::abort_flash(7);
        assert!(matches!(d.handle_key(key(KeyCode::Enter)), DialogResult::Submit(Submit::FlashResume)));
        let mut d = ConfirmDialog::abort_flash(7);
        d.handle_key(key(KeyCode::Right));
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::FlashAbort(id)) => assert_eq!(id, 7),
            _ => panic!("expected FlashAbort"),
        }
    }

    #[test]
    fn file_browser_filters_and_picks() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rc_fb_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("disk.img"), b"x").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        let target = crate::flash::FlashTarget { dev: "/dev/sdb".into(), size: 100, removable: true, ..Default::default() };
        let mut d = FileBrowserDialog::new(target, dir.clone());
        // The default *.img/*.iso/... filter shows the image + dirs, not the .txt.
        assert!(d.entries.iter().any(|e| e.name == "disk.img" && !e.is_dir));
        assert!(d.entries.iter().any(|e| e.name == "sub" && e.is_dir));
        assert!(!d.entries.iter().any(|e| e.name == "notes.txt"));
        // Picking the image emits its path + the target device.
        d.cursor = d.entries.iter().position(|e| e.name == "disk.img").unwrap();
        match d.handle_key(key(KeyCode::Enter)) {
            DialogResult::Submit(Submit::FlashBrowsePicked(p, t)) => {
                assert_eq!(p, dir.join("disk.img"));
                assert_eq!(t.dev, "/dev/sdb");
            }
            _ => panic!("expected FlashBrowsePicked"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn progress_dialog_estimates_time() {
        let mut p = ProgressDialog::new(1, "Flashing");
        p.total_total = 1000;
        p.total_done = 500;
        assert_eq!(p.eta_text(), "--:--", "no speed sample yet");
        p.samples.push((500.0, 100.0)); // 100 B/s, 500 left → 5 s
        assert_eq!(p.eta_text(), "00:05");
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
