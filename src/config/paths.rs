//! Resolution of the configuration file location (XDG).

use directories::ProjectDirs;
use std::path::PathBuf;

/// Path to `config.toml`, or `None` if no config directory can be determined.
pub fn config_file() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().join("config.toml"))
}

/// Path to the F2 user-menu file (`menu`), or `None` if undetermined.
pub fn menu_file() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().join("menu"))
}
