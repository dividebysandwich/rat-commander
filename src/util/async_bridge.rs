//! Channel plumbing between async worker tasks and the synchronous render loop.
//!
//! The render loop owns all UI state and is the only place that touches the
//! terminal. Background tasks (file ops, future remote I/O) never touch state
//! directly; they send [`AppEvent`](crate::app::event::AppEvent) values back
//! over this bounded channel.

use crate::app::event::AppEvent;
use tokio::sync::mpsc;

/// Sender half handed to background tasks.
pub type AppSender = mpsc::Sender<AppEvent>;
/// Receiver half polled by the render loop.
pub type AppReceiver = mpsc::Receiver<AppEvent>;

/// Create the app event channel. The buffer is bounded so a flood of progress
/// updates applies backpressure instead of growing without limit.
pub fn channel() -> (AppSender, AppReceiver) {
    mpsc::channel(256)
}
