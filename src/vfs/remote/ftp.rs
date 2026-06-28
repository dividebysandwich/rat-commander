//! FTP backend over suppaftp (tokio). The single control connection is shared
//! behind an async mutex; both downloads and uploads stream through a duplex
//! pipe so transfers proceed at network speed with bounded memory and accurate
//! progress.

use super::{Connection, RemoteCreds, parse_unix_listing_line};
use crate::util::{Error, Result};
use crate::vfs::membuf::{pipe_download, pipe_upload};
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use std::sync::Arc;
use suppaftp::tokio::AsyncFtpStream;
use suppaftp::types::FileType;
use tokio::sync::Mutex;

pub struct FtpFs {
    conn: Arc<Mutex<AsyncFtpStream>>,
}

pub async fn connect(creds: &RemoteCreds) -> Result<Connection> {
    let mut stream = AsyncFtpStream::connect((creds.host.as_str(), creds.port))
        .await
        .map_err(|e| Error::other(format!("FTP connect failed: {e}")))?;
    stream
        .login(&creds.user, &creds.password)
        .await
        .map_err(|e| Error::other(format!("FTP login failed: {e}")))?;
    // Binary mode so SIZE and transfers are byte-accurate.
    let _ = stream.transfer_type(FileType::Binary).await;

    let root = if creds.path.trim().is_empty() {
        stream.pwd().await.unwrap_or_else(|_| "/".to_string())
    } else {
        creds.path.clone()
    };
    let label = format!("ftp://{}@{}", creds.user, creds.host);
    Ok(Connection {
        backend: Arc::new(FtpFs {
            conn: Arc::new(Mutex::new(stream)),
        }),
        root,
        label,
    })
}

fn path_str(p: &VfsPath) -> String {
    p.path.to_string_lossy().into_owned()
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[async_trait::async_trait]
impl Vfs for FtpFs {
    fn scheme(&self) -> &str {
        "ftp"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            writable: true,
            permissions: false,
            ownership: false,
            symlinks: false,
            random_access: false,
            inode: false,
            server_rename: true,
        }
    }

    async fn read_dir(&self, dir: &VfsPath) -> Result<Vec<VfsEntry>> {
        let mut guard = self.conn.lock().await;
        let lines = guard
            .list(Some(&path_str(dir)))
            .await
            .map_err(|e| Error::other(e.to_string()))?;
        let mut out = Vec::new();
        for line in lines {
            if let Some(p) = parse_unix_listing_line(&line) {
                out.push(VfsEntry {
                    name: p.name,
                    kind: p.kind,
                    size: p.size,
                    mtime: None,
                    atime: None,
                    ctime: None,
                    inode: None,
                    mode: p.mode,
                    uid: None,
                    gid: None,
                    symlink_target: p.symlink_target,
                    symlink_broken: false,
                });
            }
        }
        Ok(out)
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        let mut guard = self.conn.lock().await;
        // FTP has no stat; SIZE succeeds for files, fails for directories.
        match guard.size(&path_str(path)).await {
            Ok(size) => Ok(VfsEntry {
                name: path.file_name(),
                kind: VfsKind::File,
                size: size as u64,
                mtime: None,
                atime: None,
                ctime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
            }),
            Err(_) => Ok(VfsEntry {
                name: path.file_name(),
                kind: VfsKind::Dir,
                size: 0,
                mtime: None,
                atime: None,
                ctime: None,
                inode: None,
                mode: None,
                uid: None,
                gid: None,
                symlink_target: None,
                symlink_broken: false,
            }),
        }
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        // Stream the data connection straight into the read pipe (holding the
        // control-connection lock for the transfer's duration), so large files
        // download chunk-by-chunk at network speed instead of buffering in RAM.
        let conn = self.conn.clone();
        let path = path_str(path);
        Ok(pipe_download(64 * 1024, move |mut w| async move {
            let mut guard = conn.lock().await;
            let mut stream = guard.retr_as_stream(&path).await.map_err(io_err)?;
            tokio::io::copy(&mut stream, &mut w).await?;
            guard.finalize_retr_stream(stream).await.map_err(io_err)?;
            Ok(())
        }))
    }

    async fn open_write(&self, path: &VfsPath, _meta: WriteMeta) -> Result<BoxWrite> {
        let conn = self.conn.clone();
        let path = path_str(path);
        // Stream the engine's bytes straight to the FTP data connection; the
        // write side blocks at network speed, so progress tracks the upload.
        Ok(pipe_upload(64 * 1024, move |mut rx| async move {
            let mut guard = conn.lock().await;
            let mut stream = guard.put_with_stream(&path).await.map_err(io_err)?;
            tokio::io::copy(&mut rx, &mut stream).await?;
            guard.finalize_put_stream(stream).await.map_err(io_err)?;
            Ok(())
        }))
    }

    async fn mkdir(&self, path: &VfsPath) -> Result<()> {
        self.conn
            .lock()
            .await
            .mkdir(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn remove_file(&self, path: &VfsPath) -> Result<()> {
        self.conn
            .lock()
            .await
            .rm(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        self.conn
            .lock()
            .await
            .rmdir(path_str(path))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }

    async fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        self.conn
            .lock()
            .await
            .rename(path_str(from), path_str(to))
            .await
            .map_err(|e| Error::other(e.to_string()))
    }
}
