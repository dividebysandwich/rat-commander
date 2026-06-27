//! Application state and the key/event dispatch that drives it.

use crate::app::event::AppEvent;
use crate::config::Config;
use crate::editor::{EditorSignal, EditorState};
use crate::ops::progress::{ProgressUpdate, TaskOutcome};
use crate::ops::CancelToken;
use crate::ops::{OpKind, OpRequest, TaskHandle, TaskId, spawn_op};
use crate::panel::sort::SortKey;
use crate::panel::Panel;
use crate::ui::cmdline::CommandLine;
use crate::ui::dialog::{
    ConfirmDialog, Dialog, DialogResult, FindDialog, FindParams, FormDialog, InputDialog,
    InputPurpose, MessageDialog, OverwriteDialog, ProgressDialog, SearchReplaceDialog,
    SearchReplaceParams, SelectDialog, Submit, UserMenuDialog,
};
use crate::usermenu::{self, UserMenuEntry};
use crate::ui::layout::SplitDir;
use crate::ui::menu::{MenuAction, MenuBarState, MenuSignal};
use crate::ui::theme::Theme;
use crate::util::async_bridge::AppSender;
use crate::viewer::{MAX_VIEW_BYTES, ViewerSignal, ViewerState};
use crate::vfs::Vfs;
use crate::vfs::archive::{self, formats::ArchiveFormat};
use crate::vfs::remote::RemoteCreds;
use crate::vfs::{VfsEntry, VfsKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use crate::vfs::registry::Registry;
use crate::vfs::VfsPath;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::collections::HashMap;

/// What the run loop should do after handling input.
pub enum Flow {
    Continue,
    Quit,
    /// Suspend the TUI and run this shell command in the active panel's cwd.
    RunCommand(String),
    /// Suspend the TUI and run an external program against a file.
    RunExternal { program: String, path: std::path::PathBuf },
    /// Ctrl-O: drop to an interactive subshell, full screen.
    SubShell,
}

const PAGE: isize = 15;

/// What a mouse point/drag on a panel should do to the entry under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointAction {
    /// Move the cursor only (left click / left drag).
    Cursor,
    /// Invert the entry's mark, once per entry entered during the gesture
    /// (right click / right drag — paint inverting).
    InvertPaint,
}

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
    next_session_id: usize,
    tx: AppSender,
    /// Whether the terminal supports 24-bit color (for gradients).
    pub truecolor: bool,
    /// Animation frame counter (drives the gradient motion).
    pub anim_phase: usize,
    tick_count: usize,
    /// CPU/memory sampler for the status widget.
    pub sampler: crate::util::sysinfo::SysSampler,
    /// Theme name to restore if the settings dialog is cancelled (live preview).
    theme_backup: Option<String>,
    /// F2 user-menu entries (loaded from the config `menu` file).
    user_menu: Vec<UserMenuEntry>,
    /// A user-menu command to run after the dialog closes (expanded).
    pending_run: Option<String>,
    /// Set when a confirmed quit should propagate out as `Flow::Quit`.
    pending_quit: bool,
    /// When a lone Esc has been pressed and we're waiting to see whether the
    /// next key is a digit (Esc-prefix function-key alias, MC style).
    pending_esc: Option<Instant>,
    /// The progress dialog set aside while an overwrite prompt is shown; restored
    /// once the user answers so the operation's progress keeps displaying.
    stashed_progress: Option<ProgressDialog>,
    /// The full terminal area from the last render, used to hit-test mouse clicks
    /// against menus and centered dialogs.
    pub last_area: Rect,
    /// The (panel, entry) last toggled by a right-drag paint, so each entry is
    /// inverted only once as the drag passes over it.
    paint_last: Option<(usize, usize)>,
}

/// How long a lone Esc is held, waiting for a digit, before it is delivered as
/// a plain Esc. Matches Midnight Commander's Esc-as-function-key behavior.
const ESC_PREFIX_TIMEOUT: Duration = Duration::from_millis(400);

/// Map a key code to a function-key number for the Esc-prefix aliases:
/// `1`..`9` => F1..F9, `0` => F10.
fn fkey_for_code(code: KeyCode) -> Option<u8> {
    match code {
        KeyCode::Char(c @ '1'..='9') => Some(c as u8 - b'0'),
        KeyCode::Char('0') => Some(10),
        _ => None,
    }
}

fn synth_fkey(n: u8) -> KeyEvent {
    KeyEvent::new(KeyCode::F(n), KeyModifiers::NONE)
}

fn esc_key() -> KeyEvent {
    KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
}

/// Parse a command-line `cd` built-in. Returns the (possibly empty) argument
/// when `cmd` is exactly the `cd` command, or `None` for anything else.
fn parse_cd(cmd: &str) -> Option<&str> {
    let t = cmd.trim();
    if t == "cd" {
        return Some("");
    }
    t.strip_prefix("cd ").map(str::trim)
}

/// The user's home directory (`$HOME` / `%USERPROFILE%`), or `/` as a fallback.
fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Lexically resolve `.` and `..` components (no filesystem access), so a
/// `cd ../foo` produces a clean absolute path rather than one littered with `..`.
fn normalize_path(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push("/");
    }
    out
}

/// Detect 24-bit color support from the environment.
fn detect_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v.contains("truecolor") || v.contains("24bit"))
        .unwrap_or(false)
}

