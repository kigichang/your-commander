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

    /// None for local, Some(conn) for SFTP.
    pub fn resolve(&self, location: &Location) -> Result<Option<Arc<SftpConnection>>, String> {
        match location {
            Location::Local => Ok(None),
            Location::Sftp { conn_id } => self.sftp(*conn_id).map(Some),
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
}
