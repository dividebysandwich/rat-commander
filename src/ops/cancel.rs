//! Cancellation token for background operations.
//!
//! A thin wrapper around tokio-util's `CancellationToken` so the rest of the
//! code depends on our own type and the engine has one source of truth.

use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub struct CancelToken(CancellationToken);

impl CancelToken {
    pub fn new() -> Self {
        CancelToken(CancellationToken::new())
    }

    /// Trip the token; all observers see `is_cancelled() == true`.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}
