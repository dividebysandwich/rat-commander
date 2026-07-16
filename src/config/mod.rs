//! Persistent configuration: external programs and behaviour flags.
//!
//! Stored as TOML under the user's XDG config directory. Loading never fails
//! hard — a missing or malformed file falls back to defaults so the app always
//! starts.

pub mod paths;

use serde::{Deserialize, Serialize};

/// A previously-used remote connection, remembered for the connect dialog's
/// dropdown. Passwords are intentionally *not* stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteHistoryEntry {
    /// Protocol scheme prefix: `"sftp"`, `"ftp"`, or `"scp"`.
    pub protocol: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub path: String,
    /// FTP passive mode for this server (see [`crate::vfs::remote::RemoteCreds`]).
    /// Defaults to `true` so entries saved before this field existed reconnect in
    /// passive mode, the FTP default.
    #[serde(default = "crate::config::default_true")]
    pub passive: bool,
}

/// serde default for [`RemoteHistoryEntry::passive`].
pub(crate) fn default_true() -> bool {
    true
}

impl RemoteHistoryEntry {
    /// One-line label for the dropdown, e.g. `user@host:22  /remote/path`.
    pub fn label(&self) -> String {
        let user = if self.user.is_empty() {
            String::new()
        } else {
            format!("{}@", self.user)
        };
        let path = if self.path.is_empty() {
            String::new()
        } else {
            format!("  {}", self.path)
        };
        format!("{user}{}:{}{path}", self.host, self.port)
    }
}

/// A panel's remembered view state: its listing format and sort order. Stored
/// per panel so the two sides restore independently across sessions.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelView {
    pub format: crate::panel::ViewFormat,
    pub sort: crate::panel::sort::SortConfig,
}

