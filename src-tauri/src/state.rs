use crate::fs::sftp::SftpConnection;
use crate::ops::queue::{ConflictChoice, OpId};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

/// Which file system a path lives on. Every fs command takes one.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Location {
    Local,
    #[serde(rename_all = "camelCase")]
    Sftp {
        conn_id: u32,
    },
    /// Browsing inside a local archive; paths are inner paths ("" = root).
    #[serde(rename_all = "camelCase")]
    Archive {
        archive: std::path::PathBuf,
    },
}

/// A resolved [`Location`], ready to act on.
pub enum Resolved {
    Local,
    Sftp(Arc<SftpConnection>),
    Archive(std::path::PathBuf),
}

impl Resolved {
    /// The SFTP connection when there is one; `Err` for archives, which the
    /// caller does not support.
    pub fn conn_or_err(self, what: &str) -> Result<Option<Arc<SftpConnection>>, String> {
        match self {
            Resolved::Local => Ok(None),
            Resolved::Sftp(conn) => Ok(Some(conn)),
            Resolved::Archive(_) => Err(format!("{what} is not supported inside archives")),
        }
    }
}

/// Shared handle for one running operation: lets the UI cancel it and answer
/// its pending conflict prompt. `cancel` is the same flag the worker's OpCtx
/// polls.
#[derive(Default)]
pub struct OpControl {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
    pub conflict_tx: Mutex<Option<Sender<ConflictChoice>>>,
}

#[derive(Default)]
pub struct AppState {
    pub config: Mutex<crate::config::Config>,
    pub connections: Mutex<HashMap<u32, Arc<SftpConnection>>>,
    next_conn_id: AtomicU32,
    pub ops: Mutex<HashMap<OpId, Arc<OpControl>>>,
    next_op_id: AtomicU64,
    pub watchers: Mutex<HashMap<u32, notify::RecommendedWatcher>>,
    /// PNG system icons keyed by "folder"/"generic"/extension.
    pub icons: Mutex<HashMap<String, Arc<Vec<u8>>>>,
    /// Cancel flags of running file searches.
    pub finds: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next_find_id: AtomicU64,
}

impl AppState {
    pub fn sftp(&self, conn_id: u32) -> Result<Arc<SftpConnection>, String> {
        self.connections
            .lock()
            .unwrap()
            .get(&conn_id)
            .cloned()
            .ok_or_else(|| format!("SFTP connection {conn_id} is gone"))
    }

    pub fn resolve(&self, location: &Location) -> Result<Resolved, String> {
        match location {
            Location::Local => Ok(Resolved::Local),
            Location::Sftp { conn_id } => self.sftp(*conn_id).map(Resolved::Sftp),
            Location::Archive { archive } => Ok(Resolved::Archive(archive.clone())),
        }
    }

    pub fn add_connection(&self, conn: Arc<SftpConnection>) -> u32 {
        let id = self.next_conn_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.connections.lock().unwrap().insert(id, conn);
        id
    }

    pub fn new_op(&self) -> (OpId, Arc<OpControl>) {
        let id = self.next_op_id.fetch_add(1, Ordering::Relaxed) + 1;
        let control = Arc::new(OpControl::default());
        self.ops.lock().unwrap().insert(id, control.clone());
        (id, control)
    }

    pub fn finish_op(&self, id: OpId) {
        self.ops.lock().unwrap().remove(&id);
    }

    pub fn new_find(&self) -> (u64, Arc<AtomicBool>) {
        let id = self.next_find_id.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.finds.lock().unwrap().insert(id, cancel.clone());
        (id, cancel)
    }

    pub fn finish_find(&self, id: u64) {
        self.finds.lock().unwrap().remove(&id);
    }
}
