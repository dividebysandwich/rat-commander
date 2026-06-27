//! Persistent configuration: external programs and behaviour flags.
//!
//! Stored as TOML under the user's XDG config directory. Loading never fails
//! hard — a missing or malformed file falls back to defaults so the app always
//! starts.

pub mod paths;

use serde::{Deserialize, Serialize};

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
    /// Active color theme (palette name).
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            editor: String::new(),
            viewer: String::new(),
            use_internal_viewer: true,
            use_internal_editor: true,
            confirm_delete: true,
            theme: "Midnight Commander".to_string(),
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
}
