//! Guards for writing untrusted trees (archive entries, SFTP downloads)
//! under a local root: a symlink in the tree must stay inside the root,
//! and no file or directory may be created through a symlink that
//! leaves it.

use std::path::{Component, Path, PathBuf};

fn escape_err(path: &Path) -> String {
    format!("{}: path escapes the destination folder", path.display())
}

/// `path` as the OS would reach it: the deepest existing ancestor with
/// every symlink resolved, then the missing tail as given. The tail is
/// created later as plain folders, so text is the truth for it.
fn real_path(path: &Path) -> Option<PathBuf> {
    let mut existing = path;
    let mut tail = Vec::new();
    while std::fs::symlink_metadata(existing).is_err() {
        match existing.components().next_back()? {
            Component::Normal(name) => tail.push(name),
            _ => return None,
        }
        existing = existing.parent()?;
    }
    let mut real = existing.canonicalize().ok()?;
    real.extend(tail.iter().rev());
    Some(real)
}

/// Where a link in the real folder `dir` pointing to `target` lands, if
/// that is inside the canonical `root`. `..` may only lead the target:
/// from a real folder each one is the true parent, but after a name that
/// is itself a link it would climb from wherever that link points. With
/// every link built this way, following names down from a link inside
/// `root` can never leave it.
fn resolve_target(root: &Path, dir: &Path, target: &str) -> Option<PathBuf> {
    if !dir.starts_with(root) {
        return None;
    }
    let mut cur = dir.to_path_buf();
    let mut descended = false;
    for c in Path::new(target).components() {
        match c {
            Component::Normal(p) => {
                cur.push(p);
                descended = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if descended || cur == root {
                    return None;
                }
                cur.pop();
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(cur)
}

/// Where a symlink at `link` pointing to `target` would land, if inside
/// the canonical `root`. The link's folder is resolved for real first, so
/// a link created through another link (a/b -> .., then a/b/esc -> ..) is
/// judged where it actually ends up. `None`: the link must not be made.
pub fn link_inside(root: &Path, link: &Path, target: &str) -> Option<PathBuf> {
    resolve_target(root, &real_path(link.parent()?)?, target)
}

/// Create `link -> target` only when the target stays inside the
/// canonical `root`. The link is made in its real folder, the one
/// that was checked.
pub fn symlink_inside(root: &Path, link: &Path, target: &str) -> Result<(), String> {
    let refuse = || {
        format!(
            "{}: symlink to {target} escapes the destination folder",
            link.display()
        )
    };
    let dir = link.parent().and_then(real_path).ok_or_else(refuse)?;
    let name = link.file_name().ok_or_else(refuse)?;
    resolve_target(root, &dir, target).ok_or_else(refuse)?;
    let at = dir.join(name);
    std::os::unix::fs::symlink(target, &at).map_err(|e| format!("{}: {e}", link.display()))
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

/// Like `create_file_nofollow`, for a destination the user already chose to
/// overwrite: a symlink there is replaced by the new file, not written
/// through.
pub fn replace_file_nofollow(path: &Path) -> Result<std::fs::File, String> {
    remove_if_link(path)?;
    create_file_nofollow(path)
}

/// Take away a symlink at `path`, if that is what is there; anything else
/// stays. For a name about to be written by a tool that follows links.
pub fn remove_if_link(path: &Path) -> Result<(), String> {
    if path.is_symlink() {
        std::fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Delete a local file, link or folder tree. A link is removed itself,
/// never the folder it points to.
pub fn remove_nofollow(path: &Path) -> Result<(), String> {
    if path.is_dir() && !path.is_symlink() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
    .map_err(|e| format!("{}: {e}", path.display()))
}

/// Remove `dir` only if nothing is left in it. `Ok(false)`: something is, and
/// it stays. Ends a folder move, where recursive removal would take along
/// whatever was not moved.
pub fn remove_dir_if_empty(dir: &Path) -> Result<bool, String> {
    match std::fs::remove_dir(dir) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(false),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

/// A name a directory listing may carry: exactly one path piece on Unix —
/// not empty, not `.`/`..`, no `/` or NUL. `dir.join(name)` then stays
/// one level under `dir` (an absolute name would replace `dir` outright).
/// A backslash passes: here it is an ordinary file-name character.
pub fn is_listing_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\0'])
}

/// A directory-entry name as a single path piece on any platform:
/// `is_listing_name`, and no backslash either. Guards `dir.join(name)`
/// against names a remote server made up.
pub fn is_plain_name(name: &str) -> bool {
    is_listing_name(name) && !name.contains('\\')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_nofollow_takes_links_not_their_targets() {
        let dir = tmp("rmnf");
        let real = dir.join("real");
        std::fs::create_dir_all(real.join("sub")).unwrap();
        std::fs::write(real.join("sub/f.txt"), b"keep").unwrap();
        std::os::unix::fs::symlink(&real, dir.join("link")).unwrap();
        remove_nofollow(&dir.join("link")).unwrap();
        assert!(dir.join("link").symlink_metadata().is_err());
        assert_eq!(std::fs::read(real.join("sub/f.txt")).unwrap(), b"keep");
        std::fs::write(dir.join("file"), b"x").unwrap();
        remove_nofollow(&dir.join("file")).unwrap();
        remove_nofollow(&real).unwrap();
        assert!(!real.exists() && !dir.join("file").exists());
        assert!(remove_nofollow(&dir.join("gone")).unwrap_err().contains("gone"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_if_link_takes_only_links() {
        let dir = tmp("rmlink");
        std::fs::write(dir.join("file"), b"keep").unwrap();
        std::fs::create_dir(dir.join("folder")).unwrap();
        std::os::unix::fs::symlink(dir.join("file"), dir.join("link")).unwrap();
        std::os::unix::fs::symlink("nowhere", dir.join("dangling")).unwrap();
        for name in ["link", "dangling", "file", "folder", "absent"] {
            remove_if_link(&dir.join(name)).unwrap();
        }
        assert!(dir.join("link").symlink_metadata().is_err());
        assert!(dir.join("dangling").symlink_metadata().is_err());
        assert_eq!(std::fs::read(dir.join("file")).unwrap(), b"keep");
        assert!(dir.join("folder").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_dir_if_empty_keeps_what_is_left() {
        let dir = tmp("rmempty");
        std::fs::create_dir_all(dir.join("full")).unwrap();
        std::fs::write(dir.join("full/f"), b"x").unwrap();
        std::fs::create_dir(dir.join("empty")).unwrap();
        assert!(!remove_dir_if_empty(&dir.join("full")).unwrap());
        assert!(dir.join("full/f").exists());
        assert!(remove_dir_if_empty(&dir.join("empty")).unwrap());
        assert!(!dir.join("empty").exists());
        assert!(remove_dir_if_empty(&dir.join("absent")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yc_sp_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn link_targets_inside_root_resolve() {
        let root = Path::new("/r");
        let at = |dir: &str, t: &str| resolve_target(root, Path::new(dir), t);
        assert_eq!(at("/r/a", "../b"), Some(PathBuf::from("/r/b")));
        assert_eq!(at("/r", "."), Some(PathBuf::from("/r")));
        assert_eq!(at("/r", "x/./y"), Some(PathBuf::from("/r/x/y")));
        assert_eq!(at("/r/a/b", "./../../c"), Some(PathBuf::from("/r/c")));
    }

    #[test]
    fn link_targets_escaping_root_are_rejected() {
        let root = Path::new("/r");
        let at = |dir: &str, t: &str| resolve_target(root, Path::new(dir), t);
        assert_eq!(at("/r", "/etc"), None);
        assert_eq!(at("/r", ".."), None);
        assert_eq!(at("/r/a", "../../.."), None);
        // A link placed outside the root is never acceptable either.
        assert_eq!(at("/other", "x"), None);
        // ".." after a name could climb from wherever that name links to.
        assert_eq!(at("/r", "x/../y"), None);
        assert_eq!(at("/r/a", "b/../../c"), None);
    }

    #[test]
    fn links_are_judged_in_their_real_folder() {
        let work = tmp("real");
        let root = work.join("root");
        std::fs::create_dir_all(root.join("a")).unwrap();
        let root = root.canonicalize().unwrap();
        // a/b points back at root: a link "inside" a/b really sits in root.
        symlink_inside(&root, &root.join("a/b"), "..").unwrap();
        assert_eq!(link_inside(&root, &root.join("a/b/esc"), ".."), None);
        assert!(symlink_inside(&root, &root.join("a/b/esc"), "..").is_err());
        assert!(!root.join("esc").is_symlink());
        // One level up from a/b's real folder (root) is outside.
        assert!(symlink_inside(&root, &root.join("a/b/ok"), "a").is_ok());
        assert!(root.join("ok").is_symlink(), "made in the real folder");
        // A missing tail under a real folder resolves by text.
        assert_eq!(
            link_inside(&root, &root.join("new/deeper/l"), "../x"),
            Some(root.join("new/x"))
        );
        let _ = std::fs::remove_dir_all(&work);
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
    fn listing_names() {
        assert!(is_listing_name("file.txt"));
        assert!(is_listing_name("back\\slash"), "an ordinary character on Unix");
        assert!(!is_listing_name(""));
        assert!(!is_listing_name("."));
        assert!(!is_listing_name(".."));
        assert!(!is_listing_name("../x"));
        assert!(!is_listing_name("/etc/passwd"));
        assert!(!is_listing_name("a/b"));
        assert!(!is_listing_name("nul\0byte"));
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
        let root = root.canonicalize().unwrap();
        assert!(symlink_inside(&root, &root.join("bad"), "/etc").is_err());
        assert!(!root.join("bad").is_symlink());
        assert!(symlink_inside(&root, &root.join("good"), "x/y").is_ok());
        assert!(root.join("good").is_symlink());
        let _ = std::fs::remove_dir_all(&work);
    }
}
