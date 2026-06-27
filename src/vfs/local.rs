//! Local-filesystem [`Vfs`] backend.

use super::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use crate::util::Result;
#[cfg(unix)]
use crate::util::Error;
use std::fs::Metadata;
#[cfg(unix)]
use std::time::{Duration, UNIX_EPOCH};
use std::time::SystemTime;
use tokio::fs;

/// The local disk. All operations run on tokio's blocking-friendly `fs` API.
pub struct LocalFs;

impl LocalFs {
    pub fn new() -> Self {
        LocalFs
    }
}

impl Default for LocalFs {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a [`VfsEntry`] from a file name and its (symlink-level) metadata.
fn entry_from_meta(name: String, meta: &Metadata, symlink_target: Option<String>) -> VfsEntry {
    let kind = if meta.file_type().is_symlink() {
        VfsKind::Symlink
    } else if meta.is_dir() {
        VfsKind::Dir
    } else if meta.is_file() {
        VfsKind::File
    } else {
        VfsKind::Other
    };

    let ext = ext_meta(meta);
    VfsEntry {
        name,
        kind,
        size: meta.len(),
        mtime: meta.modified().ok(),
        atime: ext.atime,
        ctime: ext.ctime,
        inode: ext.inode,
        mode: ext.mode,
        uid: ext.uid,
        gid: ext.gid,
        symlink_target,
    }
}

/// Platform-specific metadata fields.
struct ExtMeta {
    atime: Option<SystemTime>,
    ctime: Option<SystemTime>,
    inode: Option<u64>,
    mode: Option<u32>,
    uid: Option<u32>,
    gid: Option<u32>,
}

#[cfg(unix)]
fn ext_meta(meta: &Metadata) -> ExtMeta {
    use std::os::unix::fs::MetadataExt;
    ExtMeta {
        atime: Some(unix_to_system(meta.atime())),
        ctime: Some(unix_to_system(meta.ctime())),
        inode: Some(meta.ino()),
        mode: Some(meta.mode()),
        uid: Some(meta.uid()),
        gid: Some(meta.gid()),
    }
}

#[cfg(not(unix))]
fn ext_meta(meta: &Metadata) -> ExtMeta {
    ExtMeta {
        atime: meta.accessed().ok(),
        ctime: None,
        inode: None,
        mode: None,
        uid: None,
        gid: None,
    }
}

#[cfg(unix)]
fn unix_to_system(secs: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
    }
}

#[async_trait::async_trait]
impl Vfs for LocalFs {
    fn scheme(&self) -> &str {
        "file"
    }

    fn capabilities(&self) -> Capabilities {
        // Permissions / ownership / symlinks are Unix concepts.
        Capabilities {
            permissions: cfg!(unix),
            ownership: cfg!(unix),
            symlinks: cfg!(unix),
            inode: cfg!(unix),
            ..Capabilities::local()
        }
    }

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>> {
        let mut rd = fs::read_dir(dir.as_path()).await?;
        let mut out = Vec::new();
        while let Some(de) = rd.next_entry().await? {
            let name = de.file_name().to_string_lossy().into_owned();
            // Use symlink_metadata so symlinks show as links, not their targets.
            let meta = match fs::symlink_metadata(de.path()).await {
                Ok(m) => m,
                Err(_) => continue, // racing deletion / permission — skip
            };
            let target = if meta.file_type().is_symlink() {
                fs::read_link(de.path())
                    .await
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
            } else {
                None
            };
            out.push(entry_from_meta(name, &meta, target));
        }
        Ok(out)
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        let meta = fs::symlink_metadata(path.as_path()).await?;
        let target = if meta.file_type().is_symlink() {
            fs::read_link(path.as_path())
                .await
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };
        Ok(entry_from_meta(path.file_name(), &meta, target))
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        let f = fs::File::open(path.as_path()).await?;
        Ok(Box::new(f))
    }