/// User configuration, serialized to `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// External editor command (e.g. "vim", "code --wait"). Empty = use the
    /// internal editor.
    pub editor: String,
    /// External viewer/pager command (e.g. "less", "bat"). Empty = use the
    /// internal viewer.
    pub viewer: String,
    /// Prefer the internal viewer even when `viewer` is set.
    pub use_internal_viewer: bool,
    /// Prefer the internal editor even when `editor` is set.
    pub use_internal_editor: bool,
    /// Ask for confirmation before deleting.
    pub confirm_delete: bool,
    /// Ask before overwriting an existing destination during copy/move.
    pub confirm_overwrite: bool,
    /// Ask before opening/executing a file with its default application.
    pub confirm_execute: bool,
    /// Ask before unmounting a filesystem in the disk manager.
    pub confirm_unmount: bool,
    /// Ask for confirmation before quitting.
    pub confirm_exit: bool,
    /// Active color theme (palette name).
    pub theme: String,
    /// Active UI language (the language file's display name, e.g. "Deutsch").
    /// `None` = English (the default).
    #[serde(default)]
    pub language: Option<String>,
    /// Reshape + bidi-reorder right-to-left text (Arabic/Persian) into visual
    /// order so it reads correctly on terminals without native bidi support.
    /// Turn off on terminals that do their own bidi (mlterm, modern VTE, …).
    /// (Missing from an old config → the struct default, `true`.)
    pub reshape_rtl: bool,
    /// Terminal pixel-graphics for the progress bars, process-explorer graphs and
    /// disk-explorer treemap: `auto` (use Kitty/Sixel/iTerm2 if the terminal
    /// supports it, else fall back to cell rendering), `off`, or a forced
    /// `kitty` / `sixel` / `iterm`. (Missing from an old config → the struct
    /// default, `"auto"`.)
    pub graphics: String,
    /// 24-bit color override; `None` = auto-detect from the terminal.
    pub truecolor: Option<bool>,
    /// Enable animations (gradient motion, CPU histogram).
    pub animation: bool,
    /// Show the CPU/memory status widget in the menu bar.
    pub system_status: bool,
    /// Number of columns in the Brief (multi-column names) view.
    /// (Missing from an old config → the struct default, `2`.)
    pub brief_columns: usize,
    /// Maximum number of command-line entries kept in the persistent history
    /// (`history` file next to this config). `0` disables it. (Missing from an
    /// old config → the struct default, `100`.)
    pub command_history_max: usize,
    /// Per-panel view format and sort order, remembered across sessions
    /// (index 0 = left panel, 1 = right panel).
    #[serde(default)]
    pub panels: [PanelView; 2],
    /// Recently used remote connections (most recent first), for the connect
    /// dialog's history dropdown.
    #[serde(default)]
    pub recent_remotes: Vec<RemoteHistoryEntry>,
    /// Bookmarked local directories (absolute paths), listed and jumpable from
    /// the command palette (Ctrl-P). (Missing from an old config → empty.)
    #[serde(default)]
    pub bookmarks: Vec<String>,

    // -- Session layout, restored on the next launch (all default-on-absent) --
    /// Each panel's last *local* directory (index 0 = left, 1 = right). Empty
    /// when the panel was on a remote/archive location (not restorable without
    /// credentials) or the saved directory no longer exists.
    #[serde(default)]
    pub panel_dirs: [String; 2],
    /// Each panel's persistent listing filter (`Alt-Shift-I`); empty = none.
    #[serde(default)]
    pub panel_filters: [String; 2],
    /// Panel split: `true` = horizontal (stacked), `false` = vertical (the
    /// classic side-by-side default).
    #[serde(default)]
    pub split_horizontal: bool,
    /// Which panels were hidden (`Ctrl-F1`/`Ctrl-F2`).
    #[serde(default)]
    pub panel_hidden: [bool; 2],
    /// Half-height mode (`Ctrl-F4`).
    #[serde(default)]
    pub half_height: bool,
    /// The active panel (0 = left/top, 1 = right/bottom).
    #[serde(default)]
    pub active_panel: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            editor: String::new(),
            viewer: String::new(),
            use_internal_viewer: true,
            use_internal_editor: true,
            confirm_delete: true,
            confirm_overwrite: true,
            confirm_execute: false,
            confirm_unmount: true,
            confirm_exit: true,
            theme: "Rat Commander".to_string(),
            language: None,
            reshape_rtl: true,
            graphics: "auto".to_string(),
            truecolor: None,
            animation: false,
            system_status: true,
            brief_columns: 2,
            command_history_max: 100,
            panels: [PanelView::default(); 2],
            recent_remotes: Vec::new(),
            bookmarks: Vec::new(),
            panel_dirs: [String::new(), String::new()],
            panel_filters: [String::new(), String::new()],
            split_horizontal: false,
            panel_hidden: [false, false],
            half_height: false,
            active_panel: 0,
        }
    }
}

/// A trimmed non-empty copy of `s`, else `None`.
fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// A shell command from environment variable `var`, if set and non-empty.
fn env_command(var: &str) -> Option<String> {
    std::env::var(var).ok().and_then(|v| non_empty(&v))
}

