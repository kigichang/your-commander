use super::queue::{cancelled_err, keep_both_path, ConflictChoice, OpCtx};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const CHUNK: usize = 1024 * 1024;

/// Walk sources and sum file sizes so progress has a denominator, honoring
/// the mask and follow-symlinks options.
pub(super) fn total_bytes(ctx: &OpCtx, paths: &[PathBuf]) -> u64 {
    // One lstat per entry; a link is stat'ed again only when followed.
    fn walk(ctx: &OpCtx, path: &Path) -> u64 {
        let Ok(meta) = path.symlink_metadata() else {
            return 0;
        };
        let masked = || ctx.mask_matches(&path.file_name().unwrap_or_default().to_string_lossy());
        if meta.is_symlink() {
            // Only file links count, and only when followed.
            return match path.metadata() {
                Ok(m) if ctx.options.follow_symlinks && m.is_file() && masked() => m.len(),
                _ => 0,
            };
        }
        if meta.is_dir() {
            std::fs::read_dir(path)
                .map(|rd| rd.flatten().map(|e| walk(ctx, &e.path())).sum())
                .unwrap_or(0)
        } else if masked() {
            meta.len()
        } else {
            0
        }
    }
    paths.iter().map(|p| walk(ctx, p)).sum()
}

/// Copy (or move) `sources` into directory `dest_dir`. Returns the number of
/// top-level items processed.
pub fn copy_op(
    ctx: &mut OpCtx,
    sources: Vec<PathBuf>,
    dest_dir: PathBuf,
    remove_source: bool,
) -> Result<usize, String> {
    // A move within the same folder has nowhere to go. Refuse it up front,
    // before anything is touched, rather than rename the items.
    if remove_source
        && let Some(src) = sources.iter().find(|src| is_inside(src, &dest_dir))
    {
        return Err(format!(
            "Cannot move \"{}\": it is already in this folder",
            src.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    ctx.total_bytes = total_bytes(ctx, &sources);
    let mut copied = 0usize;
    for src in &sources {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let Some(name) = src.file_name() else { continue };
        let dest = dest_dir.join(name);
        if dest == *src {
            // Same folder: make an automatic copy under a new name.
            let alt = keep_both_path(&dest);
            copy_item(ctx, src, &alt, remove_source)?;
        } else {
            if src.is_dir() && dest.starts_with(src) {
                return Err("Cannot copy a folder into itself".to_string());
            }
            copy_item(ctx, src, &dest, remove_source)?;
        }
        copied += 1;
    }
    Ok(copied)
}

/// Is `dir` the folder that holds `path`? Compared after resolving
/// symlinks, so a folder reached through a link counts too.
fn is_inside(path: &Path, dir: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    match (parent.canonicalize(), dir.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => parent == dir,
    }
}

fn copy_item(ctx: &mut OpCtx, src: &Path, dest: &Path, remove_source: bool) -> Result<(), String> {
    if ctx.cancelled() {
        return Err(cancelled_err());
    }

    // Fast path for move on the same device: a rename is instant. Not over
    // a dangling link either: rename would replace it without asking. Nor
    // with a mask: rename would carry along what the mask leaves out.
    if remove_source
        && !ctx.has_mask()
        && !dest.exists()
        && !dest.is_symlink()
        && same_device(src, dest.parent().unwrap_or(dest))
    {
        ctx.report(&src.to_string_lossy());
        if std::fs::rename(src, dest).is_ok() {
            return Ok(());
        }
    }

    if src.is_dir() && !src.is_symlink() {
        copy_dir(ctx, src, dest, remove_source)?;
    } else {
        transfer_file(ctx, src, dest, remove_source)?;
    }
    Ok(())
}

/// `copy_file`, and on a move the source goes once its copy has arrived. A
/// file that did not arrive (conflict skipped, mask) is never removed.
fn transfer_file(
    ctx: &mut OpCtx,
    src: &Path,
    dest: &Path,
    remove_source: bool,
) -> Result<bool, String> {
    let arrived = copy_file(ctx, src, dest)?;
    if arrived && remove_source {
        std::fs::remove_file(src).map_err(|e| format!("{}: {e}", src.display()))?;
    }
    Ok(arrived)
}

/// Copy a folder's contents; on a move, each file goes as it arrives and a
/// folder only once it is empty, so whatever was skipped or masked out stays
/// where it was. Returns whether all of `src` arrived.
fn copy_dir(ctx: &mut OpCtx, src: &Path, dest: &Path, remove_source: bool) -> Result<bool, String> {
    // exists() follows links: a dangling one would raise no conflict, and
    // create_dir_all below would then fail on the name it holds.
    if (dest.exists() || dest.is_symlink()) && !dest.is_dir() {
        match ctx.ask_conflict(src, dest) {
            ConflictChoice::Overwrite => {
                std::fs::remove_file(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            }
            ConflictChoice::Skip => return Ok(false),
            ConflictChoice::KeepBoth => {
                return copy_dir(ctx, src, &keep_both_path(dest), remove_source);
            }
            ConflictChoice::CancelOp => return Err(cancelled_err()),
            _ => return Ok(false),
        }
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    // Listed up front: readdir may skip entries while files are being removed.
    let entries: Vec<_> = std::fs::read_dir(src)
        .map_err(|e| format!("{}: {e}", src.display()))?
        .flatten()
        .collect();
    let mut all_arrived = true;
    for entry in entries {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let child_src = entry.path();
        let child_dest = dest.join(entry.file_name());
        // From readdir: a link reports as a link, never as a directory.
        all_arrived &= if entry.file_type().is_ok_and(|t| t.is_dir()) {
            copy_dir(ctx, &child_src, &child_dest, remove_source)?
        } else {
            transfer_file(ctx, &child_src, &child_dest, remove_source)?
        };
    }
    if remove_source && all_arrived {
        return super::safe_path::remove_dir_if_empty(src);
    }
    Ok(all_arrived)
}

/// Copy one file, settling any conflict at `dest` with the user. Returns
/// whether it arrived: `false` means it was deliberately left behind (the
/// mask, a skipped conflict), so a move must keep the source.
pub(crate) fn copy_file(ctx: &mut OpCtx, src: &Path, dest: &Path) -> Result<bool, String> {
    let name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
    if !ctx.mask_matches(&name) {
        return Ok(false);
    }
    let mut dest = dest.to_path_buf();
    // exists() follows links: a dangling one would raise no conflict, and
    // the write below would then create whatever file it points at.
    if dest.exists() || dest.is_symlink() {
        match ctx.ask_conflict(src, &dest) {
            ConflictChoice::Overwrite => {}
            ConflictChoice::Skip => {
                ctx.done_bytes += src.metadata().map(|m| m.len()).unwrap_or(0);
                ctx.report(&src.to_string_lossy());
                return Ok(false);
            }
            ConflictChoice::KeepBoth => dest = keep_both_path(&dest),
            ConflictChoice::CancelOp => return Err(cancelled_err()),
            _ => return Ok(false),
        }
    }
    ctx.report(&src.to_string_lossy());

    // Recreate links by default; with follow-symlinks on, file links are
    // streamed like regular files (directory links always stay links).
    if src.is_symlink() && !(ctx.options.follow_symlinks && src.is_file()) {
        let target = std::fs::read_link(src).map_err(|e| format!("{}: {e}", src.display()))?;
        if dest.exists() || dest.is_symlink() {
            let _ = std::fs::remove_file(&dest);
        }
        std::os::unix::fs::symlink(&target, &dest)
            .map_err(|e| format!("{}: {e}", dest.display()))?;
        return Ok(true);
    }

    let mut reader =
        std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    // Overwrite replaces a link at `dest` rather than writing into the file
    // it points at; one that appears meanwhile fails the open.
    let mut writer = super::safe_path::replace_file_nofollow(&dest)?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        if ctx.cancelled() {
            drop(writer);
            let _ = std::fs::remove_file(&dest);
            return Err(cancelled_err());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("{}: {e}", src.display()))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .map_err(|e| format!("{}: {e}", dest.display()))?;
        ctx.done_bytes += n as u64;
        ctx.report(&src.to_string_lossy());
    }
    // Preserve permissions, mtime, and xattrs (Finder tags) when asked.
    if ctx.options.keep_meta {
        if let Ok(meta) = src.metadata() {
            let _ = std::fs::set_permissions(&dest, meta.permissions());
            if let Ok(mtime) = meta.modified() {
                let _ = writer.set_modified(mtime);
            }
        }
        copy_xattrs(src, &dest);
    }
    drop(writer);
    if ctx.options.verify {
        verify_copy(ctx, src, &dest)?;
    }
    Ok(true)
}

/// Byte-compare source and destination after a copy.
fn verify_copy(ctx: &OpCtx, src: &Path, dest: &Path) -> Result<(), String> {
    if same_contents(src, dest, &|| ctx.cancelled())? {
        Ok(())
    } else {
        Err(format!("Verify failed: {}", dest.display()))
    }
}

/// Byte-for-byte comparison of two local files (copy verification, sync's
/// "compare by content"). `cancelled` is polled per chunk; a cancel is the
/// usual cancel error.
pub(crate) fn same_contents(
    a: &Path,
    b: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<bool, String> {
    let mut fa = std::fs::File::open(a).map_err(|e| format!("{}: {e}", a.display()))?;
    let mut fb = std::fs::File::open(b).map_err(|e| format!("{}: {e}", b.display()))?;
    let mut ba = vec![0u8; CHUNK];
    let mut bb = vec![0u8; CHUNK];
    loop {
        if cancelled() {
            return Err(cancelled_err());
        }
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
            // `a` exhausted: `b` must be too.
            return Ok(fb.read(&mut bb[..1]).map_err(|e| e.to_string())? == 0);
        }
    }
}

/// Best-effort copy of extended attributes (Finder tags, quarantine, …).
/// Attributes the destination refuses (e.g. anything outside `user.*` for
/// unprivileged processes on Linux) are silently skipped.
fn copy_xattrs(src: &Path, dest: &Path) {
    let Ok(names) = xattr::list(src) else {
        return;
    };
    for name in names {
        if let Ok(Some(value)) = xattr::get(src, &name) {
            let _ = xattr::set(dest, &name, &value);
        }
    }
}

fn same_device(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.metadata(), b.metadata()) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev(),
        _ => false,
    }
}

/// Rename a single entry in place.
pub fn rename(src: &Path, new_name: &str) -> Result<(), String> {
    let Some(parent) = src.parent() else {
        return Err("no parent".to_string());
    };
    let dest = parent.join(new_name);
    // exists() follows links: rename would replace a dangling one silently.
    if dest.exists() || dest.is_symlink() {
        return Err(format!("\"{new_name}\" already exists"));
    }
    std::fs::rename(src, &dest).map_err(|e| e.to_string())
}

/// One entry of a batch rename: `from` goes to `to` by way of `temp`.
pub struct RenameStep {
    pub from: PathBuf,
    pub temp: PathBuf,
    pub to: PathBuf,
}

/// Rename every `from` to its `to` in two phases (all to temp names, then
/// all to final names) so swaps and shifts (A→B, B→A) cannot trample each
/// other. If any rename fails, the ones already done are undone in reverse
/// so the folder ends as it began. Reverse order is what keeps each undo
/// from landing on an occupied name, so the first undo that fails stops
/// the rollback (going on could overwrite the entry it left in place) and
/// the error lists where everything not undone now sits.
pub fn rename_in_two_phases(
    plan: &[RenameStep],
    mut rename: impl FnMut(&Path, &Path) -> Result<(), String>,
) -> Result<(), String> {
    // Every rename done so far, in order, to be reversed on failure.
    let mut done: Vec<(&Path, &Path)> = Vec::with_capacity(plan.len() * 2);
    let phases = plan
        .iter()
        .map(|s| (s.from.as_path(), s.temp.as_path()))
        .chain(plan.iter().map(|s| (s.temp.as_path(), s.to.as_path())));
    for (from, to) in phases {
        if let Err(err) = rename(from, to) {
            while let Some(&(back_to, now_at)) = done.last() {
                if let Err(e) = rename(now_at, back_to) {
                    // Where each entry not back home sits now, by its
                    // original name.
                    let left: Vec<String> = plan
                        .iter()
                        .filter_map(|step| {
                            let start = step.from.as_path();
                            let now = done.iter().fold(start, |at, &(f, t)| if at == f { t } else { at });
                            (now != start)
                                .then(|| format!("{} (was {})", now.display(), start.display()))
                        })
                        .collect();
                    return Err(format!(
                        "{err}; undo failed too ({e}), left as: {}",
                        left.join(", ")
                    ));
                }
                done.pop();
            }
            return Err(format!("{err}; nothing was renamed"));
        }
        done.push((from, to));
    }
    Ok(())
}

/// A name typed for a new or renamed entry must stay a single entry in
/// its folder: not empty, not `.`/`..`, no `/` (which would reach another
/// folder) and no NUL.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        return Err(format!("Invalid name: \"{name}\""));
    }
    Ok(())
}