    async fn open_write(&self, path: &VfsPath, meta: WriteMeta) -> Result<BoxWrite> {
        let f = if meta.append {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path.as_path())
                .await?
        } else {
            fs::File::create(path.as_path()).await?
        };
        Ok(Box::new(f))
    }

    async fn mkdir(&self, path: &VfsPath) -> Result<()> {
        fs::create_dir(path.as_path()).await?;
        Ok(())
    }

    async fn remove_file(&self, path: &VfsPath) -> Result<()> {
        fs::remove_file(path.as_path()).await?;
        Ok(())
    }

    async fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        fs::remove_dir(path.as_path()).await?;
        Ok(())
    }

    async fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        fs::rename(from.as_path(), to.as_path()).await?;
        Ok(())
    }

    #[cfg(unix)]
    async fn set_permissions(&self, path: &VfsPath, mode: u32) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode);
        fs::set_permissions(path.as_path(), perms).await?;
        Ok(())
    }

    #[cfg(not(unix))]
    async fn set_permissions(&self, _path: &VfsPath, _mode: u32) -> Result<()> {
        Err(crate::util::Error::Unsupported)
    }

    #[cfg(unix)]
    async fn set_owner(&self, path: &VfsPath, uid: Option<u32>, gid: Option<u32>) -> Result<()> {
        use nix::unistd::{Gid, Uid};
        let p = path.path.clone();
        tokio::task::spawn_blocking(move || {
            nix::unistd::chown(&p, uid.map(Uid::from_raw), gid.map(Gid::from_raw))
        })
        .await
        .map_err(|e| Error::other(format!("join error: {e}")))?
        .map_err(|e| Error::other(format!("chown failed: {e}")))?;
        Ok(())
    }

    #[cfg(not(unix))]
    async fn set_owner(&self, _path: &VfsPath, _uid: Option<u32>, _gid: Option<u32>) -> Result<()> {
        Err(crate::util::Error::Unsupported)
    }

    #[cfg(unix)]
    async fn symlink(&self, target: &str, link: &VfsPath) -> Result<()> {
        fs::symlink(target, link.as_path()).await?;
        Ok(())
    }

    #[cfg(not(unix))]
    async fn symlink(&self, target: &str, link: &VfsPath) -> Result<()> {
        // Windows symlink creation needs special privileges; pick file vs dir.
        let target = std::path::PathBuf::from(target);
        let link = link.path.clone();
        let is_dir = tokio::fs::metadata(&target).await.map(|m| m.is_dir()).unwrap_or(false);
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            if is_dir {
                std::os::windows::fs::symlink_dir(&target, &link)
            } else {
                std::os::windows::fs::symlink_file(&target, &link)
            }
        })
        .await
        .map_err(|e| crate::util::Error::other(format!("join error: {e}")))??;
        Ok(())
    }

    async fn read_link(&self, path: &VfsPath) -> Result<String> {
        let t = fs::read_link(path.as_path()).await?;
        Ok(t.to_string_lossy().into_owned())
    }

    #[cfg(unix)]
    async fn disk_usage(&self, path: &VfsPath) -> Result<Option<super::DiskUsage>> {
        let p = path.path.clone();
        let usage = tokio::task::spawn_blocking(move || {
            // statvfs the path; fall back to the root if the path is gone.
            let st = nix::sys::statvfs::statvfs(&p)
                .or_else(|_| nix::sys::statvfs::statvfs("/"))
                .ok()?;
            let frsize = st.fragment_size() as u64;
            let total = st.blocks() as u64 * frsize;
            // Blocks available to unprivileged users (matches `df`).
            let free = st.blocks_available() as u64 * frsize;
            Some(super::DiskUsage { total, free })
        })
        .await
        .ok()
        .flatten();
        Ok(usage)
    }

    #[cfg(not(unix))]
    async fn disk_usage(&self, _path: &VfsPath) -> Result<Option<super::DiskUsage>> {
        Ok(None)
    }
}
