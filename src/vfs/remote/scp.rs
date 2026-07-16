//! SCP backend. The scp wire protocol cannot list directories, so browsing is
//! done with shell commands over the SSH connection (the standard approach for
//! scp-based file managers), while transfers stream through `cat`.

use super::{Connection, RemoteCreds, SshHandle, parse_unix_listing_line, shell_quote, ssh_connect};
use crate::util::{Error, Result};
use crate::vfs::membuf::{pipe_download, pipe_upload};
use crate::vfs::{BoxRead, BoxWrite, Capabilities, Vfs, VfsEntry, VfsKind, VfsPath, WriteMeta};
use russh::ChannelMsg;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

pub struct ScpFs {
    handle: Arc<SshHandle>,
}

pub async fn connect(creds: &RemoteCreds) -> Result<Connection> {
    let handle = Arc::new(ssh_connect(creds).await?);
    let root = if creds.path.trim().is_empty() {
        let (out, _) = exec_capture(&handle, "pwd").await?;
        let s = String::from_utf8_lossy(&out).trim().to_string();
        if s.is_empty() { "/".to_string() } else { s }
    } else {
        creds.path.clone()
    };
    let label = format!("scp://{}@{}", creds.user, creds.host);
    Ok(Connection {
        backend: Arc::new(ScpFs { handle }),
        root,
        label,
    })
}

/// Run a command over a fresh exec channel; collect stdout and the exit code.
async fn exec_capture(handle: &SshHandle, cmd: &str) -> Result<(Vec<u8>, u32)> {
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(|e| Error::other(format!("channel open failed: {e}")))?;
    channel
        .exec(true, cmd)
        .await
        .map_err(|e| Error::other(format!("exec failed: {e}")))?;
    let mut out = Vec::new();
    let mut code = 0u32;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { data }) => out.extend_from_slice(&data),
            Some(ChannelMsg::ExitStatus { exit_status }) => code = exit_status,
            Some(_) => {}
            None => break,
        }
    }
    Ok((out, code))
}

fn path_str(p: &VfsPath) -> String {
    p.posix_path()
}

fn entry_from(name: String, p: super::ParsedListing) -> VfsEntry {
    VfsEntry {
        name,
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
    }
}

#[async_trait::async_trait]
impl Vfs for ScpFs {
    async fn open_shell(&self, rows: u16, cols: u16) -> Result<super::RemoteShellChannel> {
        super::open_shell_channel(&self.handle, rows, cols).await
    }

    fn scheme(&self) -> &str {
        "scp"
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
        let cmd = format!(
            "ls -la --time-style=long-iso -- {}",
            shell_quote(&path_str(dir))
        );
        let (out, code) = exec_capture(&self.handle, &cmd).await?;
        if code != 0 && out.is_empty() {
            return Err(Error::other("remote listing failed"));
        }
        let text = String::from_utf8_lossy(&out);
        let mut entries = Vec::new();
        for line in text.lines() {
            if let Some(p) = parse_unix_listing_line(line) {
                let name = p.name.clone();
                entries.push(entry_from(name, p));
            }
        }
        Ok(entries)
    }

    async fn stat(&self, path: &VfsPath) -> Result<VfsEntry> {
        let cmd = format!(
            "ls -lad --time-style=long-iso -- {}",
            shell_quote(&path_str(path))
        );
        let (out, code) = exec_capture(&self.handle, &cmd).await?;
        if code != 0 {
            return Err(Error::NotFound(path_str(path)));
        }
        let text = String::from_utf8_lossy(&out);
        for line in text.lines() {
            if let Some(p) = parse_unix_listing_line(line) {
                return Ok(entry_from(path.file_name(), p));
            }
        }
        // Fallback: assume a directory.
        Ok(VfsEntry {
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
        })
    }

