//! The virtual filesystem (VFS) abstraction — the spine of the application.
//!
//! Panels, the file-ops engine, the viewer, and the editor speak only to
//! `Arc<dyn Vfs>` handles plus [`VfsPath`], never to `std::fs`, network, or
//! archive crates directly. Local disk, archives, and remote servers are all
//! interchangeable implementations of [`Vfs`].

pub mod local;
pub mod path;
pub mod registry;

pub use path::VfsPath;

use crate::util::Result;
use std::time::SystemTime;
use tokio::io::{AsyncRead, AsyncWrite};

/// What kind of thing a directory entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VfsKind {
    File,
    Dir,
    Symlink,
    Other,
}

impl VfsKind {
    pub fn is_dir(self) -> bool {
        matches!(self, VfsKind::Dir)
    }
}

/// A single directory entry. This is a superset of the metadata any backend
/// might provide; fields a backend cannot supply are left `None`. The sort
/// module reads directly from these fields.
#[derive(Debug, Clone)]
pub struct VfsEntry {
    pub name: String,
    pub kind: VfsKind,
    pub size: u64,
    pub mtime: Option<SystemTime>,
    pub atime: Option<SystemTime>,
    pub ctime: Option<SystemTime>,
    pub inode: Option<u64>,
    /// Unix permission/type bits (`st_mode`), when available.
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    /// For symlinks: the raw target, if read.
    pub symlink_target: Option<String>,
}

impl VfsEntry {
    /// Whether this entry has any executable bit set (used by exec-first sort).
    pub fn is_executable(&self) -> bool {
        self.kind == VfsKind::File && self.mode.map(|m| m & 0o111 != 0).unwrap_or(false)
    }

    /// The extension (lowercased, without dot), or empty string.
    pub fn extension(&self) -> &str {
        match self.name.rfind('.') {
            // A leading dot (dotfile) is not an extension.
            Some(0) => "",
            Some(idx) => &self.name[idx + 1..],
            None => "",
        }
    }
}

/// What a backend can and cannot do. Drives which menu items / dialogs are
/// enabled and how the ops engine behaves (e.g. cross-backend copy vs rename).
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub writable: bool,
    pub permissions: bool,
    pub ownership: bool,
    pub symlinks: bool,
    /// Whether `open_read` supports cheap random access (false for tar.gz, FTP).
    pub random_access: bool,
    pub inode: bool,
    /// Whether the backend can rename server-side (vs copy+delete).
    pub server_rename: bool,
}

impl Capabilities {
    /// A fully featured local-disk profile.
    pub const fn local() -> Self {
        Capabilities {
            writable: true,
            permissions: true,
            ownership: true,
            symlinks: true,
            random_access: true,
            inode: true,
            server_rename: true,
        }
    }
}

/// Metadata hint passed to [`Vfs::open_write`]; some backends (archive writers)
/// need the size up front.
#[derive(Debug, Clone, Default)]
pub struct WriteMeta {
    pub size_hint: Option<u64>,
    pub mode: Option<u32>,
    pub mtime: Option<SystemTime>,
}

/// Boxed async byte streams returned by the read/write openers.
pub type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
pub type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// The virtual filesystem trait. Object-safe via `async_trait` so backends can
/// be stored as `Arc<dyn Vfs>` and swapped at runtime.
///
/// Note: cross-backend copy is deliberately *not* here — it lives in
/// `ops::engine` as `open_read(src)` → pump chunks → `open_write(dst)`.
/// `rename` is intra-backend only.
#[async_trait::async_trait]
pub trait Vfs: Send + Sync {
    fn scheme(&self) -> &str;
    fn capabilities(&self) -> Capabilities;

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>>;
    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry>;

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead>;
    async fn open_write(&self, path: &VfsPath, meta: WriteMeta) -> Result<BoxWrite>;

    async fn mkdir(&self, path: &VfsPath) -> Result<()>;
    async fn remove_file(&self, path: &VfsPath) -> Result<()>;
    async fn remove_dir(&self, path: &VfsPath) -> Result<()>;
    async fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()>;

    // --- Capability-gated; default to Unsupported. ---

    async fn set_permissions(&self, _path: &VfsPath, _mode: u32) -> Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    async fn set_owner(&self, _path: &VfsPath, _uid: Option<u32>, _gid: Option<u32>) -> Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    async fn symlink(&self, _target: &str, _link: &VfsPath) -> Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    async fn read_link(&self, _path: &VfsPath) -> Result<String> {
        Err(crate::util::Error::Unsupported)
    }
    async fn free_space(&self, _path: &VfsPath) -> Result<Option<u64>> {
        Ok(None)
    }
}
