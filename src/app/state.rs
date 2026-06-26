//! Application state and the key/event dispatch that drives it.

use crate::app::event::AppEvent;
use crate::config::Config;
use crate::editor::{EditorSignal, EditorState};
use crate::ops::progress::TaskOutcome;
use crate::ops::{OpKind, OpRequest, TaskHandle, TaskId, spawn_op};
use crate::panel::sort::SortKey;
use crate::panel::Panel;
use crate::ui::cmdline::CommandLine;
use crate::ui::dialog::{
    ConfirmDialog, Dialog, DialogResult, FormDialog, InputDialog, InputPurpose, MessageDialog,
    ProgressDialog, Submit,
};
use crate::ui::layout::SplitDir;
use crate::ui::menu::{MenuAction, MenuBarState, MenuSignal};
use crate::ui::theme::Theme;
use crate::util::async_bridge::AppSender;
use crate::viewer::{MAX_VIEW_BYTES, ViewerSignal, ViewerState};
use crate::vfs::Vfs;
use crate::vfs::archive::{self, formats::ArchiveFormat};
use crate::vfs::VfsKind;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use crate::vfs::registry::Registry;
use crate::vfs::VfsPath;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

/// What the run loop should do after handling input.
pub enum Flow {
    Continue,
    Quit,
    /// Suspend the TUI and run this shell command in the active panel's cwd.
    RunCommand(String),
    /// Suspend the TUI and run an external program against a file.
    RunExternal { program: String, path: std::path::PathBuf },
}

const PAGE: isize = 15;

pub struct AppState {
    pub panels: [Panel; 2],
    /// Index of the active panel (0 = left/top, 1 = right/bottom).
    pub active: usize,
    pub split: SplitDir,
    pub cmd: CommandLine,
    pub dialog: Option<Dialog>,
    pub viewer: Option<ViewerState>,
    pub editor: Option<EditorState>,
    pub menu: Option<MenuBarState>,
    pub theme: Theme,
    pub config: Config,
    pub registry: Registry,
    tasks: HashMap<TaskId, TaskHandle>,
    next_task_id: TaskId,
    tx: AppSender,
    /// Set when a confirmed quit should propagate out as `Flow::Quit`.
    pending_quit: bool,
}

impl AppState {
    pub fn new(tx: AppSender) -> Self {
        let registry = Registry::new();
        let local = registry.local();
        let cwd = VfsPath::local_cwd();
        let left = Panel::new(local.clone(), cwd.clone());
        let right = Panel::new(local, cwd);
        AppState {
            panels: [left, right],
            active: 0,
            split: SplitDir::Vertical,
            cmd: CommandLine::new(),
            dialog: None,
            viewer: None,
            editor: None,
            menu: None,
            theme: Theme::mc(),
            config: Config::load(),
            registry,
            tasks: HashMap::new(),
            next_task_id: 1,
            tx,
            pending_quit: false,
        }
    }

    /// Load both panels' directories.
    pub async fn init(&mut self) {
        let _ = self.panels[0].reload().await;
        let _ = self.panels[1].reload().await;
    }

    fn active_panel(&mut self) -> &mut Panel {
        &mut self.panels[self.active]
    }

    fn other_index(&self) -> usize {
        1 - self.active
    }

    /// Reload both panels after a filesystem-changing operation.
    pub async fn reload_all(&mut self) {
        for p in self.panels.iter_mut() {
            let _ = p.reload().await;
        }
    }

    // -- Event handling ----------------------------------------------------

