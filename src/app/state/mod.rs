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
use crate::net::{NetSignal, NetView, Pane};
use crate::panel::{Panel, ViewFormat};
use crate::proc::{ProcSignal, ProcView};
use crate::ui::cmdline::CommandLine;
use crate::ui::dialog::{
    BackgroundOpsDialog, BgRow, BoolSetting, BusyDialog, ChecksumResultDialog, CommandPaletteDialog,
    CompareDialog, CompareMode, ConfirmDialog, Dialog, DialogResult, DriveDialog,
    DupCriteria, FileBrowserDialog, FindDialog, FindParams, FlashTargetDialog, FormDialog, GotoDialog,
    ImageSaveDialog, InputDialog, InputPurpose, MessageDialog, MultiRenameDialog, OverwriteDialog,
    PaletteAction, PaletteCategory, PaletteEntry, ProgressDialog, SaveAsDialog, SearchReplaceDialog,
    SearchReplaceParams, SelectDialog, ShellHistoryDialog, Submit, UserMenuDialog,
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

/// A live remote connection the user can switch back to like a drive letter.
///
/// The backend itself is not stored here — it stays the single source of truth
/// in [`AppState::registry`], resolvable via `scheme`. `id` is the stable key
/// the UI/`Submit` path uses (so dialogs never carry a `String`); `scheme` is
/// how we tell which session a panel is currently on (`panel.cwd.scheme`).
pub struct RemoteSession {
    pub id: usize,
    /// Unique backend scheme, e.g. `"sftp-3"`.
    pub scheme: String,
    /// One-line label for the picker button, e.g. `"sftp://user@host"`.
    pub label: String,
    /// The last directory visited on this session, restored on switch-back.
    pub cwd: VfsPath,
    /// Credentials (including the in-memory password) used to open this session,
    /// kept so a *second* connection can be opened for browsing when a transfer
    /// on this session is sent to the background (see FTP reconnect). Retained in
    /// memory only for the session's lifetime — never persisted.
    pub creds: RemoteCreds,
}

/// A file transfer that can run in the background: the state kept for the
/// menu-bar mini progress bar and the "Background operations" list once its
/// modal progress dialog has been dismissed.
pub(in crate::app::state) struct BgTransfer {
    /// "Copying" / "Moving" / "Deleting".
    pub verb: &'static str,
    /// Latest progress snapshot (`None` until the first update arrives).
    pub update: Option<ProgressUpdate>,
    /// Remote backend schemes this op touches (used to decide FTP reconnect).
    pub schemes: Vec<String>,
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

/// In-memory (non-persistent) search/replace terms, kept on [`AppState`] so the
/// editor and viewer prefill their search dialogs with the last-used values even
/// across files and reopenings. Editor and viewer keep separate slots since the
/// editor stores mode-processed patterns (regex/wildcard) the viewer can't reuse.
#[derive(Default)]
pub(in crate::app::state) struct SearchMemory {
    /// Editor text-mode search pattern (as last submitted).
    pub search: String,
    /// Editor replacement string.
    pub replacement: String,
    /// Editor hex-mode search string.
    pub hex_search: String,
    /// Viewer search query.
    pub viewer_query: String,
}

/// A live FAR/NC-style quick search in the active panel. Started by Alt+letter
/// (replacing the old Alt+letter menu shortcuts); each typed char extends the
/// prefix and jumps the panel cursor to the first entry whose name starts with
/// it (case-insensitive). Cancelled by Esc, committed by Enter, or left by any
/// other key which is then re-dispatched normally.
pub(crate) struct QuickSearch {
    /// The accumulated (case-preserving) query string.
    pub query: String,
}

pub struct AppState {
    pub panels: [Panel; 2],
    /// Index of the active panel (0 = left/top, 1 = right/bottom).
    pub active: usize,
    pub split: SplitDir,
    /// Per-side panel visibility. Ctrl-F1 / Ctrl-F2 hide the left / right panel,
    /// Norton-Commander style: a hidden panel isn't drawn and the freed area
    /// exposes the backdrop. Both may be hidden at once; the menu bar and F-key
    /// bar always remain on screen. Transient — not persisted across sessions.
    pub panel_hidden: [bool; 2],
    /// Half-height mode (Ctrl-F3): both panels shrink to the top half of the
    /// body, exposing the backdrop beneath them. Transient — not persisted.
    pub half_height: bool,
    /// The command-line console: a headless terminal emulator fed the captured
    /// output of commands run from the command line, drawn behind the panels so
    /// hiding a panel or going half-height reveals the shell output underneath.
    pub console: crate::console::Console,
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
    /// The full-screen network-connections explorer, when open (Linux).
    pub netview: Option<NetView>,
    /// The full-screen visual theme editor, when open (Options → Edit themes).
    pub theme_editor: Option<crate::ui::theme_editor::ThemeEditor>,
    /// A privileged command queued while prompting for a sudo password.
    pending_sudo: Option<PendingPriv>,
    /// A flash queued while prompting for a sudo password.
    pending_flash: Option<crate::flash::FlashSpec>,
    /// A device-imaging queued while prompting for a sudo password.
    pending_image: Option<crate::flash::ImageSpec>,
    /// Cancel tokens for in-flight flash / imaging tasks, keyed by task id.
    flash_tasks: HashMap<TaskId, crate::ops::CancelToken>,
    pub theme: Theme,
    pub config: Config,
    pub registry: Registry,
    /// All open remote connections, in creation order. Each stays alive (and
    /// registered) until the user explicitly disconnects it, so a panel can
    /// switch to Local and back without losing the connection.
    pub sessions: Vec<RemoteSession>,
    /// Per-panel last local directory, restored by the "Local" button so a panel
    /// returns to where it was before going remote (drive-letter style).
    last_local_cwd: [VfsPath; 2],
    tasks: HashMap<TaskId, TaskHandle>,
    /// Live progress state for backgroundable transfers (copy/move/delete),
    /// keyed by task id. Populated for every such task so the menu-bar mini bar
    /// and the "Background operations" list keep updating even when the task's
    /// progress dialog is not the foreground one.
    pub(in crate::app::state) task_progress: HashMap<TaskId, BgTransfer>,
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
    /// Language name to restore if the settings dialog is cancelled (live preview).
    lang_backup: Option<String>,
    /// `reshape_rtl` value to restore if the settings dialog is cancelled.
    reshape_backup: Option<bool>,
    /// Terminal pixel-graphics capability (Kitty/Sixel/iTerm2), or `None` when the
    /// terminal has no graphics protocol or graphics are configured off. Every
    /// graphics-backed widget checks this and falls back to Ratatui cells.
    pub gfx: Option<crate::ui::graphics::Gfx>,
    /// `graphics` preference to restore if the settings dialog is cancelled.
    graphics_backup: Option<String>,
    /// F2 user-menu entries (loaded from the config `menu` file).
    user_menu: Vec<UserMenuEntry>,
    /// File-association rules (loaded from the config `rc.ext` file), consulted
    /// on Enter/F3/F4 to run Open/View/Edit actions and mount extfs scripts.
    ext_rules: crate::ext::ExtRules,
    /// A user-menu command to run after the dialog closes (expanded).
    pending_run: Option<String>,
    /// Set when a confirmed quit should propagate out as `Flow::Quit`.
    pending_quit: bool,
    /// When a lone Esc has been pressed and we're waiting to see whether the
    /// next key is a digit (Esc-prefix function-key alias, MC style).
    pending_esc: Option<Instant>,
    /// An active FAR/NC-style quick search in the active panel: Alt+letter
    /// starts it, plain typing extends the prefix and jumps the cursor to the
    /// first matching entry. `None` when no quick search is live.
    pub quick_search: Option<QuickSearch>,
    /// Set while Alt arms the menu accelerators (so the closed menu bar shows
    /// its highlighted hotkey letters as a hint) — only used when quick search
    /// is disabled. Cleared by the next non-Alt key.
    pub alt_hint: bool,
    /// The progress dialog set aside while an overwrite prompt is shown; restored
    /// once the user answers so the operation's progress keeps displaying.
    stashed_progress: Option<ProgressDialog>,
    /// The full terminal area from the last render, used to hit-test mouse clicks
    /// against menus and centered dialogs.
    pub last_area: Rect,
    /// The (panel, entry) last toggled by a right-drag paint, so each entry is
    /// inverted only once as the drag passes over it.
    paint_last: Option<(usize, usize)>,
    /// The last left click (panel, entry, when), for double-click detection: a
    /// second click on the same entry within [`DOUBLE_CLICK`] opens it like Enter.
    last_click: Option<(usize, usize, Instant)>,
    /// Per-panel "Details" view state (only computed while a panel uses the
    /// Details format): what to show about the *other* panel's cursor/selection,
    /// plus the background size-scan bookkeeping. Index = the panel displaying it.
    pub details: [crate::details::DetailsData; 2],
    /// After an operation completes, place a panel's cursor on a named entry: the
    /// surviving file above a delete, or the newly renamed/moved item. Stored as
    /// `(panel index, entry name)`.
    pending_focus: Option<(usize, String)>,
    /// Search/replace terms remembered in memory across editor and viewer
    /// sessions (even on different files), used to prefill their search dialogs.
    pub(in crate::app::state) search_memory: SearchMemory,
    /// Launched via `rc /edit <file>` (or the `rcedit` shim): the program opens
    /// straight into the editor and exits when it is closed.
    pub edit_only: bool,
    /// Whether the terminal's enhanced keyboard protocol is active (key
    /// release/repeat + standalone modifiers reported). Lets the editor's F-key
    /// bar track held Shift/Ctrl; set by the event loop after terminal setup.
    pub kbd_enhanced: bool,
    /// Set when this instance was launched from inside another Rat Commander's
    /// Ctrl-O subshell: it can't run its own subshell, so Ctrl-O is disabled.
    pub subshell_disabled: bool,
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

/// The top-menu index whose title starts with `c` (case-insensitive): L→0 Left,
/// F→1 File, C→2 Command, O→3 Options, R→4 Right. Used for the classic
/// Alt+letter menu shortcuts (active when quick search is disabled).
fn menu_title_index(c: char) -> Option<usize> {
    let lc = c.to_ascii_lowercase();
    crate::ui::menubar::TITLES
        .iter()
        .position(|t| t.chars().next().map(|x| x.to_ascii_lowercase()) == Some(lc))
}


/// A human label for a viewer goto mode (used in the "invalid value" message).
fn goto_mode_label(mode: crate::viewer::GotoMode) -> &'static str {
    use crate::viewer::GotoMode::*;
    match mode {
        Line => "line",
        Percent => "percent",
        DecimalOffset => "decimal offset",
        HexOffset => "hex offset",
    }
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
    } else if Path::new(dest).is_absolute() {
        // An absolute path lands on the destination (other) panel's backend.
        resolve_dest_on(dest, other_cwd)
    } else {
        // A bare name or relative path is resolved against the *source* panel —
        // mc-style, so `F6` + a new name renames in place rather than moving to
        // the opposite panel.
        resolve_dest_on(dest, active_cwd)
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

mod lifecycle;
mod keys;
mod mouse;
mod dialogs;
mod fileops;
mod checksum;
mod disk;
mod net;
mod remote;
mod find;
mod duplicates;
mod details;
mod viewer_editor;
mod ext;
mod palette;

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

/// The shell command that runs a local executable directly (its quoted absolute
/// path), used to launch ELF binaries / scripts in the foreground terminal.
fn run_program_cmd(path: &Path) -> String {
    crate::vfs::remote::shell_quote(&path.to_string_lossy())
}

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

/// The full user manual, embedded at build time. F1 opens it in the viewer's
/// Markdown render mode (see `open_help` in the `keys` submodule).
const HELP_TEXT: &str = include_str!("../../../doc/MANUAL.md");

/// The `.md` suffix makes the help viewer auto-detect Markdown and open in the
/// rendered (tags-hidden) mode rather than raw.
const HELP_NAME: &str = "Rat Commander Manual.md";

#[cfg(test)]
mod tests;