impl Config {
    /// Load the config, falling back to defaults on any error.
    pub fn load() -> Self {
        let Some(path) = paths::config_file() else {
            return Config::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    }

    /// Persist the config to disk. Returns an error string on failure.
    pub fn save(&self) -> Result<(), String> {
        let path = paths::config_file().ok_or("no config directory available")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }

    /// The external editor command: the configured `editor`, or — when that is
    /// empty — the `$VISUAL` then `$EDITOR` environment variables (the Unix
    /// convention). `None` when none is set, meaning the internal editor is used.
    pub fn external_editor(&self) -> Option<String> {
        non_empty(&self.editor)
            .or_else(|| env_command("VISUAL"))
            .or_else(|| env_command("EDITOR"))
    }

    /// The external viewer/pager command: the configured `viewer`, or `$PAGER`.
    pub fn external_viewer(&self) -> Option<String> {
        non_empty(&self.viewer).or_else(|| env_command("PAGER"))
    }

    /// Whether to use the internal viewer for the given situation.
    pub fn wants_internal_viewer(&self) -> bool {
        self.use_internal_viewer || self.external_viewer().is_none()
    }

    /// Whether to use the internal editor.
    pub fn wants_internal_editor(&self) -> bool {
        self.use_internal_editor || self.external_editor().is_none()
    }

    /// Record a successful remote connection at the front of the history,
    /// de-duplicating the same server and capping the list.
    pub fn add_recent_remote(&mut self, entry: RemoteHistoryEntry) {
        self.recent_remotes.retain(|e| {
            !(e.protocol == entry.protocol
                && e.host == entry.host
                && e.port == entry.port
                && e.user == entry.user)
        });
        self.recent_remotes.insert(0, entry);
        self.recent_remotes.truncate(20);
    }
}

/// Load the persisted command-line history (oldest first), keeping at most the
/// `max` most-recent entries. A missing file or any error yields an empty list.
pub fn load_command_history(max: usize) -> Vec<String> {
    paths::history_file().map(|p| load_history_from(&p, max)).unwrap_or_default()
}

/// Persist the command-line history (oldest first), one entry per line, keeping
/// at most the `max` most-recent entries. Best-effort; errors are ignored.
pub fn save_command_history(history: &[String], max: usize) {
    if let Some(p) = paths::history_file() {
        save_history_to(&p, history, max);
    }
}

fn load_history_from(path: &std::path::Path, max: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut lines: Vec<String> =
        text.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
    if lines.len() > max {
        lines.drain(..lines.len() - max);
    }
    lines
}

fn save_history_to(path: &std::path::Path, history: &[String], max: usize) {
    // One command per line, so skip entries with embedded newlines (a pasted
    // multi-line command) and blank entries.
    let clean: Vec<&String> = history
        .iter()
        .filter(|e| !e.trim().is_empty() && !e.contains(['\n', '\r']))
        .collect();
    let start = clean.len().saturating_sub(max);
    let body: String = clean[start..].iter().map(|e| format!("{e}\n")).collect();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, body);
}

/// How many files' cursor positions the editor remembers.
const EDITOR_POSITIONS_MAX: usize = 50;

/// One remembered editor cursor position: a file key (its [`VfsPath::display`])
/// and the 0-based line/column the cursor was on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EditorPos {
    key: String,
    line: usize,
    col: usize,
}

/// The editor's cursor-position memory, most-recent first.
#[derive(Debug, Default, Serialize, Deserialize)]
struct EditorPositions {
    #[serde(default)]
    positions: Vec<EditorPos>,
}

/// The remembered `(line, col)` cursor position for the file `key` (a
/// [`crate::vfs::VfsPath::display`] string), or `None` if not remembered.
pub fn load_editor_position(key: &str) -> Option<(usize, usize)> {
    paths::editor_positions_file().and_then(|p| position_from(&p, key))
}

/// Remember the cursor `(line, col)` for the file `key`, moving it to the front
/// and evicting the oldest beyond the 50-file cap. Best-effort; errors ignored.
pub fn save_editor_position(key: &str, line: usize, col: usize) {
    if let Some(p) = paths::editor_positions_file() {
        store_position_to(&p, key, line, col);
    }
}

fn read_editor_positions(path: &std::path::Path) -> EditorPositions {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn position_from(path: &std::path::Path, key: &str) -> Option<(usize, usize)> {
    read_editor_positions(path)
        .positions
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| (e.line, e.col))
}

