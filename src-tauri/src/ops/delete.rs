use super::queue::{cancelled_err, OpCtx};
use std::path::PathBuf;

/// Move items to the macOS Trash (restorable from Finder).
pub fn trash_op(ctx: &mut OpCtx, items: Vec<PathBuf>) -> Result<usize, String> {
    let count = items.len();
    ctx.report("");
    trash::delete_all(&items).map_err(|e| e.to_string())?;
    Ok(count)
}

/// Permanently delete items, one by one with progress.
pub fn delete_op(ctx: &mut OpCtx, items: Vec<PathBuf>) -> Result<usize, String> {
    let count = items.len();
    ctx.total_bytes = count as u64;
    for item in &items {
        if ctx.cancelled() {
            return Err(cancelled_err());
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
    Ok(count)
}
