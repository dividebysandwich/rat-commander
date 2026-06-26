//! Progress and outcome types reported by background operations.

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
