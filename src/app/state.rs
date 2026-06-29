//! Application state and the key/event dispatch that drives it.

use crate::app::event::{AppEvent, FetchKind};
use crate::config::Config;
use crate::editor::{EditorSignal, EditorState};
use crate::ops::progress::{ProgressUpdate, TaskOutcome};
use crate::ops::CancelToken;
use crate::ops::{OpKind, OpRequest, TaskHandle, TaskId, spawn_op};
use crate::diff::{DiffSignal, DiffView};
use crate::disk::{DiskSignal, DiskView};
use crate::mount::{MountSignal, MountView};
use crate::panel::sort::SortKey;
use crate::panel::Panel;
use crate::proc::{ProcSignal, ProcView};
use crate::ui::cmdline::CommandLine;
use crate::ui::dialog::{
    BusyDialog, CompareDialog, CompareMode, ConfirmDialog, Dialog, DialogResult, FindDialog,
    FindParams, FormDialog, InputDialog, InputPurpose, MessageDialog, OverwriteDialog,
    ProgressDialog, SearchReplaceDialog, SearchReplaceParams, SelectDialog, Submit, UserMenuDialog,
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

/// What a mouse point/drag on a panel should do to the entry under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PointAction {
    /// Move the cursor only (left click / left drag).
    Cursor,
    /// Invert the entry's mark, once per entry entered during the gesture
    /// (right click / right drag — paint inverting).
    InvertPaint,
}

/// A privileged disk-manager command awaiting a sudo password, plus the message
/// to show on success and the "busy" label to display while it runs.
struct PendingPriv {
    cmd: String,
    ok_msg: String,
    busy: String,
}

/// How to execute a privileged command on the background task.
enum PrivExec {
    /// Already root: run the command directly.
    Root(String),
    /// Escalate via `sudo` without a password (cached/`NOPASSWD`).
    SudoNonInteractive(String),
    /// Escalate via `sudo -S`, feeding the given password on stdin.
    SudoPassword(String, String),
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
    /// The full-screen process explorer, when open.
    pub procview: Option<ProcView>,
    /// The full-screen disk-usage explorer, when open.
    pub diskview: Option<DiskView>,
    /// The full-screen side-by-side file comparison view, when open.
    pub diffview: Option<DiffView>,
    /// The full-screen disk-mounter tool, when open.
    pub mountview: Option<MountView>,
    /// A privileged command queued while prompting for a sudo password.
    pending_sudo: Option<PendingPriv>,
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
    /// After a delete completes, place the active panel's cursor on this entry
    /// (the surviving file just above the deleted one) instead of the top.
    pending_focus: Option<String>,
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

/// Split a `scheme://rest` prefix into `(scheme, rest)`. Only a plausible scheme
/// token (alphanumerics, `-`, `+`, `.`) is accepted, so ordinary local paths
/// (which never contain `://` on Unix) are left alone.
fn split_scheme(s: &str) -> Option<(&str, &str)> {
    let idx = s.find("://")?;
    let scheme = &s[..idx];
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.'))
    {
        return None;
    }
    Some((scheme, &s[idx + 3..]))
}

/// Decide the destination [`VfsPath`] for a typed copy/move target.
///
/// - `scheme://path` resolves onto that backend — the path is absolute, or
///   relative to a panel already on that scheme.
/// - A bare path drops onto the destination panel's backend as before — *unless*
///   that panel is remote, in which case a bare path is taken as **local** (a
///   relative one joins the source panel's directory when it is local, else the
///   process cwd). This lets the user override a remote destination to a local
///   one simply by deleting the `scheme://` prefix from the prefilled field.
fn dest_vfspath(dest: &str, other_cwd: &VfsPath, active_cwd: &VfsPath) -> VfsPath {
    if let Some((scheme, rest)) = split_scheme(dest) {
        let base_path = [other_cwd, active_cwd]
            .into_iter()
            .find(|c| c.scheme == scheme && c.container.is_none())
            .map(|c| c.path.clone())
            .unwrap_or_else(|| PathBuf::from("/"));
        return resolve_dest_on(
            rest,
            &VfsPath { scheme: scheme.to_string(), path: base_path, container: None },
        );
    }
    let other_is_remote = other_cwd.container.is_none() && other_cwd.scheme != "file";
    if other_is_remote {
        // The scheme was stripped → treat as a local destination.
        let base = if active_cwd.scheme == "file" {
            VfsPath::local(active_cwd.path.clone())
        } else {
            VfsPath::local_cwd()
        };
        resolve_dest_on(dest, &base)
    } else {
        resolve_dest_on(dest, other_cwd)
    }
}

/// Resolve a typed destination string onto the destination panel's backend
/// (`base`). Absolute paths replace the path; relative ones are joined to the
/// panel's current directory. The scheme/container of `base` are preserved, so a
/// remote destination stays on its remote backend instead of becoming local.
fn resolve_dest_on(dest: &str, base: &VfsPath) -> VfsPath {
    let p = Path::new(dest);
    let path = if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.path.join(dest)
    };
    if base.scheme == "file" {
        VfsPath::local(path)
    } else {
        VfsPath {
            scheme: base.scheme.clone(),
            path,
            container: base.container.clone(),
        }
    }
}

/// Whether two files differ in content, streaming both (early-exit on the first
/// mismatch). Unreadable files are treated as differing. Callers should compare
/// sizes first so this only runs for same-size files.
async fn files_differ(
    ba: &std::sync::Arc<dyn Vfs>,
    pa: &VfsPath,
    bb: &std::sync::Arc<dyn Vfs>,
    pb: &VfsPath,
) -> bool {
    let (mut ra, mut rb) = match (ba.open_read(pa).await, bb.open_read(pb).await) {
        (Ok(a), Ok(b)) => (a, b),
        _ => return true,
    };
    let mut bufa = vec![0u8; 64 * 1024];
    let mut bufb = vec![0u8; 64 * 1024];
    loop {
        let na = read_filled(&mut ra, &mut bufa).await;
        let nb = read_filled(&mut rb, &mut bufb).await;
        if na != nb || bufa[..na] != bufb[..nb] {
            return true;
        }
        if na == 0 {
            return false; // both reached EOF in lockstep
        }
    }
}

