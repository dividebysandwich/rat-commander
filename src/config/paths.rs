//! Resolution of the configuration file location: the platform config
//! directory (XDG, `%APPDATA%`, `~/Library`), or a `config` folder beside the
//! executable when a `portable` marker file sits next to it.

use directories::ProjectDirs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Name of the marker file that, next to the executable, switches to portable
/// mode.
const PORTABLE_MARKER: &str = "portable";

/// The directory every configuration file lives in, or `None` if none can be
/// determined. Resolved once per process.
pub fn config_dir() -> Option<PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
        resolve(exe.as_deref().and_then(Path::parent))
    })
    .clone()
}

/// `<exe_dir>/config` when `exe_dir` holds the portable marker, else the
/// platform's per-user config directory.
fn resolve(exe_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = exe_dir.filter(|d| d.join(PORTABLE_MARKER).is_file()) {
        return Some(dir.join("config"));
    }
    ProjectDirs::from("", "", "rat-commander").map(|d| d.config_dir().to_path_buf())
}

/// Path to `config.toml`, or `None` if no config directory can be determined.
pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

/// Path to the FTPS servers' pinned certificates (`ftps_known_hosts`).
pub fn ftps_known_hosts_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("ftps_known_hosts"))
}

/// Path to the F2 user-menu file (`menu`), or `None` if undetermined.
pub fn menu_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("menu"))
}

/// Path to the file-association file (`rc.ext`, Midnight-Commander `mc.ext`
/// format), or `None` if the config directory can't be determined.
pub fn ext_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("rc.ext"))
}

/// Path to the user's `extfs.d/` script directory (searched, alongside the MC
/// system dirs, for extfs mount scripts), or `None` if undetermined.
pub fn extfs_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("extfs.d"))
}

/// Path to the user themes file (`themes.toml`), or `None` if undetermined.
pub fn themes_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("themes.toml"))
}

/// Path to the localization directory (`lang/`), which holds one TOML file per
/// language; or `None` if the config directory can't be determined.
pub fn lang_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("lang"))
}

/// Path to the binary-template directory (`templates/`), where the bundled
/// 010 Editor templates are deployed and the user's own `.bt` files live; or
/// `None` if the config directory can't be determined.
pub fn templates_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("templates"))
}

/// Path to the persistent command-line history file (`history`, one command per
/// line), or `None` if the config directory can't be determined.
pub fn history_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("history"))
}

/// Path to the editor cursor-position memory file (`editor-positions.toml`), or
/// `None` if the config directory can't be determined.
pub fn editor_positions_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("editor-positions.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_marker_keeps_config_beside_the_exe() {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("rc-portable-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        assert_ne!(resolve(Some(&dir)), Some(dir.join("config")), "no marker, no portable mode");
        std::fs::write(dir.join(PORTABLE_MARKER), b"").unwrap();
        assert_eq!(resolve(Some(&dir)), Some(dir.join("config")));

        std::fs::remove_dir_all(&dir).ok();
    }
}
