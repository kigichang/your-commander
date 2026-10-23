use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;

pub type OpId = u64;

/// What the user decided about one name collision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictChoice {
    Overwrite,
    OverwriteAll,
    Skip,
    SkipAll,
    /// Keep both: the incoming file gets a " 2"-style suffix.
    KeepBoth,
    CancelOp,
}

pub struct ConflictRequest {
    pub src: PathBuf,
    pub dest: PathBuf,
    pub respond: Sender<ConflictChoice>,
}

pub enum OpEvent {
    Progress {
        id: OpId,
        detail: String,
        fraction: f32,
    },
    Conflict(ConflictRequest),
    /// Ask the UI to navigate a panel somewhere (e.g. a fresh mount point).
    OpenPath {
        side: usize,
        path: PathBuf,
    },
    /// A background SSH connect finished; hand the session to a panel.
    SftpReady {
        side: usize,
        conn: Arc<crate::fs::sftp::SftpConnection>,
    },
    Done {
        id: OpId,
        result: Result<String, String>,
    },
}

/// UI-side record of one running operation.
pub struct OpHandle {
    pub id: OpId,
    pub label: String,
    pub cancel: Arc<AtomicBool>,
    pub fraction: f32,
    pub detail: String,
}

/// Worker-side context: progress reporting, cancellation, conflict prompts.
pub struct OpCtx {
    pub id: OpId,
    pub tx: Sender<OpEvent>,
    pub cancel: Arc<AtomicBool>,
    /// Sticky "apply to all" conflict policy once the user picked one.
    pub policy: Option<ConflictChoice>,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

impl OpCtx {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn report(&self, detail: &str) {
        let fraction = if self.total_bytes > 0 {
            (self.done_bytes as f32 / self.total_bytes as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let _ = self.tx.send(OpEvent::Progress {
            id: self.id,
            detail: detail.to_string(),
            fraction,
        });
    }

    pub fn open_path(&self, side: usize, path: PathBuf) {
        let _ = self.tx.send(OpEvent::OpenPath { side, path });
    }

    pub fn sftp_ready(&self, side: usize, conn: Arc<crate::fs::sftp::SftpConnection>) {
        let _ = self.tx.send(OpEvent::SftpReady { side, conn });
    }

    /// Ask the UI what to do about an existing destination. Blocks the worker.
    pub fn ask_conflict(&mut self, src: &std::path::Path, dest: &std::path::Path) -> ConflictChoice {
        match self.policy {
            Some(ConflictChoice::OverwriteAll) => return ConflictChoice::Overwrite,
            Some(ConflictChoice::SkipAll) => return ConflictChoice::Skip,
            _ => {}
        }
        let (tx, rx) = channel();
        let _ = self.tx.send(OpEvent::Conflict(ConflictRequest {
            src: src.to_path_buf(),
            dest: dest.to_path_buf(),
            respond: tx,
        }));
        let choice = rx.recv().unwrap_or(ConflictChoice::CancelOp);
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

pub struct OpsManager {
    tx: Sender<OpEvent>,
    rx: Receiver<OpEvent>,
    pub running: Vec<OpHandle>,
    pub pending_conflict: Option<ConflictRequest>,
    next_id: OpId,
    egui_ctx: egui::Context,
}

/// Result of draining worker events for one frame.
pub struct Drained {
    pub finished: Vec<(String, Result<String, String>)>,
    pub open_paths: Vec<(usize, PathBuf)>,
    pub sftp_ready: Vec<(usize, Arc<crate::fs::sftp::SftpConnection>)>,
}

impl OpsManager {
    pub fn new(egui_ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            running: Vec::new(),
            pending_conflict: None,
            next_id: 1,
            egui_ctx,
        }
    }

    pub fn is_busy(&self) -> bool {
        !self.running.is_empty()
    }

    /// Spawn a worker thread running `work`. The closure gets an [`OpCtx`]
    /// and returns the final status message.
    pub fn spawn<F>(&mut self, label: String, work: F)
    where
        F: FnOnce(&mut OpCtx) -> Result<String, String> + Send + 'static,
    {
        let id = self.next_id;
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.running.push(OpHandle {
            id,
            label,
            cancel: cancel.clone(),
            fraction: 0.0,
            detail: String::new(),
        });
        let tx = self.tx.clone();
        let egui_ctx = self.egui_ctx.clone();
        std::thread::spawn(move || {
            let mut ctx = OpCtx {
                id,
                tx: tx.clone(),
                cancel,
                policy: None,
                done_bytes: 0,
                total_bytes: 0,
            };
            let result = work(&mut ctx);
            let _ = tx.send(OpEvent::Done { id, result });
            egui_ctx.request_repaint();
        });
        self.egui_ctx.request_repaint();
    }

    /// Pull worker events into UI state. Call once per frame.
    pub fn drain(&mut self) -> Drained {
        let mut finished = Vec::new();
        let mut open_paths = Vec::new();
        let mut sftp_ready = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                OpEvent::Progress { id, detail, fraction } => {
                    if let Some(h) = self.running.iter_mut().find(|h| h.id == id) {
                        h.detail = detail;
                        h.fraction = fraction;
                    }
                }
                OpEvent::Conflict(req) => {
                    // One at a time; a second op's conflict waits in the channel.
                    if self.pending_conflict.is_none() {
                        self.pending_conflict = Some(req);
                    } else {
                        let _ = self.tx.send(OpEvent::Conflict(req));
                        break;
                    }
                }
                OpEvent::OpenPath { side, path } => {
                    open_paths.push((side, path));
                }
                OpEvent::SftpReady { side, conn } => {
                    sftp_ready.push((side, conn));
                }
                OpEvent::Done { id, result } => {
                    if let Some(pos) = self.running.iter().position(|h| h.id == id) {
                        let h = self.running.remove(pos);
                        finished.push((h.label, result));
                    }
                }
            }
        }
        if self.is_busy() {
            self.egui_ctx
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        Drained {
            finished,
            open_paths,
            sftp_ready,
        }
    }
}

/// Find a free "name 2.ext"-style path next to `dest`.
pub fn keep_both_path(dest: &std::path::Path) -> PathBuf {
    let parent = dest.parent().unwrap_or_else(|| std::path::Path::new(""));
    let stem = dest
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = dest
        .extension()
        .map(|e| e.to_string_lossy().to_string());
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
