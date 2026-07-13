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

/// Path to the file-association file (`rc.ext`, Midnight-Commander `mc.ext`
/// format), or `None` if the config directory can't be determined.
pub fn ext_file() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().join("rc.ext"))
}

/// Path to the user's `extfs.d/` script directory (searched, alongside the MC
/// system dirs, for extfs mount scripts), or `None` if undetermined.
pub fn extfs_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().join("extfs.d"))
}

/// Path to the user themes file (`themes.toml`), or `None` if undetermined.
pub fn themes_file() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().join("themes.toml"))
}

/// Path to the localization directory (`lang/`), which holds one TOML file per
/// language; or `None` if the config directory can't be determined.
pub fn lang_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().join("lang"))
}

/// Path to the persistent command-line history file (`history`, one command per
/// line), or `None` if the config directory can't be determined.
pub fn history_file() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().join("history"))
}

/// Path to the editor cursor-position memory file (`editor-positions.toml`), or
/// `None` if the config directory can't be determined.
pub fn editor_positions_file() -> Option<PathBuf> {
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().join("editor-positions.toml"))
}