/// Read until `buf` is full or EOF/error; returns how many bytes were read.
async fn read_filled<R: tokio::io::AsyncRead + Unpin>(r: &mut R, buf: &mut [u8]) -> usize {
    use tokio::io::AsyncReadExt;
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]).await {
            Ok(0) | Err(_) => break,
            Ok(n) => filled += n,
        }
    }
    filled
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
            procview: None,
            diskview: None,
            diffview: None,
            mountview: None,
            pending_sudo: None,
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
            pending_focus: None,
        }
    }

    /// Periodic tick (~100 ms): advances animation and samples system stats.
    /// Returns true when something visible changed (so the loop can redraw).
    pub fn on_tick(&mut self) -> bool {
        let mut dirty = false;
        self.tick_count = self.tick_count.wrapping_add(1);
        // Animate gradients when truecolor is on and either animations are
        // enabled, the (always-animated) process explorer is open, or a file
        // operation is running (so the progress bars pulse).
        let scanning_disk = self.diskview.as_ref().is_some_and(|d| d.scanning);
        let animate = self.truecolor
            && (self.config.animation
                || self.procview.is_some()
                || !self.tasks.is_empty()
                || scanning_disk);
        if animate {
            self.anim_phase = self.anim_phase.wrapping_add(1);
            dirty = true;
        }
        if self.config.system_status && self.tick_count.is_multiple_of(5) {
            // Sample roughly every 500 ms.
            self.sampler.sample();
            dirty = true;
        }
        // Refresh the process explorer on its (user-adjustable) interval.
        if let Some(pv) = self.procview.as_mut() {
            if pv.tick_due() {
                pv.refresh();
            }
            dirty = true;
        }
        // Keep the disk-mounter lists fresh (~every 500 ms), unless a dialog is
        // open over it (e.g. entering a path), to avoid the lists shifting.
        if self.dialog.is_none()
            && self.tick_count.is_multiple_of(5)
            && let Some(mv) = self.mountview.as_mut()
        {
            mv.refresh();
            dirty = true;
        }
        // Spin the "working…" dialog while a privileged op runs.
        if let Some(Dialog::Busy(b)) = self.dialog.as_mut() {
            b.tick();
            dirty = true;
        }
        dirty
    }

    /// Whether the loop needs periodic ticks at all (animation or stats on).
    pub fn wants_ticks(&self) -> bool {
        (self.config.animation && self.truecolor)
            || self.config.system_status
            || self.pending_esc.is_some()
            || self.procview.is_some()
            || self.mountview.is_some()
            || !self.tasks.is_empty()
            || matches!(self.dialog, Some(Dialog::Busy(_)))
            || self.diskview.as_ref().is_some_and(|d| d.scanning)
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

    /// Whether the active UI theme has a dark background (picks a fitting syntax
    /// highlighting theme).
    fn dark_ui(&self) -> bool {
        if let ratatui::style::Color::Rgb(r, g, b) = self.theme.panel_bg {
            let luma = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
            luma < 128.0
        } else {
            true
        }
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
                // After a delete, drop the cursor onto the file above the
                // deleted one rather than letting it snap to the top.
                if let Some(name) = self.pending_focus.take() {
                    let p = &mut self.panels[self.active];
                    if let Some(i) = p.entries.iter().position(|e| e.name == name) {
                        p.cursor = i;
                    }
                }
            }
            AppEvent::PrivilegedDone { ok_msg, result } => {
                // Dismiss the busy spinner, then report on the manager's status.
                if matches!(self.dialog, Some(Dialog::Busy(_))) {
                    self.dialog = None;
                }
                self.finish_privileged(result, ok_msg);
            }
            AppEvent::FindDone { id, results } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                self.panelize_results(results);
            }
            AppEvent::DiskScanProgress { generation, done, total } => {
                if let Some(dv) = self.diskview.as_mut()
                    && dv.generation == generation
                    && dv.scanning
                {
                    dv.scan_done = done;
                    dv.scan_total = total;
                }
            }
            AppEvent::DiskScanned { generation, entries } => {
                if let Some(dv) = self.diskview.as_mut()
                    && dv.generation == generation
                {
                    dv.entries = entries;
                    dv.scanning = false;
                    dv.selected = 0;
                }
            }
            AppEvent::FileFetched { id, kind, name, orig_path, temp } => {
                self.tasks.remove(&id);
                if let Some(Dialog::Progress(p)) = &self.dialog
                    && p.id == id
                {
                    self.dialog = None;
                }
                match kind {
                    FetchKind::View => {
                        // Page the downloaded copy from disk; it's deleted on close.
                        let dark = self.dark_ui();
                        let t = temp.clone();
                        let scanned =
                            tokio::task::spawn_blocking(move || crate::viewer::scan_file(&t)).await;
                        match scanned {
                            Ok(Ok((file, len, line_starts))) => {
                                let mut v = ViewerState::from_scanned(
                                    name,
                                    file,
                                    len,
                                    line_starts,
                                    Some(temp.clone()),
                                );
                                v.enable_syntax(dark);
                                self.viewer = Some(v);
                            }
                            Ok(Err(e)) => {
                                let _ = std::fs::remove_file(&temp);
                                self.show_error(format!("cannot open file: {e}"));
                            }
                            Err(_) => {
                                let _ = std::fs::remove_file(&temp);
                                self.show_error("viewer failed to open file");
                            }
                        }
                    }
                    FetchKind::Edit => {
                        // The editor edits in memory; read the temp then drop it.
                        // Saving still targets the original (remote) path.
                        match std::fs::read(&temp) {
                            Ok(bytes) => {
                                let text = String::from_utf8_lossy(&bytes).into_owned();
                                let mut ed = EditorState::new(name, orig_path, &text);
                                ed.enable_syntax(self.dark_ui());
                                self.editor = Some(ed);
                            }
                            Err(e) => self.show_error(format!("cannot open file: {e}")),
                        }
                        let _ = std::fs::remove_file(&temp);
                    }
                }
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

        // The disk manager handles its own clicks (cursor + double-click menus).
        if self.mountview.is_some() {
            let sig = self.mountview.as_mut().unwrap().handle_mouse(ev);
            self.apply_mount_signal(sig).await;
            return Flow::Continue;
        }

        // The editor and viewer handle their own mouse (cursor/marking/scroll).
        if self.editor.is_some() {
            let sig = self.editor.as_mut().unwrap().handle_mouse(ev);
            self.apply_editor_signal(sig).await;
            return Flow::Continue;
        }
        if let Some(v) = self.viewer.as_mut() {
            if let ViewerSignal::Close = v.handle_mouse(ev) {
                self.viewer = None;
            }
            return Flow::Continue;
        }

        // The remaining full-screen overlays don't use the mouse yet; swallow the
        // event so it can't move the hidden file-panel cursor underneath them.
        if self.procview.is_some() || self.diskview.is_some() || self.diffview.is_some() {
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

    /// Apply an [`EditorSignal`] (from a key or a mouse gesture): save, close,
    /// or raise the relevant modal dialog.
    async fn apply_editor_signal(&mut self, signal: EditorSignal) {
        match signal {
            EditorSignal::Stay => {}
            EditorSignal::Close => {
                self.editor = None;
                self.reload_all().await;
            }
            EditorSignal::Save { close_after } => {
                if close_after {
                    self.save_editor(true).await;
                } else {
                    let name = self.editor.as_ref().map(|e| e.name.clone()).unwrap_or_default();
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::save_editor(&name)));
                }
            }
            EditorSignal::ConfirmQuit => {
                let name = self.editor.as_ref().map(|e| e.name.clone()).unwrap_or_default();
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::editor_quit(&name)));
            }
            EditorSignal::OpenSearch => {
                self.dialog = Some(Dialog::SearchReplace(self.editor_search_dialog(false)));
            }
            EditorSignal::OpenReplace => {
                self.dialog = Some(Dialog::SearchReplace(self.editor_search_dialog(true)));
            }
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
            self.apply_editor_signal(signal).await;
            return Flow::Continue;
        }
        if let Some(v) = self.viewer.as_mut() {
            if let ViewerSignal::Close = v.handle_key(key) {
                self.viewer = None;
            }
            return Flow::Continue;
        }
        if let Some(pv) = self.procview.as_mut() {
            match pv.handle_key(key) {
                ProcSignal::Stay => {}
                ProcSignal::Close => self.procview = None,
                ProcSignal::Kill { pid, name, force } => {
                    self.dialog =
                        Some(Dialog::Confirm(ConfirmDialog::kill(pid, &name, force)));
                }
            }
            return Flow::Continue;
        }
        if self.diskview.is_some() {
            let sig = self.diskview.as_mut().unwrap().handle_key(key);
            match sig {
                DiskSignal::Stay => {}
                DiskSignal::Close => self.diskview = None,
                DiskSignal::Rescan => self.start_disk_scan(),
                DiskSignal::GoTo(path) => {
                    self.diskview = None;
                    let backend = self.registry.local();
                    self.active_panel()
                        .try_enter(VfsPath::local(path), backend, None)
                        .await;
                }
            }
            return Flow::Continue;
        }
        if self.diffview.is_some() {
            match self.diffview.as_mut().unwrap().handle_key(key) {
                DiffSignal::Stay => {}
                DiffSignal::Close => self.diffview = None,
                DiffSignal::Save => {
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::save_diff()));
                }
                DiffSignal::ConfirmQuit => {
                    self.dialog = Some(Dialog::Confirm(ConfirmDialog::diff_quit()));
                }
            }
            return Flow::Continue;
        }
        if self.mountview.is_some() {
            let sig = self.mountview.as_mut().unwrap().handle_key(key);
            self.apply_mount_signal(sig).await;
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
            MenuAction::ProcExplorer => self.open_proc_explorer(),
            MenuAction::DiskExplorer => self.open_disk_explorer(),
            MenuAction::DiskManager => self.mountview = Some(MountView::new()),
            MenuAction::CompareDirs => self.dialog = Some(Dialog::Compare(CompareDialog::new())),
            MenuAction::CompareFiles => self.open_compare_files().await,
            MenuAction::Connect(side, proto) => {
                self.dialog = Some(Dialog::Form(FormDialog::connect(
                    proto,
                    side,
                    self.config.recent_remotes.clone(),
                )))
            }
            MenuAction::Disconnect(side) => self.disconnect(side).await,
            MenuAction::Settings => self.open_settings(),
            MenuAction::Confirmations => self.open_confirmations(),
            MenuAction::Quit => return self.request_quit(),
        }
        Flow::Continue
    }

    /// Quit, prompting for confirmation only when `confirm_exit` is enabled.
    fn request_quit(&mut self) -> Flow {
        if self.config.confirm_exit {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::quit()));
            Flow::Continue
        } else {
            Flow::Quit
        }
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
            Submit::KillProcess { pid, force } => self.kill_process(pid, force),
            Submit::CompareDirs(mode) => self.compare_dirs(mode).await,
            Submit::Quit => self.pending_quit = true,
            Submit::EditorSaveQuit => self.save_editor(true).await,
            Submit::EditorSave => self.save_editor(false).await,
            Submit::DiffSave => self.save_diff().await,
            Submit::DiffSaveQuit => {
                self.save_diff().await;
                self.diffview = None;
            }
            Submit::DiffDiscardQuit => self.diffview = None,
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
                // The symlink is created in `dir` (the destination panel), so use
                // that location's backend.
                match self.registry.resolve(&dir) {
                    Ok(backend) => {
                        let link = dir.join(&name);
                        match backend.symlink(&target, &link).await {
                            Ok(()) => self.reload_all().await,
                            Err(e) => self.show_error(format!("symlink failed: {e}")),
                        }
                    }
                    Err(e) => self.show_error(e.to_string()),
                }
            }
            Submit::Settings(v) => {
                self.config.editor = v.editor;
                self.config.viewer = v.viewer;
                self.config.use_internal_viewer = v.use_internal_viewer;
                self.config.use_internal_editor = v.use_internal_editor;
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
            Submit::Confirmations(v) => {
                self.config.confirm_delete = v.delete;
                self.config.confirm_overwrite = v.overwrite;
                self.config.confirm_execute = v.execute;
                self.config.confirm_unmount = v.unmount;
                self.config.confirm_exit = v.exit;
                if let Err(e) = self.config.save() {
                    self.show_error(format!("could not save settings: {e}"));
                }
            }
            Submit::OpenWith(path) => {
                tokio::spawn(async move { launch_default(path).await });
            }
            Submit::Mount { device, path } => {
                // Create the mount point first if it doesn't exist (with consent).
                if std::path::Path::new(&path).exists() {
                    self.do_mount(device, path, false).await;
                } else {
                    self.dialog =
                        Some(Dialog::Confirm(ConfirmDialog::create_mountpoint(&device, &path)));
                }
            }
            Submit::MountCreate { device, path } => self.do_mount(device, path, true).await,
            Submit::SudoPassword(password) => self.run_pending_sudo(password).await,
            Submit::MountDevice(device) => self.prompt_mount_path(device),
            Submit::FormatDevice(device) => {
                self.dialog = Some(Dialog::Form(FormDialog::format(device)));
            }
            Submit::AskUnmount(mountpoint) => self.ask_unmount(mountpoint).await,
            Submit::DoUnmount(mountpoint) => self.do_unmount(mountpoint).await,
            Submit::SyncPath(mountpoint) => self.do_sync(mountpoint).await,
            Submit::Format(spec) => {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::format(spec)));
            }
            Submit::DoFormat(spec) => self.do_format(spec).await,
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
        // The destination defaults to the *other* panel's backend, but a typed
        // `scheme://` prefix (or its absence on a remote panel) can redirect it
        // to any registered backend — letting a local path override a remote one.
        let other = self.other_index();
        let active = self.active;
        let dst_dir = dest_vfspath(dest, &self.panels[other].cwd, &self.panels[active].cwd);
        let dst_fs = match self.registry.resolve(&dst_dir) {
            Ok(b) => b,
            Err(e) => {
                self.show_error(format!("cannot resolve destination: {e}"));
                return;
            }
        };
        // Only the local backend needs (and supports) creating the directory up
        // front; remote/other backends copy into an existing directory.
        if dst_dir.scheme == "file"
            && let Err(e) = tokio::fs::create_dir_all(dst_dir.as_path()).await
        {
            self.show_error(format!("cannot create destination: {e}"));
            return;
        }
        self.start_op(kind, sources, Some(dst_fs), Some(dst_dir));
    }

    /// The name of the first surviving entry above the cursor (skipping `..` and
    /// any entry being deleted), used to reposition the cursor after a delete.
    fn delete_anchor(&self, targets: &[VfsPath]) -> Option<String> {
        let doomed: HashSet<String> = targets.iter().map(|t| t.file_name()).collect();
        let p = &self.panels[self.active];
        (0..p.cursor).rev().find_map(|i| {
            let name = &p.entries[i].name;
            (name != ".." && !doomed.contains(name)).then(|| name.clone())
        })
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
        // For a delete, remember the surviving entry just above the deleted one
        // so the cursor lands there (not at the top) once the listing reloads.
        if kind == OpKind::Delete {
            self.pending_focus = self.delete_anchor(&sources);
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
            overwrite_all: !self.config.confirm_overwrite,
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
            KeyCode::F(10) => return self.request_quit(),
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
            KeyCode::PageUp => {
                let p = self.active_panel();
                let step = p.page.max(1) as isize;
                p.move_cursor(-step);
            }
            KeyCode::PageDown => {
                let p = self.active_panel();
                let step = p.page.max(1) as isize;
                p.move_cursor(step);
            }
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

    /// Open the full-screen process explorer.
    fn open_proc_explorer(&mut self) {
        self.procview = Some(ProcView::new());
    }

    // -- Disk manager ------------------------------------------------------

    /// Prompt for the path to mount `device` at (suggesting `/mnt/<name>`).
    fn prompt_mount_path(&mut self, device: String) {
        let base = device.rsplit('/').next().unwrap_or("disk");
        let suggest = format!("/mnt/{base}");
        self.dialog = Some(Dialog::Input(InputDialog::new(
            "Mount",
            format!("Mount {device} at:"),
            suggest,
            InputPurpose::MountPath(device),
        )));
    }

    /// Mount `device` at `path` (optionally creating the mount point first),
    /// escalating with sudo when not running as root.
    async fn do_mount(&mut self, device: String, path: String, create: bool) {
        let q = crate::mount::shell_quote;
        let cmd = if create {
            format!("mkdir -p {p} && mount {d} {p}", p = q(&path), d = q(&device))
        } else {
            format!("mount {} {}", q(&device), q(&path))
        };
        let busy = format!("Mounting {device}...");
        self.run_privileged(cmd, format!("Mounted {device} at {path}"), busy).await;
    }

    /// Apply a [`MountSignal`] produced by the disk manager (from a key or a
    /// mouse gesture): open the relevant action dialog, unmount, or close.
    async fn apply_mount_signal(&mut self, sig: MountSignal) {
        match sig {
            MountSignal::Stay => {}
            MountSignal::Close => self.mountview = None,
            MountSignal::DeviceMenu(d) => {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::device_menu(
                    &d.name,
                    &d.dev,
                    d.mountpoint.as_deref(),
                )));
            }
            MountSignal::MountMenu(mountpoint) => {
                self.dialog = Some(Dialog::Confirm(ConfirmDialog::mount_menu(&mountpoint)));
            }
            MountSignal::Unmount(mountpoint) => self.ask_unmount(mountpoint).await,
        }
    }

    /// Unmount `mountpoint`, prompting for confirmation when enabled. Essential
    /// system mount points always raise a loud red warning, regardless of the
    /// confirmation setting.
    async fn ask_unmount(&mut self, mountpoint: String) {
        if crate::mount::is_essential_mount(&mountpoint) {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::unmount_danger(&mountpoint)));
        } else if self.config.confirm_unmount {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::unmount(&mountpoint)));
        } else {
            self.do_unmount(mountpoint).await;
        }
    }

    /// Unmount the filesystem at `mountpoint`.
    async fn do_unmount(&mut self, mountpoint: String) {
        let cmd = format!("umount {}", crate::mount::shell_quote(&mountpoint));
        let busy = format!("Unmounting {mountpoint}...");
        self.run_privileged(cmd, format!("Unmounted {mountpoint}"), busy).await;
    }

    /// Flush filesystem buffers for `mountpoint` (no privileges needed).
    async fn do_sync(&mut self, mountpoint: String) {
        let cmd = format!("sync -f {}", crate::mount::shell_quote(&mountpoint));
        let result = crate::mount::run_shell(&cmd).await;
        self.finish_privileged(result, format!("Synced {mountpoint}"));
    }

    /// Run a confirmed format request (creating the chosen filesystem).
    async fn do_format(&mut self, spec: crate::mount::FormatSpec) {
        let ok = format!("Formatted {} as {}", spec.dev, spec.fs.label());
        let busy = format!("Formatting {} as {}...", spec.dev, spec.fs.label());
        let cmd = crate::mount::format_command(&spec);
        self.run_privileged(cmd, ok, busy).await;
    }

    /// Run a privileged `sh -c` command: directly when root, via non-interactive
    /// sudo when possible, otherwise queue it and prompt for a sudo password.
    /// The command runs on a background task (showing `busy` meanwhile) so the
    /// UI keeps redrawing while a slow operation like `mkfs` runs.
    async fn run_privileged(&mut self, cmd: String, ok_msg: String, busy: String) {
        if crate::mount::is_root() {
            self.spawn_privileged(PrivExec::Root(cmd), ok_msg, busy);
        } else if crate::mount::sudo_can_noninteractive().await {
            self.spawn_privileged(PrivExec::SudoNonInteractive(cmd), ok_msg, busy);
        } else {
            // Need a password: stash the command and prompt for it.
            self.pending_sudo = Some(PendingPriv { cmd, ok_msg, busy });
            self.dialog = Some(Dialog::Input(InputDialog::password(
                "Authentication required",
                "Enter sudo password:",
                InputPurpose::SudoPassword,
            )));
        }
    }

    /// Run the queued privileged command with the entered sudo `password`.
    async fn run_pending_sudo(&mut self, password: String) {
        let Some(p) = self.pending_sudo.take() else {
            return;
        };
        self.spawn_privileged(PrivExec::SudoPassword(p.cmd, password), p.ok_msg, p.busy);
    }

    /// Show a busy spinner and run a privileged command on a background task,
    /// reporting its result back through [`AppEvent::PrivilegedDone`].
    fn spawn_privileged(&mut self, exec: PrivExec, ok_msg: String, busy: String) {
        self.dialog = Some(Dialog::Busy(BusyDialog::new("Please wait", busy)));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match exec {
                PrivExec::Root(cmd) => crate::mount::run_shell(&cmd).await,
                PrivExec::SudoNonInteractive(cmd) => {
                    crate::mount::run_sudo_noninteractive(&cmd).await
                }
                PrivExec::SudoPassword(cmd, pw) => crate::mount::run_sudo_password(&cmd, &pw).await,
            };
            let _ = tx.send(AppEvent::PrivilegedDone { ok_msg, result }).await;
        });
    }

    /// Report the outcome of a privileged op on the mounter's status line and
    /// refresh its lists.
    fn finish_privileged(&mut self, result: Result<(), String>, ok_msg: String) {
        match self.mountview.as_mut() {
            Some(mv) => {
                mv.refresh();
                mv.status = match result {
                    Ok(()) => ok_msg,
                    Err(e) => format!("Error: {e}"),
                };
            }
            None => {
                if let Err(e) = result {
                    self.show_error(e);
                }
            }
        }
    }

    /// Open the full-screen disk-usage explorer at the active panel's directory.
    fn open_disk_explorer(&mut self) {
        let p = &self.panels[self.active];
        let cwd = if p.cwd.scheme == "file" {
            p.cwd.path.clone()
        } else {
            std::env::current_dir().unwrap_or_else(|_| home_dir())
        };
        self.diskview = Some(DiskView::new(cwd));
        self.start_disk_scan();
    }

    /// Kick off a background scan of the disk explorer's current directory.
    fn start_disk_scan(&mut self) {
        let Some(dv) = self.diskview.as_mut() else {
            return;
        };
        dv.generation = dv.generation.wrapping_add(1);
        dv.scanning = true;
        dv.scan_done = 0;
        dv.scan_total = 0;
        dv.entries.clear();
        dv.selected = 0;
        let generation = dv.generation;
        let cwd = dv.cwd.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let txp = tx.clone();
            let entries = tokio::task::spawn_blocking(move || {
                crate::disk::scan_dir_with(&cwd, |done, total| {
                    // Progress is advisory; drop updates if the channel is full.
                    let _ = txp.try_send(AppEvent::DiskScanProgress { generation, done, total });
                })
            })
            .await
            .unwrap_or_default();
            let _ = tx.send(AppEvent::DiskScanned { generation, entries }).await;
        });
    }

    /// Kill a process (from the explorer), then refresh the listing.
    fn kill_process(&mut self, pid: i32, force: bool) {
        #[cfg(unix)]
        {
            use nix::sys::signal::{Signal, kill};
            use nix::unistd::Pid;
            let sig = if force { Signal::SIGKILL } else { Signal::SIGTERM };
            let _ = kill(Pid::from_raw(pid), sig);
        }
        #[cfg(not(unix))]
        {
            let _ = (pid, force);
        }
        if let Some(pv) = self.procview.as_mut() {
            pv.refresh();
        }
    }

    /// Compare the two panels' files and mark the differing ones (selection).
    /// `Quick` marks files missing from the other panel; `Size` additionally
    /// marks the larger of two differently-sized files; `Content` marks both
    /// files whenever their bytes differ.
    async fn compare_dirs(&mut self, mode: CompareMode) {
        if self.panels[0].is_panelized() || self.panels[1].is_panelized() {
            return self.show_error("Cannot compare search-result panels");
        }
        let files = |p: &Panel| -> Vec<(String, u64)> {
            p.entries
                .iter()
                .filter(|e| e.kind == VfsKind::File && e.name != "..")
                .map(|e| (e.name.clone(), e.size))
                .collect()
        };
        let a = files(&self.panels[0]);
        let b = files(&self.panels[1]);
        let amap: HashMap<&str, u64> = a.iter().map(|(n, s)| (n.as_str(), *s)).collect();
        let bmap: HashMap<&str, u64> = b.iter().map(|(n, s)| (n.as_str(), *s)).collect();

        let mut mark_a: Vec<String> = Vec::new();
        let mut mark_b: Vec<String> = Vec::new();

        // Files present in only one panel are always marked there.
        for (n, _) in &a {
            if !bmap.contains_key(n.as_str()) {
                mark_a.push(n.clone());
            }
        }
        for (n, _) in &b {
            if !amap.contains_key(n.as_str()) {
                mark_b.push(n.clone());
            }
        }

        match mode {
            CompareMode::Quick => {}
            CompareMode::Size => {
                for (n, sa) in &a {
                    if let Some(sb) = bmap.get(n.as_str()) {
                        // Mark only the larger of the two.
                        if sa > sb {
                            mark_a.push(n.clone());
                        } else if sb > sa {
                            mark_b.push(n.clone());
                        }
                    }
                }
            }
            CompareMode::Content => {
                let ba = self.panels[0].backend.clone();
                let ca = self.panels[0].cwd.clone();
                let bb = self.panels[1].backend.clone();
                let cb = self.panels[1].cwd.clone();
                for (n, sa) in &a {
                    if let Some(sb) = bmap.get(n.as_str()) {
                        // Different sizes ⇒ different content (no need to read).
                        let differ = sa != sb
                            || files_differ(&ba, &ca.join(n), &bb, &cb.join(n)).await;
                        if differ {
                            mark_a.push(n.clone());
                            mark_b.push(n.clone());
                        }
                    }
                }
            }
        }

        self.panels[0].selection.clear();
        self.panels[1].selection.clear();
        for n in &mark_a {
            self.panels[0].selection.mark(n);
        }
        for n in &mark_b {
            self.panels[1].selection.mark(n);
        }
    }

    /// Open the side-by-side file comparison view on the files under the cursor
    /// in the left (panel 0) and right (panel 1) panels.
    async fn open_compare_files(&mut self) {
        let pick = |p: &Panel| -> Option<(String, VfsPath)> {
            p.current_entry()
                .filter(|e| e.kind == VfsKind::File && e.name != "..")
                .map(|e| (e.name.clone(), p.cwd.join(&e.name)))
        };
        let (Some((ln, lp)), Some((rn, rp))) = (pick(&self.panels[0]), pick(&self.panels[1])) else {
            return self.show_error("Put the cursor on a file in both panels to compare");
        };
        let lback = self.panels[0].backend.clone();
        let rback = self.panels[1].backend.clone();
        let ldata = match load_file(&lback, &lp).await {
            Ok(d) => d,
            Err(e) => return self.show_error(format!("cannot read {ln}: {e}")),
        };
        let rdata = match load_file(&rback, &rp).await {
            Ok(d) => d,
            Err(e) => return self.show_error(format!("cannot read {rn}: {e}")),
        };
        self.diffview = Some(DiffView::new(ln, lp, &ldata, rn, rp, &rdata));
    }

    /// Write the diff view's changed buffers back to disk.
    async fn save_diff(&mut self) {
        let saves = match self.diffview.as_ref() {
            Some(dv) => dv.pending_saves(),
            None => return,
        };
        if saves.is_empty() {
            return;
        }
        let mut ok = true;
        for (path, contents) in saves {
            match self.registry.resolve(&path) {
                Ok(backend) => {
                    if let Err(e) = write_file(&backend, &path, contents.as_bytes()).await {
                        self.show_error(format!("save failed: {e}"));
                        ok = false;
                    }
                }
                Err(e) => {
                    self.show_error(e.to_string());
                    ok = false;
                }
            }
        }
        if ok {
            if let Some(dv) = self.diffview.as_mut() {
                dv.mark_saved();
            }
            self.reload_all().await;
        }
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
    fn open_with_default(&mut self) {
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
        let name = e.name.clone();
        let path = p.cwd.path.join(&e.name);
        // When "confirm execute" is on, ask before launching the default app.
        if self.config.confirm_execute {
            self.dialog = Some(Dialog::Confirm(ConfirmDialog::execute(&name, path)));
        } else {
            tokio::spawn(async move { launch_default(path).await });
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
        // Prefill the destination panel's path. For a remote panel, show the
        // "scheme://path" form so the copy targets that backend; deleting the
        // "scheme://" prefix redirects the copy to a local path.
        let cwd = &self.panels[self.other_index()].cwd;
        let dest = if cwd.scheme == "file" {
            cwd.path.to_string_lossy().into_owned()
        } else {
            cwd.display()
        };
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

    fn open_confirmations(&mut self) {
        self.dialog = Some(Dialog::Form(FormDialog::confirmations(&self.config)));
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
        // The link is created in the *other* panel, pointing at the active
        // panel's file under the cursor (both prefilled, editable).
        let other = self.other_index();
        if !self.panels[other].backend.capabilities().symlinks {
            return self.show_error("This filesystem does not support symlinks");
        }
        let dir = self.panels[other].cwd.clone();
        let active = &self.panels[self.active];
        let (target, name) = match active.current_entry() {
            Some(e) if e.name != ".." => (
                active.cwd.join(&e.name).path.to_string_lossy().into_owned(),
                e.name.clone(),
            ),
            _ => (String::new(), String::new()),
        };
        self.dialog = Some(Dialog::Form(FormDialog::symlink(dir, target, name)));
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

                // Remember this server (without the password) for the dropdown.
                self.config.add_recent_remote(crate::config::RemoteHistoryEntry {
                    protocol: creds.protocol.scheme_prefix().to_string(),
                    host: creds.host,
                    port: creds.port,
                    user: creds.user,
                    path: creds.path,
                });
                let _ = self.config.save();
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
        // Prefill the backend-relative path (no "scheme://"): for a remote panel
        // it's the remote start directory, interpreted on that backend.
        let start = self.panels[self.active].cwd.path.to_string_lossy().into_owned();
        self.dialog = Some(Dialog::Find(FindDialog::new(start)));
    }

    /// Build the editor's search/replace dialog — in Hex mode (prefilled with
    /// the last hex search) when the editor is in hex mode.
    fn editor_search_dialog(&self, replace: bool) -> SearchReplaceDialog {
        match self.editor.as_ref() {
            Some(ed) if ed.is_hex() => {
                SearchReplaceDialog::new_hex(replace, ed.last_hex_search())
            }
            _ => SearchReplaceDialog::new(replace, String::new()),
        }
    }

    fn apply_search_replace(&mut self, p: SearchReplaceParams) {
        if let Some(ed) = self.editor.as_mut() {
            if ed.is_hex() {
                ed.apply_hex_search_replace(p.replace, &p.search, &p.replacement, p.hex, p.backwards);
                return;
            }
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
        let matcher =
            match crate::panel::selection::NameMatcher::build(&p.file_name, p.case_sensitive, p.shell) {
                Ok(m) => m,
                Err(e) => return self.show_error(format!("invalid pattern: {e}")),
            };
        let cwd = self.panels[self.active].cwd.clone();
        let backend = self.panels[self.active].backend.clone();
        // Non-local backends (remote, archives) are searched by name only via the
        // VFS — content search isn't reasonable over the network.
        let on_vfs = cwd.scheme != "file";

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

        let progress = move |tx2: AppSender, cur: String, found: usize| {
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
        };

        let tx = self.tx.clone();
        if on_vfs {
            // Remote / archive: walk the backend by name only.
            let start = if p.start_at.trim().is_empty() {
                cwd.clone()
            } else {
                VfsPath {
                    scheme: cwd.scheme.clone(),
                    path: PathBuf::from(p.start_at.trim()),
                    container: cwd.container.clone(),
                }
            };
            let (recursive, skip_hidden) = (p.recursive, p.skip_hidden);
            tokio::spawn(async move {
                let tx2 = tx.clone();
                let results = find_files_vfs(
                    &backend,
                    start,
                    &matcher,
                    recursive,
                    skip_hidden,
                    &cancel,
                    |cur, found| progress(tx2.clone(), cur, found),
                )
                .await;
                let _ = tx.send(AppEvent::FindDone { id, results }).await;
            });
        } else {
            // Local: the existing blocking walker (supports content search).
            let start = if p.start_at.trim().is_empty() {
                cwd.path.clone()
            } else {
                PathBuf::from(&p.start_at)
            };
            tokio::spawn(async move {
                let tx2 = tx.clone();
                let results = tokio::task::spawn_blocking(move || {
                    find_files(&start, &p, &matcher, &cancel, |cur, found| {
                        progress(tx2.clone(), cur, found)
                    })
                    .into_iter()
                    .map(|path| {
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        (VfsPath::local(path), size)
                    })
                    .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
                let _ = tx.send(AppEvent::FindDone { id, results }).await;
            });
        }
    }

    /// Panelize find-file results (with a `..` entry that returns to browsing).
    /// Results may be local or remote; the panel keeps the backend the matches
    /// live on so navigating into a result — or back out via `..` — works.
    fn panelize_results(&mut self, results: Vec<(VfsPath, u64)>) {
        if results.is_empty() {
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
        for (path, size) in results {
            entries.push(VfsEntry {
                name: path.path.to_string_lossy().into_owned(),
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
            vpaths.push(path);
        }
        // Resolve the backend the results live on (local or the remote session).
        let backend = vpaths
            .get(1)
            .and_then(|p| self.registry.resolve(p).ok())
            .unwrap_or_else(|| self.registry.local());
        let p = &mut self.panels[self.active];
        p.backend = backend;
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
        let size = e.size;
        let path = p.cwd.join(&name);
        let backend = p.backend.clone();

        if !self.config.wants_internal_viewer() {
            return Flow::RunExternal {
                program: self.config.viewer.clone(),
                path: path.path,
            };
        }

        if path.scheme == "file" {
            // Local: page straight from disk — never load the whole file. The
            // line-index scan runs off-thread so it doesn't block the reactor.
            let local = path.path.clone();
            let dark = self.dark_ui();
            let scanned = tokio::task::spawn_blocking(move || crate::viewer::scan_file(&local)).await;
            match scanned {
                Ok(Ok((file, len, line_starts))) => {
                    let mut v = ViewerState::from_scanned(name, file, len, line_starts, None);
                    v.enable_syntax(dark);
                    self.viewer = Some(v);
                }
                Ok(Err(e)) => self.show_error(format!("cannot open file: {e}")),
                Err(_) => self.show_error("viewer failed to open file"),
            }
        } else {
            // Remote/archive: stream to a temp file with a cancellable progress
            // bar; the viewer then pages from that temp copy.
            self.start_fetch(FetchKind::View, name, path, backend, size);
        }
        Flow::Continue
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
        let size = e.size;
        let path = p.cwd.join(&name);
        let backend = p.backend.clone();

        if !self.config.wants_internal_editor() {
            return Flow::RunExternal {
                program: self.config.editor.clone(),
                path: path.path,
            };
        }

        let local = path.scheme == "file";
        // Local files too big to load as text open directly in (in-place) hex mode.
        if local && size > crate::editor::MAX_TEXT_EDIT {
            match EditorState::new_hex(name, path) {
                Ok(ed) => self.editor = Some(ed),
                Err(e) => self.show_error(format!("cannot open file: {e}")),
            }
            return Flow::Continue;
        }
        if local {
            match load_file(&backend, &path).await {
                Ok(data) => {
                    let text = String::from_utf8_lossy(&data).into_owned();
                    let mut ed = EditorState::new(name, path, &text);
                    ed.enable_syntax(self.dark_ui());
                    self.editor = Some(ed);
                }
                Err(e) => self.show_error(format!("cannot open file: {e}")),
            }
            return Flow::Continue;
        }
        // Remote/archive: in-place hex editing isn't possible (no random write),
        // so editing requires loading into memory — cap the size and stream the
        // download with a cancellable progress bar.
        if size > crate::editor::MAX_TEXT_EDIT {
            self.show_error("File too large to edit over this connection");
            return Flow::Continue;
        }
        self.start_fetch(FetchKind::Edit, name, path, backend, size);
        Flow::Continue
    }

    /// Stream a (remote/archive) file to a local temp file for view/edit, showing
    /// a cancellable progress dialog. Delivers `FileFetched` on success.
    fn start_fetch(
        &mut self,
        kind: FetchKind,
        name: String,
        path: VfsPath,
        backend: std::sync::Arc<dyn Vfs>,
        total: u64,
    ) {
        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = CancelToken::new();
        let (reply, _reply_rx) = tokio::sync::mpsc::channel(1);
        self.tasks.insert(
            id,
            TaskHandle {
                id,
                cancel: cancel.clone(),
                reply,
            },
        );
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, "Reading")));

        let safe: String = name.chars().map(|c| if c == '/' { '_' } else { c }).collect();
        let temp = std::env::temp_dir().join(format!("rc_fetch_{}_{id}_{safe}", std::process::id()));
        let tx = self.tx.clone();
        let orig_path = path.clone();
        tokio::spawn(async move {
            let outcome = fetch_to_temp(&backend, &path, &temp, total, &cancel, id, &name, &tx).await;
            match outcome {
                Ok(true) => {
                    let _ = tx
                        .send(AppEvent::FileFetched { id, kind, name, orig_path, temp })
                        .await;
                }
                Ok(false) => {
                    let _ = tokio::fs::remove_file(&temp).await;
                    let _ = tx
                        .send(AppEvent::TaskDone { id, outcome: TaskOutcome::Cancelled })
                        .await;
                }
                Err(e) => {
                    let _ = tokio::fs::remove_file(&temp).await;
                    let _ = tx
                        .send(AppEvent::TaskDone { id, outcome: TaskOutcome::Failed(e) })
                        .await;
                }
            }
        });
    }

    /// Persist the editor's contents to its file, optionally closing after.
    async fn save_editor(&mut self, close_after: bool) {
        let Some(ed) = self.editor.as_ref() else {
            return;
        };
        // Hex mode writes only the changed bytes in place — never rewrite the
        // whole (possibly huge) file from the text buffer.
        if ed.is_hex() {
            let res = self.editor.as_mut().unwrap().flush_hex();
            match res {
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
            return;
        }
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

/// Stream `path` from `backend` to the local `temp` file, emitting throttled
/// progress and honoring `cancel`. Returns `Ok(true)` when complete, `Ok(false)`
/// when cancelled, or `Err` on I/O failure. The caller cleans up `temp`.
#[allow(clippy::too_many_arguments)]
async fn fetch_to_temp(
    backend: &std::sync::Arc<dyn Vfs>,
    path: &VfsPath,
    temp: &Path,
    total: u64,
    cancel: &crate::ops::CancelToken,
    id: TaskId,
    name: &str,
    tx: &AppSender,
) -> Result<bool, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut reader = backend.open_read(path).await.map_err(|e| e.to_string())?;
    let mut file = tokio::fs::File::create(temp).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut since_report = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Ok(false);
        }
        let n = reader.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).await.map_err(|e| e.to_string())?;
        done += n as u64;
        since_report += n as u64;
        // Report at most ~every 1 MB so the bar advances without flooding.
        if since_report >= 1024 * 1024 {
            since_report = 0;
            let _ = tx.try_send(AppEvent::Progress(ProgressUpdate {
                id,
                verb: "Reading",
                current_name: name.to_string(),
                file_done: done,
                file_total: total,
                total_done: done,
                total_total: total,
                files_done: 0,
                files_total: 1,
            }));
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(true)
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

/// Recursively search a VFS backend (remote/archive) for files whose names match
/// `matcher`, returning `(path, size)` pairs. Name-only — there is no content
/// search over the network. Symlinked directories are not descended (loop-safe).
async fn find_files_vfs(
    backend: &std::sync::Arc<dyn Vfs>,
    start: VfsPath,
    matcher: &crate::panel::selection::NameMatcher,
    recursive: bool,
    skip_hidden: bool,
    cancel: &crate::ops::CancelToken,
    mut progress: impl FnMut(String, usize),
) -> Vec<(VfsPath, u64)> {
    const MAX_RESULTS: usize = 50_000;
    let mut out: Vec<(VfsPath, u64)> = Vec::new();
    let mut stack = vec![start];
    let mut scanned = 0usize;
    while let Some(dir) = stack.pop() {
        if cancel.is_cancelled() {
            break;
        }
        let entries = match backend.read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries {
            if cancel.is_cancelled() {
                break;
            }
            if e.name == ".." || (skip_hidden && e.name.starts_with('.')) {
                continue;
            }
            let child = dir.join(&e.name);
            scanned += 1;
            if scanned.is_multiple_of(64) {
                progress(child.path.to_string_lossy().into_owned(), out.len());
            }
            if e.kind == VfsKind::Dir {
                // Don't follow symlinked dirs — avoids cycles on remote trees.
                if recursive && e.symlink_target.is_none() {
                    stack.push(child);
                }
                continue;
            }
            if matcher.is_match(&e.name) {
                progress(child.path.to_string_lossy().into_owned(), out.len() + 1);
                out.push((child, e.size));
                if out.len() >= MAX_RESULTS {
                    return out;
                }
            }
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
  F9 toggle hex editor (in-place; Tab switches hex/ASCII column)

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
    fn resolve_dest_preserves_remote_backend() {
        use std::path::PathBuf;
        let remote = VfsPath {
            scheme: "scp-0".to_string(),
            path: PathBuf::from("/home/user"),
            container: None,
        };
        // The unchanged (absolute) remote path stays on the remote backend.
        let d = resolve_dest_on("/home/user", &remote);
        assert_eq!(d.scheme, "scp-0");
        assert_eq!(d.path, PathBuf::from("/home/user"));
        // A relative entry joins the remote cwd (still remote).
        let d = resolve_dest_on("uploads", &remote);
        assert_eq!(d.scheme, "scp-0");
        assert_eq!(d.path, PathBuf::from("/home/user/uploads"));
        // A local base resolves to a local path.
        let local = VfsPath::local("/a/b");
        assert_eq!(resolve_dest_on("/c", &local).scheme, "file");
        assert_eq!(resolve_dest_on("sub", &local).path, PathBuf::from("/a/b/sub"));
    }

    #[test]
    fn split_scheme_recognizes_only_real_schemes() {
        assert_eq!(split_scheme("scp-0:///srv/x"), Some(("scp-0", "/srv/x")));
        assert_eq!(split_scheme("sftp-2://rel"), Some(("sftp-2", "rel")));
        assert_eq!(split_scheme("/home/user"), None);
        assert_eq!(split_scheme("relative/path"), None);
        assert_eq!(split_scheme("://nope"), None);
    }

    #[test]
    fn dest_override_remote_to_local() {
        use std::path::PathBuf;
        let remote = VfsPath { scheme: "scp-0".into(), path: PathBuf::from("/home/user"), container: None };
        let local_src = VfsPath::local("/data");

        // Keeping the scheme prefix stays on the remote backend.
        let d = dest_vfspath("scp-0:///srv/up", &remote, &local_src);
        assert_eq!(d.scheme, "scp-0");
        assert_eq!(d.path, PathBuf::from("/srv/up"));
        // A relative remote path joins the matching panel's cwd.
        let d = dest_vfspath("scp-0://uploads", &remote, &local_src);
        assert_eq!((d.scheme.as_str(), d.path), ("scp-0", PathBuf::from("/home/user/uploads")));

        // Dropping the scheme on a remote dest → local (absolute kept as-is).
        let d = dest_vfspath("/tmp/out", &remote, &local_src);
        assert_eq!((d.scheme.as_str(), d.path), ("file", PathBuf::from("/tmp/out")));
        // …and a relative one joins the (local) source panel's directory.
        let d = dest_vfspath("out", &remote, &local_src);
        assert_eq!((d.scheme.as_str(), d.path), ("file", PathBuf::from("/data/out")));

        // A bare path to a *local* destination panel behaves exactly as before.
        let local_dest = VfsPath::local("/a/b");
        let d = dest_vfspath("sub", &local_dest, &remote);
        assert_eq!((d.scheme.as_str(), d.path), ("file", PathBuf::from("/a/b/sub")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlink_dialog_prefilled_from_cursor_and_other_panel() {
        use crate::ui::dialog::{DialogResult, Submit};
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_sym_{}_{nanos}", std::process::id()));
        let src = root.join("src");
        let dest = root.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("doc.txt"), b"x").unwrap();

        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.active = 0;
        st.panels[0].cwd = VfsPath::local(&src);
        st.panels[0].backend = st.registry.local();
        st.panels[0].reload().await.unwrap();
        st.panels[1].cwd = VfsPath::local(&dest);
        st.panels[1].backend = st.registry.local();
        let idx = st.panels[0].entries.iter().position(|e| e.name == "doc.txt").unwrap();
        st.panels[0].cursor = idx;

        st.open_symlink();
        let dlg = st.dialog.as_mut().expect("symlink dialog");
        match dlg.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            DialogResult::Submit(Submit::Symlink { dir, target, name }) => {
                assert_eq!(name, "doc.txt", "link name defaults to the file");
                assert_eq!(target, src.join("doc.txt").to_string_lossy(), "target = file path");
                assert_eq!(dir.path, dest, "link is created in the other panel");
            }
            _ => panic!("expected a Symlink submit"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn compare_dirs_marks_by_mode() {
        use std::collections::HashSet;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_cmp_{}_{nanos}", std::process::id()));
        let da = root.join("a");
        let db = root.join("b");
        std::fs::create_dir_all(&da).unwrap();
        std::fs::create_dir_all(&db).unwrap();
        std::fs::write(da.join("same.txt"), b"hello").unwrap();
        std::fs::write(db.join("same.txt"), b"hello").unwrap();
        std::fs::write(da.join("big.txt"), b"AAAA").unwrap(); // larger in A
        std::fs::write(db.join("big.txt"), b"AA").unwrap();
        std::fs::write(da.join("onlyA.txt"), b"x").unwrap();
        std::fs::write(db.join("onlyB.txt"), b"y").unwrap();
        std::fs::write(da.join("diff.txt"), b"abc").unwrap(); // same size, diff content
        std::fs::write(db.join("diff.txt"), b"xyz").unwrap();

        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.panels[0].cwd = VfsPath::local(&da);
        st.panels[0].backend = st.registry.local();
        st.panels[0].reload().await.unwrap();
        st.panels[1].cwd = VfsPath::local(&db);
        st.panels[1].backend = st.registry.local();
        st.panels[1].reload().await.unwrap();

        let marked = |p: &Panel| -> HashSet<String> {
            p.entries
                .iter()
                .filter(|e| p.selection.is_marked(&e.name))
                .map(|e| e.name.clone())
                .collect()
        };
        let set = |names: &[&str]| -> HashSet<String> {
            names.iter().map(|s| s.to_string()).collect()
        };

        st.compare_dirs(CompareMode::Quick).await;
        assert_eq!(marked(&st.panels[0]), set(&["onlyA.txt"]));
        assert_eq!(marked(&st.panels[1]), set(&["onlyB.txt"]));

        st.compare_dirs(CompareMode::Size).await;
        assert_eq!(marked(&st.panels[0]), set(&["onlyA.txt", "big.txt"]));
        assert_eq!(marked(&st.panels[1]), set(&["onlyB.txt"]));

        st.compare_dirs(CompareMode::Content).await;
        assert_eq!(marked(&st.panels[0]), set(&["onlyA.txt", "big.txt", "diff.txt"]));
        assert_eq!(marked(&st.panels[1]), set(&["onlyB.txt", "big.txt", "diff.txt"]));

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
    async fn delete_anchor_targets_file_above() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_del_{}_{nanos}", std::process::id()));
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

        // Cursor on c.txt; deleting it should anchor the cursor on b.txt.
        let ci = st.panels[0].entries.iter().position(|e| e.name == "c.txt").unwrap();
        st.panels[0].cursor = ci;
        let anchor = st.delete_anchor(&[VfsPath::local(root.join("c.txt"))]);
        assert_eq!(anchor.as_deref(), Some("b.txt"));

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
    async fn page_keys_move_by_visible_page() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_page_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..100 {
            std::fs::write(root.join(format!("f{i:03}.txt")), b"x").unwrap();
        }
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.active = 0;
        st.panels[0].cwd = VfsPath::local(&root);
        st.panels[0].backend = st.registry.local();
        st.panels[0].reload().await.unwrap();

        // Render so the panel records its visible page size from the area.
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| crate::ui::draw(f, &mut st)).unwrap();
        let page = st.panels[0].page;
        assert!(page > 1, "page size should reflect the terminal height");

        st.panels[0].cursor = 0;
        st.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)).await;
        assert_eq!(st.panels[0].cursor, page, "PageDown moves one whole page");
        st.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)).await;
        assert_eq!(st.panels[0].cursor, 0, "PageUp moves back a whole page");

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
    async fn find_files_vfs_matches_names_recursively() {
        use std::sync::Arc;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rc_vfind_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), b"x").unwrap();
        std::fs::write(root.join("sub/b.txt"), b"yy").unwrap();
        std::fs::write(root.join("sub/c.log"), b"z").unwrap();

        // Exercise the VFS walker through the local backend (stands in for remote).
        let backend: Arc<dyn Vfs> = Arc::new(crate::vfs::local::LocalFs::new());
        let matcher = crate::panel::selection::NameMatcher::build("*.txt", false, true).unwrap();
        let cancel = crate::ops::CancelToken::new();
        let results =
            find_files_vfs(&backend, VfsPath::local(&root), &matcher, true, true, &cancel, |_, _| {})
                .await;

        let mut names: Vec<String> = results.iter().map(|(p, _)| p.file_name()).collect();
        names.sort();
        assert_eq!(names, vec!["a.txt", "b.txt"], "name-only, recursive, .log excluded");
        // Sizes come from the directory listing, not a second stat.
        assert!(results.iter().any(|(p, s)| p.file_name() == "b.txt" && *s == 2));

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

    #[tokio::test]
    async fn disk_mounter_opens_and_prompts_for_path() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.init().await;
        // The Command-menu action opens the mounter view.
        st.run_menu_action(crate::ui::menu::MenuAction::DiskManager).await;
        assert!(st.mountview.is_some(), "disk mounter should open");

        // Enter on a device requests a mount → the app raises a path-input dialog.
        let mv = st.mountview.as_mut().unwrap();
        mv.devices = vec![crate::mount::BlockDevice {
            name: "sdb1".into(),
            dev: "/dev/sdb1".into(),
            size: 0,
            fstype: String::new(),
            mountpoint: None,
            ..Default::default()
        }];
        mv.dev_cursor = 0;
        // Enter opens the device action menu (Mount/Format/Cancel).
        st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert!(
            matches!(st.dialog, Some(crate::ui::dialog::Dialog::Confirm(_))),
            "Enter on a device opens its action menu"
        );
        // Activating the focused "Mount" button prompts for the target path.
        st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        assert!(
            matches!(st.dialog, Some(crate::ui::dialog::Dialog::Input(_))),
            "the Mount action prompts for the target path"
        );

        // Esc cancels the (open) dialog immediately, returning to the mounter.
        st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        assert!(st.dialog.is_none());
        assert!(st.mountview.is_some(), "still on the mounter after cancel");
        // With no dialog, a lone Esc is held (function-key prefix); the next key
        // flushes it through to the mounter, which closes.
        st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        assert!(st.mountview.is_none(), "Esc closes the mounter");
    }

    #[tokio::test]
    async fn formatting_shows_busy_dialog_until_done() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        st.mountview = Some(MountView::new());

        // A privileged op in flight raises the non-dismissible busy spinner, and
        // input can't close it.
        st.dialog = Some(Dialog::Busy(BusyDialog::new("Please wait", "Formatting /dev/sdb1...")));
        st.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).await;
        st.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).await;
        assert!(matches!(st.dialog, Some(Dialog::Busy(_))), "busy spinner ignores input");

        // Completion dismisses the spinner and reports success on the status line.
        st.apply_event(AppEvent::PrivilegedDone {
            ok_msg: "Formatted /dev/sdb1 as EXT4".into(),
            result: Ok(()),
        })
        .await;
        assert!(st.dialog.is_none(), "busy dialog dismissed on completion");
        assert_eq!(st.mountview.as_ref().unwrap().status, "Formatted /dev/sdb1 as EXT4");

        // A failure surfaces as an error on the status line.
        st.dialog = Some(Dialog::Busy(BusyDialog::new("Please wait", "Formatting...")));
        st.apply_event(AppEvent::PrivilegedDone {
            ok_msg: "ok".into(),
            result: Err("mkfs failed".into()),
        })
        .await;
        assert!(st.dialog.is_none());
        assert!(st.mountview.as_ref().unwrap().status.contains("mkfs failed"));
    }

    #[tokio::test]
    async fn confirm_exit_gates_the_quit_prompt() {
        let (tx, _rx) = async_bridge::channel();
        let mut st = AppState::new(tx);
        // With confirmation off, F10 quits immediately.
        st.config.confirm_exit = false;
        let flow = st.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)).await;
        assert!(matches!(flow, Flow::Quit));
        assert!(st.dialog.is_none(), "no prompt when confirmation is off");
        // With confirmation on, F10 raises the quit dialog instead.
        st.config.confirm_exit = true;
        let flow = st.handle_key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)).await;
        assert!(matches!(flow, Flow::Continue));
        assert!(st.dialog.is_some(), "prompt shown when confirmation is on");
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
