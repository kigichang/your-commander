use crate::fs::smb;
use crate::ops::queue::{
    ConflictChoice, OpCtx, OpId, OpSink, CANCELLED,
};
use crate::ops::{archive, delete, remote, transfer};
use crate::state::{AppState, Location, OpControl};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::{Manager, State};

/// Everything a running operation tells the frontend, in order, over one
/// Tauri IPC channel.
#[derive(Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum OpEvent {
    #[serde(rename_all = "camelCase")]
    Progress {
        detail: String,
        fraction: f32,
        done_bytes: u64,
        total_bytes: u64,
    },
    #[serde(rename_all = "camelCase")]
    Conflict { src: String, dest: String },
    #[serde(rename_all = "camelCase")]
    Done {
        ok: bool,
        cancelled: bool,
        /// Number of items processed (copy/trash/extract…), 0 if n/a.
        count: usize,
        /// Created archive name, SMB mount path, … when the op produced one.
        name: Option<String>,
        /// New SFTP connection id for a connect op.
        conn_id: Option<u32>,
        error: Option<String>,
    },
}

/// Forwards worker progress/conflicts to the frontend. Progress is throttled;
/// conflicts block the worker until `resolve_conflict` answers.
struct TauriSink {
    channel: Channel<OpEvent>,
    control: Arc<OpControl>,
    last_progress: Mutex<Instant>,
}

const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

impl OpSink for TauriSink {
    fn progress(&self, detail: &str, fraction: f32, done_bytes: u64, total_bytes: u64) {
        let mut last = self.last_progress.lock().unwrap();
        if last.elapsed() < PROGRESS_INTERVAL {
            return;
        }
        *last = Instant::now();
        let _ = self.channel.send(OpEvent::Progress {
            detail: detail.to_string(),
            fraction,
            done_bytes,
            total_bytes,
        });
    }

