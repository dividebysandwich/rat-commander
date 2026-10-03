//! Application state and the key/event dispatch that drives it.

use crate::app::event::{AppEvent, FetchKind};
use crate::config::Config;
use crate::diff::hex::HexDiffSignal;
use crate::diff::{DiffSignal, DiffView};
use crate::disk::{DiskSignal, DiskView};
use crate::editor::{EditorSignal, EditorState};
use crate::mount::{MountSignal, MountView};
use crate::net::{NetSignal, NetView, Pane};
use crate::ops::CancelToken;
use crate::ops::progress::{PrivDecision, ProgressUpdate, TaskOutcome, TaskReply};
use crate::ops::{ArchiveAdd, OpKind, OpRequest, TaskHandle, TaskId, spawn_op};
use crate::panel::{Panel, ViewFormat};
use crate::proc::{ProcSignal, ProcView};
use crate::ui::cmdline::CommandLine;
use crate::ui::dialog::{
    BackgroundOpsDialog, BgRow, BoolSetting, BusyDialog, ChecksumResultDialog,
    CommandPaletteDialog, CompareDialog, CompareMode, ConfirmDialog, Dialog, DialogResult,
    DirHistoryDialog, DriveDialog, DupCriteria, FileBrowserDialog, FindDialog, FindParams,
    FlashTargetDialog, FormDialog, GeoMapDialog, GitOutputDialog, GotoDialog, HotlistDialog,
    HotlistOutcome, ImageSaveDialog, InputDialog, InputPurpose, MessageDialog, MultiRenameDialog,
    OverwriteDialog, PaletteAction, PaletteCategory, PaletteEntry, ProgressDialog, ReceiveDialog,
    SaveAsDialog, SearchReplaceDialog, SearchReplaceParams, SelectDialog, SendFileDialog,
    SettingsTab, ShellHistoryDialog, SpeedChart, StashDialog, Submit, SyncPreviewDialog,
    TabPickerDialog, TemplatePickerDialog, UserMenuDialog,
};
use crate::ui::layout::SplitDir;
use crate::ui::menu::{MenuAction, MenuBarState, MenuSignal};
use crate::ui::theme::Theme;
use crate::usermenu::{self, UserMenu};
use crate::util::async_bridge::AppSender;
use crate::vfs::Vfs;
use crate::vfs::VfsPath;
use crate::vfs::archive::{self, formats::ArchiveFormat};
use crate::vfs::registry::Registry;
use crate::vfs::remote::RemoteCreds;
use crate::vfs::{VfsEntry, VfsKind};
use crate::viewer::{MAX_VIEW_BYTES, ViewerSignal, ViewerState};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What the run loop should do after handling input.
pub enum Flow {
    Continue,
    Quit,
    /// Run this shell command in the active panel's cwd — the behind-the-panels
    /// console where one is available (so it doesn't block the UI), else a
    /// suspended foreground shell. Used by the command line and file associations.
    RunCommand(String),
    /// Suspend the TUI and run this shell command in the foreground, Midnight
    /// Commander style: the user watches its output and presses Enter to return.
    /// Used by the F2 user menu on a local panel.
    RunCommandForeground(String),
    /// Suspend the TUI and run an external program against a file.
    RunExternal {
        program: String,
        path: std::path::PathBuf,
    },
    /// Ctrl-O: drop to an interactive subshell, full screen.
    SubShell,
}

