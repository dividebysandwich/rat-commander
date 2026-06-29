//! Writing a raw disk image to a block device ("flashing"), with byte-accurate
//! progress, time estimation, and cancellation.
//!
//! The image bytes are streamed straight through — no decompression — so this is
//! for raw images (`.iso`, `.img`, …). The device I/O is done in Rust (no
//! external `dd`), so it works anywhere the device can be opened. When already
//! root we write the device directly; otherwise we re-run ourselves under
//! `sudo` as a tiny privileged writer (`rc --flash-write <device>`) and pipe the
//! image to it — still our own code doing the writing — counting the bytes we
//! feed it for the progress bar.

use crate::app::event::AppEvent;
use crate::ops::CancelToken;
use crate::ops::progress::{ProgressUpdate, TaskId, TaskOutcome};
use crate::util::async_bridge::AppSender;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

/// File-name extensions treated as raw, flashable disk images.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "iso", "img", "raw", "bin", "dd", "image", "wic", "hddimg", "sdcard",
];

/// The default file-browser filter offered for picking an image.
pub const DEFAULT_IMAGE_FILTER: &str = "*.iso *.img *.raw *.bin *.dd *.wic *.hddimg";

/// Whether `name` looks like a flashable image by its extension.
pub fn is_image_file(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .filter(|_| name.contains('.'))
        .map(|ext| {
            let ext = ext.to_ascii_lowercase();
            IMAGE_EXTENSIONS.contains(&ext.as_str())
        })
        .unwrap_or(false)
}

/// A candidate target for flashing (a whole disk or a partition).
#[derive(Debug, Clone, Default)]
pub struct FlashTarget {
    /// Device node, e.g. `/dev/sdb`.
    pub dev: String,
    /// Size in bytes.
    pub size: u64,
    /// Whether the backing disk is removable.
    pub removable: bool,
    /// Model name (for the confirmation message).
    pub model: String,
    /// Volume label (for the confirmation message).
    pub label: String,
}

impl FlashTarget {
    pub fn from_device(d: &crate::mount::BlockDevice) -> Self {
        FlashTarget {
            dev: d.dev.clone(),
            size: d.size,
            removable: d.removable,
            model: d.model.clone(),
            label: d.label.clone(),
        }
    }

    /// A short human description (`model` if known, else the device node).
    pub fn describe(&self) -> String {
        if self.model.is_empty() {
            self.dev.clone()
        } else {
            format!("{} ({})", self.dev, self.model)
        }
    }
}

/// A fully-specified flash request: which image goes onto which device.
#[derive(Debug, Clone, Default)]
pub struct FlashSpec {
    pub image_path: PathBuf,
    pub image_name: String,
    pub image_size: u64,
    pub target: FlashTarget,
}

/// How to run the privileged writer.
#[derive(Debug, Clone)]
pub enum FlashAuth {
    /// Already root — write directly.
    Root,
    /// `sudo` works without a password (cached / NOPASSWD).
    SudoNonInteractive,
    /// Escalate via `sudo`, validating this password first.
    SudoPassword(String),
}

/// Spawn the flash on the tokio runtime. Progress is reported via
/// [`AppEvent::Progress`] and a terminal [`AppEvent::FlashDone`]. The returned
/// token aborts it (killing the writer, leaving the device partially written).
pub fn spawn_flash(id: TaskId, spec: FlashSpec, auth: FlashAuth, tx: AppSender) -> CancelToken {
    let cancel = CancelToken::new();
    let task_cancel = cancel.clone();
    tokio::spawn(async move {
        let outcome = run_flash(id, spec, auth, &tx, task_cancel).await;
        let _ = tx.send(AppEvent::FlashDone { id, outcome }).await;
    });
    cancel
}

const CHUNK: usize = 4 * 1024 * 1024;

/// Hidden CLI flag: `rc --flash-write <device>` copies stdin → device as a
/// privileged helper (invoked through `sudo` when the app isn't root).
pub const FLASH_WRITE_FLAG: &str = "--flash-write";

