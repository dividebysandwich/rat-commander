//! The file-operation engine: streaming copy / move / delete across any VFS
//! backends, with progress reporting and cooperative cancellation.

use super::cancel::CancelToken;
use super::progress::{ProgressUpdate, TaskId, TaskOutcome};
use super::{OpKind, OpRequest};
use crate::util::async_bridge::AppSender;
use crate::util::{Error, Result};
use crate::vfs::{Vfs, VfsKind, VfsPath, WriteMeta};
use futures::future::BoxFuture;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Duration, Instant};

const CHUNK: usize = 64 * 1024;
const EMIT_INTERVAL: Duration = Duration::from_millis(33);

/// Run an operation to completion, returning the outcome. Always emits a final
/// progress snapshot via the channel before returning.
pub async fn run(id: TaskId, req: OpRequest, tx: AppSender, cancel: CancelToken) -> TaskOutcome {
    let verb = match req.kind {
        OpKind::Copy => "Copying",
        OpKind::Move => "Moving",
        OpKind::Delete => "Deleting",
    };
    let mut engine = Engine {
        id,
        verb,
        tx,
        cancel,
        files_total: 0,
        files_done: 0,
        total_total: 0,
        total_done: 0,
        file_total: 0,
        file_done: 0,
        current_name: String::new(),
        last_emit: Instant::now(),
    };

    match engine.execute(&req).await {
        Ok(()) => TaskOutcome::Done,
        Err(Error::Cancelled) => TaskOutcome::Cancelled,
        Err(e) => TaskOutcome::Failed(e.to_string()),
    }
}

struct Engine {
    id: TaskId,
    verb: &'static str,
    tx: AppSender,
    cancel: CancelToken,
    files_total: u64,
    files_done: u64,
    total_total: u64,
    total_done: u64,
    file_total: u64,
    file_done: u64,
    current_name: String,
    last_emit: Instant,
}

impl Engine {
    async fn execute(&mut self, req: &OpRequest) -> Result<()> {
        // Pre-scan to compute totals for the progress bars.
        for src in &req.sources {
            self.scan(&req.src_fs, src).await?;
        }
        self.emit(true);

        match req.kind {
            OpKind::Delete => {
                for src in &req.sources {
                    self.check_cancel()?;
                    self.delete_tree(&req.src_fs, src.clone()).await?;
                }
            }
            OpKind::Copy | OpKind::Move => {
                let dst_fs = req
                    .dst_fs
                    .clone()
                    .ok_or_else(|| Error::other("destination backend missing"))?;
                let dst_dir = req
                    .dst_dir
                    .clone()
                    .ok_or_else(|| Error::other("destination directory missing"))?;
                let is_move = matches!(req.kind, OpKind::Move);
                let same_backend = Arc::ptr_eq(&req.src_fs, &dst_fs);

                for src in &req.sources {
                    self.check_cancel()?;
                    let dst = dst_dir.join(src.file_name());

                    // Fast path: intra-backend move via rename.
                    if is_move && same_backend && dst_fs.capabilities().server_rename {
                        // Count the subtree before it is renamed away so the
                        // progress counters stay consistent.
                        let mut files = 0u64;
                        let mut bytes = 0u64;
                        let _ = count_tree(&req.src_fs, src, &mut files, &mut bytes).await;
                        match req.src_fs.rename(src, &dst).await {
                            Ok(()) => {
                                self.files_done += files;
                                self.total_done += bytes;
                                self.emit(true);
                                continue;
                            }
                            Err(_) => { /* fall back to copy+delete (e.g. cross-device) */ }
                        }
                    }

                    self.copy_tree(&req.src_fs, src.clone(), &dst_fs, dst).await?;
                    if is_move {
                        self.delete_tree(&req.src_fs, src.clone()).await?;
                    }
                }
            }
        }

        self.emit(true);
        Ok(())
    }

