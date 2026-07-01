//! [`VfsPath`] — a backend-scheme-tagged absolute path, with optional archive
//! nesting.
//!
//! A plain path (local/remote) has `container == None`. An archive path has
//! `scheme == "archive"`, `container == Some(archive_file_on_local_disk)`, and
//! `path` holding the absolute path *inside* the archive (root = `/`).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VfsPath {
    /// Backend scheme: `"file"`, `"archive"`, later `"sftp"`/`"ftp"`/`"scp"`.
    pub scheme: String,
    /// Absolute path inside the backend (for archives, inside the archive).
    pub path: PathBuf,
    /// The archive file (on local disk) when `scheme == "archive"`.
    pub container: Option<PathBuf>,
}

impl VfsPath {
    /// A local-filesystem path.
    pub fn local(path: impl Into<PathBuf>) -> Self {
        VfsPath {
            scheme: "file".to_string(),
            path: path.into(),
            container: None,
        }
    }

    /// A path inside an archive. `inner` is absolute within the archive.
    pub fn archive(container: impl Into<PathBuf>, inner: impl Into<PathBuf>) -> Self {
        VfsPath {
            scheme: "archive".to_string(),
            path: inner.into(),
            container: Some(container.into()),
        }
    }

    /// The current local working directory, or `/` if it cannot be determined.
    pub fn local_cwd() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        VfsPath::local(cwd)
    }

    pub fn is_archive(&self) -> bool {
        self.container.is_some()
    }

    /// True for a non-local backend (an `sftp-`/`ftp-`/`scp-` session scheme).
    /// The local disk (`file`) and archives (`archive`) both count as local.
    pub fn is_remote(&self) -> bool {
        self.scheme != "file" && self.scheme != "archive"
    }

    /// True when this points at the root of an archive.
    pub fn is_archive_root(&self) -> bool {
        self.is_archive() && (self.path == Path::new("/") || self.path.as_os_str().is_empty())
    }

    /// The archive file's name, if this is an archive path.
    pub fn container_name(&self) -> Option<String> {
        self.container
            .as_ref()
            .and_then(|c| c.file_name())
            .map(|s| s.to_string_lossy().into_owned())
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

    /// The parent path. At the root of an archive, this exits the archive back
    /// to the directory containing the archive file on local disk.
    pub fn parent(&self) -> Option<VfsPath> {
        if self.is_archive() {
            if self.is_archive_root() {
                // Exit the archive.
                let container = self.container.as_ref()?;
                return container.parent().map(VfsPath::local);
            }
            return self.path.parent().map(|p| VfsPath {
                scheme: self.scheme.clone(),
                path: p.to_path_buf(),
                container: self.container.clone(),
            });
        }
        self.path.parent().map(|p| VfsPath {
            scheme: self.scheme.clone(),
            path: p.to_path_buf(),
            container: None,
        })
    }

    /// Append a single component.
    pub fn join(&self, name: impl AsRef<Path>) -> VfsPath {
        VfsPath {
            scheme: self.scheme.clone(),
            path: self.path.join(name),
            container: self.container.clone(),
        }
    }

    /// Display string for the location bar.
    pub fn display(&self) -> String {
        if let Some(c) = &self.container {
            format!("{}!{}", c.to_string_lossy(), self.path.to_string_lossy())
        } else if self.scheme == "file" {
            self.path.to_string_lossy().into_owned()
        } else {
            format!("{}://{}", self.scheme, self.path.to_string_lossy())
        }
    }
}