/// A user-menu (F2) command paused while its `%{…}` interactive prompts are
/// answered one input dialog at a time. Once `answers` covers every `label`, the
/// template is expanded (with the answers substituted) and run.
struct PendingMenu {
    /// The raw command template, macros still unexpanded.
    template: String,
    /// The `%{label}` prompt labels, in the order they appear.
    labels: Vec<String>,
    /// Answers collected so far — one per already-shown prompt.
    answers: Vec<String>,
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
    /// The transfer's speed history, kept here rather than on the progress
    /// dialog so it outlives one: it keeps recording while the transfer runs in
    /// the background, and a dialog opened later shows the whole run instead of
    /// starting from an empty chart.
    pub chart: SpeedChart,
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

/// A live FAR/NC-style quick search in the active panel. Started by Alt-S /
/// Ctrl-S, or — when the command line is hidden — by typing any printable
/// character. Each typed char extends the prefix and jumps the panel cursor to
/// the first entry whose name starts with it (case-insensitive). Cancelled by
/// Esc, committed by Enter, or left by any other key which is then
/// re-dispatched normally.
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
    /// bar always remain on screen. Persisted across sessions.
    pub panel_hidden: [bool; 2],
    /// Half-height mode (Ctrl-F3): both panels shrink to the top half of the
    /// body, exposing the backdrop beneath them. Persisted across sessions.
    pub half_height: bool,
    /// The command-line console: a headless terminal emulator fed the captured
    /// output of commands run from the command line, drawn behind the panels so
    /// hiding a panel or going half-height reveals the shell output underneath.
    pub console: crate::console::Console,
    pub cmd: CommandLine,
    pub dialog: Option<Dialog>,
    pub viewer: Option<ViewerState>,
    pub editor: Option<EditorState>,
    /// Editors waiting under the one shown, which returns to them when it
    /// closes: a hex editor under the binary template being edited.
    pub editor_stack: Vec<EditorState>,
    pub menu: Option<MenuBarState>,
    /// Set when something needs the terminal cleared before the next frame (the
    /// editor's Ctrl-L). The main loop clears and resets it.
    pub force_clear: bool,
    /// The full-screen process explorer, when open.
    pub procview: Option<ProcView>,
    /// The full-screen disk-usage explorer, when open.
    pub diskview: Option<DiskView>,
    /// The shared directory-size cache and its background crawler. Spawned
    /// lazily the first time something wants sizes (the disk explorer or a 3D
    /// panel), so sessions that use neither never start the task.
    pub sizes: Option<crate::sizes::crawl::Crawler>,
    /// The 3D view's time machine, when one is running. At most one at a time:
    /// scrubbing two histories at once would have them fighting over the
    /// crawler, and there is only ever one thing you are looking through.
    pub timeline: Option<crate::sizes::timeline::Timeline>,
    /// The directory the crawler was last pointed at, so a redraw doesn't
    /// re-publish the same focus every frame.
    sizes_focus: Option<std::path::PathBuf>,
    /// The full-screen side-by-side file comparison view, when open.
    pub diffview: Option<DiffView>,
    /// The full-screen byte-by-byte comparison of two binary files, when open.
    pub hexdiff: Option<Box<crate::diff::hex::HexDiffView>>,
    /// The full-screen disk-mounter tool, when open.
    pub mountview: Option<MountView>,
    /// The full-screen network-connections explorer, when open (Linux).
    pub netview: Option<NetView>,
    /// The full-screen visual theme editor, when open (Options → Edit themes).
    pub theme_editor: Option<crate::ui::theme_editor::ThemeEditor>,
    /// A privileged command queued while prompting for a sudo password.
    pending_sudo: Option<PendingPriv>,
    /// A connect waiting on an SSH key passphrase: the panel it is for and the
    /// credentials to retry once the passphrase arrives.
    pending_connect: Option<(usize, RemoteCreds)>,
    /// A permission-denied answer waiting on a sudo password: the paused task
    /// and what the user chose to do about it.
    pending_priv_answer: Option<(TaskId, PrivDecision)>,
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
    /// For each file operation that consumed a panel's marked set (copy/move/
    /// delete/compress/archive), the index of the panel the selection came from
    /// (the active panel when the op started). When the task finishes, only that
    /// panel's selection is dropped — so an unrelated selection sitting on the
    /// *other* (inactive) panel is left untouched. Ops that consume no selection
    /// (sync, checksum, view-fetch) record nothing here.
    op_source: HashMap<TaskId, usize>,
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
    /// Erases the blank tail of each drawn row so terminal selections don't pick
    /// up trailing spaces (see [`crate::ui::trim`]). Carries what the last frame
    /// erased, so it must be invalidated whenever the screen is cleared behind
    /// its back — `force_full_redraw` does that.
    pub trim: crate::ui::trim::Trimmer,
    /// `graphics` preference to restore if the settings dialog is cancelled.
    graphics_backup: Option<String>,
    /// 3D view style to restore if the settings dialog is cancelled.
    space3d_backup: Option<crate::config::Space3dStyle>,
    /// The Settings tab last closed on, so the dialog reopens where it was left.
    /// Only for this session: it is where you were, not a preference.
    settings_tab: SettingsTab,
    /// The F2 user menu — entries + pattern mode, from the config `menu` file.
    user_menu: UserMenu,
    /// File-association rules (loaded from the config `rc.ext` file), consulted
    /// on Enter/F3/F4 to run Open/View/Edit actions and mount extfs scripts.
    ext_rules: crate::ext::ExtRules,
    /// A command to run in the background console after the dialog closes (expanded).
    pending_run: Option<String>,
    /// A command to run in a suspended foreground shell after the dialog closes
    /// (expanded) — the F2 user menu on a local panel, so its output is visible.
    pending_run_fg: Option<String>,
    /// A user-menu command paused while its `%{…}` interactive prompts are being
    /// answered one dialog at a time; it runs once the last answer is in.
    pending_menu: Option<PendingMenu>,
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
    /// The moment the frame now being drawn belongs to. Set by the render loop
    /// rather than read inside the renderer, so a view that rations work by
    /// wall-clock time (the 3D scene's image rebuilds) can be driven from a test
    /// — the same reason `Space3d::advance` takes its clock as an argument.
    pub frame_at: Instant,
    /// The (panel, entry) last toggled by a right-drag paint, so each entry is
    /// inverted only once as the drag passes over it.
    paint_last: Option<(usize, usize)>,
    /// Where the pointer was on the previous drag event over a 3D panel, so the
    /// orbit can be driven by how far it moved rather than where it landed.
    drag_orbit: Option<(usize, u16, u16)>,
    /// The last left click (panel, entry, when), for double-click detection: a
    /// second click on the same entry within [`DOUBLE_CLICK`] opens it like Enter.
    last_click: Option<(usize, usize, Instant)>,
    /// Per-panel "Details" view state (only computed while a panel uses the
    /// Details format): what to show about the *other* panel's cursor/selection,
    /// plus the background size-scan bookkeeping. Index = the panel displaying it.
    pub details: [crate::details::DetailsData; 2],
    /// The one audio output every audio view (the viewer's, each Details
    /// view's) plays through, so only one file is heard at a time.
    pub audio_out: crate::audio::AudioOut,
    /// Per-panel Git-status background-scan bookkeeping: the last-scanned key (the
    /// panel's local cwd, or empty for a non-local panel) and a generation counter
    /// so a stale scan result is dropped. The scanned `GitStatus` lives on the
    /// panel itself (for the renderer).
    pub(in crate::app::state) git_key: [String; 2],
    pub(in crate::app::state) git_gen: [u64; 2],
    /// After an operation completes, place a panel's cursor on a named entry: the
    /// surviving file after a delete, or the newly renamed/moved item. Stored as
    /// `(panel index, entry name)`.
    pending_focus: Option<(usize, String)>,
    /// Search/replace terms remembered in memory across editor and viewer
    /// sessions (even on different files), used to prefill their search dialogs.
    pub(in crate::app::state) search_memory: SearchMemory,
    /// Line of the first content hit for each file in the current find-file
    /// panelization, keyed by [`VfsPath::display`]. Lets F3 on a result open the
    /// viewer at the match. Replaced by each new search.
    pub(in crate::app::state) find_hit_lines: HashMap<String, u64>,
    /// Filesystem watcher behind the panels' auto-refresh, created lazily the
    /// first time a watchable directory is shown. `None` when auto-refresh is
    /// off, or when the platform refused to give us one.
    pub(in crate::app::state) watcher: Option<notify::RecommendedWatcher>,
    /// The directory each panel wants watched (`""` = none), which events are
    /// matched against.
    pub(in crate::app::state) watch_key: [String; 2],
    /// What the watcher is actually subscribed to: directory → recursive.
    pub(in crate::app::state) watch_armed: std::collections::BTreeMap<PathBuf, bool>,
    /// Watches the system refused, `(directory, recursive)`, not to be asked
    /// for again while they are still wanted.
    pub(in crate::app::state) watch_refused: std::collections::HashSet<(PathBuf, bool)>,
    /// Where the watcher's thread leaves events for the render loop.
    pub(in crate::app::state) fs_inbox: std::sync::Arc<watch::FsInbox>,
    /// How many thumbnails load at once, across both panels.
    thumb_slots: std::sync::Arc<tokio::sync::Semaphore>,
    /// When each panel was last told its directory changed. The reload waits for
    /// [`watch::DEBOUNCE`] of quiet so one command causes one re-listing.
    pub(in crate::app::state) watch_dirty: [Option<Instant>; 2],
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
    /// The running "Send file over LAN" HTTP server, alive while its dialog is
    /// open; aborted (and its temp zip removed) when the dialog closes.
    send_server: Option<crate::send::SendServer>,
    /// The running "Receive files over LAN" server, alive while its dialog is
    /// open.
    receive_server: Option<crate::receive::ReceiveServer>,
    /// The task behind a cancellable Busy spinner (a git network op or sync
    /// planning), so Esc on that spinner can abort it — otherwise an unreachable
    /// remote would hang with no escape.
    busy_task: Option<tokio::task::JoinHandle<()>>,
    /// The last command given to external panelize, used to prefill the dialog.
    /// Deliberately not the shell history: this command was never run at the
    /// prompt, so it must not come back on Alt-P. Session-only.
    last_panelize: String,
    /// The viewer's running `git blame`, aborted (killing git) when the viewer
    /// closes, and the generation that tells its answer from an older one's.
    blame_task: Option<tokio::task::JoinHandle<()>>,
    blame_gen: u64,
    /// Numbers the GeoJSON map dialog's background reads, so a late one for a
    /// dialog already closed or reopened is dropped.
    geo_gen: u64,
    /// The same, for the multi-rename dialog's background EXIF / tag read.
    rename_gen: u64,
    /// Git activity calendars already counted, newest last, so moving the
    /// Details view back over an item doesn't run `git log` again.
    activity_cache:
        std::collections::VecDeque<(String, std::sync::Arc<crate::git::activity::Activity>)>,
    /// Each Details view's pending or running activity count, aborted (which
    /// kills its `git log`) when the view moves on before it finishes.
    activity_task: [Option<tokio::task::JoinHandle<()>>; 2],
    /// When the user last pressed a key or moved the mouse, which the
    /// screensaver's idle timer counts from.
    pub last_input: Instant,
    /// The screensaver, while it is up.
    pub saver: Option<crate::saver::Saver>,
    /// Which dialog was open when the screensaver started: a different one
    /// appearing (a copy stopping to ask about an overwrite) ends it, so the
    /// question is on screen for whoever comes back.
    saver_dialog: Option<std::mem::Discriminant<Dialog>>,
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

/// Parse a command-line `cd` built-in. Returns the (possibly empty) argument,
/// with its shell quoting removed, when `cmd` is a plain `cd` to one directory.
/// Anything the shell itself has to interpret — a second word, `&&` or `;`, a
/// variable, a glob, `~user` — returns `None` so the line goes to the console
/// shell, whose directory change the active panel then follows.
fn parse_cd(cmd: &str) -> Option<String> {
    let t = cmd.trim();
    if t == "cd" {
        return Some(String::new());
    }
    let rest = t.strip_prefix("cd")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim();
    if rest.starts_with('~') && !(rest == "~" || rest.starts_with("~/")) {
        return None;
    }
    let mut word = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => loop {
                match chars.next()? {
                    '\'' => break,
                    c => word.push(c),
                }
            },
            '"' => loop {
                match chars.next()? {
                    '"' => break,
                    '$' | '`' => return None,
                    '\\' => match chars.next()? {
                        c @ ('"' | '\\') => word.push(c),
                        c => {
                            word.push('\\');
                            word.push(c);
                        }
                    },
                    c => word.push(c),
                }
            },
            '\\' => word.push(chars.next()?),
            c if c.is_whitespace() || "$`;&|<>(){}[]*?!#".contains(c) => return None,
            c => word.push(c),
        }
    }
    Some(word)
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
        || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.'))
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
    // "archive.zip!/dir" — the form the prompt is prefilled with for a panel
    // inside an archive — lands inside that archive.
    for cwd in [other_cwd, active_cwd] {
        if let Some(inner) = container_dest_prefix(cwd).and_then(|p| dest.strip_prefix(&p)) {
            return resolve_dest_on(if inner.is_empty() { "/" } else { inner }, cwd);
        }
    }
    if other_cwd.is_archive() {
        // The archive prefix was removed → a place on the local disk, as for
        // a remote destination below.
        return if Path::new(dest).is_absolute() {
            VfsPath::local(dest)
        } else {
            resolve_dest_on(dest, active_cwd)
        };
    }
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

