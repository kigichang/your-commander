//! Find files (design screen 1h): a streaming recursive search over a local
//! or SFTP folder. Matches arrive over a channel as they are found.

use crate::fs::provider::FsEntry;
use crate::fs::sftp::SftpConnection;
use crate::fs::local;
use crate::ops::queue::{compile_mask, glob_match};
use crate::state::{AppState, Location};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::{Manager, State};

/// Files larger than this are skipped by the content search.
const CONTENT_MAX: u64 = 64 * 1024 * 1024;
const CHUNK: usize = 1024 * 1024;

#[derive(Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum FindEvent {
    Found { entry: FsEntry },
    Scanning { dir: String },
    #[serde(rename_all = "camelCase")]
    Done { count: usize, error: Option<String> },
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindParams {
    pub mask: String,
    pub content: Option<String>,
    /// Keep files modified at or after this Unix epoch ms.
    pub modified_after: Option<u64>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
}

struct Search {
    cancel: Arc<AtomicBool>,
    channel: Channel<FindEvent>,
    conn: Option<Arc<SftpConnection>>,
    mask: Vec<Vec<u8>>,
    needle: Vec<u8>,
    params: FindParams,
    count: usize,
    last_scan: Instant,
}

impl Search {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn matches(&self, entry: &FsEntry) -> bool {
        if !self.mask.is_empty() {
            let name = entry.name.to_ascii_lowercase();
            if !self.mask.iter().any(|p| glob_match(p, name.as_bytes())) {
                return false;
            }
        }
        if let Some(after) = self.params.modified_after
            && entry.modified.unwrap_or(0) < after
        {
            return false;
        }
        if let Some(min) = self.params.min_size
            && entry.size < min
        {
            return false;
        }
        if let Some(max) = self.params.max_size
            && entry.size > max
        {
            return false;
        }
        true
    }

    /// Case-insensitive (ASCII) substring search, streamed in chunks with an
    /// overlap so matches across chunk borders are not lost.
    fn contains_text(&self, entry: &FsEntry) -> bool {
        if entry.size > CONTENT_MAX {
            return false;
        }
        match &self.conn {
            None => {
                let Ok(file) = std::fs::File::open(&entry.path) else {
                    return false;
                };
                self.scan_reader(file)
            }
            Some(conn) => match conn.read_limited(&entry.path, CONTENT_MAX) {
                Ok(bytes) => self.scan_reader(std::io::Cursor::new(bytes)),
                Err(_) => false,
            },
        }
    }

    fn scan_reader<R: Read>(&self, mut reader: R) -> bool {
        let needle = &self.needle;
        let keep = needle.len().saturating_sub(1);
        let mut buf = vec![0u8; CHUNK + keep];
        let mut filled = 0usize;
        loop {
            if self.cancelled() {
                return false;
            }
            let n = match reader.read(&mut buf[filled..]) {
                Ok(0) => return false,
                Ok(n) => n,
                Err(_) => return false,
            };
            let end = filled + n;
            buf[..end].make_ascii_lowercase();
            if buf[..end]
                .windows(needle.len())
                .any(|w| w == needle.as_slice())
            {
                return true;
            }
            // Carry the tail over so border-spanning matches survive.
            let tail = end.saturating_sub(keep);
            buf.copy_within(tail..end, 0);
            filled = end - tail;
        }
    }

    fn list(&self, dir: &Path) -> Result<Vec<FsEntry>, String> {
        match &self.conn {
            None => local::list_dir(dir).map_err(|e| e.to_string()),
            Some(conn) => conn.list(dir).map_err(|e| e.to_string()),
        }
    }

    fn walk(&mut self, dir: &Path) {
        if self.cancelled() {
            return;
        }
        if self.last_scan.elapsed() > Duration::from_millis(100) {
            self.last_scan = Instant::now();
            let _ = self.channel.send(FindEvent::Scanning {
                dir: dir.to_string_lossy().to_string(),
            });
        }
        let Ok(entries) = self.list(dir) else {
            return; // unreadable folders are skipped, not fatal
        };
        for entry in entries {
            if self.cancelled() {
                return;
            }
            if entry.is_dir {
                if !entry.is_symlink {
                    self.walk(&entry.path);
                }
                continue;
            }
            if !self.matches(&entry) {
                continue;
            }
            if !self.needle.is_empty() && !self.contains_text(&entry) {
                continue;
            }
            self.count += 1;
            let _ = self.channel.send(FindEvent::Found { entry });
        }
    }
}

#[tauri::command]
pub fn start_find(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    location: Location,
    dir: PathBuf,
    params: FindParams,
    on_event: Channel<FindEvent>,
) -> Result<u64, String> {
    let conn = state.resolve(&location)?;
    let (id, cancel) = state.new_find();
    std::thread::spawn(move || {
        let mut search = Search {
            cancel,
            channel: on_event,
            conn,
            mask: compile_mask(&params.mask),
            needle: params
                .content
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .into_bytes(),
            params,
            count: 0,
            last_scan: Instant::now(),
        };
        search.walk(&dir);
        let _ = search.channel.send(FindEvent::Done {
            count: search.count,
            error: None,
        });
        app.state::<AppState>().finish_find(id);
    });
    Ok(id)
}

#[tauri::command]
pub fn cancel_find(state: State<'_, AppState>, find_id: u64) {
    if let Some(cancel) = state.finds.lock().unwrap().get(&find_id) {
        cancel.store(true, Ordering::Relaxed);
    }
}
