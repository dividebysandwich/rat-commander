//! Events delivered to the render loop from background tasks.
//!
//! Terminal input is handled separately (read directly in the loop); this
//! channel carries only asynchronous results so the loop never blocks on I/O.

use crate::disk::DiskEntry;
use crate::ops::progress::{ConflictInfo, ProgressUpdate, TaskId, TaskOutcome};

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A throttled progress snapshot from the ops engine.
    Progress(ProgressUpdate),
    /// A copy/move hit an existing destination; the engine is paused awaiting the
    /// user's overwrite decision (sent back via the task's reply channel).
    Conflict(ConflictInfo),
    /// A background task finished (success, cancel, or failure).
    TaskDone { id: TaskId, outcome: TaskOutcome },
    /// A find-file task finished (or was aborted); carries the paths collected
    /// so far so partial results can still be panelized.
    FindDone {
        id: TaskId,
        paths: Vec<std::path::PathBuf>,
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
}