    async fn open_read(&self, path: &VfsPath) -> Result<BoxRead> {
        // Stream `cat`'s stdout over a fresh channel straight into the read pipe,
        // so multi-gigabyte files transfer chunk-by-chunk (bounded memory) and
        // the progress bar advances from the first bytes — instead of buffering
        // the whole file into RAM before the copy can start.
        let handle = self.handle.clone();
        let cmd = format!("cat -- {}", shell_quote(&path_str(path)));
        Ok(pipe_download(1024 * 1024, move |mut w| async move {
            let mut channel = handle
                .channel_open_session()
                .await
                .map_err(|e| std::io::Error::other(format!("channel open failed: {e}")))?;
            channel
                .exec(true, cmd)
                .await
                .map_err(|e| std::io::Error::other(format!("exec failed: {e}")))?;
            loop {
                match channel.wait().await {
                    Some(ChannelMsg::Data { data }) => w.write_all(&data).await?,
                    Some(ChannelMsg::ExtendedData { .. }) => {} // stderr: ignore
                    Some(_) => {}
                    None => break, // channel closed → EOF
                }
            }
            w.shutdown().await?;
            Ok(())
        }))
    }

    async fn open_write(&self, path: &VfsPath, _meta: WriteMeta) -> Result<BoxWrite> {
        let handle = self.handle.clone();
        let path = path_str(path);
        // Pipe the engine's bytes to `cat > path` over a fresh channel and stream
        // them as they arrive, so the progress bar tracks the real upload.
        Ok(pipe_upload(64 * 1024, move |rx| async move {
            let mut channel = handle
                .channel_open_session()
                .await
                .map_err(|e| std::io::Error::other(format!("channel open failed: {e}")))?;
            channel
                .exec(true, format!("cat > {}", shell_quote(&path)))
                .await
                .map_err(|e| std::io::Error::other(format!("exec failed: {e}")))?;
            // Stream the engine's bytes to the remote `cat`, then EOF so it
            // finishes. (Keeping the channel — rather than `into_stream()` — lets
            // us read the exit status below.)
            channel
                .data(rx)
                .await
                .map_err(|e| std::io::Error::other(format!("upload failed: {e}")))?;
            channel
                .eof()
                .await
                .map_err(|e| std::io::Error::other(format!("eof failed: {e}")))?;
            // Check the remote command actually succeeded. A failed `cat`
            // (permission, quota, disk full) must surface as an error, not a
            // silently truncated or empty file reported as a successful write.
            let mut code = 0u32;
            while let Some(msg) = channel.wait().await {
                if let ChannelMsg::ExitStatus { exit_status } = msg {
                    code = exit_status;
                }
            }
            if code != 0 {
                return Err(std::io::Error::other(format!(
                    "remote write failed (exit status {code})"
                )));
            }
            Ok(())
        }))
    }

    async fn mkdir(&self, path: &VfsPath) -> Result<()> {
        run_ok(&self.handle, &format!("mkdir -- {}", shell_quote(&path_str(path)))).await
    }

    async fn remove_file(&self, path: &VfsPath) -> Result<()> {
        run_ok(&self.handle, &format!("rm -f -- {}", shell_quote(&path_str(path)))).await
    }

    async fn remove_dir(&self, path: &VfsPath) -> Result<()> {
        run_ok(&self.handle, &format!("rmdir -- {}", shell_quote(&path_str(path)))).await
    }

    async fn rename(&self, from: &VfsPath, to: &VfsPath) -> Result<()> {
        run_ok(
            &self.handle,
            &format!(
                "mv -- {} {}",
                shell_quote(&path_str(from)),
                shell_quote(&path_str(to))
            ),
        )
        .await
    }
}

/// Run a command and require exit code 0.
async fn run_ok(handle: &SshHandle, cmd: &str) -> Result<()> {
    let (_, code) = exec_capture(handle, cmd).await?;
    if code == 0 {
        Ok(())
    } else {
        Err(Error::other(format!("remote command failed: {cmd}")))
    }
}
