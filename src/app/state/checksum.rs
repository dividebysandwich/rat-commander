//! File checksumming: the background task that streams a file through the chosen
//! hash while reporting progress, and hands the result to the result dialog.

use super::*;
use crate::util::checksum::{ChecksumKind, ChecksumReport, normalize_expected};

impl AppState {
    /// Start a background checksum of `path` using `kind`. A determinate progress
    /// dialog shows while it runs (Esc/Enter aborts); the result arrives via
    /// [`AppEvent::ChecksumDone`] and opens the result dialog. `expected` is the
    /// user's comparison digest (empty ⇒ no comparison).
    pub(in crate::app::state) fn start_checksum(
        &mut self,
        path: VfsPath,
        kind: ChecksumKind,
        expected: String,
    ) {
        let backend = match self.registry.resolve(&path) {
            Ok(b) => b,
            Err(e) => return self.show_error(format!("cannot open file: {e}")),
        };
        let name = path.file_name();
        let expected = normalize_expected(&expected);

        let id = self.next_task_id;
        self.next_task_id += 1;
        let cancel = CancelToken::new();
        // A checksum never prompts for overwrite; the reply channel is unused but
        // keeps the `TaskHandle` shape uniform with copy/move tasks.
        let (reply, _reply_rx) = tokio::sync::mpsc::channel(1);
        self.tasks.insert(id, TaskHandle { id, cancel: cancel.clone(), reply });
        self.dialog = Some(Dialog::Progress(ProgressDialog::new(id, "Checksumming")));

        let tx = self.tx.clone();
        tokio::spawn(async move {
            let outcome = match compute_checksum(&backend, &path, kind, id, &name, &cancel, &tx).await
            {
                Ok(Some(digest)) => Ok(ChecksumReport { kind, name, digest, expected }),
                Ok(None) => Err(None),      // aborted → just close the dialog
                Err(msg) => Err(Some(msg)), // I/O failure → show an error
            };
            let _ = tx.send(AppEvent::ChecksumDone { id, result: outcome }).await;
        });
    }
}

/// Stream `path` through `kind`, emitting throttled progress and honouring
/// `cancel`. Returns `Ok(Some(hex))` on completion, `Ok(None)` when aborted, or
/// `Err(msg)` on an I/O failure.
async fn compute_checksum(
    backend: &std::sync::Arc<dyn crate::vfs::Vfs>,
    path: &VfsPath,
    kind: ChecksumKind,
    id: TaskId,
    name: &str,
    cancel: &CancelToken,
    tx: &AppSender,
) -> Result<Option<String>, String> {
    use tokio::io::AsyncReadExt;
    // Best-effort size for the progress total; 0 just means an indeterminate bar
    // that fills on completion.
    let total = backend.stat(path).await.map(|e| e.size).unwrap_or(0);
    let mut reader = backend.open_read(path).await.map_err(|e| e.to_string())?;
    let mut hasher = kind.hasher();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut since_report = 0u64;

    let report = |done: u64| {
        let _ = tx.try_send(AppEvent::Progress(ProgressUpdate {
            id,
            verb: "Checksumming",
            current_name: name.to_string(),
            file_done: done,
            file_total: total,
            total_done: done,
            total_total: total,
            files_done: 0,
            files_total: 1,
        }));
    };
    report(0);
    loop {
        if cancel.is_cancelled() {
            return Ok(None);
        }
        let n = reader.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        done += n as u64;
        since_report += n as u64;
        // Report at most ~every 1 MB so the bar advances without flooding.
        if since_report >= 1024 * 1024 {
            since_report = 0;
            report(done);
        }
    }
    report(done);
    Ok(Some(hasher.finalize()))
}
