//! Modal dialogs: text input, confirmation, progress, and messages.
//!
//! Phase 1 keeps these in one module as small state machines. Each dialog
//! consumes key events and reports a [`DialogResult`]; the app acts on
//! `Submit`/`Abort` outcomes.

pub(crate) mod widgets;

mod backgroundops;
mod checksum;
mod compare;
mod confirm;
mod drive;
mod find;
mod flash;
mod form;
mod goto;
mod history;
mod input;
mod message;
mod multirename;
mod overwrite;
mod palette;
mod progress;
mod saveas;
mod search;
mod select;
mod usermenu;

// Shared widget helpers used by `src/disk/render.rs` and kept accessible at the
// dialog module root.
pub(crate) use widgets::pulse_fill;
pub use widgets::centered;
// Shared marked-input editor, reused by the viewer's footer search prompt.
pub(crate) use widgets::edit_text_marked;

// Re-exported so the in-module test suite (`mod tests`) can reach these via
// `use super::*`.
#[cfg(test)]
pub(crate) use flash::SaveFocus;
#[cfg(test)]
pub(crate) use widgets::mix_rgb;

pub use backgroundops::{BackgroundOpsDialog, BgRow};
pub use checksum::ChecksumResultDialog;
pub use compare::CompareDialog;
pub use confirm::ConfirmDialog;
pub use drive::DriveDialog;
pub use find::{FindDialog, FindParams};
pub use flash::{FileBrowserDialog, FlashTargetDialog, ImageSaveDialog};
pub use form::FormDialog;
pub use goto::GotoDialog;
pub use history::ShellHistoryDialog;
pub use input::{InputDialog, InputPurpose};
pub use message::MessageDialog;
pub use multirename::MultiRenameDialog;
pub use overwrite::OverwriteDialog;
pub use palette::{BoolSetting, CommandPaletteDialog, PaletteAction, PaletteCategory, PaletteEntry};
pub use progress::{BusyDialog, ProgressDialog};
pub use saveas::SaveAsDialog;
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
    /// The editor's "Save as" browser (choose a path to write the buffer to).
    SaveAs(SaveAsDialog),
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
    /// The result of a File → Checksum computation (digest + pass/fail verdict).
    ChecksumResult(ChecksumResultDialog),
    /// The list of running background transfers.
    BackgroundOps(BackgroundOpsDialog),
    /// The Ctrl-H command history window, shown above the command line.
    ShellHistory(ShellHistoryDialog),
    /// The Ctrl-P command palette (fuzzy search over actions/settings/etc.).
    CommandPalette(CommandPaletteDialog),
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
    /// Send the running transfer with this id to the background (close its
    /// progress dialog but keep the task running).
    Background(TaskId),
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
    /// "Save as" target chosen for the editor: write the buffer to this path.
    EditorSaveAs(std::path::PathBuf),
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
    /// Set permissions on these targets to `mode`; recurse into directories when
    /// the flag is set.
    Chmod(Vec<VfsPath>, u32, bool),
    /// Set ownership of these targets; recurse into directories when the flag is
    /// set.
    Chown(Vec<VfsPath>, String, String, bool),
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
    /// Find files identical between the two panels (by the chosen criteria) and
    /// mark them in both.
    FindDuplicates(DupCriteria),
    /// Open/execute a local file with its default application (confirmed).
    OpenWith(std::path::PathBuf),
    /// Run a local executable file directly in the foreground (confirmed).
    RunProgram(std::path::PathBuf),
    /// Copy a command chosen from the Shell History window into the command
    /// line (without running it).
    RecallCommand(String),
    /// Mount `device` at `path` (disk manager); the app handles create-if-missing.
    Mount { device: String, path: String },
    /// Create the (missing) mount point and then mount.
    MountCreate { device: String, path: String },
    /// A sudo password entered for a queued privileged command.
    SudoPassword(String),
    /// A root password (possibly blank) for the network-connections explorer.
    NetworkPassword(String),
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
    /// Return a panel (`side`) to its last local directory, keeping any open
    /// remote sessions alive (drive picker "Local" button).
    GoLocal(usize),
    /// Switch a panel (`side`) to an already-open remote session by id.
    SwitchSession(usize, usize),
    /// Ask (with a Yes/No confirm) to disconnect the remote session with this id.
    AskDisconnectSession(usize),
    /// Confirmed: tear down the remote session with this id.
    DisconnectSession(usize),
    /// Apply a batch rename: each `(source, new name)` pair renames the source
    /// to the new name in its own directory.
    MultiRename(Vec<(VfsPath, String)>),
    /// Compute a checksum of `path` with the chosen algorithm, optionally
    /// comparing against a user-supplied digest (`expected`, empty = none).
    Checksum {
        path: VfsPath,
        kind: crate::util::checksum::ChecksumKind,
        expected: String,
    },
    /// Bring the background transfer with this id back to the foreground (re-open
    /// its progress dialog).
    ForegroundTask(TaskId),
    /// Run an entry chosen from the command palette (Ctrl-P).
    Palette(PaletteAction),
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