impl AppState {
    pub fn new(tx: AppSender) -> Self {
        let registry = Registry::new();
        let local = registry.local();
        let cwd = VfsPath::local_cwd();
        let left = Panel::new(local.clone(), cwd.clone());
        let right = Panel::new(local, cwd);
        let config = Config::load();
        let truecolor = config.truecolor.unwrap_or_else(detect_truecolor);
        let theme = Theme::by_name(&config.theme, truecolor);
        AppState {
            panels: [left, right],
            active: 0,
            split: SplitDir::Vertical,
            cmd: CommandLine::new(),
            dialog: None,
            viewer: None,
            editor: None,
            menu: None,
            theme,
            config,
            registry,
            tasks: HashMap::new(),
            next_task_id: 1,
            next_session_id: 0,
            tx,
            truecolor,
            anim_phase: 0,
            tick_count: 0,
            sampler: crate::util::sysinfo::SysSampler::new(),
            theme_backup: None,
            user_menu: usermenu::load_or_create(),
            pending_run: None,
            pending_quit: false,
            pending_esc: None,
            stashed_progress: None,
            last_area: Rect::new(0, 0, 0, 0),
            paint_last: None,
        }
    }

    /// Periodic tick (~100 ms): advances animation and samples system stats.
    /// Returns true when something visible changed (so the loop can redraw).
    pub fn on_tick(&mut self) -> bool {
        let mut dirty = false;
        if self.config.animation && self.truecolor {
            self.anim_phase = self.anim_phase.wrapping_add(1);
            dirty = true;
        }
        if self.config.system_status {
            self.tick_count = self.tick_count.wrapping_add(1);
            // Sample roughly every 500 ms.
            if self.tick_count.is_multiple_of(5) {
                self.sampler.sample();
                dirty = true;
            }
        }
        dirty
    }