    fn conflict(&self, src: &Path, dest: &Path) -> ConflictChoice {
        let (tx, rx) = channel();
        *self.control.conflict_tx.lock().unwrap() = Some(tx);
        let _ = self.channel.send(OpEvent::Conflict {
            src: src.to_string_lossy().to_string(),
            dest: dest.to_string_lossy().to_string(),
        });
        let choice = rx.recv().unwrap_or(ConflictChoice::CancelOp);
        *self.control.conflict_tx.lock().unwrap() = None;
        choice
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum OpRequest {
    /// Copy or move between any two locations (local↔local, local↔SFTP).
    #[serde(rename_all = "camelCase")]
    Copy {
        src: Location,
        /// For SFTP sources the frontend already knows which entries are
        /// directories; local sources ignore this.
        sources: Vec<SourceItem>,
        dest: Location,
        dest_dir: PathBuf,
        remove_source: bool,
    },
    #[serde(rename_all = "camelCase")]
    Trash { items: Vec<PathBuf> },
    #[serde(rename_all = "camelCase")]
    Delete {
        location: Location,
        items: Vec<PathBuf>,
    },
    #[serde(rename_all = "camelCase")]
    Compress {
        sources: Vec<PathBuf>,
        dest: PathBuf,
        format: String,
    },
    #[serde(rename_all = "camelCase")]
    Extract {
        archives: Vec<PathBuf>,
        dest_dir: PathBuf,
    },
    #[serde(rename_all = "camelCase")]
    SmbMount {
        server: String,
        share: String,
        username: String,
        password: String,
    },
}

#[derive(serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceItem {
    pub path: PathBuf,
    pub is_dir: bool,
}

struct OpOutcome {
    count: usize,
    name: Option<String>,
}

fn run_request(
    ctx: &mut OpCtx,
    state: &AppState,
    request: OpRequest,
) -> Result<OpOutcome, String> {
    let counted = |count: usize| OpOutcome { count, name: None };
    match request {
        OpRequest::Copy {
            src,
            sources,
            dest,
            dest_dir,
            remove_source,
        } => {
            let src_conn = state.resolve(&src)?;
            let dest_conn = state.resolve(&dest)?;
            let paths: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
            match (src_conn, dest_conn) {
                (None, None) => {
                    transfer::copy_op(ctx, paths, dest_dir, remove_source).map(counted)
                }
                (None, Some(conn)) => {
                    remote::upload_op(ctx, conn, paths, dest_dir, remove_source).map(counted)
                }
                (Some(conn), None) => {
                    let pairs = sources.into_iter().map(|s| (s.path, s.is_dir)).collect();
                    remote::download_op(ctx, conn, pairs, dest_dir, remove_source).map(counted)
                }
                (Some(_), Some(_)) => Err("Remote-to-remote copy is not supported".to_string()),
            }
        }
        OpRequest::Trash { items } => delete::trash_op(ctx, items).map(counted),
        OpRequest::Delete { location, items } => match state.resolve(&location)? {
            None => delete::delete_op(ctx, items).map(counted),
            Some(conn) => remote::delete_remote_op(ctx, conn, items).map(counted),
        },
        OpRequest::Compress {
            sources,
            dest,
            format,
        } => {
            let format = match format.as_str() {
                "zip" => archive::ArchiveFormat::Zip,
                "tar" => archive::ArchiveFormat::Tar,
                "tar.gz" | "tgz" => archive::ArchiveFormat::TarGz,
                "gz" => archive::ArchiveFormat::Gz,
                other => return Err(format!("Unknown archive format: {other}")),
            };
            archive::compress_op(ctx, sources, dest, format).map(|name| OpOutcome {
                count: 1,
                name: Some(name),
            })
        }
        OpRequest::Extract { archives, dest_dir } => {
            archive::extract_op(ctx, archives, dest_dir).map(counted)
        }
        OpRequest::SmbMount {
            server,
            share,
            username,
            password,
        } => {
            ctx.report(&format!("{server}/{share}"));
            smb::mount(&server, &share, &username, &password).map(|path| OpOutcome {
                count: 0,
                name: Some(path.to_string_lossy().to_string()),
            })
        }
    }
}

/// Start a long-running operation; progress, conflicts, and completion arrive
/// over `on_event`. Returns the op id used by cancel/resolve.
#[tauri::command]
pub fn start_op(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: OpRequest,
    on_event: Channel<OpEvent>,
) -> Result<OpId, String> {
    let (id, control) = state.new_op();
    let sink = TauriSink {
        channel: on_event.clone(),
        control: control.clone(),
        last_progress: Mutex::new(Instant::now() - PROGRESS_INTERVAL),
    };
    let cancel = control.cancel.clone();
    let app = app.clone();
    std::thread::spawn(move || {
        let mut ctx = OpCtx::new(cancel, Box::new(sink));
        let result = {
            let state = app.state::<AppState>();
            run_request(&mut ctx, &state, request)
        };
        let event = match result {
            Ok(outcome) => OpEvent::Done {
                ok: true,
                cancelled: false,
                count: outcome.count,
                name: outcome.name,
                conn_id: None,
                error: None,
            },
            Err(e) => OpEvent::Done {
                ok: false,
                cancelled: e == CANCELLED,
                count: 0,
                name: None,
                conn_id: None,
                error: Some(e),
            },
        };
        let _ = on_event.send(event);
        app.state::<AppState>().finish_op(id);
    });
    Ok(id)
}

#[tauri::command]
pub fn cancel_op(state: State<'_, AppState>, op_id: OpId) {
    if let Some(control) = state.ops.lock().unwrap().get(&op_id) {
        control.cancel.store(true, Ordering::Relaxed);
        // A worker blocked on a conflict prompt must be released too.
        if let Some(tx) = control.conflict_tx.lock().unwrap().take() {
            let _ = tx.send(ConflictChoice::CancelOp);
        }
    }
}

#[tauri::command]
pub fn resolve_conflict(
    state: State<'_, AppState>,
    op_id: OpId,
    choice: ConflictChoice,
) -> Result<(), String> {
    let ops = state.ops.lock().unwrap();
    let control = ops
        .get(&op_id)
        .ok_or_else(|| format!("operation {op_id} is gone"))?;
    let tx = control
        .conflict_tx
        .lock()
        .unwrap()
        .take()
        .ok_or_else(|| "no pending conflict".to_string())?;
    tx.send(choice).map_err(|e| e.to_string())
}