    pub async fn apply_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Progress(u) => {
                if let Some(Dialog::Progress(p)) = &mut self.dialog
                    && p.id == u.id
                {
                    p.update(&u);
                }
            }
            AppEvent::TaskDone { id, outcome } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                if let TaskOutcome::Failed(msg) = outcome {
                    self.dialog = Some(Dialog::Message(MessageDialog::error(msg)));
                }
                // Drop selections that were just operated on, then refresh.
                for p in self.panels.iter_mut() {
                    p.selection.clear();
                }
                self.reload_all().await;
            }
        }
    }

    // -- Key handling ------------------------------------------------------

    pub async fn handle_key(&mut self, key: KeyEvent) -> Flow {
        if self.dialog.is_some() {
            let res = self.dialog.as_mut().unwrap().handle_key(key);
            return self.handle_dialog_result(res).await;
        }
        if self.editor.is_some() {
            let signal = self.editor.as_mut().unwrap().handle_key(key);
            match signal {
                EditorSignal::Stay => {}
                EditorSignal::Close => {
                    self.editor = None;
                    self.reload_all().await;
                }
                EditorSignal::Save { close_after } => self.save_editor(close_after).await,
                EditorSignal::ConfirmQuit => {
                    let name = self
                        .editor
                        .as_ref()
                        .map(|e| e.name.clone())
                        .unwrap_or_default();
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::editor_quit(&name)));
                }
            }
            return Flow::Continue;
        }
        if let Some(v) = self.viewer.as_mut() {
            if let ViewerSignal::Close = v.handle_key(key) {
                self.viewer = None;
            }
            return Flow::Continue;
        }
        if self.menu.is_some() {
            return self.handle_menu_key(key).await;
        }
        self.handle_panel_key(key).await
    }

    async fn handle_menu_key(&mut self, key: KeyEvent) -> Flow {
        let signal = self.menu.as_mut().unwrap().handle_key(key);
        match signal {
            MenuSignal::Stay => Flow::Continue,
            MenuSignal::Close => {
                self.menu = None;
                Flow::Continue
            }
            MenuSignal::Activate(action) => {
                self.menu = None;
                self.run_menu_action(action).await
            }
        }
    }

    async fn run_menu_action(&mut self, action: MenuAction) -> Flow {
        match action {
            MenuAction::Separator => {}
            MenuAction::View => return self.open_view().await,
            MenuAction::Edit => return self.open_edit().await,
            MenuAction::Copy => self.open_transfer_dialog(OpKind::Copy),
            MenuAction::Move => self.open_transfer_dialog(OpKind::Move),
            MenuAction::Mkdir => self.open_mkdir(),
            MenuAction::Delete => self.open_delete_dialog(),
            MenuAction::Chmod => self.open_chmod(),
            MenuAction::Chown => self.open_chown(),
            MenuAction::Symlink => self.open_symlink(),
            MenuAction::Compress => self.open_compress(),
            MenuAction::SelectGroup => self.open_select_group(true),
            MenuAction::UnselectGroup => self.open_select_group(false),
            MenuAction::Invert => self.invert_selection(),
            MenuAction::SetFormat(side, fmt) => self.panels[side].format = fmt,
            MenuAction::SetSort(side, key) => {
                self.panels[side].sort.key = key;
                self.panels[side].resort();
            }
            MenuAction::ToggleReverse(side) => {
                self.panels[side].sort.reverse = !self.panels[side].sort.reverse;
                self.panels[side].resort();
            }
            MenuAction::SwapPanels => self.panels.swap(0, 1),
            MenuAction::Refresh => self.reload_all().await,
            MenuAction::ToggleSplit => self.split = self.split.toggle(),
            MenuAction::Settings => self.open_settings(),
            MenuAction::Quit => self.dialog = Some(Dialog::Confirm(ConfirmDialog::quit())),
        }
        Flow::Continue
    }

    async fn handle_dialog_result(&mut self, res: DialogResult) -> Flow {
        match res {
            DialogResult::None => Flow::Continue,
            DialogResult::Cancel => {
                self.dialog = None;
                Flow::Continue
            }
            DialogResult::Submit(s) => {
                self.dialog = None;
                self.handle_submit(s).await;
                if self.pending_quit {
                    Flow::Quit
                } else {
                    Flow::Continue
                }
            }
            DialogResult::Abort(id) => {
                if let Some(h) = self.tasks.get(&id) {
                    h.cancel.cancel();
                }
                // Keep the progress dialog until TaskDone confirms cancellation.
                Flow::Continue
            }
        }
    }

    async fn handle_submit(&mut self, submit: Submit) {
        match submit {
            Submit::MkDir(name) => {
                let path = self.panels[self.active].cwd.join(&name);
                let backend = self.panels[self.active].backend.clone();
                match backend.mkdir(&path).await {
                    Ok(()) => {
                        let _ = self.panels[self.active].reload_keeping(Some(&name)).await;
                    }
                    Err(e) => self.show_error(format!("mkdir failed: {e}")),
                }
            }
            Submit::Copy(sources, dest) => self.begin_transfer(OpKind::Copy, sources, &dest).await,
            Submit::Move(sources, dest) => self.begin_transfer(OpKind::Move, sources, &dest).await,
            Submit::Delete(targets) => {
                if targets.iter().any(|t| t.is_archive()) {
                    self.start_archive_remove(targets);
                } else {
                    self.start_op(OpKind::Delete, targets, None, None);
                }
            }
            Submit::Compress(sources, name) => self.start_compress(sources, name),
            Submit::Quit => self.pending_quit = true,
            Submit::EditorSaveQuit => self.save_editor(true).await,
            Submit::EditorDiscardQuit => {
                self.editor = None;
                self.reload_all().await;
            }
            Submit::SelectGroup(pattern) => self.apply_select_group(&pattern, true),
            Submit::UnselectGroup(pattern) => self.apply_select_group(&pattern, false),
            Submit::Chmod(path, mode) => {
                let backend = self.panels[self.active].backend.clone();
                match backend.set_permissions(&path, mode).await {
                    Ok(()) => {
                        let _ = self.panels[self.active].reload().await;
                    }
                    Err(e) => self.show_error(format!("chmod failed: {e}")),
                }
            }
            Submit::Chown(path, owner, group) => self.apply_chown(path, &owner, &group).await,
            Submit::Symlink { dir, target, name } => {
                let backend = self.panels[self.active].backend.clone();
                let link = dir.join(&name);
                match backend.symlink(&target, &link).await {
                    Ok(()) => {
                        let _ = self.panels[self.active].reload_keeping(Some(&name)).await;
                    }
                    Err(e) => self.show_error(format!("symlink failed: {e}")),
                }
            }
            Submit::Settings(v) => {
                self.config.editor = v.editor;
                self.config.viewer = v.viewer;
                self.config.use_internal_viewer = v.use_internal_viewer;
                self.config.use_internal_editor = v.use_internal_editor;
                self.config.confirm_delete = v.confirm_delete;
                if let Err(e) = self.config.save() {
                    self.show_error(format!("could not save settings: {e}"));
                }
            }
        }
    }

    fn apply_select_group(&mut self, pattern: &str, select: bool) {
        let p = &mut self.panels[self.active];
        let res = if select {
            p.selection.select_group(&p.entries, pattern, true)
        } else {
            p.selection.unselect_group(&p.entries, pattern)
        };
        if let Err(e) = res {
            self.show_error(format!("invalid pattern: {e}"));
        }
    }

    async fn apply_chown(&mut self, path: VfsPath, owner: &str, group: &str) {
        let uid = match resolve_uid(owner) {
            Ok(u) => u,
            Err(e) => return self.show_error(e),
        };
        let gid = match resolve_gid(group) {
            Ok(g) => g,
            Err(e) => return self.show_error(e),
        };
        let backend = self.panels[self.active].backend.clone();
        match backend.set_owner(&path, uid, gid).await {
            Ok(()) => {
                let _ = self.panels[self.active].reload().await;
            }
            Err(e) => self.show_error(format!("chown failed: {e}")),
        }
    }

    async fn begin_transfer(&mut self, kind: OpKind, sources: Vec<VfsPath>, dest: &str) {
        // Resolve the destination directory (absolute, or relative to cwd).
        let dst_dir = self.resolve_dest(dest);
        // Ensure the destination directory exists (local backend).
        if let Err(e) = tokio::fs::create_dir_all(dst_dir.as_path()).await {
            self.show_error(format!("cannot create destination: {e}"));
            return;
        }
        let dst_fs = self.registry.local();
        self.start_op(kind, sources, Some(dst_fs), Some(dst_dir));
    }

    fn resolve_dest(&self, dest: &str) -> VfsPath {
        let p = std::path::Path::new(dest);
        if p.is_absolute() {
            VfsPath::local(p)
        } else {
            VfsPath::local(self.panels[self.active].cwd.path.join(dest))
        }
    }

    fn start_op(
        &mut self,
        kind: OpKind,
        sources: Vec<VfsPath>,
        dst_fs: Option<std::sync::Arc<dyn crate::vfs::Vfs>>,
        dst_dir: Option<VfsPath>,
    ) {
        if sources.is_empty() {
            self.show_error("No files selected");
            return;
        }
        let id = self.next_task_id;
        self.next_task_id += 1;
        let verb = match kind {
            OpKind::Copy => "Copying",
            OpKind::Move => "Moving",
            OpKind::Delete => "Deleting",
        };
        let req = OpRequest {
            kind,
            src_fs: self.panels[self.active].backend.clone(),
            sources,
            dst_fs,
            dst_dir,
        };
        let handle = spawn_op(id, req, self.tx.clone());
        self.tasks.insert(id, handle);
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, verb)));
    }

    fn show_error(&mut self, msg: impl Into<String>) {
        self.dialog = Some(Dialog::Message(MessageDialog::error(msg)));
    }

    fn show_info(&mut self, title: &str, msg: impl Into<String>) {
        self.dialog = Some(Dialog::Message(MessageDialog {
            title: title.to_string(),
            message: msg.into(),
            is_error: false,
        }));
    }

    async fn handle_panel_key(&mut self, key: KeyEvent) -> Flow {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            // -- Quit / function keys --
            KeyCode::F(10) => self.dialog = Some(Dialog::Confirm(ConfirmDialog::quit())),
            KeyCode::Char('q') if ctrl => return Flow::Quit, // immediate fallback if F10 is intercepted
            KeyCode::F(1) => self.show_info("Help", HELP_TEXT),
            KeyCode::F(2) => self.menu = Some(MenuBarState::new()),
            KeyCode::F(3) => return self.open_view().await,
            KeyCode::F(4) => return self.open_edit().await,
            KeyCode::F(5) => self.open_transfer_dialog(OpKind::Copy),
            KeyCode::F(6) => self.open_transfer_dialog(OpKind::Move),
            KeyCode::F(7) => self.open_mkdir(),
            KeyCode::F(8) => self.open_delete_dialog(),
            KeyCode::F(9) => self.menu = Some(MenuBarState::new()),

            // -- Panel navigation --
            KeyCode::Up => self.active_panel().move_cursor(-1),
            KeyCode::Down => self.active_panel().move_cursor(1),
            KeyCode::PageUp => self.active_panel().move_cursor(-PAGE),
            KeyCode::PageDown => self.active_panel().move_cursor(PAGE),
            KeyCode::Home => self.active_panel().move_home(),
            KeyCode::End => self.active_panel().move_end(),
            KeyCode::Insert => self.active_panel().toggle_mark_and_advance(),
            KeyCode::Tab => self.active = self.other_index(),

            // -- Enter: run command or descend --
            KeyCode::Enter => {
                if !self.cmd.is_empty() {
                    return Flow::RunCommand(self.cmd.take());
                }
                self.enter_dir().await;
            }

            // -- Command-line editing --
            KeyCode::Left => self.cmd.move_left(),
            KeyCode::Right => self.cmd.move_right(),
            KeyCode::Backspace => self.cmd.backspace(),
            KeyCode::Delete => self.cmd.delete(),
            KeyCode::Esc => self.cmd.clear(),

            // -- View / sort / layout toggles (Ctrl chords) --
            KeyCode::Char('r') if ctrl => {
                let _ = self.active_panel().reload().await;
            }
            KeyCode::Char('t') if ctrl => self.split = self.split.toggle(),
            KeyCode::Char('w') if ctrl => {
                let p = self.active_panel();
                p.format = p.format.toggle();
            }
            KeyCode::Char('s') if ctrl => self.cycle_sort(),
            KeyCode::Char('e') if ctrl => {
                let p = self.active_panel();
                p.sort.reverse = !p.sort.reverse;
                p.resort();
            }

            // -- Selection by wildcard (only when the command line is empty) --
            KeyCode::Char('+') if self.cmd.is_empty() => self.open_select_group(true),
            KeyCode::Char('-') if self.cmd.is_empty() => self.open_select_group(false),
            KeyCode::Char('*') if self.cmd.is_empty() => self.invert_selection(),

            // -- Otherwise, type into the command line --
            KeyCode::Char(c) => self.cmd.insert(c),

            _ => {}
        }
        Flow::Continue
    }

    async fn enter_dir(&mut self) {
        let p = &self.panels[self.active];
        // Directory / ".." navigation first, then "enter archive file".
        let target = p
            .target_dir_under_cursor()
            .or_else(|| archive_target_under_cursor(p));
        let Some((newcwd, focus)) = target else {
            return;
        };
        // Re-resolve the backend: navigation may cross backends (local↔archive).
        let backend = match self.registry.resolve(&newcwd) {
            Ok(b) => b,
            Err(e) => return self.show_error(e.to_string()),
        };
        let p = self.active_panel();
        p.cwd = newcwd;
        p.backend = backend;
        p.selection.clear();
        let _ = p.reload_keeping(focus.as_deref()).await;
    }

    fn cycle_sort(&mut self) {
        let p = self.active_panel();
        let cur = p.sort.key;
        let idx = SortKey::ALL.iter().position(|k| *k == cur).unwrap_or(0);
        p.sort.key = SortKey::ALL[(idx + 1) % SortKey::ALL.len()];
        p.resort();
    }

    fn open_transfer_dialog(&mut self, kind: OpKind) {
        let sources = self.panels[self.active].operation_targets();
        if sources.is_empty() {
            self.show_error("No files selected");
            return;
        }
        // Destination is an archive → add into it (rebuild), not a file copy.
        if self.panels[self.other_index()].cwd.is_archive() {
            if self.panels[self.active].cwd.is_archive() {
                self.show_error("Cannot copy directly between archives; extract first");
                return;
            }
            let dest = self.panels[self.other_index()].cwd.clone();
            self.start_archive_add(kind, sources, dest);
            return;
        }
        let dest = self.panels[self.other_index()].cwd.display();
        let (title, purpose) = match kind {
            OpKind::Copy => ("Copy", InputPurpose::CopyDest(sources)),
            OpKind::Move => ("Move", InputPurpose::MoveDest(sources)),
            OpKind::Delete => unreachable!(),
        };
        let prompt = format!("{title} to:");
        self.dialog = Some(Dialog::Input(InputDialog::new(title, prompt, dest, purpose)));
    }

    fn open_delete_dialog(&mut self) {
        let targets = self.panels[self.active].operation_targets();
        if targets.is_empty() {
            self.show_error("No files selected");
            return;
        }
        if self.config.confirm_delete {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::delete(targets)));
        } else {
            self.start_op(OpKind::Delete, targets, None, None);
        }
    }

    fn open_mkdir(&mut self) {
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Create directory",
            "Enter directory name:",
            "",
            InputPurpose::MkDir,
        )));
    }

    fn open_select_group(&mut self, select: bool) {
        let (title, prompt, purpose) = if select {
            ("Select group", "Pattern (e.g. *.txt):", InputPurpose::SelectGroup)
        } else {
            ("Unselect group", "Pattern (e.g. *.txt):", InputPurpose::UnselectGroup)
        };
        self.dialog = Some(Dialog::Input(InputDialog::new(title, prompt, "*", purpose)));
    }

    fn invert_selection(&mut self) {
        let p = &mut self.panels[self.active];
        let names: Vec<String> = p
            .entries
            .iter()
            .filter(|e| e.name != "..")
            .map(|e| e.name.clone())
            .collect();
        for n in names {
            p.selection.toggle(&n);
        }
    }

    fn open_settings(&mut self) {
        self.dialog = Some(Dialog::Form(FormDialog::settings(&self.config)));
    }

    fn open_chmod(&mut self) {
        let p = &self.panels[self.active];
        if !p.backend.capabilities().permissions {
            return self.show_error("This filesystem does not support permissions");
        }
        let Some(e) = p.current_entry() else {
            return self.show_error("No file under cursor");
        };
        if e.name == ".." {
            return self.show_error("No file under cursor");
        }
        let path = p.cwd.join(&e.name);
        let mode = e.mode.unwrap_or(0o644) & 0o777;
        self.dialog = Some(Dialog::Form(FormDialog::chmod(path, mode)));
    }

    fn open_chown(&mut self) {
        let p = &self.panels[self.active];
        if !p.backend.capabilities().ownership {
            return self.show_error("This filesystem does not support ownership");
        }
        let Some(e) = p.current_entry() else {
            return self.show_error("No file under cursor");
        };
        if e.name == ".." {
            return self.show_error("No file under cursor");
        }
        let path = p.cwd.join(&e.name);
        let owner = e
            .uid
            .and_then(uid_name)
            .unwrap_or_else(|| e.uid.map(|u| u.to_string()).unwrap_or_default());
        let group = e
            .gid
            .and_then(gid_name)
            .unwrap_or_else(|| e.gid.map(|g| g.to_string()).unwrap_or_default());
        self.dialog = Some(Dialog::Form(FormDialog::chown(path, owner, group)));
    }

    fn open_symlink(&mut self) {
        let p = &self.panels[self.active];
        if !p.backend.capabilities().symlinks {
            return self.show_error("This filesystem does not support symlinks");
        }
        let dir = p.cwd.clone();
        self.dialog = Some(Dialog::Form(FormDialog::symlink(dir)));
    }

    // -- Archives ----------------------------------------------------------

    fn open_compress(&mut self) {
        let p = &self.panels[self.active];
        if p.cwd.is_archive() {
            return self.show_error("Compress from a local directory");
        }
        let sources = p.operation_targets();
        if sources.is_empty() {
            return self.show_error("No files selected");
        }
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Compress",
            "Archive name (.zip .7z .tar.gz .tar.bz2 .tar.xz):",
            "archive.tar.gz",
            InputPurpose::Compress(sources),
        )));
    }

    fn start_compress(&mut self, sources: Vec<VfsPath>, name: String) {
        let format = match ArchiveFormat::from_name(&name) {
            Some(ArchiveFormat::Rar) => return self.show_error("Cannot create RAR archives"),
            Some(f) => f,
            None => {
                return self
                    .show_error("Unknown type (use .zip .7z .tar.gz .tar.bz2 .tar.xz)");
            }
        };
        let dest = self.panels[self.active].cwd.path.join(&name);
        let local: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
        self.spawn_archive_op("Compressing", move || {
            archive::create_archive(format, &dest, &local)
        });
    }

    fn start_archive_add(&mut self, kind: OpKind, sources: Vec<VfsPath>, dest: VfsPath) {
        let Some(container) = dest.container.clone() else {
            return self.show_error("destination is not an archive");
        };
        let dest_inner = dest.path.to_string_lossy().into_owned();
        let local: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
        let is_move = matches!(kind, OpKind::Move);
        self.spawn_archive_op("Updating archive", move || {
            archive::add_to_archive(&container, &dest_inner, &local)?;
            if is_move {
                for s in &local {
                    let _ = remove_local(s);
                }
            }
            Ok(())
        });
    }

    fn start_archive_remove(&mut self, targets: Vec<VfsPath>) {
        let Some(container) = targets.first().and_then(|t| t.container.clone()) else {
            return;
        };
        let set: HashSet<String> = targets
            .iter()
            .map(|t| t.path.to_string_lossy().into_owned())
            .collect();
        self.spawn_archive_op("Updating archive", move || {
            archive::remove_from_archive(&container, &set)
        });
    }

    /// Spawn a blocking archive mutation; shows a progress dialog and reloads
    /// panels when it finishes (via the usual `TaskDone` path).
    fn spawn_archive_op<F>(&mut self, verb: &'static str, f: F)
    where
        F: FnOnce() -> crate::util::Result<()> + Send + 'static,
    {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let outcome = match tokio::task::spawn_blocking(f).await {
                Ok(Ok(())) => TaskOutcome::Done,
                Ok(Err(e)) => TaskOutcome::Failed(e.to_string()),
                Err(e) => TaskOutcome::Failed(e.to_string()),
            };
            let _ = tx.send(AppEvent::TaskDone { id, outcome }).await;
        });
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, verb)));
    }

    /// F3: view the file under the cursor (internal viewer or external pager).
    async fn open_view(&mut self) -> Flow {
        let p = &self.panels[self.active];
        let Some(e) = p.current_entry() else {
            return Flow::Continue;
        };
        if e.kind.is_dir() {
            return Flow::Continue;
        }
        let name = e.name.clone();
        let path = p.cwd.join(&name);
        let backend = p.backend.clone();

        if self.config.wants_internal_viewer() {
            match load_file(&backend, &path).await {
                Ok(data) => self.viewer = Some(ViewerState::new(name, data)),
                Err(e) => self.show_error(format!("cannot open file: {e}")),
            }
            Flow::Continue
        } else {
            Flow::RunExternal {
                program: self.config.viewer.clone(),
                path: path.path,
            }
        }
    }

    /// F4: edit the file under the cursor with the internal editor (or a
    /// configured external editor).
    async fn open_edit(&mut self) -> Flow {
        let p = &self.panels[self.active];
        let Some(e) = p.current_entry() else {
            return Flow::Continue;
        };
        if e.kind.is_dir() {
            return Flow::Continue;
        }
        let name = e.name.clone();
        let path = p.cwd.join(&name);
        let backend = p.backend.clone();

        if self.config.wants_internal_editor() {
            match load_file(&backend, &path).await {
                Ok(data) => {
                    let text = String::from_utf8_lossy(&data).into_owned();
                    self.editor = Some(EditorState::new(name, path, &text));
                }
                Err(e) => self.show_error(format!("cannot open file: {e}")),
            }
            Flow::Continue
        } else {
            Flow::RunExternal {
                program: self.config.editor.clone(),
                path: path.path,
            }
        }
    }

    /// Persist the editor's contents to its file, optionally closing after.
    async fn save_editor(&mut self, close_after: bool) {
        let Some(ed) = self.editor.as_ref() else {
            return;
        };
        let contents = ed.contents();
        let path = ed.path.clone();
        let backend = match self.registry.resolve(&path) {
            Ok(b) => b,
            Err(e) => return self.show_error(e.to_string()),
        };
        match write_file(&backend, &path, contents.as_bytes()).await {
            Ok(()) => {
                if close_after {
                    self.editor = None;
                    self.reload_all().await;
                } else if let Some(ed) = self.editor.as_mut() {
                    ed.mark_saved();
                }
            }
            Err(e) => self.show_error(format!("save failed: {e}")),
        }
    }
}

