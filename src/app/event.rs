//! Events delivered to the render loop from background tasks.
//!
//! Terminal input is handled separately (read directly in the loop); this
//! channel carries only asynchronous results so the loop never blocks on I/O.

use crate::disk::DiskEntry;
use crate::net::Scan;
use crate::ops::progress::{ConflictInfo, ProgressUpdate, TaskId, TaskOutcome};
use crate::util::checksum::ChecksumReport;
use crate::vfs::VfsPath;

/// Why a file was fetched to a local temp (so the handler opens the right view).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchKind {
    View,
    Edit,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A throttled progress snapshot from the ops engine.
    Progress(ProgressUpdate),
    /// A copy/move hit an existing destination; the engine is paused awaiting the
    /// user's overwrite decision (sent back via the task's reply channel).
    Conflict(ConflictInfo),
    /// A background task finished (success, cancel, or failure).
    TaskDone { id: TaskId, outcome: TaskOutcome },
    /// A privileged disk-manager command (mount/unmount/format) run in the
    /// background finished; carries its result and the success message to show.
    PrivilegedDone {
        ok_msg: String,
        result: Result<(), String>,
    },
    /// An image-flash task finished (success, cancel, or failure).
    FlashDone {
        id: TaskId,
        outcome: TaskOutcome,
    },
    /// A device-imaging ("create image") task finished.
    ImageDone {
        id: TaskId,
        outcome: TaskOutcome,
    },
    /// A file-checksum task finished. `Ok(report)` on success (the report also
    /// carries any comparison verdict); `Err(Some(msg))` on I/O failure;
    /// `Err(None)` when the user aborted (the progress dialog just closes).
    ChecksumDone {
        id: TaskId,
        result: Result<ChecksumReport, Option<String>>,
    },
    /// A find-file task finished (or was aborted); carries the matching files
    /// (path + size) collected so far so partial results can still be panelized.
    /// Paths may be local or remote, depending on the searched backend.
    FindDone {
        id: TaskId,
        results: Vec<(VfsPath, u64)>,
    },
    /// A find-duplicates task finished (or was cancelled). Carries the file names
    /// to mark in the left and right panels (identical per the chosen criteria);
    /// partial on cancel.
    DuplicatesFound {
        id: TaskId,
        left: Vec<String>,
        right: Vec<String>,
    },
    /// A "Details" panel's background size scan reported progress (`done` marks
    /// the final update). `viewer` is the panel displaying the details; a stale
    /// `generation` is ignored.
    DetailsTally {
        viewer: usize,
        generation: u64,
        total: u64,
        files: u64,
        dirs: u64,
        done: bool,
    },
    /// Progress of an in-flight disk-explorer scan: `done` of `total` immediate
    /// subdirectories sized so far. `generation` guards against stale updates.
    DiskScanProgress {
        generation: u64,
        done: usize,
        total: usize,
    },
    /// A disk-explorer background scan finished; `generation` lets the view drop
    /// results from a directory it has already navigated away from.
    DiskScanned {
        generation: u64,
        entries: Vec<DiskEntry>,
    },
    /// A network-explorer `ss` scan finished; `generation` lets the view drop a
    /// result from a scan it has already superseded.
    NetworkScanned {
        generation: u64,
        result: Result<Scan, String>,
    },
    /// A view/edit fetch streamed a (remote/archive) file to a local temp file;
    /// the handler opens it (paged viewer, or editor targeting `orig_path`).
    FileFetched {
        id: TaskId,
        kind: FetchKind,
        name: String,
        orig_path: VfsPath,
        temp: std::path::PathBuf,
    },
}
