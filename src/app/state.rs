//! Application state and the key/event dispatch that drives it.

use crate::app::event::AppEvent;
use crate::ops::progress::TaskOutcome;
use crate::ops::{OpKind, OpRequest, TaskHandle, TaskId, spawn_op};
use crate::panel::sort::SortKey;
use crate::panel::Panel;
use crate::ui::cmdline::CommandLine;
use crate::ui::dialog::{
    ConfirmDialog, Dialog, DialogResult, InputDialog, InputPurpose, MessageDialog, ProgressDialog,
    Submit,
};
use crate::ui::layout::SplitDir;
use crate::ui::theme::Theme;
use crate::util::async_bridge::AppSender;
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
}

const PAGE: isize = 15;

pub struct AppState {
    pub panels: [Panel; 2],
    /// Index of the active panel (0 = left/top, 1 = right/bottom).
    pub active: usize,
    pub split: SplitDir,
    pub cmd: CommandLine,
    pub dialog: Option<Dialog>,
    pub theme: Theme,
    pub registry: Registry,
    tasks: HashMap<TaskId, TaskHandle>,
    next_task_id: TaskId,
    tx: AppSender,
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
            theme: Theme::mc(),
            registry,
            tasks: HashMap::new(),
            next_task_id: 1,
            tx,
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
        self.handle_panel_key(key).await
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
                Flow::Continue
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
            KeyCode::F(10) => return Flow::Quit,
            KeyCode::Char('q') if ctrl => return Flow::Quit, // fallback if F10 is intercepted
            KeyCode::F(1) => self.show_info("Help", HELP_TEXT),
            KeyCode::F(2) => self.show_info("Menu", "Pulldown menus arrive in Phase 2."),
            KeyCode::F(3) => self.show_info("View", "Internal viewer arrives in Phase 2."),
            KeyCode::F(4) => self.show_info("Edit", "Internal editor arrives in Phase 3."),
            KeyCode::F(5) => self.open_transfer_dialog(OpKind::Copy),
            KeyCode::F(6) => self.open_transfer_dialog(OpKind::Move),
            KeyCode::F(7) => {
                self.dialog = Some(Dialog::Input(InputDialog::new(
                    "Create directory",
                    "Enter directory name:",
                    "",
                    InputPurpose::MkDir,
                )));
            }
            KeyCode::F(8) => self.open_delete_dialog(),
            KeyCode::F(9) => self.show_info("Menu", "Pulldown menus arrive in Phase 2."),

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
        self.dialog = Some(Dialog::Confirm(ConfirmDialog::delete(targets)));
    }
}

const HELP_TEXT: &str = "rat-commander — Tab: switch panel, Enter: open dir / run command, \
Insert: mark, F5 copy, F6 move, F7 mkdir, F8 delete, F10 quit. \
Ctrl-S: cycle sort, Ctrl-E: reverse, Ctrl-W: brief/full, Ctrl-T: split, Ctrl-R: reload.";
