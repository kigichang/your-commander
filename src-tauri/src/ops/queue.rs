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

/// Preset answer for name collisions, chosen in the copy dialog.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IfExists {
    #[default]
    Ask,
    Skip,
    /// Overwrite only when the source is newer; needs both mtimes, so it
    /// falls back to asking when either side cannot be stat'ed (remote).
    ReplaceOlder,
    /// Keep both under a " 2"-style name.
    Rename,
}

/// Options from the copy/move dialog (design screen 1e).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CopyOptions {
    pub verify: bool,
    /// Preserve permissions, mtime, and xattrs (Finder tags).
    pub keep_meta: bool,
    /// `*.jpg;*.png`-style mask; files only, folders always traverse.
    pub mask: Option<String>,
    /// Copy symlinked files' contents instead of recreating the link.
    pub follow_symlinks: bool,
    pub if_exists: IfExists,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            verify: false,
            keep_meta: true,
            mask: None,
            follow_symlinks: false,
            if_exists: IfExists::Ask,
        }
    }
}

/// Case-insensitive `*`/`?` glob, used by the file mask.
fn glob_match(pattern: &[u8], name: &[u8]) -> bool {
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star_p, mut star_n) = (usize::MAX, 0usize);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p].eq_ignore_ascii_case(&name[n])) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star_p = p;
            star_n = n;
            p += 1;
        } else if star_p != usize::MAX {
            p = star_p + 1;
            star_n += 1;
            n = star_n;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

/// Where a running operation reports to. The Tauri layer forwards progress
/// and conflicts to the frontend; tests plug in a canned implementation.
pub trait OpSink: Send {
    fn progress(&self, detail: &str, fraction: f32, done_bytes: u64, total_bytes: u64);
    /// Ask what to do about an existing destination. Blocks the worker.
    fn conflict(&self, src: &Path, dest: &Path) -> ConflictChoice;
}

/// Worker-side context: progress reporting, cancellation, pause, conflict
/// prompts, and the copy options.
pub struct OpCtx {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
    /// Sticky "apply to all" conflict policy once the user picked one.
    pub policy: Option<ConflictChoice>,
    pub options: CopyOptions,
    /// Lower-cased mask patterns compiled from `options.mask`.
    mask: Vec<Vec<u8>>,
    pub done_bytes: u64,
    pub total_bytes: u64,
    sink: Box<dyn OpSink>,
}

impl OpCtx {
    pub fn new(cancel: Arc<AtomicBool>, pause: Arc<AtomicBool>, sink: Box<dyn OpSink>) -> Self {
        Self {
            cancel,
            pause,
            policy: None,
            options: CopyOptions::default(),
            mask: Vec::new(),
            done_bytes: 0,
            total_bytes: 0,
            sink,
        }
    }

    pub fn set_options(&mut self, options: CopyOptions) {
        self.mask = options
            .mask
            .as_deref()
            .unwrap_or("")
            .split(';')
            .map(|p| p.trim().to_ascii_lowercase().into_bytes())
            .filter(|p| !p.is_empty() && p != b"*" && p != b"*.*")
            .collect();
        self.options = options;
    }

    /// Does this file name pass the dialog's mask? Folders are not masked.
    pub fn mask_matches(&self, name: &str) -> bool {
        self.mask.is_empty() || self.mask.iter().any(|p| glob_match(p, name.as_bytes()))
    }

    /// Also the pause point: every op loop polls this, so a paused op simply
    /// parks here until resumed or cancelled.
    pub fn cancelled(&self) -> bool {
        while self.pause.load(Ordering::Relaxed) && !self.cancel.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
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
        match self.options.if_exists {
            IfExists::Skip => return ConflictChoice::Skip,
            IfExists::Rename => return ConflictChoice::KeepBoth,
            IfExists::ReplaceOlder => {
                if let (Ok(s), Ok(d)) = (std::fs::metadata(src), std::fs::metadata(dest))
                    && let (Ok(sm), Ok(dm)) = (s.modified(), d.modified())
                {
                    return if sm > dm {
                        ConflictChoice::Overwrite
                    } else {
                        ConflictChoice::Skip
                    };
                }
                // Can't compare (remote side): fall through to asking.
            }
            IfExists::Ask => {}
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
        Self::new(
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
            Box::new(TestSink(choice)),
        )
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
    fn mask_globs() {
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        ctx.set_options(CopyOptions {
            mask: Some("*.jpg;IMG_??.png".into()),
            ..Default::default()
        });
        assert!(ctx.mask_matches("photo.JPG"));
        assert!(ctx.mask_matches("IMG_42.png"));
        assert!(!ctx.mask_matches("IMG_123.png"));
        assert!(!ctx.mask_matches("doc.pdf"));
        // Empty / *.* masks match everything.
        ctx.set_options(CopyOptions::default());
        assert!(ctx.mask_matches("anything"));
    }

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
