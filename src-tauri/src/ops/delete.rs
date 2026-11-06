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

/// Put Trash items back where they came from (Finder's Put Back records on
/// macOS, .trashinfo on Linux). A missing original folder is recreated; an
/// item whose original path is taken again stays in the Trash. Every item
/// is tried; failures are reported together at the end.
pub fn restore_op(ctx: &mut OpCtx, items: Vec<PathBuf>) -> Result<usize, String> {
    let trash = crate::platform::trash_dir()?;
    ctx.total_bytes = items.len() as u64;
    let mut restored = 0;
    let mut errors = Vec::new();
    for item in &items {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        ctx.report(&item.to_string_lossy());
        match restore_one(&trash, item) {
            Ok(()) => restored += 1,
            Err(e) => errors.push(e),
        }
        ctx.done_bytes += 1;
    }
    if errors.is_empty() {
        Ok(restored)
    } else {
        Err(errors.join("\n"))
    }
}

fn restore_one(trash: &std::path::Path, item: &std::path::Path) -> Result<(), String> {
    let name = item.file_name().unwrap_or_default().to_string_lossy().into_owned();
    if item.parent() != Some(trash) || item.symlink_metadata().is_err() {
        return Err(format!("{name}: not in the Trash"));
    }
    let dest = crate::platform::trash_origin(item)?;
    if dest.symlink_metadata().is_ok() {
        return Err(format!("{name}: {} already exists", dest.display()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::rename(item, &dest).map_err(|e| format!("{name}: {e}"))?;
    crate::platform::trash_forget(item);
    Ok(())
}

/// Permanently delete everything in the user's Trash folder (the one the
/// toolbar Trash button shows), with progress.
pub fn empty_trash_op(ctx: &mut OpCtx) -> Result<usize, String> {
    let trash = crate::platform::trash_dir()?;
    let items: Vec<PathBuf> = std::fs::read_dir(&trash)
        .map_err(|e| format!("{}: {e}", trash.display()))?
        .flatten()
        .filter(|e| !crate::platform::trash_is_bookkeeping(&e.file_name()))
        .map(|e| e.path())
        .collect();
    let count = delete_op(ctx, items.clone());
    for item in &items {
        if item.symlink_metadata().is_err() {
            crate::platform::trash_forget(item);
        }
    }
    let count = count?;
    crate::platform::trash_after_empty();
    Ok(count)
}

/// Round trip through the real freedesktop home trash, redirected into a
/// scratch folder with XDG_DATA_HOME: trash → restore → trash → empty.
#[cfg(all(test, target_os = "linux"))]
mod linux_trash_tests {
    use super::*;
    use crate::ops::queue::ConflictChoice;

    #[test]
    fn restore_and_empty_round_trip() {
        let base = std::env::temp_dir().join(format!("yc_trash_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let data = base.join("data");
        let work = base.join("work dir");
        std::fs::create_dir_all(&work).unwrap();
        // Only this test reads XDG_DATA_HOME in this crate's test binary.
        unsafe { std::env::set_var("XDG_DATA_HOME", &data) };
        let trash = crate::platform::trash_dir().unwrap();
        assert_eq!(trash, data.join("Trash/files"));

        let file = work.join("a 中.txt");
        std::fs::write(&file, "x").unwrap();
        trash::delete(&file).unwrap();
        assert!(!file.exists());
        let in_trash = trash.join("a 中.txt");
        assert!(in_trash.exists());

        // Restore puts it back (the folder is recreated if it vanished).
        std::fs::remove_dir_all(&work).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::Skip);
        assert_eq!(restore_op(&mut ctx, vec![in_trash.clone()]).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "x");
        assert!(!data.join("Trash/info/a 中.txt.trashinfo").exists());

        // A taken original path keeps the item in the Trash.
        trash::delete(&file).unwrap();
        std::fs::write(&file, "new").unwrap();
        let err = restore_op(&mut ctx, vec![in_trash.clone()]).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert!(in_trash.exists());

        // Empty removes the items and their .trashinfo files.
        std::fs::create_dir_all(work.join("d/e")).unwrap();
        trash::delete(work.join("d")).unwrap();
        assert_eq!(empty_trash_op(&mut ctx).unwrap(), 2);
        assert_eq!(std::fs::read_dir(&trash).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(data.join("Trash/info")).unwrap().count(), 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");

        unsafe { std::env::remove_var("XDG_DATA_HOME") };
        let _ = std::fs::remove_dir_all(&base);
    }
}