/// Read a file fully into memory (capped just above the viewer limit).
async fn load_file(backend: &std::sync::Arc<dyn Vfs>, path: &VfsPath) -> crate::util::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let reader = backend.open_read(path).await?;
    let mut buf = Vec::new();
    reader
        .take((MAX_VIEW_BYTES + 1) as u64)
        .read_to_end(&mut buf)
        .await?;
    Ok(buf)
}

/// Write all bytes to a file, truncating/creating it.
async fn write_file(
    backend: &std::sync::Arc<dyn Vfs>,
    path: &VfsPath,
    data: &[u8],
) -> crate::util::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut w = backend
        .open_write(path, crate::vfs::WriteMeta::default())
        .await?;
    w.write_all(data).await?;
    w.flush().await?;
    Ok(())
}

/// If the cursor is on a local archive file, the path to enter it at its root.
fn archive_target_under_cursor(p: &Panel) -> Option<(VfsPath, Option<String>)> {
    if p.cwd.scheme != "file" {
        return None;
    }
    let e = p.current_entry()?;
    if e.kind != VfsKind::File {
        return None;
    }
    ArchiveFormat::from_name(&e.name)?;
    let file_path = p.cwd.path.join(&e.name);
    Some((VfsPath::archive(file_path, "/"), None))
}