fn store_position_to(path: &std::path::Path, key: &str, line: usize, col: usize) {
    let mut data = read_editor_positions(path);
    data.positions.retain(|e| e.key != key);
    data.positions.insert(0, EditorPos { key: key.to_string(), line, col });
    data.positions.truncate(EDITOR_POSITIONS_MAX);
    let Ok(text) = toml::to_string(&data) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(host: &str, path: &str) -> RemoteHistoryEntry {
        RemoteHistoryEntry {
            protocol: "sftp".into(),
            host: host.into(),
            port: 22,
            user: "u".into(),
            path: path.into(),
            passive: true,
        }
    }

    #[test]
    fn panel_views_round_trip_through_toml() {
        use crate::panel::ViewFormat;
        use crate::panel::sort::SortKey;

        let mut c = Config::default();
        c.panels[0].format = ViewFormat::Brief;
        c.panels[0].sort.key = SortKey::Size;
        c.panels[0].sort.reverse = true;
        c.panels[1].format = ViewFormat::Details;
        c.panels[1].sort.key = SortKey::Extension;

        // Serialize + parse back, as save()/load() would.
        let text = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&text).unwrap();

        assert_eq!(back.panels[0].format, ViewFormat::Brief);
        assert_eq!(back.panels[0].sort.key, SortKey::Size);
        assert!(back.panels[0].sort.reverse);
        assert_eq!(back.panels[1].format, ViewFormat::Details);
        assert_eq!(back.panels[1].sort.key, SortKey::Extension);
    }

    #[test]
    fn old_config_without_panels_field_uses_defaults() {
        // A config file predating the panel-state field still parses.
        let back: Config = toml::from_str("theme = \"Nord\"\n").unwrap();
        assert_eq!(back.panels[0].format, crate::panel::ViewFormat::Full);
        assert_eq!(back.brief_columns, 2);
    }

    #[test]
    fn add_recent_dedupes_caps_and_orders() {
        let mut c = Config::default();
        for i in 0..25 {
            c.add_recent_remote(entry(&format!("h{i}"), ""));
        }
        assert_eq!(c.recent_remotes.len(), 20, "capped at 20");
        assert_eq!(c.recent_remotes[0].host, "h24", "most recent first");

        // Re-adding an existing server moves it to the front and updates its path.
        c.add_recent_remote(entry("h10", "/new"));
        assert_eq!(c.recent_remotes[0].host, "h10");
        assert_eq!(c.recent_remotes[0].path, "/new");
        assert_eq!(
            c.recent_remotes.iter().filter(|e| e.host == "h10").count(),
            1,
            "no duplicate"
        );
    }

    #[test]
    fn command_history_round_trips_and_caps_at_max() {
        let dir = std::env::temp_dir().join(format!("rc_hist_cfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("history");

        let hist: Vec<String> = ["one", "two", "three", "four"].iter().map(|s| s.to_string()).collect();
        // Save keeps only the most-recent `max` entries…
        save_history_to(&path, &hist, 2);
        assert_eq!(load_history_from(&path, 10), vec!["three".to_string(), "four".to_string()]);
        // …and load also caps (e.g. after the max was lowered).
        save_history_to(&path, &hist, 10);
        assert_eq!(load_history_from(&path, 1), vec!["four".to_string()]);
        // Blank / multi-line entries are not persisted; a missing file → empty.
        save_history_to(&path, &["ok".into(), "  ".into(), "a\nb".into()], 100);
        assert_eq!(load_history_from(&path, 100), vec!["ok".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_history_from(&path, 100).is_empty());
    }

    #[test]
    fn remote_history_defaults_passive_true_for_old_entries() {
        // An entry saved before the PASV field existed reconnects in passive mode.
        let e: RemoteHistoryEntry =
            toml::from_str("protocol = \"ftp\"\nhost = \"h\"\nport = 21\n").unwrap();
        assert!(e.passive);
        // A stored value is honoured either way.
        let e: RemoteHistoryEntry =
            toml::from_str("protocol = \"ftp\"\nhost = \"h\"\nport = 21\npassive = false\n").unwrap();
        assert!(!e.passive);
    }

    #[test]
    fn config_ignores_unknown_and_missing_fields() {
        // An old config without `command_history_max` gets the default; an
        // unknown key (e.g. the removed `quick_search`) is ignored.
        let c: Config = toml::from_str("quick_search = true\nbrief_columns = 3\n").unwrap();
        assert_eq!(c.command_history_max, 100);
        assert_eq!(c.brief_columns, 3);
    }

    #[test]
    fn editor_positions_round_trip_move_to_front_and_cap() {
        let dir = std::env::temp_dir().join(format!("rc_edpos_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("editor-positions.toml");

        // A missing file → no remembered position.
        assert_eq!(position_from(&path, "/a"), None);

        store_position_to(&path, "/a", 10, 2);
        store_position_to(&path, "/b", 5, 0);
        assert_eq!(position_from(&path, "/a"), Some((10, 2)));
        assert_eq!(position_from(&path, "/b"), Some((5, 0)));

        // Re-storing the same file updates it and moves it to the front.
        store_position_to(&path, "/a", 33, 7);
        assert_eq!(position_from(&path, "/a"), Some((33, 7)));
        assert_eq!(read_editor_positions(&path).positions[0].key, "/a");

        // Only the 50 most-recent files are kept.
        for i in 0..60 {
            store_position_to(&path, &format!("/f{i}"), i, 0);
        }
        let data = read_editor_positions(&path);
        assert_eq!(data.positions.len(), EDITOR_POSITIONS_MAX);
        assert_eq!(position_from(&path, "/f59"), Some((59, 0)), "newest kept");
        assert_eq!(position_from(&path, "/f0"), None, "oldest evicted");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod env_fallback_tests {
    use super::*;

    #[test]
    fn non_empty_trims_and_rejects_blank() {
        assert_eq!(non_empty("  vim  ").as_deref(), Some("vim"));
        assert_eq!(non_empty("   "), None);
        assert_eq!(non_empty(""), None);
    }

    #[test]
    fn external_program_prefers_the_configured_value() {
        // A configured command wins outright — the environment is never consulted,
        // so this is deterministic regardless of the test runner's env.
        let c = Config {
            use_internal_editor: false,
            use_internal_viewer: false,
            editor: "code --wait".into(),
            viewer: "  bat  ".into(),
            ..Config::default()
        };
        assert_eq!(c.external_editor().as_deref(), Some("code --wait"));
        assert_eq!(c.external_viewer().as_deref(), Some("bat"), "trimmed");
        // With the internal toggle off and an external configured, the external wins.
        assert!(!c.wants_internal_editor() && !c.wants_internal_viewer());
    }

    #[test]
    fn external_program_falls_back_to_visual_editor_pager_env() {
        // editor/viewer empty; internal toggles off so `wants_internal_*` reflects
        // purely whether an external command resolved (config or env).
        let c = Config {
            use_internal_editor: false,
            use_internal_viewer: false,
            ..Config::default()
        };
        // Save and clear the vars this test drives, restore them afterward.
        let vars = ["VISUAL", "EDITOR", "PAGER"];
        let saved: Vec<Option<String>> = vars.iter().map(|k| std::env::var(k).ok()).collect();
        let set = |k: &str, v: &str| unsafe { std::env::set_var(k, v) };
        let clear = |k: &str| unsafe { std::env::remove_var(k) };

        vars.iter().for_each(|k| clear(k));
        assert_eq!(c.external_editor(), None, "no config, no env → the internal editor");
        assert!(c.wants_internal_editor() && c.wants_internal_viewer(), "nothing external → internal");

        set("EDITOR", "vi");
        assert_eq!(c.external_editor().as_deref(), Some("vi"));
        set("VISUAL", "nvim");
        assert_eq!(c.external_editor().as_deref(), Some("nvim"), "$VISUAL beats $EDITOR");
        set("PAGER", "less");
        assert_eq!(c.external_viewer().as_deref(), Some("less"));
        assert!(!c.wants_internal_editor() && !c.wants_internal_viewer(), "env external now used");

        for (k, v) in vars.iter().zip(saved) {
            match v {
                Some(val) => set(k, &val),
                None => clear(k),
            }
        }
    }
}
