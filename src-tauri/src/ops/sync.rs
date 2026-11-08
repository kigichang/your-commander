//! Folder compare / synchronize (design screen 1g). Compare builds a diff of
//! two trees (files only; folders materialize on copy); sync copies each
//! row in its chosen direction, overwriting the other side.

use crate::fs::provider::epoch_millis;
use crate::fs::sftp::SftpConnection;
use crate::ops::queue::{cancelled_err, ConflictChoice, OpCtx};
use crate::ops::safe_path::{create_dir_inside, is_listing_name};
use crate::ops::transfer;
use std::collections::{BTreeMap, HashSet};
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
        // file_type() comes from readdir itself on macOS/Linux, and for a
        // non-link entry metadata()'s lstat is the stat: one call a file.
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            collect_local(base, &child_rel, out)?;
        } else if let Ok(meta) = entry.metadata() {
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
    super::transfer::same_contents(a, b, &|| false)
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
    /// The source side's size as compare saw it. A remote source would cost
    /// a round trip per row to stat, so its share of the progress total
    /// comes from here.
    #[serde(default)]
    pub size: Option<u64>,
}

/// The folders along `rel`'s parent chain under `base` not yet in `known`,
/// shallowest first; they are added to `known`. Every row of a sync shares
/// one `known`, so each remote folder is checked once per sync rather than
/// once per file beneath it.
fn unchecked_parents(known: &mut HashSet<PathBuf>, base: &Path, rel: &str) -> Vec<PathBuf> {
    let mut dir = base.to_path_buf();
    let mut out = Vec::new();
    if let Some((parents, _file)) = rel.rsplit_once('/') {
        for part in parents.split('/') {
            dir.push(part);
            if known.insert(dir.clone()) {
                out.push(dir.clone());
            }
        }
    }
    out
}

/// Make sure every folder along `rel`'s parent chain exists on the remote,
/// and that none already there is a link leading out of `base`.
fn ensure_remote_parents(
    conn: &SftpConnection,
    known: &mut HashSet<PathBuf>,
    base: &Path,
    rel: &str,
) -> Result<(), String> {
    for dir in unchecked_parents(known, base, rel) {
        if conn.exists(&dir).is_none() {
            conn.mkdir(&dir)?;
        } else if !conn.real_path(&dir)?.starts_with(conn.real_path(base)?) {
            return Err(format!("{}: path escapes the destination folder", dir.display()));
        }
    }
    Ok(())
}

