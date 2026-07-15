//! SFTP backend over russh + russh-sftp.

use super::{Connection, RemoteCreds, SshHandle, ssh_connect};
use crate::util::{Error, Result};
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use russh_sftp::client::SftpSession;
use russh_sftp::protocol::{FileAttributes, FileType};
use std::time::{Duration, UNIX_EPOCH};

pub struct SftpFs {
    /// Kept alive so the SSH connection task keeps running; also used to open an
    /// interactive shell channel (`Ctrl-O` on an SFTP panel).
    handle: SshHandle,
    sftp: SftpSession,
}

/// Connect, open the sftp subsystem, and resolve the initial directory.
pub async fn connect(creds: &RemoteCreds) -> Result<Connection> {
    let handle = ssh_connect(creds).await?;
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|e| Error::other(format!("channel open failed: {e}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| Error::other(format!("sftp subsystem failed: {e}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| Error::other(format!("sftp init failed: {e}")))?;

    let root = if creds.path.trim().is_empty() {
        sftp.canonicalize(".")
            .await
            .unwrap_or_else(|_| "/".to_string())
    } else {
        creds.path.clone()
    };
    let label = format!("sftp://{}@{}", creds.user, creds.host);
    Ok(Connection {
        backend: std::sync::Arc::new(SftpFs {
            handle,
            sftp,
        }),
        root,
        label,
    })
}

fn path_str(p: &VfsPath) -> String {
    p.posix_path()
}

fn kind_of(ft: FileType) -> VfsKind {
    if ft.is_dir() {
        VfsKind::Dir
    } else if ft.is_symlink() {
        VfsKind::Symlink
    } else {
        VfsKind::File
    }
}

fn entry_from(name: String, kind: VfsKind, m: &FileAttributes) -> VfsEntry {
    let mtime = m
        .mtime
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));
    let atime = m
        .atime
        .map(|t| UNIX_EPOCH + Duration::from_secs(t as u64));
    VfsEntry {
        name,
        kind,
        size: m.size.unwrap_or(0),
        mtime,
        atime,
        ctime: None,
        inode: None,
        mode: m.permissions,
        uid: m.uid,
        gid: m.gid,
        symlink_target: None,
        symlink_broken: false,
    }
}

#[async_trait::async_trait]
impl Vfs for SftpFs {
    async fn open_shell(&self, rows: u16, cols: u16) -> Result<super::RemoteShellChannel> {
        super::open_shell_channel(&self.handle, rows, cols).await
    }

    fn scheme(&self) -> &str {
        "sftp"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            writable: true,
            permissions: true,
            ownership: false,
            symlinks: true,
            random_access: true,
            inode: false,
            server_rename: true,
        }
    }

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>> {
        let rd = self
            .sftp
            .read_dir(path_str(dir))
            .await
            .map_err(|e| Error::other(e.to_string()))?;
        let mut out = Vec::new();
        for entry in rd {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }
            let kind = kind_of(entry.file_type());
            out.push(entry_from(name, kind, &entry.metadata()));
        }
        Ok(out)
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        let m = self
            .sftp
            .metadata(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))?;
        let kind = kind_of(m.file_type());
        Ok(entry_from(path.file_name(), kind, &m))
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        let f = self
            .sftp
            .open(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))?;
        Ok(Box::new(f))
    }

    async fn open_write(&self, path: &VfsPath, _meta: WriteMeta) -> Result<BoxWrite> {
        let f = self
            .sftp
            .create(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))?;
        Ok(Box::new(f))
    }

    async fn mkdir(&self, path: &VfsPath) -> Result<()> {
        self.sftp
            .create_dir(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn remove_file(&self, path: &VfsPath) -> Result<()> {
        self.sftp
            .remove_file(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        self.sftp
            .remove_dir(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        self.sftp
            .rename(path_str(from), path_str(to))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn set_permissions(&self, path: &VfsPath, mode: u32) -> Result<()> {
        let attrs = FileAttributes {
            permissions: Some(mode),
            ..Default::default()
        };
        self.sftp
            .set_metadata(path_str(path), attrs)
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn set_mtime(&self, path: &VfsPath, mtime: std::time::SystemTime) -> Result<()> {
        // SFTP stores whole seconds since the epoch. `atime` must be set with it
        // (the protocol's ACMODTIME flag covers both), so mirror the value.
        let secs = mtime
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| Error::other(e.to_string()))?
            .as_secs() as u32;
        let attrs = FileAttributes {
            mtime: Some(secs),
            atime: Some(secs),
            ..Default::default()
        };
        self.sftp
            .set_metadata(path_str(path), attrs)
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn symlink(&self, target: &str, link: &VfsPath) -> Result<()> {
        self.sftp
            .symlink(path_str(link), target)
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn read_link(&self, path: &VfsPath) -> Result<String> {
        self.sftp
            .read_link(path_str(path))
            .await
            .map(|p| p.to_string())
            .map_err(|e| Error::other(e.to_string()))
    }
}
