//! Guards for writing untrusted trees (archive entries, SFTP downloads)
//! under a local root: a symlink in the tree must stay inside the root,
//! and no file or directory may be created through a symlink that
//! leaves it.

use std::path::{Component, Path, PathBuf};

fn escape_err(path: &Path) -> String {
    format!("{}: path escapes the destination folder", path.display())
}

/// Where a symlink at `link` pointing to `target` would land, resolved
/// lexically from `root`. `None` when the target is absolute, or climbs
/// above `root` via `..` — the link must not be created in that case.
pub fn link_inside(root: &Path, link: &Path, target: &str) -> Option<PathBuf> {
    let parent = link.parent()?.strip_prefix(root).ok()?;
    let target = Path::new(target);
    let mut cur = root.to_path_buf();
    for c in parent.components().chain(target.components()) {
        match c {
            Component::Normal(p) => cur.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                if cur == root {
                    return None;
                }
                cur.pop();
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(cur)
}

/// Create `link -> target` only when the target stays inside `root`.
pub fn symlink_inside(root: &Path, link: &Path, target: &str) -> Result<(), String> {
    if link_inside(root, link, target).is_none() {
        return Err(format!(
            "{}: symlink to {target} escapes the destination folder",
            link.display()
        ));
    }
    std::os::unix::fs::symlink(target, link).map_err(|e| format!("{}: {e}", link.display()))
}

/// Create `dir` (and missing parents) under the canonical `root`, refusing
/// to go through any symlink that resolves outside it. Returns the
/// canonical path of `dir`.
pub fn create_dir_inside(root: &Path, dir: &Path) -> Result<PathBuf, String> {
    // The deepest ancestor that already exists is where symlinks could
    // redirect us; the pieces below it are created fresh.
    let mut existing = dir;
    while std::fs::symlink_metadata(existing).is_err() {
        existing = existing.parent().ok_or_else(|| escape_err(dir))?;
    }
    let canon = existing
        .canonicalize()
        .map_err(|e| format!("{}: {e}", existing.display()))?;
    if !canon.starts_with(root) {
        return Err(escape_err(dir));
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let canon = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    if !canon.starts_with(root) {
        return Err(escape_err(dir));
    }
    Ok(canon)
}

/// `File::create` that refuses to write through a symlink at `path`.
pub fn create_file_nofollow(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// A directory-entry name as a single path piece: no separators, not
/// `.`/`..`, not empty. Guards `dir.join(name)` against names a remote
/// server made up.
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yc_sp_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn link_targets_inside_root_resolve() {
        let root = Path::new("/r");
        assert_eq!(
            link_inside(root, Path::new("/r/a/l"), "../b"),
            Some(PathBuf::from("/r/b"))
        );
        assert_eq!(
            link_inside(root, Path::new("/r/l"), "."),
            Some(PathBuf::from("/r"))
        );
        assert_eq!(
            link_inside(root, Path::new("/r/l"), "x/./y"),
            Some(PathBuf::from("/r/x/y"))
        );
        assert_eq!(
            link_inside(root, Path::new("/r/a/l"), "b/../../c"),
            Some(PathBuf::from("/r/c"))
        );
    }

    #[test]
    fn link_targets_escaping_root_are_rejected() {
        let root = Path::new("/r");
        assert_eq!(link_inside(root, Path::new("/r/l"), "/etc"), None);
        assert_eq!(link_inside(root, Path::new("/r/l"), ".."), None);
        assert_eq!(link_inside(root, Path::new("/r/a/l"), "../../.."), None);
        assert_eq!(link_inside(root, Path::new("/r/l"), "x/../../y"), None);
        // A link placed outside the root is never acceptable either.
        assert_eq!(link_inside(root, Path::new("/other/l"), "x"), None);
    }

    #[test]
    fn plain_names() {
        assert!(is_plain_name("file.txt"));
        assert!(is_plain_name(".hidden"));
        assert!(!is_plain_name(""));
        assert!(!is_plain_name("."));
        assert!(!is_plain_name(".."));
        assert!(!is_plain_name("a/b"));
        assert!(!is_plain_name("..\\x"));
    }

    #[test]
    fn dirs_are_not_created_through_escaping_links() {
        let work = tmp("dir");
        let outside = work.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let root = work.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let root_canon = root.canonicalize().unwrap();
        std::os::unix::fs::symlink(&outside, root.join("esc")).unwrap();
        std::os::unix::fs::symlink("real", root.join("ok")).unwrap();

        let err = create_dir_inside(&root_canon, &root.join("esc/sub/deep")).unwrap_err();
        assert!(err.contains("escapes"), "{err}");
        assert!(!outside.join("sub").exists(), "nothing created outside");

        let made = create_dir_inside(&root_canon, &root.join("a/b")).unwrap();
        assert_eq!(made, root_canon.join("a/b"));

        // A link that stays inside is fine once its target exists.
        std::fs::create_dir_all(root.join("real")).unwrap();
        let made = create_dir_inside(&root_canon, &root.join("ok/c")).unwrap();
        assert_eq!(made, root_canon.join("real/c"));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn files_are_not_written_through_links() {
        let work = tmp("file");
        let victim = work.join("victim.txt");
        std::fs::write(&victim, b"keep").unwrap();
        let link = work.join("link");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        assert!(create_file_nofollow(&link).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert!(create_file_nofollow(&work.join("plain.txt")).is_ok());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn symlink_inside_refuses_escapes() {
        let work = tmp("ln");
        let root = work.join("root");
        std::fs::create_dir_all(&root).unwrap();
        assert!(symlink_inside(&root, &root.join("bad"), "/etc").is_err());
        assert!(!root.join("bad").is_symlink());
        assert!(symlink_inside(&root, &root.join("good"), "x/y").is_ok());
        assert!(root.join("good").is_symlink());
        let _ = std::fs::remove_dir_all(&work);
    }
}