/// Where `dest` really goes under the local folder `base`: its parent chain
/// is created, and refused if a link along it leads out of `base`. Compare
/// skips links, so such a row reads as a file missing on this side.
fn real_local_dest(base: &Path, dest: &Path) -> Result<PathBuf, String> {
    let root = base
        .canonicalize()
        .map_err(|e| format!("{}: {e}", base.display()))?;
    let (Some(parent), Some(name)) = (dest.parent(), dest.file_name()) else {
        return Err(format!("{}: not a file path", dest.display()));
    };
    Ok(create_dir_inside(&root, parent)?.join(name))
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
    // Rows come from `compare`, whose listings hold only plain names, but
    // check anyway: each `rel` is joined onto both folders, and a "..",
    // empty or absolute piece would land the copy outside them.
    if let Some(bad) = items.iter().find(|i| !i.rel.split('/').all(is_listing_name)) {
        return Err(format!("{:?}: not a path inside the folder", bad.rel));
    }
    // The user already approved each row: overwrite without prompting.
    ctx.policy = Some(ConflictChoice::OverwriteAll);

    // Progress total: a local source is stat'ed now; a remote one uses the
    // size compare reported.
    ctx.total_bytes = items
        .iter()
        .map(|item| {
            let (conn, dir) = match item.direction {
                Direction::Ltr => (&left_conn, &left_dir),
                Direction::Rtl => (&right_conn, &right_dir),
            };
            match conn {
                None => dir.join(&item.rel).metadata().map(|m| m.len()).unwrap_or(0),
                Some(_) => item.size.unwrap_or(0),
            }
        })
        .sum();

    let mut remote_dirs = HashSet::new();
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
                let dest = real_local_dest(dest_base, &dest)?;
                transfer::copy_file(ctx, &src, &dest)?;
            }
            (None, Some(conn)) => {
                ensure_remote_parents(conn, &mut remote_dirs, dest_base, &item.rel)?;
                conn.upload_file(ctx, &src, &dest)?;
            }
            (Some(conn), None) => {
                let dest = real_local_dest(dest_base, &dest)?;
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
            SyncItem { rel: "only_left.txt".into(), direction: Direction::Ltr, size: None },
            SyncItem { rel: "sub/deep.txt".into(), direction: Direction::Ltr, size: None },
            SyncItem { rel: "only_right.txt".into(), direction: Direction::Rtl, size: None },
            SyncItem { rel: "diff.txt".into(), direction: Direction::Ltr, size: None },
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

    #[test]
    fn sync_refuses_paths_that_leave_the_folders() {
        let work = tmp("esc");
        let (left, right) = (work.join("l"), work.join("r"));
        std::fs::create_dir_all(left.join("sub")).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::write(left.join("sub/f.txt"), b"x").unwrap();
        let outside = work.join("f.txt");
        for rel in ["../f.txt", "sub/../../f.txt", "/etc/f.txt", "", "sub//f.txt", "./sub/f.txt"] {
            let items = vec![
                // A good row first: nothing is copied when any row is bad.
                SyncItem { rel: "sub/f.txt".into(), direction: Direction::Ltr, size: None },
                SyncItem { rel: rel.into(), direction: Direction::Ltr, size: None },
            ];
            let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
            let err = sync_op(&mut ctx, None, left.clone(), None, right.clone(), items)
                .unwrap_err();
            assert!(err.contains("not a path inside"), "{rel:?}: {err}");
            assert!(!right.join("sub/f.txt").exists(), "{rel:?}");
        }
        assert!(!outside.exists());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn local_listing_skips_links() {
        let base = tmp("links");
        std::fs::create_dir_all(base.join("dir")).unwrap();
        std::fs::write(base.join("dir/a.txt"), b"abc").unwrap();
        std::os::unix::fs::symlink("dir", base.join("dir_link")).unwrap();
        std::os::unix::fs::symlink("dir/a.txt", base.join("file_link")).unwrap();
        let mut out = BTreeMap::new();
        collect_local(&base, "", &mut out).unwrap();
        assert_eq!(out.keys().collect::<Vec<_>>(), vec!["dir/a.txt"]);
        assert_eq!(out["dir/a.txt"].size, 3);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn each_remote_folder_is_checked_once_per_sync() {
        let base = Path::new("/srv/dest");
        let mut known = HashSet::new();
        let mut check = |rel: &str| unchecked_parents(&mut known, base, rel);
        assert_eq!(
            check("a/b/c/one.txt"),
            vec![base.join("a"), base.join("a/b"), base.join("a/b/c")]
        );
        assert!(check("a/b/c/two.txt").is_empty(), "same folder: nothing new");
        assert_eq!(check("a/b/d/three.txt"), vec![base.join("a/b/d")]);
        assert!(check("top.txt").is_empty(), "the base itself is never checked");
        // 500 files four folders deep: four checks, not 2,000.
        let mut known = HashSet::new();
        let checks: usize = (0..500)
            .map(|i| unchecked_parents(&mut known, base, &format!("p/q/r/s/f{i}.txt")).len())
            .sum();
        assert_eq!(checks, 4);
    }

    #[test]
    fn sync_does_not_write_through_a_parent_link_leaving_the_folder() {
        let work = tmp("parent_link");
        let (left, right, outside) = (work.join("l"), work.join("r"), work.join("outside"));
        std::fs::create_dir_all(left.join("sub")).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(left.join("sub/f.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, right.join("sub")).unwrap();

        // Compare skips the link, so the row reads as "only on the left".
        let rows = compare(None, &left, None, &right, false).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].right.is_none());

        let item = || SyncItem { rel: "sub/f.txt".into(), direction: Direction::Ltr, size: None };
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        let err = sync_op(&mut ctx, None, left.clone(), None, right.clone(), vec![item()])
            .unwrap_err();
        assert!(err.contains("escapes"), "{err}");
        assert!(!outside.join("f.txt").exists());

        // A link that stays inside the folder still works.
        std::fs::remove_file(right.join("sub")).unwrap();
        std::fs::create_dir_all(right.join("real")).unwrap();
        std::os::unix::fs::symlink("real", right.join("sub")).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        sync_op(&mut ctx, None, left.clone(), None, right.clone(), vec![item()]).unwrap();
        assert_eq!(std::fs::read(right.join("real/f.txt")).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(&work);
    }

    /// Against a real sshd (`YC_TEST_SSHD_PORT`, `YC_TEST_SSH_KEY`): the same
    /// guard on the remote side of a sync, and on a download into a local link.
    #[test]
    fn remote_sync_does_not_follow_parent_links_out_of_the_folder() {
        use crate::fs::sftp::{connect_confirming_key, SshAuth};
        let (Ok(port), Ok(key_path)) = (
            std::env::var("YC_TEST_SSHD_PORT"),
            std::env::var("YC_TEST_SSH_KEY"),
        ) else {
            eprintln!("skipped: set YC_TEST_SSHD_PORT and YC_TEST_SSH_KEY");
            return;
        };
        let work = tmp("remote_parent").canonicalize().unwrap();
        let auth = SshAuth::Key {
            path: Some(PathBuf::from(key_path)),
            passphrase: None,
        };
        let user = std::env::var("USER").unwrap();
        let conn = connect_confirming_key(
            &work.join("known_hosts"),
            "127.0.0.1",
            port.parse().unwrap(),
            &user,
            &auth,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let (left, right, outside) = (work.join("l"), work.join("r"), work.join("outside"));
        std::fs::create_dir_all(left.join("sub")).unwrap();
        std::fs::create_dir_all(&right).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(left.join("sub/f.txt"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, right.join("sub")).unwrap();
        let item = |direction| SyncItem { rel: "sub/f.txt".into(), direction, size: None };

        // Upload: the destination is the remote side.
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        let err = sync_op(
            &mut ctx,
            None,
            left.clone(),
            Some(conn.clone()),
            right.clone(),
            vec![item(Direction::Ltr)],
        )
        .unwrap_err();
        assert!(err.contains("escapes"), "{err}");
        assert!(!outside.join("f.txt").exists());

        // Download: the destination is the local side.
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        let err = sync_op(
            &mut ctx,
            Some(conn.clone()),
            left.clone(),
            None,
            right.clone(),
            vec![item(Direction::Ltr)],
        );
        // Ltr with a remote left: the source is remote, the local right is written.
        let err = err.unwrap_err();
        assert!(err.contains("escapes"), "{err}");
        assert!(!outside.join("f.txt").exists());

        // A plain folder on the remote side still works.
        std::fs::remove_file(right.join("sub")).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        sync_op(&mut ctx, None, left.clone(), Some(conn), right.clone(), vec![item(Direction::Ltr)])
            .unwrap();
        assert_eq!(std::fs::read(right.join("sub/f.txt")).unwrap(), b"x");
        let _ = std::fs::remove_dir_all(&work);
    }
}
