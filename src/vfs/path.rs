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

    /// A path inside a file mounted via an MC-style `extfs` script. The `prefix`
    /// (e.g. `"uzip"`, `"iso9660"`) is both the backend scheme and the script
    /// name; `container` is the archive file on local disk, `inner` the absolute
    /// path within the mount (root = `/`). Structurally identical to an archive
    /// path (container-backed → treated as local, not remote).
    pub fn extfs(prefix: &str, container: impl Into<PathBuf>, inner: impl Into<PathBuf>) -> Self {
        VfsPath {
            scheme: prefix.to_string(),
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

    /// True for the built-in `archive` backend specifically (as opposed to an
    /// `extfs` mount, which is also container-backed). Used to route archive
    /// mutations to the in-process rebuild path rather than the extfs scripts.
    pub fn is_native_archive(&self) -> bool {
        self.scheme == "archive"
    }

    /// True for a non-local backend (an `sftp-`/`ftp-`/`scp-` session scheme).
    /// The local disk (`file`) and any container-backed path (built-in archives
    /// and `extfs` mounts) all count as local — their files can be extracted to
    /// a temp on local disk, so they are exempt from the one-remote invariant.
    pub fn is_remote(&self) -> bool {
        self.container.is_none() && self.scheme != "file"
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

    /// The path as a POSIX (forward-slash) string, for use on the wire with
    /// remote backends (SFTP/SCP/FTP), which speak forward slashes regardless of
    /// the host OS. On Windows the stored `PathBuf` can contain backslashes
    /// (`PathBuf::join` uses the OS separator), so we rewrite the OS separator to
    /// `/`. On Unix `MAIN_SEPARATOR` is already `/`, so this is a no-op that
    /// preserves any literal backslash in a (legitimate) POSIX file name.
    pub fn posix_path(&self) -> String {
        self.path
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/")
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
        // The local filesystem uses native joins (correct on every OS); remote
        // and archive paths are always POSIX, so join with `/` explicitly rather
        // than let Windows insert a backslash into a server/archive path.
        let path = if self.scheme == "file" {
            self.path.join(name)
        } else {
            let base = self.posix_path();
            let base = base.trim_end_matches('/');
            let comp = name.as_ref().to_string_lossy();
            let comp = comp.trim_matches('/');
            PathBuf::from(format!("{base}/{comp}"))
        };
        VfsPath {
            scheme: self.scheme.clone(),
            path,
            container: self.container.clone(),
        }
    }

    /// Display string for the location bar.
    pub fn display(&self) -> String {
        if let Some(c) = &self.container {
            format!("{}!{}", c.to_string_lossy(), self.posix_path())
        } else if self.scheme == "file" {
            self.path.to_string_lossy().into_owned()
        } else {
            format!("{}://{}", self.scheme, self.posix_path())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(path: &str) -> VfsPath {
        VfsPath { scheme: "sftp-0".into(), path: PathBuf::from(path), container: None }
    }

    /// The OS separator is normalized to `/` for the wire (the Windows bug: a
    /// `PathBuf` join there inserts `\`, which the POSIX server rejects). Written
    /// with `MAIN_SEPARATOR` so it exercises the conversion on both platforms.
    #[test]
    fn posix_path_normalizes_os_separator() {
        let sep = std::path::MAIN_SEPARATOR;
        let p = remote(&format!("/home/user{sep}sub{sep}file.txt"));
        assert_eq!(p.posix_path(), "/home/user/sub/file.txt");
    }

    /// Remote joins are always POSIX — never the native separator — so browsing
    /// into a subdirectory produces a valid server path on Windows too.
    #[test]
    fn remote_join_is_posix() {
        assert_eq!(remote("/home/user").join("sub").posix_path(), "/home/user/sub");
        // Root and trailing-slash bases don't double the separator.
        assert_eq!(remote("/").join("home").posix_path(), "/home");
        assert_eq!(remote("/home/").join("user").posix_path(), "/home/user");
        // The location bar shows forward slashes for remote paths.
        assert_eq!(remote("/a/b").join("c").display(), "sftp-0:///a/b/c");
    }

    /// Archive (non-local, non-remote) paths also join with forward slashes.
    #[test]
    fn archive_join_is_posix() {
        let a = VfsPath::archive("/tmp/x.zip", "/dir");
        assert_eq!(a.join("file.txt").path, PathBuf::from("/dir/file.txt"));
    }

    /// Local joins stay native (unchanged behavior).
    #[test]
    fn local_join_is_native() {
        let l = VfsPath::local("/tmp/dir");
        assert_eq!(l.join("f").path, PathBuf::from("/tmp/dir").join("f"));
    }
}
