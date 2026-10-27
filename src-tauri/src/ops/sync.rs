//! Folder compare / synchronize (design screen 1g). Compare builds a diff of
//! two trees (files only; folders materialize on copy); sync copies each
//! row in its chosen direction, overwriting the other side.

use crate::fs::provider::epoch_millis;
use crate::fs::sftp::SftpConnection;
use crate::ops::queue::{cancelled_err, ConflictChoice, OpCtx};
use crate::ops::transfer;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Allow for coarse filesystem timestamps (FAT, SMB) when comparing mtimes.
const MTIME_TOLERANCE_MS: u64 = 2000;

#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideInfo {
    pub size: u64,
    pub modified: Option<u64>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRow {
    pub rel: String,
    pub left: Option<SideInfo>,
    pub right: Option<SideInfo>,
    /// Only meaningful when both sides exist.
    pub equal: bool,
}

fn collect_local(base: &Path, rel: &str, out: &mut BTreeMap<String, SideInfo>) -> Result<(), String> {
    let dir = base.join(rel);
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        let path = entry.path();
        if path.is_symlink() {
            continue;
        }
        if path.is_dir() {
            collect_local(base, &child_rel, out)?;
        } else if let Ok(meta) = path.metadata() {
            out.insert(
                child_rel,
                SideInfo {
                    size: meta.len(),
                    modified: meta.modified().ok().map(epoch_millis),
                },
            );
        }
    }
    Ok(())
}

fn collect_remote(
    conn: &SftpConnection,
    base: &Path,
    rel: &str,
    out: &mut BTreeMap<String, SideInfo>,
) -> Result<(), String> {
    let dir = base.join(rel);
    let entries = conn.list(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        if entry.is_symlink {
            continue;
        }
        let child_rel = if rel.is_empty() {
            entry.name.clone()
        } else {
            format!("{rel}/{}", entry.name)
        };
        if entry.is_dir {
            collect_remote(conn, base, &child_rel, out)?;
        } else {
            out.insert(
                child_rel,
                SideInfo {
                    size: entry.size,
                    modified: entry.modified,
                },
            );
        }
    }
    Ok(())
}

fn files_equal(a: &Path, b: &Path) -> Result<bool, String> {
    let mut fa = std::fs::File::open(a).map_err(|e| format!("{}: {e}", a.display()))?;
    let mut fb = std::fs::File::open(b).map_err(|e| format!("{}: {e}", b.display()))?;
    let mut ba = vec![0u8; 256 * 1024];
    let mut bb = vec![0u8; 256 * 1024];
    loop {
        let na = fa.read(&mut ba).map_err(|e| e.to_string())?;
        let mut filled = 0;
        while filled < na {
            let n = fb.read(&mut bb[filled..na]).map_err(|e| e.to_string())?;
            if n == 0 {
                return Ok(false);
            }
            filled += n;
        }
        if ba[..na] != bb[..na] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(fb.read(&mut bb[..1]).map_err(|e| e.to_string())? == 0);
        }
    }
}

pub fn compare(
    left_conn: Option<&SftpConnection>,
    left_dir: &Path,
    right_conn: Option<&SftpConnection>,
    right_dir: &Path,
    by_content: bool,
) -> Result<Vec<SyncRow>, String> {
    let mut left = BTreeMap::new();
    match left_conn {
        None => collect_local(left_dir, "", &mut left)?,
        Some(c) => collect_remote(c, left_dir, "", &mut left)?,
    }
    let mut right = BTreeMap::new();
    match right_conn {
        None => collect_local(right_dir, "", &mut right)?,
        Some(c) => collect_remote(c, right_dir, "", &mut right)?,
    }

    let mut rows = Vec::new();
    let keys: std::collections::BTreeSet<&String> = left.keys().chain(right.keys()).collect();
    for rel in keys {
        let l = left.get(rel).copied();
        let r = right.get(rel).copied();
        let equal = match (&l, &r) {
            (Some(a), Some(b)) if a.size == b.size => {
                if by_content && left_conn.is_none() && right_conn.is_none() {
                    files_equal(&left_dir.join(rel), &right_dir.join(rel))?
                } else {
                    match (a.modified, b.modified) {
                        (Some(ma), Some(mb)) => ma.abs_diff(mb) <= MTIME_TOLERANCE_MS,
                        _ => true, // size match is all we have
                    }
                }
            }
            _ => false,
        };
        rows.push(SyncRow {
            rel: rel.clone(),
            left: l,
            right: r,
            equal,
        });
    }
    Ok(rows)
}

