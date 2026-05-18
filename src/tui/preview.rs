//! Lazy preview lock — yazi audit §6.
//!
//! When the cursor moves to a new row, the previous preview job is
//! cancelled and a fresh one is spawned. The lock holds the
//! [`tokio_util::sync::CancellationToken`] of the in-flight job so the
//! caller can flip it on selection change without tracking handles
//! manually.
//!
//! M4.3 wires the RecordingList as the first consumer: landing on a
//! row spawns a [`crate::media::probe_file`] task whose result feeds
//! `media_info_cache`. Switching rows cancels the in-flight probe;
//! Detail / Schedule previews migrate to the same substrate next.

use tokio_util::sync::CancellationToken;

/// State for the active preview job. Distinct from [`Task`] (the
/// long-running job registry) — previews are debounced and short-
/// lived, so they don't earn a registry row.
#[derive(Debug, Default)]
pub struct PreviewLock {
    /// Cancellation handle for the currently-spawned preview. `None`
    /// means no preview is in flight.
    pub current: Option<CancellationToken>,
    /// Identifier of the row this preview belongs to. Render code uses
    /// it to detect "the row I'm rendering matches the one being
    /// previewed" — otherwise display the previous cached value.
    pub key: Option<String>,
}

impl PreviewLock {
    /// Start a new preview for `key`. Returns a fresh CancellationToken
    /// the caller passes to the spawned future. Any previously running
    /// preview is cancelled.
    pub fn start(&mut self, key: impl Into<String>) -> CancellationToken {
        if let Some(prev) = self.current.take() {
            prev.cancel();
        }
        let token = CancellationToken::new();
        self.current = Some(token.clone());
        self.key = Some(key.into());
        token
    }

    /// Cancel the in-flight preview without starting a new one. Called
    /// when the user navigates away from the pane.
    pub fn cancel(&mut self) {
        if let Some(prev) = self.current.take() {
            prev.cancel();
        }
        self.key = None;
    }

    /// Return true iff the in-flight preview matches `key`. Workers
    /// poll this before committing their result so a late completion
    /// doesn't overwrite a fresh selection.
    pub fn is_current(&self, key: &str) -> bool {
        self.key.as_deref() == Some(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_cancels_previous() {
        let mut lock = PreviewLock::default();
        let t1 = lock.start("a");
        assert!(!t1.is_cancelled());
        let _t2 = lock.start("b");
        assert!(
            t1.is_cancelled(),
            "starting a new preview must cancel the old one"
        );
        assert!(lock.is_current("b"));
        assert!(!lock.is_current("a"));
    }

    #[test]
    fn cancel_clears_state() {
        let mut lock = PreviewLock::default();
        let t = lock.start("a");
        lock.cancel();
        assert!(t.is_cancelled());
        assert!(lock.current.is_none());
        assert!(lock.key.is_none());
    }
}
