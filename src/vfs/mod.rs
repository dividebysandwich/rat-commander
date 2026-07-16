//! The virtual filesystem (VFS) abstraction — the spine of the application.
//!
//! Panels, the file-ops engine, the viewer, and the editor speak only to
//! `Arc<dyn Vfs>` handles plus [`VfsPath`], never to `std::fs`, network, or
//! archive crates directly. Local disk, archives, and remote servers are all
//! interchangeable implementations of [`Vfs`].

pub mod archive;
pub mod extfs;
pub mod local;
pub mod membuf;
pub mod path;
pub mod registry;
pub mod remote;

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
    /// For symlinks: whether the target could not be resolved (dangling link).
    /// Always `false` for non-symlinks and backends that don't probe targets.
    pub symlink_broken: bool,
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
    #[allow(dead_code)] // populated by every backend; not consumed by a caller yet
    pub random_access: bool,
    #[allow(dead_code)]
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
// size_hint/mode/mtime are upload hints no backend consumes yet; `append` is used.
#[allow(dead_code)]
pub struct WriteMeta {
    pub size_hint: Option<u64>,
    pub mode: Option<u32>,
    pub mtime: Option<SystemTime>,
    /// Open the destination for appending instead of truncating (used by the
    /// overwrite dialog's "Append" choice). Backends that can't append ignore it.
    pub append: bool,
}

/// Capacity of the filesystem/volume that holds a path. Shown on the panel's
/// bottom border (used / total, like Midnight Commander).
#[derive(Debug, Clone, Copy)]
pub struct DiskUsage {
    pub total: u64,
    pub free: u64,
}

impl DiskUsage {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// Percentage of capacity in use (0..=100).
    pub fn percent_used(&self) -> u8 {
        if self.total == 0 {
            0
        } else {
            ((self.used() as u128 * 100) / self.total as u128).min(100) as u8
        }
    }
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
    // Part of the backend contract; retained though no caller reads it yet.
    #[allow(dead_code)]
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
    /// Stamp `path` with a modification time, if the backend can.
    ///
    /// Directory sync uses this to give a copied file the *source's* timestamp,
    /// which is what makes a mirror converge: without it the destination would be
    /// stamped "now", look newer than its source on the next run, and be copied
    /// again forever (and a two-way sync would ping-pong the file between the two
    /// sides). A backend that can't oblige returns [`Unsupported`] and sync simply
    /// falls back to re-copying that file when it next runs.
    ///
    /// [`Unsupported`]: crate::util::Error::Unsupported
    async fn set_mtime(&self, _path: &VfsPath, _mtime: SystemTime) -> Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    async fn set_owner(&self, _path: &VfsPath, _uid: Option<u32>, _gid: Option<u32>) -> Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    async fn symlink(&self, _target: &str, _link: &VfsPath) -> Result<()> {
        Err(crate::util::Error::Unsupported)
    }
    // Backend contract; symlink reads aren't wired to a caller yet.
    #[allow(dead_code)]
    async fn read_link(&self, _path: &VfsPath) -> Result<String> {
        Err(crate::util::Error::Unsupported)
    }
    /// Total/free capacity of the volume holding `path`, if the backend can
    /// report it (local disk only by default).
    async fn disk_usage(&self, _path: &VfsPath) -> Result<Option<DiskUsage>> {
        Ok(None)
    }

    /// Open an interactive shell (PTY channel) on the backend's remote host, for
    /// the `Ctrl-O` console and command line. Only the SSH-based backends
    /// (SFTP/SCP) support it; the rest return `Unsupported`.
    async fn open_shell(&self, _rows: u16, _cols: u16) -> Result<remote::RemoteShellChannel> {
        Err(crate::util::Error::Unsupported)
    }
}

/// A configurable in-memory [`Vfs`] for tests, so the transfer/save paths can be
/// exercised without a real filesystem or remote server. It can report a file of
/// a given size, hand out a reader that fails after N bytes, and record whether
/// the writer it returns was `shutdown` (i.e. properly finalized).
#[cfg(test)]
pub(crate) mod testmock {
    use super::*;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

    #[derive(Default)]
    pub(crate) struct MockVfs {
        /// Size reported by `stat` and produced by `open_read`.
        pub file_size: usize,
        /// `open_read` yields this many bytes, then returns an I/O error; `None`
        /// = a clean reader of `file_size` bytes.
        pub read_fail_after: Option<usize>,
        /// Set true once a writer from `open_write` has `poll_shutdown` polled,
        /// i.e. the caller finalized the write rather than only flushing.
        pub shutdown_called: Arc<AtomicBool>,
    }

    impl MockVfs {
        pub(crate) fn arc(self) -> Arc<dyn Vfs> {
            Arc::new(self)
        }
    }

    /// A writer that discards bytes but flips a flag when shut down.
    struct ProbeWriter {
        flag: Arc<AtomicBool>,
    }
    impl AsyncWrite for ProbeWriter {
        fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, b: &[u8]) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(b.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            self.flag.store(true, Ordering::SeqCst);
            Poll::Ready(Ok(()))
        }
    }

    /// A reader that yields `remaining` bytes of `b'x'`, then errors.
    struct FailingReader {
        remaining: usize,
    }
    impl AsyncRead for FailingReader {
        fn poll_read(mut self: Pin<&mut Self>, _: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
            if self.remaining == 0 {
                return Poll::Ready(Err(std::io::Error::other("simulated read failure")));
            }
            let n = self.remaining.min(buf.remaining()).min(4096);
            buf.put_slice(&vec![b'x'; n]);
            self.remaining -= n;
            Poll::Ready(Ok(()))
        }
    }

    #[async_trait::async_trait]
    impl Vfs for MockVfs {
        fn scheme(&self) -> &str {
            "mock"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                writable: true,
                permissions: false,
                ownership: false,
                symlinks: false,
                random_access: false,
                inode: false,
                server_rename: false,
            }
        }
        async fn read_dir(&self, _: &VfsPath) -> Result<Vec<VfsEntry>> {
            Ok(Vec::new())
        }
        async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
            Ok(VfsEntry {
                name: path.file_name(),
                kind: VfsKind::File,
                size: self.file_size as u64,
                mtime: None,
                atime: None,
                ctime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
            })
        }
        async fn open_read(&self, _: &VfsPath) -> Result<BoxRead> {
            Ok(Box::new(FailingReader {
                remaining: self.read_fail_after.unwrap_or(self.file_size),
            }))
        }
        async fn open_write(&self, _: &VfsPath, _: WriteMeta) -> Result<BoxWrite> {
            Ok(Box::new(ProbeWriter { flag: self.shutdown_called.clone() }))
        }
        async fn mkdir(&self, _: &VfsPath) -> Result<()> {
            Ok(())
        }
        async fn remove_file(&self, _: &VfsPath) -> Result<()> {
            Ok(())
        }
        async fn remove_dir(&self, _: &VfsPath) -> Result<()> {
            Ok(())
        }
        async fn rename(&self, _: &VfsPath, _: &VfsPath) -> Result<()> {
            Err(crate::util::Error::Unsupported)
        }
    }
}