#[derive(Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Ltr,
    Rtl,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncItem {
    pub rel: String,
    pub direction: Direction,
}

/// Make sure every folder along `rel`'s parent chain exists on the remote.
fn ensure_remote_parents(conn: &SftpConnection, base: &Path, rel: &str) -> Result<(), String> {
    let mut dir = base.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        dir = dir.join(part);
        if conn.exists(&dir).is_none() {
            conn.mkdir(&dir)?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn sync_op(
    ctx: &mut OpCtx,
    left_conn: Option<Arc<SftpConnection>>,
    left_dir: PathBuf,
    right_conn: Option<Arc<SftpConnection>>,
    right_dir: PathBuf,
    items: Vec<SyncItem>,
) -> Result<usize, String> {
    if left_conn.is_some() && right_conn.is_some() {
        return Err("Remote-to-remote sync is not supported".to_string());
    }
    // The user already approved each row: overwrite without prompting.
    ctx.policy = Some(ConflictChoice::OverwriteAll);

    // Best-effort total for the progress bar (local sources only).
    ctx.total_bytes = items
        .iter()
        .map(|item| {
            let (conn, dir) = match item.direction {
                Direction::Ltr => (&left_conn, &left_dir),
                Direction::Rtl => (&right_conn, &right_dir),
            };
            match conn {
                None => dir.join(&item.rel).metadata().map(|m| m.len()).unwrap_or(0),
                Some(_) => 0,
            }
        })
        .sum();

    let mut count = 0usize;
    for item in &items {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let (src_conn, src_base, dest_conn, dest_base) = match item.direction {
            Direction::Ltr => (&left_conn, &left_dir, &right_conn, &right_dir),
            Direction::Rtl => (&right_conn, &right_dir, &left_conn, &left_dir),
        };
        let src = src_base.join(&item.rel);
        let dest = dest_base.join(&item.rel);
        ctx.report(&item.rel);
        match (src_conn, dest_conn) {
            (None, None) => {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                transfer::copy_file(ctx, &src, &dest)?;
            }
            (None, Some(conn)) => {
                ensure_remote_parents(conn, dest_base, &item.rel)?;
                conn.upload_file(ctx, &src, &dest)?;
            }
            (Some(conn), None) => {
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                conn.download_file(ctx, &src, &dest)?;
            }
            (Some(_), Some(_)) => unreachable!(),
        }
        count += 1;
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yc_sync_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn compare_and_sync_roundtrip() {
        let left = tmp("l");
        let right = tmp("r");
        std::fs::create_dir_all(left.join("sub")).unwrap();
        std::fs::write(left.join("only_left.txt"), b"L").unwrap();
        std::fs::write(left.join("sub/deep.txt"), b"deep").unwrap();
        std::fs::write(left.join("same.txt"), b"same").unwrap();
        std::fs::write(right.join("same.txt"), b"same").unwrap();
        std::fs::write(right.join("only_right.txt"), b"R").unwrap();
        std::fs::write(left.join("diff.txt"), b"left!").unwrap();
        std::fs::write(right.join("diff.txt"), b"right").unwrap();

        let rows = compare(None, &left, None, &right, true).unwrap();
        let get = |rel: &str| rows.iter().find(|r| r.rel == rel).unwrap();
        assert!(get("only_left.txt").right.is_none());
        assert!(get("only_right.txt").left.is_none());
        assert!(get("same.txt").equal);
        assert!(!get("diff.txt").equal);
        assert!(get("sub/deep.txt").right.is_none());

        let items = vec![
            SyncItem { rel: "only_left.txt".into(), direction: Direction::Ltr },
            SyncItem { rel: "sub/deep.txt".into(), direction: Direction::Ltr },
            SyncItem { rel: "only_right.txt".into(), direction: Direction::Rtl },
            SyncItem { rel: "diff.txt".into(), direction: Direction::Ltr },
        ];
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        let n = sync_op(&mut ctx, None, left.clone(), None, right.clone(), items).unwrap();
        assert_eq!(n, 4);
        assert_eq!(std::fs::read(right.join("sub/deep.txt")).unwrap(), b"deep");
        assert_eq!(std::fs::read(left.join("only_right.txt")).unwrap(), b"R");
        assert_eq!(std::fs::read(right.join("diff.txt")).unwrap(), b"left!");

        for d in [&left, &right] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