/// Remove a local file or directory tree (used after a move-into-archive).
fn remove_local(p: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(p)?.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

/// Resolve a user name or numeric uid string into a uid (or `None` if empty).
fn resolve_uid(s: &str) -> Result<Option<u32>, String> {
    if s.is_empty() {
        return Ok(None);
    }
    if let Ok(n) = s.parse::<u32>() {
        return Ok(Some(n));
    }
    match nix::unistd::User::from_name(s) {
        Ok(Some(u)) => Ok(Some(u.uid.as_raw())),
        Ok(None) => Err(format!("no such user: {s}")),
        Err(e) => Err(e.to_string()),
    }
}

/// Resolve a group name or numeric gid string into a gid (or `None` if empty).
fn resolve_gid(s: &str) -> Result<Option<u32>, String> {
    if s.is_empty() {
        return Ok(None);
    }
    if let Ok(n) = s.parse::<u32>() {
        return Ok(Some(n));
    }
    match nix::unistd::Group::from_name(s) {
        Ok(Some(g)) => Ok(Some(g.gid.as_raw())),
        Ok(None) => Err(format!("no such group: {s}")),
        Err(e) => Err(e.to_string()),
    }
}

fn uid_name(uid: u32) -> Option<String> {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|u| u.name)
}

fn gid_name(gid: u32) -> Option<String> {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
        .ok()
        .flatten()
        .map(|g| g.name)
}

