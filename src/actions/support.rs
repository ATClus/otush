//! Cancellation and finish-guard primitives shared by actions. (split from `actions.rs`; same behavior).

use crate::context::AppContext;
use std::future::Future;
use std::time::Duration;

pub(crate) const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Drop guard that notifies the [`TranscriptionCoordinator`] when the
/// transcription pipeline finishes — whether it completes normally or panics.
pub(crate) struct FinishGuard(pub(crate) AppContext);

impl Drop for FinishGuard {
    fn drop(&mut self) {
        self.0.coordinator.notify_processing_finished();
        // The pipeline just freed its large transient buffers (captured PCM,
        // WAV copy, engine scratch); hand the cached pages back to the OS so
        // they don't sit in malloc arenas until they get swapped out (#1792).
        crate::memory::trim_freed_memory();
    }
}

// Shortcut Action Trait

pub(crate) async fn complete_unless_cancelled<F, C>(
    operation: F,
    is_cancelled: C,
) -> Option<F::Output>
where
    F: Future,
    C: Fn() -> bool,
{
    tokio::pin!(operation);

    loop {
        if is_cancelled() {
            return None;
        }

        if let Ok(result) =
            tokio::time::timeout(CANCELLATION_POLL_INTERVAL, operation.as_mut()).await
        {
            return Some(result);
        }
    }
}
