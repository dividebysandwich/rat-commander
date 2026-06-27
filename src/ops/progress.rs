//! Progress and outcome types reported by background operations.

use std::time::SystemTime;

/// Identifies a running background task.
pub type TaskId = u64;

/// A throttled progress snapshot emitted by the ops engine.
#[derive(Debug, Clone)]
pub struct ProgressUpdate {
    pub id: TaskId,
    /// Human label for the operation ("Copying", "Moving", "Deleting").
    pub verb: &'static str,
    /// Name of the file currently being processed.
    pub current_name: String,
    /// Bytes done / total for the current file (total may be 0 for dirs).
    pub file_done: u64,
    pub file_total: u64,
    /// Bytes done / total across the whole operation.
    pub total_done: u64,
    pub total_total: u64,
    /// Files completed / total file count.
    pub files_done: u64,
    pub files_total: u64,
}

/// How a task ended.
#[derive(Debug, Clone)]
pub enum TaskOutcome {
    Done,
    Cancelled,
    Failed(String),
}

/// Details of a copy/move destination that already exists, sent to the UI so it
/// can show the overwrite-confirmation dialog.
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    pub id: TaskId,
    /// Bare file name in conflict.
    pub name: String,
    /// The incoming (source) file.
    pub new_path: String,
    pub new_size: u64,
    pub new_mtime: Option<SystemTime>,
    /// The existing (destination) file.
    pub old_path: String,
    pub old_size: u64,
    pub old_mtime: Option<SystemTime>,
}

/// A global overwrite rule chosen from the dialog's "overwrite all files" row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteRule {
    /// Overwrite every conflicting file.
    All,
    /// Overwrite only when the destination is older than the source ("update").
    Older,
    /// Never overwrite.
    None,
    /// Overwrite only when the destination is smaller than the source.
    Smaller,
    /// Overwrite only when the file sizes differ.
    SizeDiffers,
}

impl OverwriteRule {
    /// Whether, under this rule, a given source should overwrite the existing
    /// destination.
    pub fn should_overwrite(self, new_size: u64, new_mtime: Option<SystemTime>, old_size: u64, old_mtime: Option<SystemTime>) -> bool {
        match self {
            OverwriteRule::All => true,
            OverwriteRule::None => false,
            OverwriteRule::Older => old_mtime
                .zip(new_mtime)
                .map(|(o, n)| o < n)
                .unwrap_or(true),
            OverwriteRule::Smaller => old_size < new_size,
            OverwriteRule::SizeDiffers => old_size != new_size,
        }
    }
}

/// The user's answer to an overwrite prompt, sent back to the engine.
#[derive(Debug, Clone, Copy)]
pub enum OverwriteDecision {
    /// Overwrite just this file.
    OverwriteOnce,
    /// Skip just this file.
    SkipOnce,
    /// Append the source onto the existing destination (local files).
    AppendOnce,
    /// Apply a rule to this and all remaining conflicts. `skip_empty` additionally
    /// protects any destination from being clobbered by a zero-length source.
    Policy {
        rule: OverwriteRule,
        skip_empty: bool,
    },
    /// Abort the whole operation.
    Abort,
}

/// What the engine should do with a single conflicting file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyAction {
    Overwrite,
    Skip,
    Append,
}
