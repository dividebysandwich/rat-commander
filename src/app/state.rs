//! Application state and the key/event dispatch that drives it.

use crate::app::event::AppEvent;
use crate::config::Config;
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
            MenuAction::Edit => return self.open_edit(),
            MenuAction::Copy => self.open_transfer_dialog(OpKind::Copy),
            MenuAction::Move => self.open_transfer_dialog(OpKind::Move),
            MenuAction::Mkdir => self.open_mkdir(),
            MenuAction::Delete => self.open_delete_dialog(),
            MenuAction::Chmod => self.open_chmod(),
            MenuAction::Chown => self.open_chown(),
            MenuAction::Symlink => self.open_symlink(),
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
            Submit::Delete(targets) => self.start_op(OpKind::Delete, targets, None, None),
            Submit::Quit => self.pending_quit = true,
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
            KeyCode::F(4) => return self.open_edit(),
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
        let target = self.active_panel().target_dir_under_cursor();
        if let Some((newcwd, focus)) = target {
            let p = self.active_panel();
            p.cwd = newcwd;
            p.selection.clear();
            let _ = p.reload_keeping(focus.as_deref()).await;
        }
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

    /// F4: edit the file under the cursor. The internal editor lands in Phase 3,
    /// so for now this launches a configured external editor.
    fn open_edit(&mut self) -> Flow {
        let p = &self.panels[self.active];
        let Some(e) = p.current_entry() else {
            return Flow::Continue;
        };
        if e.kind.is_dir() {
            return Flow::Continue;
        }
        let path = p.cwd.join(&e.name);
        if self.config.editor.trim().is_empty() {
            self.show_error(
                "Internal editor arrives in Phase 3. Configure an external editor in Settings (F9 → Options).",
            );
            Flow::Continue
        } else {
            Flow::RunExternal {
                program: self.config.editor.clone(),
                path: path.path,
            }
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
