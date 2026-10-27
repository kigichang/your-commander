use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub type OpId = u64;

/// Error message for a user-cancelled operation. The frontend matches on it
/// to suppress the error toast.
pub const CANCELLED: &str = "__cancelled__";

pub fn cancelled_err() -> String {
    CANCELLED.to_string()
}

/// What the user decided about one name collision.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictChoice {
    Overwrite,
    OverwriteAll,
    Skip,
    SkipAll,
    /// Keep both: the incoming file gets a " 2"-style suffix.
    KeepBoth,
    CancelOp,
}

/// Where a running operation reports to. The Tauri layer forwards progress
/// and conflicts to the frontend; tests plug in a canned implementation.
pub trait OpSink: Send {
    fn progress(&self, detail: &str, fraction: f32, done_bytes: u64, total_bytes: u64);
    /// Ask what to do about an existing destination. Blocks the worker.
    fn conflict(&self, src: &Path, dest: &Path) -> ConflictChoice;
}

/// Worker-side context: progress reporting, cancellation, conflict prompts.
pub struct OpCtx {
    pub cancel: Arc<AtomicBool>,
    /// Sticky "apply to all" conflict policy once the user picked one.
    pub policy: Option<ConflictChoice>,
    pub done_bytes: u64,
    pub total_bytes: u64,
    sink: Box<dyn OpSink>,
}

impl OpCtx {
    pub fn new(cancel: Arc<AtomicBool>, sink: Box<dyn OpSink>) -> Self {
        Self {
            cancel,
            policy: None,
            done_bytes: 0,
            total_bytes: 0,
            sink,
        }
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn report(&self, detail: &str) {
        let fraction = if self.total_bytes > 0 {
            (self.done_bytes as f32 / self.total_bytes as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.sink
            .progress(detail, fraction, self.done_bytes, self.total_bytes);
    }

    /// Ask the UI what to do about an existing destination. Blocks the worker.
    pub fn ask_conflict(&mut self, src: &Path, dest: &Path) -> ConflictChoice {
        match self.policy {
            Some(ConflictChoice::OverwriteAll) => return ConflictChoice::Overwrite,
            Some(ConflictChoice::SkipAll) => return ConflictChoice::Skip,
            _ => {}
        }
        let choice = self.sink.conflict(src, dest);
        match choice {
            ConflictChoice::OverwriteAll => {
                self.policy = Some(ConflictChoice::OverwriteAll);
                ConflictChoice::Overwrite
            }
            ConflictChoice::SkipAll => {
                self.policy = Some(ConflictChoice::SkipAll);
                ConflictChoice::Skip
            }
            other => other,
        }
    }
}

/// Sink used by unit tests: swallows progress, answers conflicts with a
/// canned choice.
#[cfg(test)]
pub struct TestSink(pub ConflictChoice);

#[cfg(test)]
impl OpSink for TestSink {
    fn progress(&self, _detail: &str, _fraction: f32, _done: u64, _total: u64) {}
    fn conflict(&self, _src: &Path, _dest: &Path) -> ConflictChoice {
        self.0
    }
}

#[cfg(test)]
impl OpCtx {
    /// Context for unit tests: no cancellation, conflicts answered by `choice`.
    pub fn for_test(choice: ConflictChoice) -> Self {
        Self::new(Arc::new(AtomicBool::new(false)), Box::new(TestSink(choice)))
    }
}

/// Find a free "name 2.ext"-style path next to `dest`.
pub fn keep_both_path(dest: &Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new(""));
    let stem = dest
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = dest.extension().map(|e| e.to_string_lossy().to_string());
    for n in 2..10_000 {
        let name = match &ext {
            Some(e) => format!("{stem} {n}.{e}"),
            None => format!("{stem} {n}"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    dest.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_both_picks_free_name() {
        let dir = std::env::temp_dir().join(format!("yc_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("file.txt");
        std::fs::write(&base, b"x").unwrap();
        let alt = keep_both_path(&base);
        assert_eq!(alt.file_name().unwrap().to_string_lossy(), "file 2.txt");
        std::fs::write(&alt, b"x").unwrap();
        let alt2 = keep_both_path(&base);
        assert_eq!(alt2.file_name().unwrap().to_string_lossy(), "file 3.txt");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