async fn run_flash(
    id: TaskId,
    spec: FlashSpec,
    auth: FlashAuth,
    tx: &AppSender,
    cancel: CancelToken,
) -> TaskOutcome {
    // Validate the sudo password up front; the writer then runs `sudo -n`.
    if let FlashAuth::SudoPassword(pw) = &auth
        && let Err(e) = crate::mount::sudo_validate(pw).await
    {
        return TaskOutcome::Failed(format!("authentication failed: {e}"));
    }
    let _ = tx.try_send(progress_event(id, &spec.image_name, spec.image_size, 0)); // show 0%

    match auth {
        // Already privileged: write the device directly from this process.
        FlashAuth::Root => write_direct(id, &spec, tx, cancel).await,
        // Otherwise re-run ourselves under sudo as a privileged writer (still our
        // own code doing the I/O — no `dd`) and read its progress back.
        FlashAuth::SudoNonInteractive | FlashAuth::SudoPassword(_) => {
            write_via_helper(id, &spec, tx, cancel).await
        }
    }
}

/// Copy `image` → `device` in this (privileged) process, off the async runtime.
async fn write_direct(
    id: TaskId,
    spec: &FlashSpec,
    tx: &AppSender,
    cancel: CancelToken,
) -> TaskOutcome {
    let (device, image) = (spec.target.dev.clone(), spec.image_path.clone());
    let name = spec.image_name.clone();
    let total = spec.image_size;
    let tx = tx.clone();
    let dev_msg = device.clone();
    let res = tokio::task::spawn_blocking(move || {
        // Throttle progress events to ~10/s; always send the final 100%.
        let mut last: Option<Instant> = None;
        flash_copy(
            &image,
            &device,
            total,
            |synced| {
                let now = Instant::now();
                if synced >= total
                    || last.is_none_or(|t| now.duration_since(t) >= Duration::from_millis(100))
                {
                    last = Some(now);
                    let _ = tx.try_send(progress_event(id, &name, total, synced));
                }
            },
            || cancel.is_cancelled(),
        )
    })
    .await;
    match res {
        Ok(Ok(())) => TaskOutcome::Done,
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::Interrupted => TaskOutcome::Cancelled,
        Ok(Err(e)) => {
            let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
                " (run as root/administrator)"
            } else {
                ""
            };
            TaskOutcome::Failed(format!("writing {dev_msg} failed: {e}{hint}"))
        }
        Err(_) => TaskOutcome::Failed("flash task aborted".to_string()),
    }
}

/// Run a privileged copy of ourselves (`sudo rc --flash-write <device> <image>`)
/// and turn the committed-byte counts it prints into progress events.
async fn write_via_helper(
    id: TaskId,
    spec: &FlashSpec,
    tx: &AppSender,
    cancel: CancelToken,
) -> TaskOutcome {
    use std::process::Stdio;
    use tokio::process::Command;

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return TaskOutcome::Failed(format!("cannot locate the program: {e}")),
    };
    // The privileged helper opens the image itself, so give it an absolute path.
    let image = std::fs::canonicalize(&spec.image_path).unwrap_or_else(|_| spec.image_path.clone());
    let mut child = match Command::new("sudo")
        .arg("-n")
        .arg(&exe)
        .arg(FLASH_WRITE_FLAG)
        .arg(&spec.target.dev)
        .arg(&image)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return TaskOutcome::Failed(format!("cannot start privileged writer: {e}")),
    };

    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let (total, name) = (spec.image_size, spec.image_name.clone());

    // The helper prints a committed-byte count after each sync; relay it.
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(l)) => {
                    if let Ok(n) = l.trim().parse::<u64>() {
                        let _ = tx.try_send(progress_event(id, &name, total, n.min(total)));
                    }
                }
                _ => break, // EOF or read error: the helper has finished
            },
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return TaskOutcome::Cancelled;
            }
        }
    }

    let status = match child.wait().await {
        Ok(s) => s,
        Err(e) => return TaskOutcome::Failed(format!("writer wait failed: {e}")),
    };
    if !status.success() {
        let mut err = String::new();
        let _ = stderr.read_to_string(&mut err).await;
        let err = err.trim().trim_start_matches("[sudo] password for").trim().to_string();
        return TaskOutcome::Failed(if err.is_empty() {
            "privileged writer failed".to_string()
        } else {
            err
        });
    }
    let _ = tx.try_send(progress_event(id, &name, total, total));
    TaskOutcome::Done
}

