//! Events delivered to the render loop from background tasks.
//!
//! Terminal input is handled separately (read directly in the loop); this
//! channel carries only asynchronous results so the loop never blocks on I/O.

use crate::ops::progress::{ProgressUpdate, TaskId, TaskOutcome};

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// A throttled progress snapshot from the ops engine.
    Progress(ProgressUpdate),
    /// A background task finished (success, cancel, or failure).
    TaskDone { id: TaskId, outcome: TaskOutcome },
}
