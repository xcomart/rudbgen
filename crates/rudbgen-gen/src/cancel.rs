//! Stopping a run that is already going.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The flag a run checks between files.
///
/// A run is driven from a background thread while the button that stops it is
/// on the UI thread, so the two share one atomic. Cloning gives another view
/// of the *same* flag, never a second one.
///
/// The token is not reset by a finished run: a token is one run's, and reusing
/// a cancelled one would start a job that stops at its first file.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A token that has not been cancelled.
    pub fn new() -> Self {
        CancelToken::default()
    }

    /// Ask the run to stop at the next file boundary.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether the run has been asked to stop.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// A handle that can only cancel, for the side that owns the button.
    ///
    /// The distinction is a documentation one — the flag is the same — but it
    /// keeps `is_cancelled` out of the UI, where the answer would be stale by
    /// the time it is painted anyway.
    pub fn handle(&self) -> CancelHandle {
        CancelHandle(Arc::clone(&self.0))
    }
}

/// The cancelling half of a [`CancelToken`].
#[derive(Clone, Debug)]
pub struct CancelHandle(Arc<AtomicBool>);

impl CancelHandle {
    /// Ask the run to stop at the next file boundary.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_cancels_the_token_it_came_from() {
        let token = CancelToken::new();
        let handle = token.handle();
        assert!(!token.is_cancelled());

        // The point of the split: another thread holds the handle.
        std::thread::spawn(move || handle.cancel()).join().unwrap();
        assert!(token.is_cancelled());
    }

    #[test]
    fn a_clone_is_the_same_flag() {
        let token = CancelToken::new();
        let clone = token.clone();
        clone.cancel();
        assert!(token.is_cancelled());
    }
}
