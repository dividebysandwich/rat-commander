//! The file-operation subsystem: request types, the engine, progress, cancel.

pub mod cancel;
pub mod engine;
pub mod progress;

pub use cancel::CancelToken;
pub use progress::TaskId;

use crate::app::event::AppEvent;
use crate::util::async_bridge::AppSender;
use crate::vfs::{Vfs, VfsPath};
use std::sync::Arc;

/// Which operation to perform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Copy,
    Move,
    Delete,
}

/// A fully-resolved operation request handed to the engine.
pub struct OpRequest {
    pub kind: OpKind,
    pub src_fs: Arc<dyn Vfs>,
    pub sources: Vec<VfsPath>,
    pub dst_fs: Option<Arc<dyn Vfs>>,
    pub dst_dir: Option<VfsPath>,
}

/// A handle to a running task, stored by the app so the progress dialog's
/// Abort button can trip its cancel token.
pub struct TaskHandle {
    pub id: TaskId,
    pub cancel: CancelToken,
}

/// Spawn the operation on the tokio runtime. The task reports progress and a
/// terminal [`AppEvent::TaskDone`] over `tx`.
pub fn spawn_op(id: TaskId, req: OpRequest, tx: AppSender) -> TaskHandle {
    let cancel = CancelToken::new();
    let task_cancel = cancel.clone();
    let done_tx = tx.clone();
    tokio::spawn(async move {
        let outcome = engine::run(id, req, tx, task_cancel).await;
        let _ = done_tx.send(AppEvent::TaskDone { id, outcome }).await;
    });
    TaskHandle { id, cancel }
}
