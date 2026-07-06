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
    /// FAR/NC-style quick search in the active panel: Alt+letter starts an
    /// incremental name search that jumps the cursor to the first match. When
    /// disabled, Alt+letter opens the matching top menu (classic behavior).
    pub quick_search: bool,
    /// Per-panel view format and sort order, remembered across sessions
    /// (index 0 = left panel, 1 = right panel).
    #[serde(default)]
    pub panels: [PanelView; 2],
    /// Recently used remote connections (most recent first), for the connect
    /// dialog's history dropdown.
    #[serde(default)]
    pub recent_remotes: Vec<RemoteHistoryEntry>,
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
            theme: "Midnight Commander".to_string(),
            language: None,
            reshape_rtl: true,
            graphics: "auto".to_string(),
            quick_search: true,
            truecolor: None,
            animation: true,
            system_status: true,
            brief_columns: 2,
            panels: [PanelView::default(); 2],
            recent_remotes: Vec::new(),
        }
    }
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

    /// Whether to use the internal viewer for the given situation.
    pub fn wants_internal_viewer(&self) -> bool {
        self.use_internal_viewer || self.viewer.trim().is_empty()
    }

    /// Whether to use the internal editor.
    pub fn wants_internal_editor(&self) -> bool {
        self.use_internal_editor || self.editor.trim().is_empty()
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
}
