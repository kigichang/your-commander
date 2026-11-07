use super::queue::{cancelled_err, keep_both_path, ConflictChoice, OpCtx};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const CHUNK: usize = 1024 * 1024;

/// Walk sources and sum file sizes so progress has a denominator, honoring
/// the mask and follow-symlinks options.
pub(super) fn total_bytes(ctx: &OpCtx, paths: &[PathBuf]) -> u64 {
    fn walk(ctx: &OpCtx, path: &Path) -> u64 {
        if path.is_symlink() && !(ctx.options.follow_symlinks && path.is_file()) {
            return 0;
        }
        if path.is_dir() {
            std::fs::read_dir(path)
                .map(|rd| rd.flatten().map(|e| walk(ctx, &e.path())).sum())
                .unwrap_or(0)
        } else if ctx.mask_matches(&path.file_name().unwrap_or_default().to_string_lossy()) {
            path.metadata().map(|m| m.len()).unwrap_or(0)
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

    // Fast path for move on the same device: a rename is instant.
    if remove_source && !dest.exists() && same_device(src, dest.parent().unwrap_or(dest)) {
        ctx.report(&src.to_string_lossy());
        if std::fs::rename(src, dest).is_ok() {
            return Ok(());
        }
    }

    if src.is_dir() && !src.is_symlink() {
        copy_dir(ctx, src, dest)?;
        if remove_source {
            std::fs::remove_dir_all(src).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    } else {
        copy_file(ctx, src, dest)?;
        if remove_source {
            std::fs::remove_file(src).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

fn copy_dir(ctx: &mut OpCtx, src: &Path, dest: &Path) -> Result<(), String> {
    if dest.exists() && !dest.is_dir() {
        match ctx.ask_conflict(src, dest) {
            ConflictChoice::Overwrite => {
                std::fs::remove_file(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
            }
            ConflictChoice::Skip => return Ok(()),
            ConflictChoice::KeepBoth => {
                return copy_dir(ctx, src, &keep_both_path(dest));
            }
            ConflictChoice::CancelOp => return Err(cancelled_err()),
            _ => return Ok(()),
        }
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?;
    for entry in entries.flatten() {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let child_src = entry.path();
        let child_dest = dest.join(entry.file_name());
        if child_src.is_dir() && !child_src.is_symlink() {
            copy_dir(ctx, &child_src, &child_dest)?;
        } else {
            copy_file(ctx, &child_src, &child_dest)?;
        }
    }
    Ok(())
}

pub(crate) fn copy_file(ctx: &mut OpCtx, src: &Path, dest: &Path) -> Result<(), String> {
    let name = src.file_name().unwrap_or_default().to_string_lossy().to_string();
    if !ctx.mask_matches(&name) {
        return Ok(());
    }
    let mut dest = dest.to_path_buf();
    if dest.exists() {
        match ctx.ask_conflict(src, &dest) {
            ConflictChoice::Overwrite => {}
            ConflictChoice::Skip => {
                ctx.done_bytes += src.metadata().map(|m| m.len()).unwrap_or(0);
                ctx.report(&src.to_string_lossy());
                return Ok(());
            }
            ConflictChoice::KeepBoth => dest = keep_both_path(&dest),
            ConflictChoice::CancelOp => return Err(cancelled_err()),
            _ => return Ok(()),
        }
    }
    ctx.report(&src.to_string_lossy());

    // Recreate links by default; with follow-symlinks on, file links are
    // streamed like regular files (directory links always stay links).
    if src.is_symlink() && !(ctx.options.follow_symlinks && src.is_file()) {
        let target = std::fs::read_link(src).map_err(|e| format!("{}: {e}", src.display()))?;
        if dest.exists() {
            let _ = std::fs::remove_file(&dest);
        }
        std::os::unix::fs::symlink(&target, &dest)
            .map_err(|e| format!("{}: {e}", dest.display()))?;
        return Ok(());
    }

    let mut reader =
        std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut writer =
        std::fs::File::create(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
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
    Ok(())
}

/// Byte-compare source and destination after a copy.
fn verify_copy(ctx: &OpCtx, src: &Path, dest: &Path) -> Result<(), String> {
    let fail = || format!("Verify failed: {}", dest.display());
    let mut a = std::fs::File::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
    let mut b = std::fs::File::open(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut ba = vec![0u8; CHUNK];
    let mut bb = vec![0u8; CHUNK];
    loop {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let na = a.read(&mut ba).map_err(|e| e.to_string())?;
        let mut filled = 0;
        while filled < na {
            let n = b.read(&mut bb[filled..na]).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err(fail());
            }
            filled += n;
        }
        if ba[..na] != bb[..na] {
            return Err(fail());
        }
        if na == 0 {
            // Source exhausted: destination must be too.
            let n = b.read(&mut bb[..1]).map_err(|e| e.to_string())?;
            return if n == 0 { Ok(()) } else { Err(fail()) };
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
    if dest.exists() {
        return Err(format!("\"{new_name}\" already exists"));
    }
    std::fs::rename(src, &dest).map_err(|e| e.to_string())
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
}