/// The `"archive.zip!"` prefix that names a path inside `cwd`'s archive (or
/// extfs mount) in a copy/move destination, or `None` when `cwd` is not in one.
fn container_dest_prefix(cwd: &VfsPath) -> Option<String> {
    cwd.container.as_ref().map(|c| format!("{}!", c.to_string_lossy()))
}

/// Resolve a typed destination string onto the destination panel's backend
/// (`base`). Absolute paths replace the path; relative ones are joined to the
/// panel's current directory. The scheme/container of `base` are preserved, so a
/// remote destination stays on its remote backend instead of becoming local.
fn resolve_dest_on(dest: &str, base: &VfsPath) -> VfsPath {
    let p = Path::new(dest);
    let path = if p.is_absolute() { p.to_path_buf() } else { base.path.join(dest) };
    if base.scheme == "file" {
        VfsPath::local(path)
    } else {
        VfsPath { scheme: base.scheme.clone(), path, container: base.container.clone() }
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

/// Follow a relative `/`-separated path from `base` one component at a time, so
/// `..` goes through [`VfsPath::parent`] — which steps out of an archive at its
/// root — rather than being joined literally onto a backend path.
fn walk_path(base: VfsPath, rel: &str) -> VfsPath {
    rel.split('/').fold(base, |at, comp| match comp {
        "" | "." => at,
        ".." => at.parent().unwrap_or(at),
        name => at.join(name),
    })
}

/// Detect 24-bit color support from the environment.
fn detect_truecolor() -> bool {
    std::env::var("COLORTERM")
        .map(|v| v.contains("truecolor") || v.contains("24bit"))
        .unwrap_or(false)
}

mod activity;
mod checksum;
mod details;
mod dialogs;
mod disk;
mod duplicates;
mod ext;
mod fileops;
mod find;
mod git;
mod keys;
mod lifecycle;
mod mouse;
mod navigation;
mod net;
mod palette;
mod panelize;
mod receive;
mod remote;
mod sendfile;
mod sizes;
mod syncdirs;
mod tabs;
mod thumbs;
mod viewer_editor;
pub mod watch;

/// Read a file fully into memory (capped just above the viewer limit).
async fn load_file(
    backend: &std::sync::Arc<dyn Vfs>,
    path: &VfsPath,
) -> crate::util::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let reader = backend.open_read(path).await?;
    let mut buf = Vec::new();
    reader.take((MAX_VIEW_BYTES + 1) as u64).read_to_end(&mut buf).await?;
    Ok(buf)
}

/// Longest edge a viewer image is scaled to for fullscreen display.
const VIEW_IMAGE_MAX_EDGE: u32 = 2000;
/// Largest local image (bytes) opened in the fullscreen F3 viewer.
const VIEW_IMAGE_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Decode a local image file for the fullscreen viewer: a full decode scaled to
/// a display cap, with the original dimensions recorded. `None` when it is too
/// large or can't be decoded (the caller then falls back to the raw view). The
/// decode (CPU-heavy) runs on the blocking pool.
async fn load_view_image(path: &Path) -> Option<crate::viewer::ViewerImage> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    if meta.len() > VIEW_IMAGE_MAX_BYTES {
        return None;
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    tokio::task::spawn_blocking(move || {
        if crate::util::img::is_svg_bytes(&bytes) {
            // A vector has no pixel size of its own to keep, so a small one is
            // drawn larger to stay crisp. It goes on white, as a browser shows
            // it: most SVGs assume a page behind them, and dark strokes on a
            // transparent ground would vanish into a dark theme.
            let (mut img, orig) = crate::util::img::render_svg(&bytes, VIEW_IMAGE_MAX_EDGE, 1024)?;
            crate::util::img::flatten_onto(&mut img, [255, 255, 255]);
            let sig = crate::util::img::image_sig(&img);
            return Some(crate::viewer::ViewerImage { img, sig, orig });
        }
        let full = image::load_from_memory(&bytes).ok()?;
        let orig = (full.width(), full.height());
        let img = full.thumbnail(VIEW_IMAGE_MAX_EDGE, VIEW_IMAGE_MAX_EDGE).to_rgba8();
        let sig = crate::util::img::image_sig(&img);
        Some(crate::viewer::ViewerImage { img, sig, orig })
    })
    .await
    .ok()?
}

/// Parse a local model file for the fullscreen viewer. `None` when it is too
/// large or does not parse (the caller then falls back to the raw view), which
/// is also what a `.stl` that is not really one takes.
///
/// Parsing (CPU-heavy, and unbounded in the file's own size) runs on the
/// blocking pool, like the image decode above.
async fn load_view_model(path: &Path) -> Option<crate::viewer::ViewerModel> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    if meta.len() > crate::mesh::MAX_MODEL_BYTES {
        return None;
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    tokio::task::spawn_blocking(move || {
        crate::mesh::load(&bytes, &name).map(crate::viewer::ViewerModel::new)
    })
    .await
    .ok()?
}

/// Probe a local audio file for the viewer, and start drawing it — and playing
/// it, when *Auto-play audio in the viewer* is on. `None` when `name` is not an
/// audio file or it does not decode (the caller then shows the raw view).
/// `name` is the original file name: a fetched temp copy has no extension of
/// its own to hint the format with.
///
/// Only the viewer comes through here. The Details view builds its audio
/// preview itself and never starts it playing.
pub(in crate::app::state) async fn load_view_audio(
    path: &Path,
    name: &str,
    config: &crate::config::Config,
    out: crate::audio::AudioOut,
) -> Option<crate::audio::AudioView> {
    if !crate::audio::is_audio_name(name) {
        return None;
    }
    let (p, hint) = (path.to_path_buf(), crate::audio::hint_of(name));
    let info = tokio::task::spawn_blocking(move || crate::audio::probe(&p, &hint)).await.ok()??;
    let mut view = crate::audio::AudioView::new(info, config.audio_display, out);
    if config.audio_autoplay {
        view.toggle_play();
    }
    Some(view)
}

/// How long F3 waits for a binary's analysis before showing the viewer anyway.
/// Enough for an ordinary executable to open straight into its lists; a large
/// debug build keeps analysing behind an "Analyzing…" screen instead.
const BINARY_SETTLE: std::time::Duration = std::time::Duration::from_millis(250);

/// Open the viewer in Binary mode when the file at `path` is an executable or a
/// library, with the analysis running in the background.
async fn open_binary(v: &mut crate::viewer::ViewerState, path: &Path) {
    let p = path.to_path_buf();
    let sniffed = tokio::task::spawn_blocking(move || crate::viewer::binary::sniff_file(&p))
        .await
        .unwrap_or(false);
    if sniffed {
        v.analyze_binary(path.to_path_buf());
        v.settle_binary(BINARY_SETTLE).await;
    }
}

/// Show what a certificate or key file holds — unless the file opened as a
/// binary. The file is read and parsed off the main thread.
async fn open_certs(v: &mut crate::viewer::ViewerState, path: &Path) {
    if v.is_binary_mode() {
        return;
    }
    let (p, name) = (path.to_path_buf(), v.name.clone());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let report = tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut head = Vec::with_capacity(64);
        let mut file = std::fs::File::open(&p).ok()?;
        if file.metadata().ok()?.len() > crate::certs::MAX_BYTES {
            return None;
        }
        file.by_ref().take(64).read_to_end(&mut head).ok()?;
        if !crate::certs::sniff(&name, &head) {
            return None;
        }
        let mut data = head;
        file.read_to_end(&mut data).ok()?;
        crate::certs::inspect(&name, &data, now)
    })
    .await
    .ok()
    .flatten();
    if let Some(r) = report {
        v.set_certs(r);
    }
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
    let mut w = backend.open_write(path, crate::vfs::WriteMeta::default()).await?;
    w.write_all(data).await?;
    // `shutdown()` (not just `flush()`) is what finalizes the write: for the
    // remote backends the writer is a pipe whose flush only pushes bytes into the
    // channel, while closing the pipe and awaiting the upload's result — and any
    // error it carries — happens on shutdown. Without this an editor save over
    // SFTP/SCP/FTP could report success while the upload was incomplete or failed.
    // (Mirrors the copy engine's contract; see `src/ops/engine.rs` copy_file.)
    w.shutdown().await?;
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

