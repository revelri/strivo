//! Async task registry — substrate for the M4 yazi-grade polish.
//!
//! Tracks every long-running operation the TUI cares about: live
//! recordings, transcoding, archiver back-catalog pulls, Crunchr
//! analyses, theme imports. Each [`Task`] carries a typed
//! [`Progress`] snapshot the status bar / future "Tasks" pane renders;
//! cancellation is cooperative through a [`tokio_util::sync::CancellationToken`].
//!
//! Adoption is incremental. M4.1.a wires the scaffold + the recording
//! pipeline as the first consumer; archiver / Crunchr / transcode
//! migrate to TaskRegistry-driven progress in follow-up commits.
//!
//! Yazi audit reference: §4 "Async task manager".

use std::collections::HashMap;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Stable identifier for a task. Distinct from RecordingJob.id so the
/// same recording can have a Record task + a Transcode task without
/// collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub Uuid);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

/// What kind of work the task represents. Used for status-bar grouping
/// and the upcoming tasks pane's filter chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
    Record,
    Transcode,
    ArchiverPull,
    CrunchrAnalyze,
    ThemeImport,
}

impl TaskKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Record => "rec",
            Self::Transcode => "trans",
            Self::ArchiverPull => "pull",
            Self::CrunchrAnalyze => "crunchr",
            Self::ThemeImport => "theme",
        }
    }
}

/// Live snapshot of a task's progress. Workers update by replacing the
/// `Progress` field on the registry entry.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Started; no percent / bytes yet.
    Indeterminate,
    /// Byte-based — surfaces "1.2 GB / 3.4 GB" in the status bar.
    Bytes { done: u64, total: Option<u64> },
    /// Percent in [0.0, 1.0].
    Percent { value: f32 },
    /// Counted units, e.g. "12 / 80 chunks".
    Counted { done: u32, total: u32 },
    /// Done — surfaces as "✓ 1.2 GB" briefly before the entry is reaped.
    Done,
    /// Failed — surfaces in red with the error message; entry is kept
    /// until the user dismisses it.
    Failed { error: String },
}

impl Progress {
    /// Human-readable summary for status-bar / one-line render.
    pub fn label(&self) -> String {
        match self {
            Self::Indeterminate => "…".to_string(),
            Self::Bytes { done, total } => match total {
                Some(t) if *t > 0 => format!("{}/{}", human_bytes(*done), human_bytes(*t)),
                _ => human_bytes(*done),
            },
            Self::Percent { value } => format!("{}%", (value * 100.0).round() as u32),
            Self::Counted { done, total } => format!("{done}/{total}"),
            Self::Done => "done".to_string(),
            Self::Failed { error } => format!("failed: {error}"),
        }
    }

    /// Returns true when the task is in a terminal state (Done/Failed).
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed { .. })
    }
}

fn human_bytes(b: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if b >= GB {
        format!("{:.2} GB", b as f64 / GB as f64)
    } else if b >= MB {
        format!("{:.1} MB", b as f64 / MB as f64)
    } else if b >= KB {
        format!("{:.0} KB", b as f64 / KB as f64)
    } else {
        format!("{b} B")
    }
}

#[derive(Debug)]
pub struct Task {
    pub id: TaskId,
    pub kind: TaskKind,
    pub title: String,
    pub progress: Progress,
    pub cancel: CancellationToken,
    pub started_at: Instant,
    /// When the task entered a terminal state. Used by the reaper to
    /// drop completed entries after a short visible-tail window.
    pub finished_at: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct TaskRegistry {
    inner: HashMap<TaskId, Task>,
    /// Insertion order so the status bar consistently shows the most
    /// recent task first without sorting on every render.
    order: Vec<TaskId>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new task. The caller holds the CancellationToken
    /// clone if it intends to cooperatively cancel from elsewhere.
    pub fn start(
        &mut self,
        kind: TaskKind,
        title: impl Into<String>,
        cancel: CancellationToken,
    ) -> TaskId {
        let id = TaskId::new();
        let task = Task {
            id,
            kind,
            title: title.into(),
            progress: Progress::Indeterminate,
            cancel,
            started_at: Instant::now(),
            finished_at: None,
        };
        self.order.push(id);
        self.inner.insert(id, task);
        id
    }

    pub fn set_progress(&mut self, id: TaskId, progress: Progress) {
        if let Some(t) = self.inner.get_mut(&id) {
            if progress.is_terminal() && t.finished_at.is_none() {
                t.finished_at = Some(Instant::now());
            }
            t.progress = progress;
        }
    }

    pub fn complete(&mut self, id: TaskId) {
        self.set_progress(id, Progress::Done);
    }

    pub fn fail(&mut self, id: TaskId, error: impl Into<String>) {
        self.set_progress(
            id,
            Progress::Failed {
                error: error.into(),
            },
        );
    }

    /// Drop terminal entries older than `keep_after`. Called once per
    /// frame; non-terminal entries are never reaped.
    pub fn reap(&mut self, keep_after: std::time::Duration) {
        let now = Instant::now();
        let to_drop: Vec<TaskId> = self
            .inner
            .iter()
            .filter(|(_, t)| {
                t.finished_at
                    .map(|at| now.duration_since(at) > keep_after)
                    .unwrap_or(false)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in &to_drop {
            self.inner.remove(id);
        }
        self.order.retain(|id| !to_drop.contains(id));
    }

    /// Request cooperative cancellation for `id`. The worker is
    /// responsible for polling the token between chunks.
    pub fn cancel(&self, id: TaskId) {
        if let Some(t) = self.inner.get(&id) {
            t.cancel.cancel();
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.inner.get(&id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Task> {
        self.order.iter().filter_map(|id| self.inner.get(id))
    }

    /// Count of tasks that haven't entered a terminal state yet.
    pub fn active(&self) -> usize {
        self.inner
            .values()
            .filter(|t| !t.progress.is_terminal())
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_then_progress_then_done() {
        let mut r = TaskRegistry::new();
        let cancel = CancellationToken::new();
        let id = r.start(TaskKind::Record, "test.mkv", cancel.clone());
        assert_eq!(r.active(), 1);
        r.set_progress(
            id,
            Progress::Bytes {
                done: 1024,
                total: Some(2048),
            },
        );
        assert_eq!(r.get(id).unwrap().progress.label(), "1 KB/2 KB");
        r.complete(id);
        assert_eq!(r.active(), 0);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn reap_drops_old_terminal_entries() {
        let mut r = TaskRegistry::new();
        let id = r.start(TaskKind::ThemeImport, "neon", CancellationToken::new());
        r.complete(id);
        std::thread::sleep(std::time::Duration::from_millis(20));
        r.reap(std::time::Duration::from_millis(10));
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn reap_keeps_active_tasks() {
        let mut r = TaskRegistry::new();
        r.start(TaskKind::Record, "live", CancellationToken::new());
        std::thread::sleep(std::time::Duration::from_millis(20));
        r.reap(std::time::Duration::from_millis(10));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn cancel_propagates_through_token() {
        let mut r = TaskRegistry::new();
        let cancel = CancellationToken::new();
        let id = r.start(TaskKind::CrunchrAnalyze, "x", cancel.clone());
        r.cancel(id);
        assert!(cancel.is_cancelled());
    }
}
