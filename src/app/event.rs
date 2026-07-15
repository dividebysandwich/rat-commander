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

/// Which guided Git dialog to open once the repository's branches/remotes have
/// been read in the background (see [`AppEvent::GitInfo`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitInfoForm {
    Checkout,
    Push,
    Fetch,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// The persistent console subshell produced output; a coalesced signal to
    /// wake the render loop so the backdrop repaints. Carries no data — the
    /// output is already in the shared emulator.
    ConsoleOutput,
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
    /// A reverse-DNS lookup for a peer IP finished (`host` = `None` = no PTR).
    ReverseDnsResolved {
        ip: String,
        host: Option<String>,
    },
    /// A background Git-status scan for panel `side` finished; a stale
    /// `generation` is ignored. `status` is `None` when the directory is not a
    /// git work tree (or git is unavailable).
    GitStatusScanned {
        side: usize,
        generation: u64,
        status: Option<Box<crate::git::GitStatus>>,
    },
    /// A background Details-view preview load finished for panel `viewer`; a stale
    /// `generation` is ignored.
    DetailsPreview {
        viewer: usize,
        generation: u64,
        preview: Box<crate::details::Preview>,
    },
    /// A "Send file over LAN" selection finished being zipped to a temp archive;
    /// `Ok(path)` gives the archive to serve, `Err(msg)` reports a failure. `name`
    /// is the friendly download name to advertise. Only used for the multi-file /
    /// directory case (a lone file skips zipping).
    SendPrepared {
        name: String,
        result: Result<std::path::PathBuf, String>,
    },
    /// A device fully downloaded the shared file from the LAN send server; the
    /// open Send dialog bumps its download counter.
    FileSent,
    /// A Git command finished; `title` names it (e.g. `"push"`). The handler shows
    /// the output (or closes quietly when a successful command said nothing) and
    /// refreshes the panels' VCS state.
    GitDone {
        title: String,
        out: crate::git::ops::GitOutput,
    },
    /// The branches/remotes behind a guided Git dialog were read; open `form`
    /// populated with them.
    GitInfo {
        form: GitInfoForm,
        info: Box<crate::git::ops::RepoInfo>,
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