/// Which attributes must match for the "Find duplicates" tool to treat two
/// same-named files as identical. With all of `size`/`date`/`content` off, only
/// the file name is compared.
#[derive(Debug, Clone, Copy)]
pub struct DupCriteria {
    pub size: bool,
    pub date: bool,
    pub content: bool,
    /// Whether the file-name match is case-sensitive (default on).
    pub case_sensitive: bool,
}

/// Values collected by the settings form.
#[derive(Debug, Clone)]
pub struct SettingsValues {
    pub editor: String,
    pub viewer: String,
    pub use_internal_viewer: bool,
    pub use_internal_editor: bool,
    pub theme: String,
    /// The chosen UI language (a language file's display name).
    pub language: String,
    pub truecolor: bool,
    pub animation: bool,
    pub system_status: bool,
    /// Reshape + bidi-reorder RTL text for display.
    pub reshape_rtl: bool,
    /// Terminal pixel-graphics preference (`auto|off|kitty|sixel|iterm`).
    pub graphics: String,
    /// Number of columns in the Brief view.
    pub brief_columns: usize,
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
    /// The panel index this dialog should be anchored over (the drive/connection
    /// picker opens over its target panel), or `None` to center on the whole
    /// screen like every other dialog.
    pub fn anchor_panel(&self) -> Option<usize> {
        match self {
            Dialog::Drive(d) => Some(d.side()),
            _ => None,
        }
    }

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
            Dialog::SaveAs(d) => d.handle_key(key),
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
            Dialog::ChecksumResult(d) => d.handle_key(key),
            Dialog::BackgroundOps(d) => d.handle_key(key),
            Dialog::ShellHistory(d) => d.handle_key(key),
            Dialog::CommandPalette(d) => d.handle_key(key),
        }
    }

    pub fn render(
        &mut self,
        f: &mut Frame,
        area: Rect,
        theme: &Theme,
        gfx: Option<&mut crate::ui::graphics::Gfx>,
    ) {
        match self {
            Dialog::Input(d) => d.render(f, area, theme, gfx),
            Dialog::Confirm(d) => d.render(f, area, theme, gfx),
            Dialog::Progress(d) => d.render(f, area, theme, gfx),
            Dialog::Busy(d) => d.render(f, area, theme),
            Dialog::Goto(d) => d.render(f, area, theme, gfx),
            Dialog::FlashTarget(d) => d.render(f, area, theme),
            Dialog::FileBrowser(d) => d.render(f, area, theme),
            Dialog::ImageSave(d) => d.render(f, area, theme),
            Dialog::SaveAs(d) => d.render(f, area, theme),
            Dialog::Drive(d) => d.render(f, area, theme, gfx),
            Dialog::MultiRename(d) => d.render(f, area, theme, gfx),
            Dialog::Message(d) => d.render(f, area, theme, gfx),
            Dialog::Form(d) => d.render(f, area, theme, gfx),
            Dialog::Select(d) => d.render(f, area, theme, gfx),
            Dialog::SearchReplace(d) => d.render(f, area, theme, gfx),
            Dialog::Find(d) => d.render(f, area, theme, gfx),
            Dialog::UserMenu(d) => d.render(f, area, theme),
            Dialog::Overwrite(d) => d.render(f, area, theme, gfx),
            Dialog::Compare(d) => d.render(f, area, theme, gfx),
            Dialog::ChecksumResult(d) => d.render(f, area, theme, gfx),
            Dialog::BackgroundOps(d) => d.render(f, area, theme),
            Dialog::ShellHistory(d) => d.render(f, area, theme),
            Dialog::CommandPalette(d) => d.render(f, area, theme),
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
            Dialog::ChecksumResult(d) => return d.handle_click(col, row),
            Dialog::Confirm(d) => {
                let rect = d.box_rect(area);
                return d.handle_click(rect, col, row);
            }
            // Any click dismisses a message box.
            Dialog::Message(_) => return DialogResult::Cancel,
            // The progress dialog hit-tests its buttons: a backgroundable transfer
            // has "To background"/"Abort", and an indeterminate scan has "Abort".
            // A plain determinate dialog has no clickable buttons, so a stray click
            // can't cancel it.
            Dialog::Progress(d) => return d.handle_click(col, row),
            // The busy spinner can't be dismissed at all.
            Dialog::Busy(_) => return DialogResult::None,
            // The Goto dialog hit-tests its radios and OK/Cancel buttons.
            Dialog::Goto(d) => return d.handle_click(area, col, row),
            // The flash dialogs hit-test their scrolling lists.
            Dialog::FlashTarget(d) => return d.handle_click(area, col, row),
            Dialog::FileBrowser(d) => return d.handle_click(area, col, row),
            Dialog::ImageSave(d) => return d.handle_click(area, col, row),
            Dialog::SaveAs(d) => return d.handle_click(area, col, row),
            Dialog::Drive(d) => return d.handle_click(area, col, row),
            Dialog::MultiRename(d) => return d.handle_click(area, col, row),
            Dialog::Find(d) => return d.handle_click(area, col, row),
            Dialog::Select(d) => return d.handle_click(area, col, row),
            Dialog::UserMenu(d) => return d.handle_click(area, col, row),
            Dialog::ShellHistory(d) => return d.handle_click(area, col, row),
            Dialog::CommandPalette(d) => return d.handle_click(area, col, row),
            Dialog::BackgroundOps(d) => return d.handle_click(area, col, row),
            // The connect form's history chevron/dropdown and the Choice
            // dropdowns take clicks first.
            Dialog::Form(d) => {
                if let Some(res) = d.click_dropdown(col, row) {
                    return res;
                }
                if let Some(res) = d.click_choice(area, col, row) {
                    return res;
                }
                // A click on a text field focuses it (placing the caret) and a
                // click on a checkbox toggles it.
                if let Some(res) = d.click_field(area, col, row) {
                    return res;
                }
                // A click on the OK/Cancel button row: focus that button first so
                // the synthetic Enter/Esc submits or cancels the form instead of
                // acting on the currently-focused field (e.g. opening a dropdown).
                let rect = d.outer_rect(area);
                let in_box = col >= rect.x
                    && col < rect.x + rect.width
                    && row >= rect.y
                    && row < rect.y + rect.height;
                let button_row = rect.y + rect.height.saturating_sub(2);
                if in_box && row == button_row {
                    let primary = col < rect.x + rect.width / 2;
                    d.focus_button(primary);
                    let code = if primary { KeyCode::Enter } else { KeyCode::Esc };
                    return self.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
                }
            }
            // A click on a text field / radio / checkbox is handled here; the
            // OK/Cancel button row falls through to the generic handler below.
            Dialog::Input(d) => {
                if let Some(res) = d.click_field(area, col, row) {
                    return res;
                }
            }
            Dialog::SearchReplace(d) => {
                if let Some(res) = d.click_field(area, col, row) {
                    return res;
                }
            }
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

    /// Route a mouse-wheel scroll to dialogs with a scrollable region. `delta` is
    /// in rows (positive = down). Returns `None` for dialogs that don't scroll.
    pub fn handle_scroll(&mut self, delta: isize) -> DialogResult {
        match self {
            Dialog::MultiRename(d) => {
                d.handle_scroll(delta);
            }
            // Scroll an open Choice dropdown's highlight (settings theme/language).
            Dialog::Form(d) => {
                d.scroll_choice(delta);
            }
            Dialog::CommandPalette(d) => {
                return d.handle_scroll(delta);
            }
            _ => {}
        }
        DialogResult::None
    }

    /// The centered bounding box of dialogs whose buttons live on the last row.
    /// `None` for dialogs handled specially or that ignore clicks.
    fn click_bounds(&self, area: Rect) -> Option<Rect> {
        let aw = area.width;
        let r = match self {
            Dialog::Input(_) => centered(area, 60u16.min(aw.saturating_sub(4)), 7),
            Dialog::Form(d) => d.outer_rect(area),
            Dialog::SearchReplace(d) => {
                centered(area, 64u16.min(aw.saturating_sub(2)), if d.replace { 14 } else { 12 })
            }
            _ => return None,
        };
        Some(r)
    }
}

#[cfg(test)]
mod tests;