    /// Whether the loop needs periodic ticks at all (animation or stats on).
    pub fn wants_ticks(&self) -> bool {
        (self.config.animation && self.truecolor)
            || self.config.system_status
            || self.pending_esc.is_some()
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
            AppEvent::Conflict(info) => {
                // The engine is paused awaiting a decision. Stash the progress
                // dialog and raise the overwrite prompt over it.
                if let Some(Dialog::Progress(p)) = self.dialog.take() {
                    self.stashed_progress = Some(p);
                }
                self.dialog = Some(Dialog::Overwrite(OverwriteDialog::new(info)));
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
            AppEvent::FindDone { id, paths } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                self.panelize_results(paths);
            }
        }
    }

    // -- Key handling ------------------------------------------------------

    /// Top-level key entry point. Implements Midnight-Commander-style Esc-prefix
    /// function-key aliases (Esc-1..Esc-9 => F1..F9, Esc-0 => F10) before
    /// dispatching to the active mode. The aliases are active in the base modes
    /// (panels, editor, viewer); dialogs and the pulldown menu keep Esc as an
    /// immediate cancel.
    pub async fn handle_key(&mut self, key: KeyEvent) -> Flow {
        let prefixable = self.dialog.is_none() && self.menu.is_none();
        if prefixable {
            if self.pending_esc.take().is_some() {
                // The previous key was a lone Esc; this key completes the
                // sequence. A digit becomes the matching function key.
                if let Some(n) = fkey_for_code(key.code) {
                    return self.route_key(synth_fkey(n)).await;
                }
                // Otherwise deliver the held Esc, then this key normally.
                let _ = self.route_key(esc_key()).await;
                return self.route_key(key).await;
            }
            if key.code == KeyCode::Esc && key.modifiers.is_empty() {
                // Hold the Esc; the next key (or a tick timeout) resolves it.
                self.pending_esc = Some(Instant::now());
                return Flow::Continue;
            }
            // Fast path: terminals send Esc+digit pressed together as Alt+digit.
            if key.modifiers.contains(KeyModifiers::ALT)
                && let Some(n) = fkey_for_code(key.code)
            {
                return self.route_key(synth_fkey(n)).await;
            }
        }
        self.route_key(key).await
    }

    /// Deliver a held Esc once its function-key window has elapsed without a
    /// following key (called from the event loop's tick).
    pub async fn flush_expired_esc(&mut self) -> Flow {
        if let Some(t) = self.pending_esc
            && t.elapsed() >= ESC_PREFIX_TIMEOUT
        {
            self.pending_esc = None;
            return self.route_key(esc_key()).await;
        }
        Flow::Continue
    }

    /// Handle a mouse event. Left clicks/drags move the cursor and drive the
    /// menus and dialogs; right clicks/drags mark files.
    pub async fn handle_mouse(&mut self, ev: MouseEvent) -> Flow {
        let area = self.last_area;
        let (col, row) = (ev.column, ev.row);
        let left_down = matches!(ev.kind, MouseEventKind::Down(MouseButton::Left));

        // A modal dialog gets first claim on a left click.
        if self.dialog.is_some() {
            if left_down {
                let res = self.dialog.as_mut().unwrap().handle_click(area, col, row);
                // Live theme preview, mirroring the keyboard path.
                if let Some(Dialog::Form(fd)) = &self.dialog
                    && let Some(name) = fd.theme_choice()
                    && name != self.theme.name
                {
                    self.theme = Theme::by_name(name, self.truecolor);
                }
                return self.handle_dialog_result(res).await;
            }
            return Flow::Continue;
        }

        // Then the pulldown menu.
        if self.menu.is_some() {
            if left_down {
                let signal = self.menu.as_mut().unwrap().click(area, col, row);
                return match signal {
                    MenuSignal::Stay => Flow::Continue,
                    MenuSignal::Close => {
                        self.menu = None;
                        Flow::Continue
                    }
                    MenuSignal::Activate(action) => {
                        self.menu = None;
                        self.run_menu_action(action).await
                    }
                };
            }
            return Flow::Continue;
        }

        // A fresh press starts a new gesture; forget the last painted entry.
        if matches!(ev.kind, MouseEventKind::Down(_)) {
            self.paint_last = None;
        }

        // Base mode: the menu bar, then the file panels.
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // A click on the menu bar (top row) opens that menu.
                if let Some(i) = MenuBarState::title_index_at(area, col, row) {
                    self.menu = Some(MenuBarState::new(i));
                } else {
                    self.panel_point(col, row, PointAction::Cursor);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.panel_point(col, row, PointAction::Cursor)
            }
            MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Drag(MouseButton::Right) => {
                self.panel_point(col, row, PointAction::InvertPaint)
            }
            _ => {}
        }
        Flow::Continue
    }

    /// Map a screen point to a panel entry: activate that panel, move the cursor
    /// onto the entry (every action), and optionally toggle/paint its mark.
    fn panel_point(&mut self, col: u16, row: u16, action: PointAction) {
        let pi = if self.panels[0].hit.is_some_and(|h| h.in_panel(col, row)) {
            0
        } else if self.panels[1].hit.is_some_and(|h| h.in_panel(col, row)) {
            1
        } else {
            return;
        };
        self.active = pi;
        let p = &mut self.panels[pi];
        let Some(hit) = p.hit else { return };
        let Some(idx) = hit.index_at(col, row, p.entries.len()) else {
            return;
        };
        // The cursor follows the pointer for every action (incl. drags).
        p.cursor = idx;
        if matches!(action, PointAction::Cursor) {
            return;
        }
        // Invert the mark, but only once per entry as the drag enters it, so a
        // run of drag events over the same file doesn't flip it repeatedly.
        if self.paint_last == Some((pi, idx)) {
            return;
        }
        self.paint_last = Some((pi, idx));
        let p = &mut self.panels[pi];
        // Selection never touches the "..".
        if let Some(e) = p.entries.get(idx)
            && e.name != ".."
        {
            let name = e.name.clone();
            p.selection.toggle(&name);
        }
    }

    async fn route_key(&mut self, key: KeyEvent) -> Flow {
        if self.dialog.is_some() {
            let res = self.dialog.as_mut().unwrap().handle_key(key);
            // Live theme preview: apply the settings form's current theme choice.
            if let Some(Dialog::Form(fd)) = &self.dialog
                && let Some(name) = fd.theme_choice()
                && name != self.theme.name
            {
                self.theme = Theme::by_name(name, self.truecolor);
            }
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
                EditorSignal::OpenSearch => {
                    self.dialog =
                        Some(Dialog::SearchReplace(SearchReplaceDialog::new(false, String::new())));
                }
                EditorSignal::OpenReplace => {
                    self.dialog =
                        Some(Dialog::SearchReplace(SearchReplaceDialog::new(true, String::new())));
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
            MenuAction::FindFile => self.open_find_dialog(),
            MenuAction::Connect(side, proto) => {
                self.dialog = Some(Dialog::Form(FormDialog::connect(proto, side)))
            }
            MenuAction::Disconnect(side) => self.disconnect(side).await,
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
                // Revert a live theme preview when the settings dialog is cancelled.
                if let Some(name) = self.theme_backup.take() {
                    self.theme = Theme::by_name(&name, self.truecolor);
                }
                Flow::Continue
            }
            DialogResult::Submit(s) => {
                self.dialog = None;
                self.theme_backup = None; // keep any previewed theme
                self.handle_submit(s).await;
                if self.pending_quit {
                    Flow::Quit
                } else if let Some(cmd) = self.pending_run.take() {
                    Flow::RunCommand(cmd)
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
            DialogResult::Overwrite(id, decision) => {
                // Send the decision back to the paused engine, then restore the
                // operation's progress dialog. (On Abort, TaskDone will close it.)
                if let Some(h) = self.tasks.get(&id) {
                    let _ = h.reply.try_send(decision);
                }
                self.dialog = self.stashed_progress.take().map(Dialog::Progress);
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
            Submit::Connect(side, creds) => self.connect_remote(side, creds).await,
            Submit::UserCommand(tpl) => self.pending_run = Some(self.expand_macros(&tpl)),
            Submit::Quit => self.pending_quit = true,
            Submit::EditorSaveQuit => self.save_editor(true).await,
            Submit::EditorDiscardQuit => {
                self.editor = None;
                self.reload_all().await;
            }
            Submit::Select {
                select,
                pattern,
                files_only,
                case_sensitive,
                shell,
            } => self.apply_select(select, &pattern, files_only, case_sensitive, shell),
            Submit::SearchReplace(p) => self.apply_search_replace(p),
            Submit::Find(p) => self.start_find(p),
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
                self.config.theme = v.theme;
                self.config.truecolor = Some(v.truecolor);
                self.config.animation = v.animation;
                self.config.system_status = v.system_status;
                self.truecolor = v.truecolor;
                // Re-theme the running UI immediately.
                self.theme = Theme::by_name(&self.config.theme, self.truecolor);
                if let Err(e) = self.config.save() {
                    self.show_error(format!("could not save settings: {e}"));
                }
            }
        }
    }

    fn apply_select(
        &mut self,
        select: bool,
        pattern: &str,
        files_only: bool,
        case_sensitive: bool,
        shell: bool,
    ) {
        let p = &mut self.panels[self.active];
        let res = if select {
            p.selection
                .select_group(&p.entries, pattern, files_only, case_sensitive, shell)
        } else {
            p.selection
                .unselect_group(&p.entries, pattern, case_sensitive, shell)
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
            KeyCode::F(1) => self.open_help(),
            KeyCode::F(2) => self.open_user_menu(),
            KeyCode::F(3) => return self.open_view().await,
            KeyCode::F(4) => return self.open_edit().await,
            KeyCode::F(5) => self.open_transfer_dialog(OpKind::Copy),
            KeyCode::F(6) => self.open_transfer_dialog(OpKind::Move),
            KeyCode::F(7) => self.open_mkdir(),
            KeyCode::F(8) => self.open_delete_dialog(),
            KeyCode::F(9) => self.open_menu(),

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
                    let cmd = self.cmd.take();
                    // A built-in `cd` changes the active panel instead of being
                    // run in a (throwaway) subshell where it would have no effect.
                    if let Some(arg) = parse_cd(&cmd) {
                        self.change_dir(arg).await;
                        return Flow::Continue;
                    }
                    return Flow::RunCommand(cmd);
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
            KeyCode::Char('o') if ctrl => return Flow::SubShell,
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
            // Not a directory/archive: open a local file with its default app.
            self.open_with_default();
            return;
        };
        // Re-resolve the backend: navigation may cross backends (local↔archive).
        let backend = match self.registry.resolve(&newcwd) {
            Ok(b) => b,
            Err(e) => return self.show_error(e.to_string()),
        };
        // Atomic move: if the target can't be listed (e.g. permission denied),
        // the panel stays where it is rather than getting stuck in it.
        self.active_panel()
            .try_enter(newcwd, backend, focus.as_deref())
            .await;
    }

    /// Handle a `cd` typed at the command line: change the active panel's
    /// directory. Supports `cd` / `cd ~` (home), `cd /abs`, `cd rel`, and `cd ..`.
    /// If the target can't be listed, the panel stays put (no blocking error).
    async fn change_dir(&mut self, arg: &str) {
        let arg = arg.trim();
        let cur = self.panels[self.active].cwd.clone();

        let newcwd: VfsPath = if cur.scheme == "file" {
            let target: PathBuf = if arg.is_empty() || arg == "~" {
                home_dir()
            } else if let Some(rest) = arg.strip_prefix("~/") {
                home_dir().join(rest)
            } else {
                let raw = Path::new(arg);
                if raw.is_absolute() {
                    raw.to_path_buf()
                } else {
                    cur.path.join(raw)
                }
            };
            VfsPath::local(normalize_path(&target))
        } else {
            // Inside an archive/remote backend: support `..` and relative joins.
            match arg {
                "" | "~" => return,
                ".." => match cur.parent() {
                    Some(p) => p,
                    None => return,
                },
                _ => cur.join(arg),
            }
        };

        let backend = match self.registry.resolve(&newcwd) {
            Ok(b) => b,
            Err(e) => return self.show_error(e.to_string()),
        };
        self.active_panel().try_enter(newcwd, backend, None).await;
    }

    /// Open the local file under the cursor with the system default program
    /// (xdg-open), but only if a MIME handler is actually defined for it. Runs
    /// detached so the TUI keeps running.
    fn open_with_default(&self) {
        let p = &self.panels[self.active];
        if p.cwd.scheme != "file" {
            return;
        }
        let Some(e) = p.current_entry() else {
            return;
        };
        if e.kind != VfsKind::File {
            return;
        }
        let path = p.cwd.path.join(&e.name);
        tokio::spawn(async move { launch_default(path).await });
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
            return;
        }
        // A search-result panel is not a real destination directory.
        if self.panels[self.other_index()].is_panelized() {
            self.show_error("Cannot copy into a search-result panel");
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
        self.dialog = Some(Dialog::Select(SelectDialog::new(select)));
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
        // Remember the current theme so Esc can revert a live preview.
        self.theme_backup = Some(self.config.theme.clone());
        self.dialog = Some(Dialog::Form(FormDialog::settings(&self.config, self.truecolor)));
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
            return;
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

    // -- Remote connections ------------------------------------------------

    async fn connect_remote(&mut self, side: usize, creds: RemoteCreds) {
        match crate::vfs::remote::connect(&creds).await {
            Ok(conn) => {
                let scheme = format!("{}-{}", creds.protocol.scheme_prefix(), self.next_session_id);
                self.next_session_id += 1;
                self.registry.register(scheme.clone(), conn.backend.clone());
                let cwd = VfsPath {
                    scheme,
                    path: PathBuf::from(&conn.root),
                    container: None,
                };
                let p = &mut self.panels[side];
                p.cwd = cwd;
                p.backend = conn.backend;
                p.selection.clear();
                let _ = p.reload().await;
            }
            Err(e) => self.show_error(format!("Connection failed: {e}")),
        }
    }

    async fn disconnect(&mut self, side: usize) {
        if self.panels[side].cwd.scheme == "file" {
            return;
        }
        let local = self.registry.local();
        let p = &mut self.panels[side];
        p.cwd = VfsPath::local_cwd();
        p.backend = local;
        p.selection.clear();
        let _ = p.reload().await;
    }

    fn open_menu(&mut self) {
        // F9 opens the pulldown menu matching the active panel: Left (0)/Right (4).
        let active = if self.active == 0 { 0 } else { 4 };
        self.menu = Some(MenuBarState::new(active));
    }

    fn open_user_menu(&mut self) {
        if self.user_menu.is_empty() {
            return self.show_error("No user-menu entries (see the config 'menu' file)");
        }
        self.dialog = Some(Dialog::UserMenu(UserMenuDialog::new(self.user_menu.clone())));
    }

    /// Expand mc-style menu macros against the active panel.
    fn expand_macros(&self, tpl: &str) -> String {
        use crate::vfs::remote::shell_quote;
        let p = &self.panels[self.active];
        let cwd = p.cwd.path.to_string_lossy().into_owned();
        let cur = p.current_entry().map(|e| e.name.clone()).unwrap_or_default();
        let marked: Vec<String> = p.selection.marked_names(&p.entries).iter().map(|n| shell_quote(n)).collect();
        let tagged = marked.join(" ");
        let selected = if marked.is_empty() {
            shell_quote(&cur)
        } else {
            tagged.clone()
        };

        // Scan for %X macros (%% → literal %).
        let mut out = String::with_capacity(tpl.len());
        let mut chars = tpl.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => out.push('%'),
                Some('f') | Some('p') => out.push_str(&cur),
                Some('d') => out.push_str(&cwd),
                Some('t') => out.push_str(&tagged),
                Some('s') => out.push_str(&selected),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        }
        out
    }

    fn open_find_dialog(&mut self) {
        let start = if self.panels[self.active].cwd.scheme == "file" {
            self.panels[self.active].cwd.path.to_string_lossy().into_owned()
        } else {
            String::new()
        };
        self.dialog = Some(Dialog::Find(FindDialog::new(start)));
    }

    fn apply_search_replace(&mut self, p: SearchReplaceParams) {
        if let Some(ed) = self.editor.as_mut() {
            ed.apply_search_replace(
                p.replace,
                &p.search,
                &p.replacement,
                p.regex,
                p.case_sensitive,
                p.whole_words,
                p.backwards,
            );
        }
    }

    /// Launch a cancellable find-file search; a progress dialog shows the
    /// current path and lets the user abort. Results arrive via `FindDone`.
    fn start_find(&mut self, p: FindParams) {
        let start = if p.start_at.trim().is_empty() {
            self.panels[self.active].cwd.path.clone()
        } else {
            PathBuf::from(&p.start_at)
        };
        let matcher =
            match crate::panel::selection::NameMatcher::build(&p.file_name, p.case_sensitive, p.shell) {
                Ok(m) => m,
                Err(e) => return self.show_error(format!("invalid pattern: {e}")),
            };

        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = CancelToken::new();
        // Find tasks never prompt for overwrite; an unused reply channel keeps
        // the handle shape uniform.
        let (reply, _reply_rx) = tokio::sync::mpsc::channel(1);
        self.tasks.insert(
            id,
            TaskHandle {
                id,
                cancel: cancel.clone(),
                reply,
            },
        );
        self.dialog = Some(Dialog::Progress(ProgressDialog::find(id)));

        let tx = self.tx.clone();
        tokio::spawn(async move {
            let tx2 = tx.clone();
            let paths = tokio::task::spawn_blocking(move || {
                find_files(&start, &p, &matcher, &cancel, |cur, found| {
                    let _ = tx2.try_send(AppEvent::Progress(ProgressUpdate {
                        id,
                        verb: "Searching",
                        current_name: cur,
                        file_done: 0,
                        file_total: 0,
                        total_done: 0,
                        total_total: 0,
                        files_done: found as u64,
                        files_total: 0,
                    }));
                })
            })
            .await
            .unwrap_or_default();
            let _ = tx.send(AppEvent::FindDone { id, paths }).await;
        });
    }

    /// Panelize find-file results (with a `..` entry that returns to browsing).
    fn panelize_results(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return self.show_error("No files found");
        }
        let cwd = self.panels[self.active].cwd.clone();
        let mut entries = vec![VfsEntry {
            name: "..".to_string(),
            kind: VfsKind::Dir,
            size: 0,
            mtime: None,
            atime: None,
            ctime: None,
            inode: None,
            mode: None,
            uid: None,
            gid: None,
            symlink_target: None,
            symlink_broken: false,
        }];
        let mut vpaths = vec![cwd]; // dummy path paired with ".."
        for path in paths {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            entries.push(VfsEntry {
                name: path.to_string_lossy().into_owned(),
                kind: VfsKind::File,
                size,
                mtime: None,
                atime: None,
                ctime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
            });
            vpaths.push(VfsPath::local(path));
        }
        let local = self.registry.local();
        let p = &mut self.panels[self.active];
        p.backend = local;
        p.set_results(entries, vpaths);
    }

    /// F1: show the help screen (reuses the scrollable text viewer).
    fn open_help(&mut self) {
        self.viewer = Some(ViewerState::new(
            HELP_NAME.to_string(),
            HELP_TEXT.as_bytes().to_vec(),
        ));
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

/// Recursively find files under `start`, reporting progress and honouring
/// cancellation. Returns whatever was collected (partial on abort).
fn find_files(
    start: &Path,
    p: &FindParams,
    matcher: &crate::panel::selection::NameMatcher,
    cancel: &crate::ops::CancelToken,
    mut progress: impl FnMut(String, usize),
) -> Vec<PathBuf> {
    const MAX_RESULTS: usize = 50_000;

    let content_needle = if p.content.is_empty() {
        None
    } else if p.case_sensitive {
        Some(p.content.clone())
    } else {
        Some(p.content.to_lowercase())
    };

    let mut walker = walkdir::WalkDir::new(start);
    if !p.recursive {
        walker = walker.max_depth(1);
    }
    let mut out = Vec::new();
    let mut scanned = 0usize;
    for entry in walker.into_iter().filter_entry(|e| {
        // Skip hidden files/dirs (but never the start dir itself).
        !(p.skip_hidden && e.depth() > 0 && e.file_name().to_string_lossy().starts_with('.'))
    }) {
        if cancel.is_cancelled() {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        // Throttle progress reporting (every 64 entries scanned).
        scanned += 1;
        if scanned.is_multiple_of(64) {
            progress(entry.path().to_string_lossy().into_owned(), out.len());
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !matcher.is_match(&name) {
            continue;
        }
        if let Some(needle) = &content_needle {
            match std::fs::read(entry.path()) {
                Ok(bytes) => {
                    let hay = String::from_utf8_lossy(&bytes);
                    let hay = if p.case_sensitive {
                        hay.into_owned()
                    } else {
                        hay.to_lowercase()
                    };
                    if !hay.contains(needle.as_str()) {
                        continue;
                    }
                }
                Err(_) => continue,
            }
        }
        out.push(entry.path().to_path_buf());
        progress(entry.path().to_string_lossy().into_owned(), out.len());
        if out.len() >= MAX_RESULTS {
            break;
        }
    }
    out
}

/// Open `path` with the system default application (detached), if one exists.
#[cfg(target_os = "linux")]
async fn launch_default(path: PathBuf) {
    // Only launch when a MIME handler is actually defined for the file.
    if has_mime_handler(&path).await {
        let _ = tokio::process::Command::new("xdg-open")
            .arg(&path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }
}

#[cfg(target_os = "macos")]
async fn launch_default(path: PathBuf) {
    let _ = tokio::process::Command::new("open").arg(&path).spawn();
}

#[cfg(windows)]
async fn launch_default(path: PathBuf) {
    // `cmd /C start "" "<path>"` opens the file with its registered handler.
    let _ = tokio::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(&path)
        .spawn();
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn launch_default(_path: PathBuf) {}

/// Whether the system has a default MIME handler for `path`.
#[cfg(target_os = "linux")]
async fn has_mime_handler(path: &Path) -> bool {
    let Ok(ft) = tokio::process::Command::new("xdg-mime")
        .args(["query", "filetype"])
        .arg(path)
        .output()
        .await
    else {
        return false;
    };
    let mime = String::from_utf8_lossy(&ft.stdout).trim().to_string();
    if mime.is_empty() {
        return false;
    }
    let Ok(def) = tokio::process::Command::new("xdg-mime")
        .args(["query", "default", &mime])
        .output()
        .await
    else {
        return false;
    };
    !String::from_utf8_lossy(&def.stdout).trim().is_empty()
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
#[cfg(unix)]
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
#[cfg(unix)]
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

#[cfg(not(unix))]
fn resolve_uid(_s: &str) -> Result<Option<u32>, String> {
    Err("ownership is not supported on this platform".to_string())
}

#[cfg(not(unix))]
fn resolve_gid(_s: &str) -> Result<Option<u32>, String> {
    Err("ownership is not supported on this platform".to_string())
}

#[cfg(unix)]
fn uid_name(uid: u32) -> Option<String> {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid))
        .ok()
        .flatten()
        .map(|u| u.name)
}

#[cfg(unix)]
fn gid_name(gid: u32) -> Option<String> {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid))
        .ok()
        .flatten()
        .map(|g| g.name)
}

#[cfg(not(unix))]
fn uid_name(_uid: u32) -> Option<String> {
    None
}

#[cfg(not(unix))]
fn gid_name(_gid: u32) -> Option<String> {
    None
}

const HELP_TEXT: &str = "\
rat-commander — Help
====================

PANEL NAVIGATION
  Up/Down/PgUp/PgDn/Home/End   move the cursor
  Enter                        open directory / enter archive / run command line
  Tab                          switch active panel
  Insert                       mark file and advance
  + / - / *                    select group / unselect group / invert selection
  Ctrl-R                       re-read the active panel
  Ctrl-S / Ctrl-E              cycle sort key / toggle reverse
  Ctrl-W                       toggle brief / full listing
  Ctrl-T                       toggle vertical / horizontal split

FUNCTION KEYS
  F1  Help (this screen)       F2  User menu (config 'menu' file)
  F3  View file                F4  Edit file
  F5  Copy                     F6  Rename / move
  F7  Make directory           F8  Delete
  F9  Pulldown menu            F10 Quit
  Esc then 1..9 / 0            alias for F1..F9 / F10 (works in editor & viewer)
  Ctrl-O                       toggle the persistent subshell (Ctrl-O to return)
  Ctrl-Q                       quit immediately

VIEWER (F3)
  F2 wrap   F4 hex/text   F7 search   n next   Esc/F10 quit

EDITOR (F4)
  F2 save   F3 mark block   F5 copy   F6 move   F8 delete block
  F4 search & replace   F7 search   Ctrl-Z/Ctrl-Y undo/redo
  Ctrl-V paste   Esc/F10 quit (prompts if modified)

ARCHIVES
  Enter an archive file (.zip .tar.gz .tar.bz2 .tar.xz .7z .rar) to browse it.
  Copy files into/out of an archive panel; Delete removes from the archive.
  F9 -> File -> Compress... builds a new archive from the selection.
  RAR is read-only (no tool can create RAR archives).

REMOTE (F9 -> Command)
  SFTP / FTP / SCP connection... opens a login dialog and mounts the server in
  the active panel. Disconnect returns the panel to the local filesystem.
  Copy/move/delete work between local, remote and archive panels.

FIND FILE (F9 -> Command)
  Searches recursively with a progress dialog (Esc/Enter aborts; results so
  far are kept). Results open in the panel; the .. entry returns to browsing.

OTHER (F9 -> File / Options)
  Chmod, Chown, Symlink, Compress, and Settings (theme, external editor/
  viewer, confirm-delete). Many color themes are available; with a truecolor
  terminal the bars and cursor use a gradient.

Press Esc or F10 to close this help.";

const HELP_NAME: &str = "Help (F1)";

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

    #[cfg(unix)]
    #[tokio::test]
    async fn cannot_enter_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_perm_{}_{nanos}", std::process::id()));
        let secret = root.join("secret");
        std::fs::create_dir_all(&secret).unwrap();
        std::fs::write(root.join("visible.txt"), b"hi").unwrap();
        // Remove all permissions on the subdirectory.
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();

        // If we can still read it (e.g. running as root), the scenario doesn't
        // apply — skip rather than assert a false negative.
        let denied = std::fs::read_dir(&secret).is_err();

        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.panels[0].cwd = VfsPath::local(&root);
        st.panels[0].backend = st.registry.local();
        st.panels[0].reload().await.unwrap();
        st.active = 0;

        let idx = st.panels[0]
            .entries
            .iter()
            .position(|e| e.name == "secret")
            .unwrap();
        st.panels[0].cursor = idx;
        st.enter_dir().await;

        if denied {
            assert_eq!(
                st.panels[0].cwd.path, root,
                "should not have entered the unreadable directory"
            );
            assert!(st.panels[0].error.is_none(), "no error should be left behind");
            // The listing is intact so the user can keep navigating.
            assert!(
                st.panels[0].entries.iter().any(|e| e.name == "visible.txt"),
                "panel listing should be preserved"
            );
        }

        // Restore permissions so cleanup can remove the tree.
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o755)).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn parse_cd_recognizes_the_builtin() {
        assert_eq!(parse_cd("cd"), Some(""));
        assert_eq!(parse_cd("cd /tmp"), Some("/tmp"));
        assert_eq!(parse_cd("  cd   foo  "), Some("foo"));
        assert_eq!(parse_cd("cdfoo"), None);
        assert_eq!(parse_cd("ls"), None);
    }

    #[test]
    fn normalize_path_resolves_dotdot() {
        assert_eq!(normalize_path(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize_path(Path::new("/a/./b")), PathBuf::from("/a/b"));
    }

    #[tokio::test]
    async fn cd_changes_active_panel_directory() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_cd_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("child")).unwrap();

        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.active = 0;
        st.panels[0].cwd = VfsPath::local(&root);
        st.panels[0].backend = st.registry.local();
        st.panels[0].reload().await.unwrap();

        // Relative cd descends.
        st.change_dir("child").await;
        assert_eq!(st.panels[0].cwd.path, root.join("child"));
        // `cd ..` ascends back.
        st.change_dir("..").await;
        assert_eq!(st.panels[0].cwd.path, root);
        // cd to a non-existent directory leaves the panel where it is.
        st.change_dir("nope").await;
        assert_eq!(st.panels[0].cwd.path, root);

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn mouse_clicks_move_cursor_and_mark_in_panel() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_mouse_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for n in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            std::fs::write(root.join(n), b"x").unwrap();
        }

        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.active = 1; // start on the other panel to prove activation switches
        st.panels[0].cwd = VfsPath::local(&root);
        st.panels[0].backend = st.registry.local();
        st.panels[0].reload().await.unwrap();

        // Render once to populate the panel hit geometry.
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();

        let hit = st.panels[0].hit.expect("panel hit recorded");
        // Aim at the third visible row.
        let col = hit.body.x + 1;
        let row = hit.body.y + 2;
        let target = hit.index_at(col, row, st.panels[0].entries.len()).unwrap();

        // Left-click moves the cursor there and activates the left panel.
        st.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
        .await;
        assert_eq!(st.active, 0, "left panel should become active");
        assert_eq!(st.panels[0].cursor, target, "cursor should jump to clicked row");

        // Right-click marks the entry under the pointer.
        let name = st.panels[0].entries[target].name.clone();
        st.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        })
        .await;
        assert!(st.panels[0].selection.is_marked(&name), "right-click marks the file");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn right_drag_inverts_selection_across_files() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_drag_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for n in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            std::fs::write(root.join(n), b"x").unwrap();
        }

        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.active = 0;
        st.panels[0].cwd = VfsPath::local(&root);
        st.panels[0].backend = st.registry.local();
        st.panels[0].reload().await.unwrap();
        // Pre-select a.txt and b.txt.
        st.panels[0].selection.mark("a.txt");
        st.panels[0].selection.mark("b.txt");

        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
        let hit = st.panels[0].hit.expect("hit");

        let col = hit.body.x + 1;
        // Press on a.txt, then drag across a (again), b, then c.
        for (kind, name) in [
            (MouseEventKind::Down(MouseButton::Right), "a.txt"),
            (MouseEventKind::Drag(MouseButton::Right), "a.txt"), // same cell: no double-flip
            (MouseEventKind::Drag(MouseButton::Right), "b.txt"),
            (MouseEventKind::Drag(MouseButton::Right), "c.txt"),
        ] {
            let idx = st.panels[0].entries.iter().position(|e| e.name == name).unwrap();
            let row = hit.body.y + (idx - hit.offset) as u16;
            st.handle_mouse(MouseEvent { kind, column: col, row, modifiers: KeyModifiers::NONE })
                .await;
        }

        let sel = &st.panels[0].selection;
        assert!(!sel.is_marked("a.txt"), "a was selected → inverted off");
        assert!(!sel.is_marked("b.txt"), "b was selected → inverted off");
        assert!(sel.is_marked("c.txt"), "c was unselected → inverted on");
        assert!(!sel.is_marked("d.txt"), "d untouched");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn mouse_click_on_menu_bar_opens_menu() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.last_area = Rect::new(0, 0, 120, 30);
        assert!(st.menu.is_none());
        // The "File" title sits a few columns in on the top row.
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 8,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        st.handle_mouse(click).await;
        assert!(st.menu.is_some(), "clicking the menu bar should open a menu");
    }

    #[test]
    fn find_files_by_name_and_content() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_find_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), b"hello there").unwrap();
        std::fs::write(root.join("sub/b.txt"), b"world").unwrap();
        std::fs::write(root.join("c.log"), b"hello again").unwrap();

        let run = |p: &FindParams| {
            let m = crate::panel::selection::NameMatcher::build(&p.file_name, p.case_sensitive, p.shell)
                .unwrap();
            let c = crate::ops::CancelToken::new();
            find_files(&root, p, &m, &c, |_, _| {})
        };

        let by_name = FindParams {
            start_at: String::new(),
            file_name: "*.txt".into(),
            content: String::new(),
            recursive: true,
            case_sensitive: false,
            skip_hidden: true,
            shell: true,
        };
        assert_eq!(run(&by_name).len(), 2, "two .txt files");

        let by_content = FindParams {
            file_name: "*".into(),
            content: "HELLO".into(),
            ..by_name
        };
        assert_eq!(run(&by_content).len(), 2, "two files contain 'hello'");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn theme_preview_applies_and_reverts_on_cancel() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        let original = st.theme.name.clone();
        st.open_settings();
        // The Theme choice is the first (focused) field; Space cycles it.
        st.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)).await;
        assert_ne!(st.theme.name, original, "theme preview should apply live");
        // Esc cancels → revert to the original theme.
        st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        assert_eq!(st.theme.name, original, "cancel should revert the preview");
    }

    #[tokio::test]
    async fn f1_opens_help_in_viewer() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        assert!(st.viewer.is_none());
        st.open_help();
        assert!(st.viewer.is_some(), "F1 should open the help viewer");
    }

    #[test]
    fn esc_prefix_maps_digits_to_function_keys() {
        assert_eq!(fkey_for_code(KeyCode::Char('1')), Some(1));
        assert_eq!(fkey_for_code(KeyCode::Char('9')), Some(9));
        assert_eq!(fkey_for_code(KeyCode::Char('0')), Some(10));
        assert_eq!(fkey_for_code(KeyCode::Char('a')), None);
        assert_eq!(fkey_for_code(KeyCode::Esc), None);
    }

    #[tokio::test]
    async fn esc_then_digit_acts_as_function_key() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        assert!(st.viewer.is_none());
        // A lone Esc (no dialog/menu) is held, not acted on immediately.
        st.handle_key(esc_key()).await;
        assert!(st.pending_esc.is_some(), "lone Esc should be held");
        // The following '1' completes Esc-1 => F1 => help viewer.
        st.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE))
            .await;
        assert!(st.viewer.is_some(), "Esc-1 should act as F1 (help)");
        assert!(st.pending_esc.is_none(), "the sequence is resolved");
    }

    #[tokio::test]
    async fn alt_digit_acts_as_function_key() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        assert!(st.viewer.is_none());
        // Terminals deliver a fast Esc+digit as Alt+digit; that is an F-key too.
        st.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT))
            .await;
        assert!(st.viewer.is_some(), "Alt-1 should act as F1 (help)");
        assert!(st.pending_esc.is_none());
    }

    #[tokio::test]
    async fn esc_then_nondigit_delivers_plain_esc() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        for c in "abc".chars() {
            st.cmd.insert(c);
        }
        st.handle_key(esc_key()).await;
        assert!(st.pending_esc.is_some());
        // A non-digit resolves the held Esc as a plain Esc (clears the cmd line)
        // and then delivers the key itself.
        st.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .await;
        assert!(st.pending_esc.is_none());
        assert_eq!(st.cmd.buffer, "x", "Esc cleared the line, then 'x' was typed");
    }
}