/// The native provider that claims the entry under the cursor, if any — the step
/// between the built-in archive formats and the `rc.ext` rules. See
/// [`crate::vfs::native`] for why the order is what it is.
fn native_target_under_cursor(p: &Panel) -> Option<(VfsPath, Option<String>)> {
    if p.cwd.scheme != "file" {
        return None;
    }
    let e = p.current_entry()?;
    let file = p.cwd.path.join(&e.name);
    crate::vfs::native::probe(&file, e.kind).map(|o| (o.path, None))
}

/// How much of a file we read at a time when grepping. Windows overlap by the
/// needle's own `overlap()` so a match straddling a seam is still found.
const GREP_WINDOW: usize = 64 * 1024;

/// Search `path` for `needle`, returning the **1-based line number** of the first
/// match (`None` when there is none, the file can't be read, or it looks binary).
///
/// Streams the file in overlapping windows rather than reading it whole, so a
/// multi-gigabyte file costs a fixed amount of memory. Files whose first window
/// holds a NUL byte are treated as binary and skipped, the way `grep` does.
///
/// Carries the same caveat as the viewer's own windowed search (see
/// [`crate::viewer::search::RE_OVERLAP`]): a single *regex* match longer than
/// 64 KiB can be missed at a window seam.
fn grep_file(path: &Path, needle: &crate::viewer::search::Needle) -> Option<u64> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let overlap = needle.overlap().min(GREP_WINDOW - 1);
    // A literal, case-sensitive needle is by far the common case; `memmem` is a
    // proper substring search where `Needle::find` is a naive scan, and this walks
    // a whole tree rather than the viewer's single window.
    let fast = match needle {
        crate::viewer::search::Needle::Bytes {
            pat,
            case_insensitive: false,
            whole_words: false,
        } => Some(memchr::memmem::Finder::new(pat).into_owned()),
        _ => None,
    };

    let mut buf = vec![0u8; GREP_WINDOW];
    // Bytes carried over from the previous window, and how many lines ended
    // before the start of `buf` — together these turn a window-local hit offset
    // into a file-wide line number.
    let mut carry = 0usize;
    let mut lines_before = 0u64;
    let mut first = true;
    loop {
        let read = match file.read(&mut buf[carry..]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return None,
        };
        let filled = carry + read;
        let window = &buf[..filled];
        if first {
            first = false;
            if memchr::memchr(0, window).is_some() {
                return None; // binary
            }
        }
        let hit = match &fast {
            Some(f) => f.find(window),
            None => needle.find(window, 0),
        };
        if let Some(at) = hit {
            return Some(
                lines_before + memchr::memchr_iter(b'\n', &window[..at]).count() as u64 + 1,
            );
        }
        if filled < GREP_WINDOW {
            break; // last (short) window, no match
        }
        // Keep the tail as the next window's prefix; everything before it is
        // behind us for good, so fold its newlines into the running count.
        let keep = overlap.min(filled);
        let drop_to = filled - keep;
        lines_before += memchr::memchr_iter(b'\n', &window[..drop_to]).count() as u64;
        buf.copy_within(drop_to..filled, 0);
        carry = keep;
    }
    None
}

