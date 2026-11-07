pub mod config;
pub mod drag;
pub mod find;
pub mod fs;
pub mod ops;
pub mod plantuml;
pub mod preview;
pub mod sftp;
pub mod smb;
pub mod sync;
pub mod system;

/// Run blocking work (file system, subprocesses, network) off the async
/// runtime's core threads. A panicked worker comes back as an Err.
pub(crate) async fn blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}