    /// Recursively accumulate file count and byte totals.
    fn scan<'a>(&'a mut self, fs: &'a Arc<dyn Vfs>, path: &'a VfsPath) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let entry = fs.stat(path).await?;
            match entry.kind {
                VfsKind::Dir => {
                    for child in fs.read_dir(path).await? {
                        let cp = path.join(&child.name);
                        self.scan(fs, &cp).await?;
                    }
                }
                _ => {
                    self.files_total += 1;
                    self.total_total += entry.size;
                }
            }
            Ok(())
        })
    }

    fn copy_tree<'a>(
        &'a mut self,
        src_fs: &'a Arc<dyn Vfs>,
        src: VfsPath,
        dst_fs: &'a Arc<dyn Vfs>,
        dst: VfsPath,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.check_cancel()?;
            let entry = src_fs.stat(&src).await?;
            match entry.kind {
                VfsKind::Dir => {
                    // Create destination directory (ignore "already exists").
                    let _ = dst_fs.mkdir(&dst).await;
                    for child in src_fs.read_dir(&src).await? {
                        let cs = src.join(&child.name);
                        let cd = dst.join(&child.name);
                        self.copy_tree(src_fs, cs, dst_fs, cd).await?;
                    }
                    Ok(())
                }
                VfsKind::Symlink => {
                    // Recreate the link if the destination backend supports it.
                    if let Some(target) = entry.symlink_target {
                        let _ = dst_fs.symlink(&target, &dst).await;
                    }
                    self.files_done += 1;
                    self.emit(true);
                    Ok(())
                }
                _ => self.copy_file(src_fs, &src, dst_fs, &dst, entry.size, entry.mode).await,
            }
        })
    }

    async fn copy_file(
        &mut self,
        src_fs: &Arc<dyn Vfs>,
        src: &VfsPath,
        dst_fs: &Arc<dyn Vfs>,
        dst: &VfsPath,
        size: u64,
        mode: Option<u32>,
    ) -> Result<()> {
        self.current_name = src.file_name();
        self.file_total = size;
        self.file_done = 0;
        self.emit(true);

        let mut reader = src_fs.open_read(src).await?;
        let meta = WriteMeta {
            size_hint: Some(size),
            mode,
            mtime: None,
        };
        let mut writer = match dst_fs.open_write(dst, meta).await {
            Ok(w) => w,
            Err(e) => return Err(e),
        };

        let mut buf = vec![0u8; CHUNK];
        loop {
            if self.cancel.is_cancelled() {
                // Clean up the partial destination file.
                drop(writer);
                let _ = dst_fs.remove_file(dst).await;
                return Err(Error::Cancelled);
            }
            let n = reader.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            if let Err(e) = writer.write_all(&buf[..n]).await {
                drop(writer);
                let _ = dst_fs.remove_file(dst).await;
                return Err(e.into());
            }
            self.file_done += n as u64;
            self.total_done += n as u64;
            self.emit(false);
        }
        // shutdown() flushes AND closes — required so remote/buffering writers
        // (SFTP File, FTP/SCP CollectWriter) actually finalize the upload.
        writer.shutdown().await?;
        drop(writer);

        // Best-effort: preserve permissions.
        if let Some(m) = mode {
            let _ = dst_fs.set_permissions(dst, m).await;
        }

        self.files_done += 1;
        self.emit(true);
        Ok(())
    }

    fn delete_tree<'a>(
        &'a mut self,
        fs: &'a Arc<dyn Vfs>,
        path: VfsPath,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.check_cancel()?;
            let entry = fs.stat(&path).await?;
            match entry.kind {
                VfsKind::Dir => {
                    for child in fs.read_dir(&path).await? {
                        let cp = path.join(&child.name);
                        self.delete_tree(fs, cp).await?;
                    }
                    fs.remove_dir(&path).await?;
                }
                _ => {
                    self.current_name = path.file_name();
                    fs.remove_file(&path).await?;
                    self.files_done += 1;
                    self.emit(false);
                }
            }
            Ok(())
        })
    }

    fn check_cancel(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Emit a progress snapshot, throttled unless `force`.
    fn emit(&mut self, force: bool) {
        let now = Instant::now();
        if !force && now.duration_since(self.last_emit) < EMIT_INTERVAL {
            return;
        }
        self.last_emit = now;
        let update = ProgressUpdate {
            id: self.id,
            verb: self.verb,
            current_name: self.current_name.clone(),
            file_done: self.file_done,
            file_total: self.file_total,
            total_done: self.total_done,
            total_total: self.total_total,
            files_done: self.files_done,
            files_total: self.files_total,
        };
        // Best-effort; if the channel is full we simply drop this frame.
        let _ = self.tx.try_send(crate::app::event::AppEvent::Progress(update));
    }
}

/// Count files/bytes of a (possibly already-moved) subtree. Used only to keep
/// the progress counters consistent after a fast-path rename. Errors are
/// swallowed because the source may no longer exist.
fn count_tree<'a>(
    fs: &'a Arc<dyn Vfs>,
    path: &'a VfsPath,
    files: &'a mut u64,
    bytes: &'a mut u64,
) -> BoxFuture<'a, Result<()>> {
    Box::pin(async move {
        let entry = fs.stat(path).await?;
        if entry.kind == VfsKind::Dir {
            for child in fs.read_dir(path).await? {
                let cp = path.join(&child.name);
                count_tree(fs, &cp, files, bytes).await?;
            }
        } else {
            *files += 1;
            *bytes += entry.size;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::{OpKind, OpRequest};
    use crate::util::async_bridge;
    use crate::vfs::local::LocalFs;
    use std::path::PathBuf;

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rc_test_{tag}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn copies_a_directory_tree() {
        let root = unique_dir("copy");
        let src = root.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello world").unwrap();
        std::fs::write(src.join("sub/b.bin"), vec![7u8; 5000]).unwrap();
        let dst_dir = root.join("dest");

        let fs: Arc<dyn Vfs> = Arc::new(LocalFs::new());
        let (tx, _rx) = async_bridge::channel();
        let req = OpRequest {
            kind: OpKind::Copy,
            src_fs: fs.clone(),
            sources: vec![VfsPath::local(&src)],
            dst_fs: Some(fs.clone()),
            dst_dir: Some(VfsPath::local(&dst_dir)),
        };
        // The app would create dst_dir first; mirror that here.
        std::fs::create_dir_all(&dst_dir).unwrap();

        let outcome = run(1, req, tx, CancelToken::new()).await;
        assert!(matches!(outcome, TaskOutcome::Done), "outcome: {outcome:?}");

        assert_eq!(
            std::fs::read(dst_dir.join("src/a.txt")).unwrap(),
            b"hello world"
        );
        assert_eq!(
            std::fs::read(dst_dir.join("src/sub/b.bin")).unwrap().len(),
            5000
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn deletes_a_directory_tree() {
        let root = unique_dir("del");
        let victim = root.join("victim");
        std::fs::create_dir_all(victim.join("inner")).unwrap();
        std::fs::write(victim.join("inner/x"), b"x").unwrap();

        let fs: Arc<dyn Vfs> = Arc::new(LocalFs::new());
        let (tx, _rx) = async_bridge::channel();
        let req = OpRequest {
            kind: OpKind::Delete,
            src_fs: fs.clone(),
            sources: vec![VfsPath::local(&victim)],
            dst_fs: None,
            dst_dir: None,
        };
        let outcome = run(2, req, tx, CancelToken::new()).await;
        assert!(matches!(outcome, TaskOutcome::Done));
        assert!(!victim.exists());

        std::fs::remove_dir_all(&root).ok();
    }
}