const HELP_TEXT: &str = "rat-commander — Tab: switch panel, Enter: open dir / run command, \
Insert: mark, F3 view, F4 edit, F5 copy, F6 move, F7 mkdir, F8 delete, F9/F2 menu, F10 quit. \
+ select group, - unselect, * invert. Ctrl-S cycle sort, Ctrl-E reverse, Ctrl-W brief/full, \
Ctrl-T split, Ctrl-R reload. In viewer: F2 wrap, F4 hex/text, F7 search, n next. \
Chmod/Chown/Symlink/Settings live in the F9 menu.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::async_bridge;

    #[tokio::test]
    async fn enters_zip_archive_and_lists_contents() {
        // Build a temp dir with a zip to browse.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_nav_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/file.txt"), b"hi").unwrap();
        std::fs::write(root.join("top.txt"), b"top").unwrap();
        let zip = root.join("test.zip");
        archive::create_archive(
            ArchiveFormat::Zip,
            &zip,
            &[root.join("sub"), root.join("top.txt")],
        )
        .unwrap();

        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.panels[0].cwd = VfsPath::local(&root);
        st.panels[0].backend = st.registry.local();
        st.panels[0].reload().await.unwrap();

        // Put the cursor on the zip and "enter" it.
        let idx = st.panels[0]
            .entries
            .iter()
            .position(|e| e.name == "test.zip")
            .unwrap();
        st.panels[0].cursor = idx;
        st.active = 0;
        st.enter_dir().await;

        assert!(st.panels[0].cwd.is_archive(), "should be inside the archive");
        let names: Vec<String> = st.panels[0].entries.iter().map(|e| e.name.clone()).collect();
        assert!(names.contains(&"sub".to_string()), "names: {names:?}");
        assert!(names.contains(&"top.txt".to_string()), "names: {names:?}");
        assert!(names.contains(&"..".to_string()), "archive has parent link");

        std::fs::remove_dir_all(&root).ok();
    }
}