/// Recursively find files under `start`, reporting progress and honouring
/// cancellation. Returns whatever was collected (partial on abort), each match
/// paired with the line of its first content hit (`None` when searching by name
/// only).
fn find_files(
    start: &Path,
    p: &FindParams,
    matcher: &crate::panel::selection::NameMatcher,
    cancel: &crate::ops::CancelToken,
    mut progress: impl FnMut(String, usize),
) -> Vec<(PathBuf, Option<u64>)> {
    const MAX_RESULTS: usize = 50_000;

    // Reuse the viewer's matcher so regex / case / whole-word all mean exactly
    // what they mean in the viewer's own search. Whole-word and hex aren't
    // exposed by this dialog, so they are fixed off here.
    let content_needle = if p.content.is_empty() {
        None
    } else {
        match crate::viewer::search::Needle::build(
            &p.content,
            p.regex_content,
            p.case_sensitive,
            false,
            false,
        ) {
            Some(n) => Some(n),
            // An unusable pattern (bad regex) would otherwise silently match
            // nothing; the caller validates first, so this is belt and braces.
            None => return Vec::new(),
        }
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
        let line = match &content_needle {
            Some(needle) => match grep_file(entry.path(), needle) {
                Some(line) => Some(line),
                None => continue,
            },
            None => None,
        };
        out.push((entry.path().to_path_buf(), line));
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
) -> Vec<crate::app::event::FindHit> {
    const MAX_RESULTS: usize = 50_000;
    let mut out: Vec<crate::app::event::FindHit> = Vec::new();
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
                out.push(crate::app::event::FindHit { path: child, size: e.size, line: None });
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
    let _ = tokio::process::Command::new("cmd").args(["/C", "start", ""]).arg(&path).spawn();
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
    let Ok(def) =
        tokio::process::Command::new("xdg-mime").args(["query", "default", &mime]).output().await
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
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)).ok().flatten().map(|u| u.name)
}

#[cfg(unix)]
fn gid_name(gid: u32) -> Option<String> {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid)).ok().flatten().map(|g| g.name)
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
