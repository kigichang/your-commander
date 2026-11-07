use super::queue::{cancelled_err, ConflictChoice, OpCtx};
use super::safe_path::{create_dir_inside, is_plain_name, symlink_inside};
use super::transfer::total_bytes;
use crate::fs::provider::FsEntry;
use crate::fs::sftp::SftpConnection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Remote counterpart of `keep_both_path`: probe the server for a free name.
fn remote_keep_both(conn: &SftpConnection, dest: &Path) -> PathBuf {
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
        if conn.exists(&candidate).is_none() {
            return candidate;
        }
    }
    dest.to_path_buf()
}

/// Remote counterpart of `transfer::total_bytes`: sum the sizes the
/// download will actually stream, honoring the mask.
fn remote_total_bytes(
    ctx: &OpCtx,
    conn: &SftpConnection,
    paths: &[(PathBuf, bool, bool)],
) -> u64 {
    fn walk_dir(ctx: &OpCtx, conn: &SftpConnection, dir: &Path) -> u64 {
        conn.list(dir)
            .map(|entries| {
                entries
                    .iter()
                    .map(|e| {
                        if e.is_symlink {
                            0
                        } else if e.is_dir {
                            walk_dir(ctx, conn, &e.path)
                        } else if ctx.mask_matches(&e.name) {
                            e.size
                        } else {
                            0
                        }
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    // A file's size comes from its parent's listing. The selected files
    // nearly always share one parent, so each parent is listed only once.
    let mut listings: HashMap<&Path, Vec<FsEntry>> = HashMap::new();
    let mut total = 0;
    for (path, is_dir, is_symlink) in paths {
        if *is_symlink {
            // Recreated as a link (or streamed, where size is unknown anyway).
        } else if *is_dir {
            total += walk_dir(ctx, conn, path);
        } else if !ctx.mask_matches(&path.file_name().unwrap_or_default().to_string_lossy()) {
            // Skipped by download_item, so not part of the total.
        } else if let Some(parent) = path.parent() {
            total += listings
                .entry(parent)
                .or_insert_with(|| conn.list(parent).unwrap_or_default())
                .iter()
                .find(|e| e.path == *path)
                .map_or(0, |e| e.size);
        }
    }
    total
}

/// Copy or move local `sources` into the remote directory `remote_dir`.
/// Returns the number of top-level items processed.
pub fn upload_op(
    ctx: &mut OpCtx,
    conn: Arc<SftpConnection>,
    sources: Vec<PathBuf>,
    remote_dir: PathBuf,
    remove_source: bool,
) -> Result<usize, String> {
    ctx.total_bytes = total_bytes(ctx, &sources);
    let mut count = 0usize;
    for src in &sources {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let Some(name) = src.file_name() else { continue };
        upload_item(ctx, &conn, src, &remote_dir.join(name))?;
        if remove_source {
            let result = if src.is_dir() && !src.is_symlink() {
                std::fs::remove_dir_all(src)
            } else {
                std::fs::remove_file(src)
            };
            result.map_err(|e| format!("{}: {e}", src.display()))?;
        }
        count += 1;
    }
    Ok(count)
}

fn upload_item(
    ctx: &mut OpCtx,
    conn: &SftpConnection,
    src: &Path,
    dest: &Path,
) -> Result<(), String> {
    if ctx.cancelled() {
        return Err(cancelled_err());
    }
    // Recreate links by default; with follow-symlinks on, file links are
    // uploaded like regular files (directory links always stay links).
    if src.is_symlink() && !(ctx.options.follow_symlinks && src.is_file()) {
        if !ctx.mask_matches(&src.file_name().unwrap_or_default().to_string_lossy()) {
            return Ok(());
        }
        let target = std::fs::read_link(src).map_err(|e| format!("{}: {e}", src.display()))?;
        let mut dest = dest.to_path_buf();
        if conn.exists(&dest).is_some() {
            match ctx.ask_conflict(src, &dest) {
                ConflictChoice::Overwrite => conn.remove_recursive(&dest)?,
                ConflictChoice::Skip => return Ok(()),
                ConflictChoice::KeepBoth => dest = remote_keep_both(conn, &dest),
                ConflictChoice::CancelOp => return Err(cancelled_err()),
                _ => return Ok(()),
            }
        }
        ctx.report(&src.to_string_lossy());
        return conn.make_symlink(&dest, &target.to_string_lossy());
    }
    if src.is_dir() && !src.is_symlink() {
        match conn.exists(dest) {
            Some(true) => {}
            Some(false) => {
                return Err(format!("\"{}\" already exists", dest.to_string_lossy()))
            }
            None => conn.mkdir(dest)?,
        }
        let entries =
            std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
        for entry in entries.flatten() {
            upload_item(ctx, conn, &entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        if !ctx.mask_matches(&src.file_name().unwrap_or_default().to_string_lossy()) {
            return Ok(());
        }
        let mut dest = dest.to_path_buf();
        if conn.exists(&dest).is_some() {
            match ctx.ask_conflict(src, &dest) {
                ConflictChoice::Overwrite => {}
                ConflictChoice::Skip => {
                    ctx.done_bytes += src.metadata().map(|m| m.len()).unwrap_or(0);
                    return Ok(());
                }
                ConflictChoice::KeepBoth => dest = remote_keep_both(conn, &dest),
                ConflictChoice::CancelOp => return Err(cancelled_err()),
                _ => return Ok(()),
            }
        }
        conn.upload_file(ctx, src, &dest)
    }
}

/// Copy or move remote `sources` (path, is_dir, is_symlink) into local
/// `local_dir`. Returns the number of top-level items processed.
pub fn download_op(
    ctx: &mut OpCtx,
    conn: Arc<SftpConnection>,
    sources: Vec<(PathBuf, bool, bool)>,
    local_dir: PathBuf,
    remove_source: bool,
) -> Result<usize, String> {
    ctx.total_bytes = remote_total_bytes(ctx, &conn, &sources);
    // Everything the server sends lands under this folder: symlinks it
    // reports must point inside it, and nothing is written through one
    // that does not.
    let root = local_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", local_dir.display()))?;
    let mut count = 0usize;
    for (src, is_dir, is_symlink) in &sources {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let Some(name) = src.file_name() else { continue };
        download_item(
            ctx,
            &conn,
            &root,
            src,
            *is_dir,
            *is_symlink,
            &root.join(name),
        )?;
        if remove_source {
            conn.remove_recursive(src)?;
        }
        count += 1;
    }
    Ok(count)
}

fn download_item(
    ctx: &mut OpCtx,
    conn: &SftpConnection,
    root: &Path,
    src: &Path,
    is_dir: bool,
    is_symlink: bool,
    dest: &Path,
) -> Result<(), String> {
    if ctx.cancelled() {
        return Err(cancelled_err());
    }
    // Recreate links by default; with follow-symlinks on, file links are
    // downloaded like regular files (directory links always stay links).
    if is_symlink && !(ctx.options.follow_symlinks && !is_dir) {
        if !ctx.mask_matches(&src.file_name().unwrap_or_default().to_string_lossy()) {
            return Ok(());
        }
        let target = conn.read_link(src)?;
        let mut dest = dest.to_path_buf();
        if dest.exists() || dest.is_symlink() {
            match ctx.ask_conflict(src, &dest) {
                ConflictChoice::Overwrite => {
                    if dest.is_dir() && !dest.is_symlink() {
                        std::fs::remove_dir_all(&dest)
                    } else {
                        std::fs::remove_file(&dest)
                    }
                    .map_err(|e| format!("{}: {e}", dest.display()))?;
                }
                ConflictChoice::Skip => return Ok(()),
                ConflictChoice::KeepBoth => dest = super::queue::keep_both_path(&dest),
                ConflictChoice::CancelOp => return Err(cancelled_err()),
                _ => return Ok(()),
            }
        }
        ctx.report(&src.to_string_lossy());
        return symlink_inside(root, &dest, &target);
    }
    if is_dir {
        create_dir_inside(root, dest)?;
        let entries = conn
            .list(src)
            .map_err(|e| format!("{}: {e}", src.display()))?;
        for entry in entries {
            // A hostile server can name entries anything; only a plain
            // name may be joined onto the local folder.
            if !is_plain_name(&entry.name) {
                return Err(format!(
                    "{}: refusing entry name {:?}",
                    src.display(),
                    entry.name
                ));
            }
            download_item(
                ctx,
                conn,
                root,
                &entry.path,
                entry.is_dir,
                entry.is_symlink,
                &dest.join(&entry.name),
            )?;
        }
        Ok(())
    } else {
        if !ctx.mask_matches(&src.file_name().unwrap_or_default().to_string_lossy()) {
            return Ok(());
        }
        let mut dest = dest.to_path_buf();
        if dest.exists() {
            match ctx.ask_conflict(src, &dest) {
                ConflictChoice::Overwrite => {}
                ConflictChoice::Skip => return Ok(()),
                ConflictChoice::KeepBoth => dest = super::queue::keep_both_path(&dest),
                ConflictChoice::CancelOp => return Err(cancelled_err()),
                _ => return Ok(()),
            }
        }
        conn.download_file(ctx, src, &dest)
    }
}

/// Permanently delete remote items (there is no remote trash).
pub fn delete_remote_op(
    ctx: &mut OpCtx,
    conn: Arc<SftpConnection>,
    items: Vec<PathBuf>,
) -> Result<usize, String> {
    let count = items.len();
    for item in &items {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        ctx.report(&item.to_string_lossy());
        conn.remove_recursive(item)?;
    }
    Ok(count)
}