/// The privileged writer subcommand (`rc --flash-write <device> <image>`): copy
/// the image to the device, printing the committed-byte count after each sync so
/// the parent can show real progress. Returns a process exit code.
pub fn helper_main(device: &str, image: &str) -> i32 {
    use std::io::Write;
    let total = std::fs::metadata(image).map(|m| m.len()).unwrap_or(0);
    let stdout = std::io::stdout();
    let res = flash_copy(
        Path::new(image),
        device,
        total,
        |synced| {
            // Line-buffered + flushed so the parent sees each step immediately.
            let mut h = stdout.lock();
            let _ = writeln!(h, "{synced}");
            let _ = h.flush();
        },
        || false, // the parent aborts by killing us
    );
    match res {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("flash-write {device}: {e}");
            1
        }
    }
}

/// Sync interval: small enough for smooth progress, large enough to avoid
/// hammering devices that dislike frequent cache flushes (~100 steps for big
/// images, at least every chunk for small ones).
fn sync_window(total: u64) -> u64 {
    (total / 100).clamp(CHUNK as u64, 64 * 1024 * 1024)
}

/// Copy `image` to `device`, fsync-ing every [`sync_window`] bytes so progress
/// reflects what is actually on the device (not just what the page cache
/// accepted). `report(committed)` fires after each sync; `cancelled()` aborts.
fn flash_copy(
    image: &Path,
    device: &str,
    total: u64,
    mut report: impl FnMut(u64),
    mut cancelled: impl FnMut() -> bool,
) -> std::io::Result<()> {
    use std::io::{Read, Write};
    let mut img = std::fs::File::open(image)?;
    let mut dev = std::fs::OpenOptions::new().write(true).open(device)?;
    let window = sync_window(total);
    let mut buf = vec![0u8; CHUNK];
    let (mut written, mut unsynced) = (0u64, 0u64);
    loop {
        if cancelled() {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "aborted"));
        }
        let n = img.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dev.write_all(&buf[..n])?;
        written += n as u64;
        unsynced += n as u64;
        if unsynced >= window {
            dev.sync_data()?; // block until these bytes are durably on the device
            unsynced = 0;
            report(written);
        }
    }
    dev.flush()?;
    dev.sync_all()?;
    report(written);
    Ok(())
}

fn progress_event(id: TaskId, name: &str, total: u64, done: u64) -> AppEvent {
    AppEvent::Progress(ProgressUpdate {
        id,
        verb: "Flashing",
        current_name: name.to_string(),
        file_done: done,
        file_total: total,
        total_done: done,
        total_total: total,
        files_done: 0,
        files_total: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_image_extensions() {
        for n in ["ubuntu.iso", "RPI.IMG", "disk.raw", "x.bin", "a.dd", "y.wic"] {
            assert!(is_image_file(n), "{n} should be an image");
        }
        for n in ["notes.txt", "archive.zip", "noext", "image", "x.iso.gz"] {
            assert!(!is_image_file(n), "{n} should not be an image");
        }
    }

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rc_{tag}_{}_{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn flash_copy_writes_bytes_and_reports_committed_progress() {
        let dir = tmp_dir("flash");
        let img = dir.join("img.bin");
        let target = dir.join("target.bin");
        // Larger than the sync window (4 MiB) so progress arrives in steps.
        let data: Vec<u8> = (0..12_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&img, &data).unwrap();
        // The "device" must already exist (we open it for writing, not create).
        std::fs::write(&target, vec![0u8; data.len()]).unwrap();

        let mut reported = Vec::new();
        flash_copy(&img, target.to_str().unwrap(), data.len() as u64, |c| reported.push(c), || false)
            .unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), data, "device gets the image bytes");
        assert!(reported.len() >= 3, "progress advances in steps: {reported:?}");
        assert!(reported.windows(2).all(|w| w[0] <= w[1]), "monotonic");
        assert_eq!(*reported.last().unwrap(), data.len() as u64, "final report is 100%");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn flash_copy_aborts_when_cancelled() {
        let dir = tmp_dir("flashc");
        let img = dir.join("img.bin");
        let target = dir.join("target.bin");
        std::fs::write(&img, vec![7u8; 8 * 1024 * 1024]).unwrap();
        std::fs::write(&target, vec![0u8; 8 * 1024 * 1024]).unwrap();

        let err = flash_copy(&img, target.to_str().unwrap(), 8 * 1024 * 1024, |_| {}, || true)
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sync_window_scales_with_size() {
        assert_eq!(sync_window(1_000), CHUNK as u64, "tiny images sync every chunk");
        assert_eq!(sync_window(10_000 * 1024 * 1024), 64 * 1024 * 1024, "huge images cap the window");
    }
}
