//! [`VfsPath`] — a backend-scheme-tagged absolute path.
//!
//! Phase 1 only needs `file://` paths, so this is a thin wrapper over an
//! absolute [`PathBuf`] plus a scheme string. The type is deliberately the one
//! choke point through which all path manipulation flows, so that Phase 4
//! (archive nesting) and Phase 5 (remote roots) can extend it without touching
//! callers.

use std::path::{Path, PathBuf};

/// An absolute path within a particular VFS backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VfsPath {
    /// Backend scheme: `"file"`, later `"sftp"`, `"ftp"`, `"scp"`, `"archive"`.
    pub scheme: String,
    /// Absolute path inside the backend's namespace.
    pub path: PathBuf,
}

impl VfsPath {
    /// A local-filesystem path.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        VfsPath {
            scheme: "file".to_string(),
            path: path.into(),
        }
    }

    /// The current local working directory, or `/` if it cannot be determined.
    pub fn local_cwd() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        VfsPath::local(cwd)
    }

    /// Borrow the inner path.
    pub fn as_path(&self) -> &Path {
        &self.path
    }

    /// The final component (file name), or the whole path for the root.
    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    /// The parent path, or `None` at the backend root.
    pub fn parent(&self) -> Option<VfsPath> {
        self.path.parent().map(|p| VfsPath {
            scheme: self.scheme.clone(),
            path: p.to_path_buf(),
        })
    }

    /// Append a single component.
    pub fn join(&self, name: impl AsRef<Path>) -> VfsPath {
        VfsPath {
            scheme: self.scheme.clone(),
            path: self.path.join(name),
        }
    }

    /// Display string for the location bar.
    pub fn display(&self) -> String {
        if self.scheme == "file" {
            self.path.to_string_lossy().into_owned()
        } else {
            format!("{}://{}", self.scheme, self.path.to_string_lossy())
        }
    }
}