/// Create a folder inside `dir`.
pub fn new_folder(dir: &Path, name: &str) -> Result<(), String> {
    let dest = dir.join(name);
    if dest.exists() {
        return Err(format!("\"{name}\" already exists"));
    }
    std::fs::create_dir(&dest).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::queue::OpCtx;

    #[test]
    fn same_contents_compares_bytes_and_length() {
        let dir = std::env::temp_dir().join(format!("yc_tr_same_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let big: Vec<u8> = (0..CHUNK * 2 + 7).map(|i| (i % 253) as u8).collect();
        let write = |name: &str, data: &[u8]| {
            let p = dir.join(name);
            std::fs::write(&p, data).unwrap();
            p
        };
        let a = write("a", &big);
        let same = write("same", &big);
        let mut flipped = big.clone();
        *flipped.last_mut().unwrap() ^= 1;
        let differs = write("differs", &flipped);
        let longer = write("longer", &[&big[..], b"!"].concat());
        let never = || false;
        assert!(same_contents(&a, &same, &never).unwrap());
        assert!(!same_contents(&a, &differs, &never).unwrap());
        assert!(!same_contents(&a, &longer, &never).unwrap());
        assert!(!same_contents(&longer, &a, &never).unwrap());
        assert_eq!(same_contents(&a, &same, &|| true).unwrap_err(), cancelled_err());
        assert!(same_contents(&a, &dir.join("missing"), &never).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_ctx() -> OpCtx {
        OpCtx::for_test(ConflictChoice::CancelOp)
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yc_tx_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copy_then_move_directory_tree() {
        let src_root = tmp("src");
        let dest_root = tmp("dest");
        let dir = src_root.join("proj");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("sub/b.txt"), b"world").unwrap();

        let mut ctx = test_ctx();
        copy_op(&mut ctx, vec![dir.clone()], dest_root.clone(), false).unwrap();
        assert_eq!(
            std::fs::read(dest_root.join("proj/sub/b.txt")).unwrap(),
            b"world"
        );
        assert!(dir.exists(), "copy must keep the source");

        // Move it under a new name area: move the copied tree back as rename-path.
        let move_dest = tmp("move_dest");
        let mut ctx2 = test_ctx();
        copy_op(&mut ctx2, vec![dest_root.join("proj")], move_dest.clone(), true).unwrap();
        assert!(move_dest.join("proj/a.txt").exists());
        assert!(!dest_root.join("proj").exists(), "move must remove source");

        for d in [&src_root, &dest_root, &move_dest] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn move_within_the_same_folder_is_refused() {
        let root = tmp("same_dir");
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        std::fs::write(root.join("b.txt"), b"b").unwrap();
        let other = tmp("same_dir_other");
        std::fs::write(other.join("c.txt"), b"c").unwrap();

        // One source already in the destination aborts the whole move,
        // before the others are touched.
        let mut ctx = test_ctx();
        let err = copy_op(
            &mut ctx,
            vec![other.join("c.txt"), root.join("a.txt")],
            root.clone(),
            true,
        )
        .unwrap_err();
        assert!(err.contains("a.txt"), "names the offending item: {err}");
        assert!(other.join("c.txt").exists(), "nothing was moved");
        let mut names: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "b.txt"], "nothing renamed or added");

        // The same folder reached through a symlink is still the same folder.
        let link = std::env::temp_dir().join(format!("yc_tx_{}_same_dir_ln", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let mut ctx = test_ctx();
        assert!(copy_op(&mut ctx, vec![root.join("a.txt")], link.clone(), true).is_err());
        assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"a");

        // A copy into the same folder still duplicates.
        let mut ctx = test_ctx();
        copy_op(&mut ctx, vec![root.join("a.txt")], root.clone(), false).unwrap();
        assert!(root.join("a 2.txt").exists());

        let _ = std::fs::remove_file(&link);
        for d in [&root, &other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn copy_honors_file_mask() {
        let src_root = tmp("m_src");
        let dest_root = tmp("m_dest");
        let dir = src_root.join("mix");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep.jpg"), b"jpg").unwrap();
        std::fs::write(dir.join("drop.txt"), b"txt").unwrap();

        let mut ctx = test_ctx();
        ctx.set_options(crate::ops::queue::CopyOptions {
            mask: Some("*.jpg".into()),
            ..Default::default()
        });
        copy_op(&mut ctx, vec![dir], dest_root.clone(), false).unwrap();
        assert!(dest_root.join("mix/keep.jpg").exists());
        assert!(!dest_root.join("mix/drop.txt").exists());

        for d in [&src_root, &dest_root] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    /// A swap plus a shift in one folder, as the dialog would plan it.
    fn swap_plan(dir: &Path) -> Vec<RenameStep> {
        for (n, body) in [("a", "A"), ("b", "B"), ("c", "C")] {
            std::fs::write(dir.join(n), body).unwrap();
        }
        [("a", "b"), ("b", "a"), ("c", "d")]
            .iter()
            .enumerate()
            .map(|(i, (from, to))| RenameStep {
                from: dir.join(from),
                temp: dir.join(format!(".tmp{i}")),
                to: dir.join(to),
            })
            .collect()
    }

    fn snapshot(dir: &Path) -> Vec<(String, String)> {
        let mut v: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read_to_string(e.path()).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    }

    fn pair(n: &str, b: &str) -> (String, String) {
        (n.to_string(), b.to_string())
    }

    #[test]
    fn two_phase_rename_swaps_and_shifts() {
        let dir = tmp("rn_ok");
        let plan = swap_plan(&dir);
        rename_in_two_phases(&plan, |f, t| std::fs::rename(f, t).map_err(|e| e.to_string()))
            .unwrap();
        assert_eq!(snapshot(&dir), vec![pair("a", "B"), pair("b", "A"), pair("d", "C")]);
    }

    #[test]
    fn failure_in_either_phase_restores_the_folder() {
        // Six renames in all (3 per phase); fail each one in turn.
        for fail_at in 0..6 {
            let dir = tmp(&format!("rn_fail{fail_at}"));
            let plan = swap_plan(&dir);
            let before = snapshot(&dir);
            let mut n = 0;
            let err = rename_in_two_phases(&plan, |f, t| {
                n += 1;
                if n == fail_at + 1 {
                    return Err("disk said no".into());
                }
                std::fs::rename(f, t).map_err(|e| e.to_string())
            })
            .unwrap_err();
            assert!(err.starts_with("disk said no; nothing was renamed"), "{err}");
            assert_eq!(snapshot(&dir), before, "failing rename #{fail_at}");
        }
    }

    #[test]
    fn failed_undo_names_what_is_left() {
        let dir = tmp("rn_stuck");
        let plan = swap_plan(&dir);
        let mut n = 0;
        // Fail the 5th rename, and the first undo after it.
        let err = rename_in_two_phases(&plan, |f, t| {
            n += 1;
            if n == 5 || n == 6 {
                return Err("gone".into());
            }
            std::fs::rename(f, t).map_err(|e| e.to_string())
        })
        .unwrap_err();
        assert!(err.contains("undo failed too"), "{err}");
        // Undoing the 4th rename (.tmp0 -> b) failed: each original entry
        // is reported once, where it now sits.
        let at = |n: &str| dir.join(n).display().to_string();
        for (now, was) in [("b", "a"), (".tmp1", "b"), (".tmp2", "c")] {
            assert!(err.contains(&format!("{} (was {})", at(now), at(was))), "{err}");
        }
        assert!(!err.contains(".tmp0"), "{err}");
        // Nothing was overwritten: every original body still exists.
        let mut bodies: Vec<String> = snapshot(&dir).into_iter().map(|(_, b)| b).collect();
        bodies.sort();
        assert_eq!(bodies, ["A", "B", "C"]);
    }

    #[test]
    fn names_must_stay_in_their_folder() {
        for bad in ["", ".", "..", "../x", "a/b", "/abs", "nul\0x"] {
            assert!(check_name(bad).is_err(), "{bad:?} accepted");
        }
        for ok in ["a", ".hidden", "..x", "x..", "a b", "back\\slash"] {
            assert!(check_name(ok).is_ok(), "{ok:?} refused");
        }
    }

    #[test]
    fn copy_recreates_symlinks_and_delete_keeps_target() {
        let src_root = tmp("ln_src");
        let dest_root = tmp("ln_dest");
        let dir = src_root.join("tree");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("real.txt"), b"data").unwrap();
        std::os::unix::fs::symlink("real.txt", dir.join("link.txt")).unwrap();
        std::os::unix::fs::symlink("/nonexistent/target", dir.join("broken")).unwrap();

        let mut ctx = test_ctx();
        copy_op(&mut ctx, vec![dir.clone()], dest_root.clone(), false).unwrap();
        let copied = dest_root.join("tree/link.txt");
        assert!(copied.is_symlink(), "link must stay a link");
        assert_eq!(std::fs::read_link(&copied).unwrap(), PathBuf::from("real.txt"));
        assert_eq!(std::fs::read(&copied).unwrap(), b"data");
        assert!(dest_root.join("tree/broken").is_symlink(), "broken links copy too");

        let mut ctx2 = test_ctx();
        crate::ops::delete::delete_op(&mut ctx2, vec![dir.join("link.txt")]).unwrap();
        assert!(!dir.join("link.txt").is_symlink(), "link removed");
        assert!(dir.join("real.txt").exists(), "target untouched");

        for d in [&src_root, &dest_root] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn copy_skips_conflicts_with_skip_all_policy() {
        let src_root = tmp("c_src");
        let dest_root = tmp("c_dest");
        std::fs::write(src_root.join("f.txt"), b"new").unwrap();
        std::fs::write(dest_root.join("f.txt"), b"old").unwrap();

        let mut ctx = test_ctx();
        ctx.policy = Some(crate::ops::queue::ConflictChoice::SkipAll);
        copy_op(&mut ctx, vec![src_root.join("f.txt")], dest_root.clone(), false).unwrap();
        assert_eq!(std::fs::read(dest_root.join("f.txt")).unwrap(), b"old");

        for d in [&src_root, &dest_root] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn folder_copied_onto_a_dangling_link_raises_a_conflict() {
        let work = tmp("dir_dangling");
        let (src, dest) = (work.join("src"), work.join("dest"));
        std::fs::create_dir_all(src.join("proj")).unwrap();
        std::fs::write(src.join("proj/a.txt"), b"new").unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        let target = work.join("missing");
        let link = dest.join("proj");
        let reset = || {
            let _ = std::fs::remove_dir_all(&dest);
            std::fs::create_dir_all(&dest).unwrap();
            std::os::unix::fs::symlink(&target, &link).unwrap();
        };
        let copy = |choice| {
            let mut ctx = OpCtx::for_test(choice);
            copy_op(&mut ctx, vec![src.join("proj")], dest.clone(), false)
        };

        reset();
        copy(ConflictChoice::Skip).unwrap();
        assert!(link.is_symlink(), "skip leaves the link");

        reset();
        copy(ConflictChoice::KeepBoth).unwrap();
        assert!(link.is_symlink(), "keep both leaves the link");
        assert_eq!(std::fs::read(dest.join("proj 2/a.txt")).unwrap(), b"new");

        reset();
        copy(ConflictChoice::Overwrite).unwrap();
        assert!(!link.is_symlink(), "overwrite replaces the link");
        assert_eq!(std::fs::read(link.join("a.txt")).unwrap(), b"new");
        assert!(!target.exists(), "nothing was created at the link's target");

        // A link to a real folder is still a folder to merge into.
        let real = work.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::create_dir_all(&dest).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        copy(ConflictChoice::CancelOp).unwrap();
        assert_eq!(std::fs::read(real.join("a.txt")).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn dangling_link_at_dest_is_a_conflict_not_a_way_out() {
        let work = tmp("dangle");
        let src = work.join("src");
        let dest = work.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("a.txt"), b"new").unwrap();
        let target = work.join("planted.txt");
        std::os::unix::fs::symlink(&target, dest.join("a.txt")).unwrap();

        // Cancel at the prompt: the prompt was raised, nothing was written.
        let err = copy_op(&mut test_ctx(), vec![src.join("a.txt")], dest.clone(), false)
            .unwrap_err();
        assert_eq!(err, cancelled_err());
        assert!(!target.exists(), "no file created at the link's target");

        // Overwrite replaces the link itself.
        let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
        copy_op(&mut ctx, vec![src.join("a.txt")], dest.clone(), false).unwrap();
        assert!(!target.exists());
        assert!(!dest.join("a.txt").is_symlink());
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"new");

        // A move on the same device asks too, rather than renaming over it.
        std::fs::remove_file(dest.join("a.txt")).unwrap();
        std::os::unix::fs::symlink(&target, dest.join("a.txt")).unwrap();
        let err = copy_op(&mut test_ctx(), vec![src.join("a.txt")], dest.clone(), true)
            .unwrap_err();
        assert_eq!(err, cancelled_err());
        assert!(dest.join("a.txt").is_symlink(), "the link is left alone");
        assert!(src.join("a.txt").exists(), "the source stays");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn overwrite_replaces_a_link_instead_of_writing_through_it() {
        let work = tmp("through");
        let src = work.join("src");
        let dest = work.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("a.txt"), b"new").unwrap();
        let victim = work.join("victim.txt");
        std::fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(&victim, dest.join("a.txt")).unwrap();

        let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
        copy_op(&mut ctx, vec![src.join("a.txt")], dest.clone(), false).unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert!(!dest.join("a.txt").is_symlink());
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"new");

        // A link copied over a dangling link replaces it too.
        std::os::unix::fs::symlink("elsewhere", src.join("l")).unwrap();
        std::os::unix::fs::symlink(work.join("gone2"), dest.join("l")).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
        copy_op(&mut ctx, vec![src.join("l")], dest.clone(), false).unwrap();
        assert_eq!(std::fs::read_link(dest.join("l")).unwrap(), PathBuf::from("elsewhere"));
        let _ = std::fs::remove_dir_all(&work);
    }

    /// A tree with a file, a folder, a link to each, and a broken link.
    fn link_tree(work: &Path) -> PathBuf {
        let src = work.join("src");
        std::fs::create_dir_all(src.join("dir")).unwrap();
        std::fs::write(src.join("f.txt"), b"12345").unwrap();
        std::fs::write(src.join("dir/inner.txt"), b"123").unwrap();
        std::os::unix::fs::symlink("f.txt", src.join("file_link")).unwrap();
        std::os::unix::fs::symlink("dir", src.join("dir_link")).unwrap();
        std::os::unix::fs::symlink("missing", src.join("broken")).unwrap();
        src
    }

    #[test]
    fn totals_and_folder_copies_treat_links_as_before() {
        let work = tmp("links");
        let src = link_tree(&work);

        // Links count nothing unless followed; then only file links do.
        let mut ctx = test_ctx();
        assert_eq!(total_bytes(&ctx, std::slice::from_ref(&src)), 8);
        let mut opts = ctx.options.clone();
        opts.follow_symlinks = true;
        ctx.set_options(opts);
        assert_eq!(total_bytes(&ctx, std::slice::from_ref(&src)), 13);
        assert_eq!(total_bytes(&ctx, &[src.join("broken")]), 0);

        // A folder copy recreates links, never walks into a folder link.
        let dest = work.join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        copy_op(&mut test_ctx(), vec![src.clone()], dest.clone(), false).unwrap();
        let out = dest.join("src");
        assert!(out.join("dir_link").is_symlink());
        assert!(out.join("file_link").is_symlink());
        assert!(out.join("broken").is_symlink());
        assert_eq!(std::fs::read(out.join("dir/inner.txt")).unwrap(), b"123");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn move_keeps_a_file_it_skipped() {
        let src_root = tmp("mvskip_src");
        let dest_root = tmp("mvskip_dest");
        std::fs::write(src_root.join("f.txt"), b"new").unwrap();
        std::fs::write(dest_root.join("f.txt"), b"old").unwrap();

        let mut ctx = test_ctx();
        ctx.policy = Some(crate::ops::queue::ConflictChoice::SkipAll);
        copy_op(&mut ctx, vec![src_root.join("f.txt")], dest_root.clone(), true).unwrap();
        assert_eq!(std::fs::read(dest_root.join("f.txt")).unwrap(), b"old");
        assert_eq!(std::fs::read(src_root.join("f.txt")).unwrap(), b"new", "source kept");

        for d in [&src_root, &dest_root] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn folder_move_removes_only_what_arrived() {
        let src_root = tmp("mvdir_src");
        let dest_root = tmp("mvdir_dest");
        let proj = src_root.join("proj");
        std::fs::create_dir_all(proj.join("skips/deep")).unwrap();
        std::fs::create_dir_all(proj.join("moves")).unwrap();
        std::fs::write(proj.join("skips/deep/clash.txt"), b"new").unwrap();
        std::fs::write(proj.join("moves/fine.txt"), b"fine").unwrap();
        std::fs::write(proj.join("top.txt"), b"top").unwrap();
        // The destination already has the folder (no rename) and one clash.
        std::fs::create_dir_all(dest_root.join("proj/skips/deep")).unwrap();
        std::fs::write(dest_root.join("proj/skips/deep/clash.txt"), b"old").unwrap();

        let mut ctx = test_ctx();
        ctx.policy = Some(crate::ops::queue::ConflictChoice::SkipAll);
        copy_op(&mut ctx, vec![proj.clone()], dest_root.clone(), true).unwrap();

        let out = dest_root.join("proj");
        assert_eq!(std::fs::read(out.join("skips/deep/clash.txt")).unwrap(), b"old");
        assert_eq!(std::fs::read(out.join("moves/fine.txt")).unwrap(), b"fine");
        assert_eq!(std::fs::read(out.join("top.txt")).unwrap(), b"top");
        // The skipped file and the folders holding it stay; the rest is gone.
        assert_eq!(std::fs::read(proj.join("skips/deep/clash.txt")).unwrap(), b"new");
        assert!(!proj.join("moves").exists(), "fully moved folder removed");
        assert!(!proj.join("top.txt").exists());

        for d in [&src_root, &dest_root] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn folder_move_with_nothing_skipped_removes_the_folder() {
        let src_root = tmp("mvall_src");
        let dest_root = tmp("mvall_dest");
        let proj = src_root.join("proj");
        std::fs::create_dir_all(proj.join("sub")).unwrap();
        std::fs::write(proj.join("sub/a.txt"), b"a").unwrap();
        std::os::unix::fs::symlink("sub/a.txt", proj.join("link")).unwrap();
        // Forces the copy path: the folder exists on the other side.
        std::fs::create_dir_all(dest_root.join("proj")).unwrap();

        copy_op(&mut test_ctx(), vec![proj.clone()], dest_root.clone(), true).unwrap();
        assert!(!proj.exists());
        assert_eq!(std::fs::read(dest_root.join("proj/sub/a.txt")).unwrap(), b"a");
        assert!(dest_root.join("proj/link").is_symlink());

        for d in [&src_root, &dest_root] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn move_with_a_mask_moves_only_what_matches() {
        // Same device with a free name is rename's fast path, which has to
        // yield to the mask; an existing folder goes the copy way. Both
        // must end the same.
        for existing in [false, true] {
            let src_root = tmp("mvmask_src");
            let dest_root = tmp("mvmask_dest");
            let mix = src_root.join("mix");
            std::fs::create_dir_all(&mix).unwrap();
            std::fs::write(mix.join("keep.jpg"), b"jpg").unwrap();
            std::fs::write(mix.join("other.txt"), b"txt").unwrap();
            std::fs::write(src_root.join("single.txt"), b"one").unwrap();
            if existing {
                std::fs::create_dir_all(dest_root.join("mix")).unwrap();
            }

            let mut ctx = test_ctx();
            ctx.set_options(crate::ops::queue::CopyOptions {
                mask: Some("*.jpg".into()),
                ..Default::default()
            });
            copy_op(
                &mut ctx,
                vec![mix.clone(), src_root.join("single.txt")],
                dest_root.clone(),
                true,
            )
            .unwrap();
            assert!(dest_root.join("mix/keep.jpg").exists(), "existing={existing}");
            assert!(!mix.join("keep.jpg").exists(), "existing={existing}");
            assert_eq!(std::fs::read(mix.join("other.txt")).unwrap(), b"txt", "existing={existing}");
            assert!(!dest_root.join("mix/other.txt").exists(), "existing={existing}");
            assert!(src_root.join("single.txt").exists(), "existing={existing}");
            assert!(!dest_root.join("single.txt").exists(), "existing={existing}");

            for d in [&src_root, &dest_root] {
                let _ = std::fs::remove_dir_all(d);
            }
        }
    }

    #[test]
    fn rename_refuses_a_name_held_by_a_dangling_link() {
        let dir = tmp("rn_dangling");
        std::fs::write(dir.join("a.txt"), b"a").unwrap();
        std::os::unix::fs::symlink("nowhere", dir.join("b.txt")).unwrap();
        assert!(rename(&dir.join("a.txt"), "b.txt").unwrap_err().contains("already exists"));
        assert!(dir.join("b.txt").is_symlink(), "the link is still there");
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"a");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
