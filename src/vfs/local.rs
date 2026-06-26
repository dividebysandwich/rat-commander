//! Local-filesystem [`Vfs`] backend.

use super::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use crate::util::{Error, Result};
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

    VfsEntry {
        name,
        kind,
        size: meta.len(),
        mtime: Some(meta.modified().unwrap_or(UNIX_EPOCH)),
        atime: Some(unix_to_system(meta.atime())),
        ctime: Some(unix_to_system(meta.ctime())),
        inode: Some(meta.ino()),
        mode: Some(meta.mode()),
        uid: Some(meta.uid()),
        gid: Some(meta.gid()),
        symlink_target,
    }
}

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
        Capabilities::local()
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

    async fn open_write(&self, path: &VfsPath, _meta: WriteMeta) -> Result<BoxWrite> {
        let f = fs::File::create(path.as_path()).await?;
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

    async fn set_permissions(&self, path: &VfsPath, mode: u32) -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode);
        fs::set_permissions(path.as_path(), perms).await?;
        Ok(())
    }

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

    async fn symlink(&self, target: &str, link: &VfsPath) -> Result<()> {
        fs::symlink(target, link.as_path()).await?;
        Ok(())
    }

    async fn read_link(&self, path: &VfsPath) -> Result<String> {
        let t = fs::read_link(path.as_path()).await?;
        Ok(t.to_string_lossy().into_owned())
    }
}
