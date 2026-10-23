use super::queue::OpCtx;
use rust_i18n::t;
use std::path::PathBuf;

/// Move items to the macOS Trash (restorable from Finder).
pub fn trash_op(ctx: &mut OpCtx, items: Vec<PathBuf>) -> Result<String, String> {
    let count = items.len();
    ctx.report(&t!("status.trashing"));
    trash::delete_all(&items).map_err(|e| e.to_string())?;
    Ok(t!("status.trashed", count = count).to_string())
}

/// Permanently delete items, one by one with progress.
pub fn delete_op(ctx: &mut OpCtx, items: Vec<PathBuf>) -> Result<String, String> {
    let count = items.len();
    ctx.total_bytes = count as u64;
    for item in &items {
        if ctx.cancelled() {
            return Err(t!("status.cancelled").to_string());
        }
        ctx.report(&item.to_string_lossy());
        let result = if item.is_dir() && !item.is_symlink() {
            std::fs::remove_dir_all(item)
        } else {
            std::fs::remove_file(item)
        };
        result.map_err(|e| format!("{}: {e}", item.display()))?;
        ctx.done_bytes += 1;
    }
    Ok(t!("status.deleted", count = count).to_string())
}
