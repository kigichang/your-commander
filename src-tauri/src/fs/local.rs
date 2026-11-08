use super::provider::{epoch_millis, FsEntry};
use std::io;
use std::path::{Path, PathBuf};

/// Stateless local directory listing.
pub fn list_dir(path: &Path) -> io::Result<Vec<FsEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        // DirEntry::metadata is an lstat for symlinks: follow the link for
        // type/size/date, falling back to the link's own data when broken.
        let is_symlink = entry
            .file_type()
            .map(|t| t.is_symlink())
            .unwrap_or(false);
        let meta = if is_symlink {
            std::fs::metadata(&path).or_else(|_| entry.metadata())
        } else {
            entry.metadata()
        };
        let (is_dir, size, modified) = match meta {
            // `m` is already the target's metadata (or a broken link's
            // own): asking `path` again would cost one more stat per entry.
            Ok(m) => (m.is_dir(), m.len(), m.modified().ok().map(epoch_millis)),
            Err(_) => (false, 0, None),
        };
        let ext = if is_dir {
            String::new()
        } else {
            path.extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        let is_hidden = name.starts_with('.');
        entries.push(FsEntry {
            name,
            path,
            is_dir,
            is_symlink,
            size,
            modified,
            ext,
            is_hidden,
        });
    }
    Ok(entries)
}

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// A mounted volume shown in the panel "drives" menu (includes SMB mounts).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeInfo {
    pub name: String,
    pub path: PathBuf,
    pub free: u64,
    pub total: u64,
    /// False while the volume doesn't answer statfs in time (still spinning
    /// up, or a dead network mount) — shown but not clickable in the menu.
    pub ready: bool,
    /// The boot volume (its /Volumes entry links to "/") — not ejectable.
    pub is_system: bool,
}

fn statfs(path: &Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c_path.as_ptr(), &mut stat) } != 0 {
        return None;
    }
    let block = stat.f_bsize as u64;
    Some((stat.f_bavail * block, stat.f_blocks * block))
}

/// statfs for every mount at once under one shared deadline: a sleeping
/// disk or a dead network mount can hang the syscall for tens of seconds,
/// which must not stall the drives menu, and three dead mounts must cost
/// one timeout, not three. Stuck threads finish (and are discarded) on
/// their own.
fn statfs_all(paths: &[PathBuf], ms: u64) -> Vec<Option<(u64, u64)>> {
    stat_all_with(paths, ms, statfs)
}

fn stat_all_with(
    paths: &[PathBuf],
    ms: u64,
    stat: fn(&Path) -> Option<(u64, u64)>,
) -> Vec<Option<(u64, u64)>> {
    let (tx, rx) = std::sync::mpsc::channel();
    for (i, path) in paths.iter().enumerate() {
        let (tx, path) = (tx.clone(), path.clone());
        std::thread::spawn(move || {
            let _ = tx.send((i, stat(&path)));
        });
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    let mut out = vec![None; paths.len()];
    for _ in 0..paths.len() {
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(wait) {
            Ok((i, result)) => out[i] = result,
            Err(_) => break,
        }
    }
    out
}

/// Free/total space for each volume; one that doesn't answer in 800 ms
/// stays not ready.
fn fill_space(volumes: &mut [VolumeInfo]) {
    let paths: Vec<PathBuf> = volumes.iter().map(|v| v.path.clone()).collect();
    for (v, stat) in volumes.iter_mut().zip(statfs_all(&paths, 800)) {
        if let Some((free, total)) = stat {
            (v.free, v.total, v.ready) = (free, total, true);
        }
    }
}

#[cfg(target_os = "macos")]
pub fn volumes() -> Vec<VolumeInfo> {
    let mut out = Vec::new();
    // The boot volume itself ("/") is Macintosh HD; /Volumes holds the rest
    // (and a firmlink to the boot volume on most systems).
    if let Ok(read) = std::fs::read_dir("/Volumes") {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            // read_link is an lstat on the /Volumes entry itself — it never
            // touches the (possibly sleeping) volume behind it.
            let is_system = std::fs::read_link(&path)
                .map(|t| t == Path::new("/"))
                .unwrap_or(false);
            out.push(VolumeInfo {
                name,
                path,
                free: 0,
                total: 0,
                ready: false,
                is_system,
            });
        }
    }
    fill_space(&mut out);
    out.sort_by_key(|v| v.name.to_lowercase());
    out
}

#[cfg(target_os = "linux")]
pub fn volumes() -> Vec<VolumeInfo> {
    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let mut found = linux_volumes::parse_mountinfo(&text);
    found.extend(gvfs_mounts());
    let mut out: Vec<VolumeInfo> = found
        .into_iter()
        .map(|(name, path, is_system)| VolumeInfo {
            name,
            path,
            free: 0,
            total: 0,
            ready: false,
            is_system,
        })
        .collect();
    fill_space(&mut out);
    // The root filesystem first, then the rest alphabetical.
    out.sort_by_key(|v| (!v.is_system, v.name.to_lowercase()));
    out
}

/// GVfs FUSE mounts (SMB shares mounted by us or GNOME Files) live under
/// $XDG_RUNTIME_DIR/gvfs, one directory per mount.
#[cfg(target_os = "linux")]
fn gvfs_mounts() -> Vec<(String, PathBuf, bool)> {
    use crate::platform::smb;
    let Ok(read) = std::fs::read_dir(smb::gvfs_dir()) else {
        return vec![];
    };
    read.flatten()
        .map(|entry| {
            let dir = entry.file_name().to_string_lossy().to_string();
            let name = smb::display_name(&dir).unwrap_or_else(|| dir.clone());
            (name, entry.path(), false)
        })
        .collect()
}

/// Pure parsing helpers for the Linux volume list; compiled under test on
/// every platform so the fixtures run with the regular macOS `cargo test`.
#[cfg(any(target_os = "linux", test))]
mod linux_volumes {
    use std::path::PathBuf;

    /// Real filesystems worth showing in the drives menu. Everything else in
    /// mountinfo is kernel plumbing (proc, sysfs, cgroups), snap squashfs
    /// loops, or overlay noise.
    fn listed_fstype(fstype: &str) -> bool {
        matches!(
            fstype,
            "ext2" | "ext3" | "ext4" | "btrfs" | "xfs" | "f2fs" | "vfat" | "exfat" | "ntfs"
                | "ntfs3" | "fuseblk" | "hfsplus" | "iso9660" | "udf" | "cifs" | "smb3"
                | "nfs" | "nfs4"
        ) || fstype.starts_with("fuse.")
    }

    /// Mount points users plug things into; the rest of the tree (snap,
    /// /boot, /var/…) stays out of the menu.
    fn listed_mount_point(mp: &str) -> bool {
        mp == "/"
            || mp.starts_with("/media/")
            || mp.starts_with("/run/media/")
            || (mp.starts_with("/mnt/") && mp.len() > 5)
    }

    /// Decode mountinfo's octal escapes (\040 space, \011 tab, \012 newline,
    /// \134 backslash).
    fn unescape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 3 < bytes.len() {
                let oct = &s[i + 1..i + 4];
                if let Ok(v) = u8::from_str_radix(oct, 8) {
                    out.push(v as char);
                    i += 4;
                    continue;
                }
            }
            out.push(bytes[i] as char);
            i += 1;
        }
        out
    }

    /// (name, path, is_system) for each mountinfo line worth listing.
    /// Format per line: `id parent major:minor root MOUNT_POINT options
    /// [optional…] - FSTYPE source super_options`.
    pub fn parse_mountinfo(text: &str) -> Vec<(String, PathBuf, bool)> {
        let mut out: Vec<(String, PathBuf, bool)> = Vec::new();
        for line in text.lines() {
            let Some((left, right)) = line.split_once(" - ") else {
                continue;
            };
            let Some(mount_point) = left.split_whitespace().nth(4) else {
                continue;
            };
            let Some(fstype) = right.split_whitespace().next() else {
                continue;
            };
            let mp = unescape(mount_point);
            if !listed_mount_point(&mp) || !listed_fstype(fstype) {
                continue;
            }
            if out.iter().any(|(_, p, _)| *p == PathBuf::from(&mp)) {
                continue;
            }
            let is_system = mp == "/";
            let name = if is_system {
                root_volume_name()
            } else {
                mp.rsplit('/').next().unwrap_or(&mp).to_string()
            };
            out.push((name, PathBuf::from(mp), is_system));
        }
        out
    }

    /// The root filesystem row is named after the machine, like Finder
    /// names the boot volume.
    fn root_volume_name() -> String {
        std::fs::read_to_string("/etc/hostname")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "System".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::stat_all_with;
    use std::path::Path;

    #[test]
    fn stuck_mounts_share_one_deadline() {
        fn fake(path: &Path) -> Option<(u64, u64)> {
            if path.to_string_lossy().starts_with("/dead") {
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
            Some((path.as_os_str().len() as u64, 1))
        }
        let paths: Vec<PathBuf> = ["/dead1", "/ok", "/dead22", "/dead333", "/fine"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let start = std::time::Instant::now();
        let got = stat_all_with(&paths, 300, fake);
        // One shared 300 ms deadline, not 300 ms per dead mount (900 ms).
        assert!(start.elapsed() < std::time::Duration::from_millis(700), "{:?}", start.elapsed());
        assert_eq!(got, vec![None, Some((3, 1)), None, None, Some((5, 1))]);
        assert_eq!(stat_all_with(&[], 300, fake), vec![]);
    }

    use super::linux_volumes::parse_mountinfo;
    use std::path::PathBuf;

    // Trimmed from a real Ubuntu 24.04 desktop: root ext4, snap squashfs
    // noise, a USB stick in /media, a cifs mount in /mnt, and an escaped
    // space in a mount point.
    const MOUNTINFO: &str = "\
25 30 0:23 / /sys rw,nosuid,nodev,noexec,relatime shared:7 - sysfs sysfs rw
26 30 0:5 / /proc rw,nosuid,nodev,noexec,relatime shared:13 - proc proc rw
30 1 252:1 / / rw,relatime shared:1 - ext4 /dev/vda1 rw
41 30 7:2 / /snap/core22/1380 ro,nodev,relatime shared:32 - squashfs /dev/loop2 ro
47 30 0:36 / /run/user/1000 rw,nosuid,nodev,relatime shared:38 - tmpfs tmpfs rw,size=401384k
88 30 8:17 / /media/kigi/My\\040Stick rw,nosuid,nodev,relatime shared:401 - vfat /dev/sdb1 rw
92 30 0:52 / /mnt/nas rw,relatime shared:420 - cifs //nas/media rw,vers=3.1.1
95 30 0:55 / /mnt rw,relatime shared:430 - tmpfs tmpfs rw
";

    #[test]
    fn parses_ubuntu_mountinfo() {
        let vols = parse_mountinfo(MOUNTINFO);
        let paths: Vec<&PathBuf> = vols.iter().map(|(_, p, _)| p).collect();
        assert_eq!(
            paths,
            vec![
                &PathBuf::from("/"),
                &PathBuf::from("/media/kigi/My Stick"),
                &PathBuf::from("/mnt/nas"),
            ]
        );
        // Root row is the only system volume; USB keeps its label as name.
        assert!(vols[0].2);
        assert_eq!(vols[1].0, "My Stick");
        assert!(!vols[1].2);
        assert_eq!(vols[2].0, "nas");
    }

    #[test]
    fn parse_mountinfo_tolerates_garbage() {
        assert!(parse_mountinfo("").is_empty());
        assert!(parse_mountinfo("not a mountinfo line\nanother - one two\n").is_empty());
    }

    #[test]
    fn listing_follows_links_for_type_and_size() {
        let dir = std::env::temp_dir().join(format!("yc_local_ls_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("f.txt"), b"12345").unwrap();
        std::os::unix::fs::symlink("sub", dir.join("dir_link")).unwrap();
        std::os::unix::fs::symlink("f.txt", dir.join("file_link")).unwrap();
        std::os::unix::fs::symlink("missing", dir.join("broken")).unwrap();
        let list = super::list_dir(&dir).unwrap();
        let get = |n: &str| list.iter().find(|e| e.name == n).unwrap();
        assert!(get("sub").is_dir && !get("sub").is_symlink);
        assert!(!get("f.txt").is_dir);
        assert_eq!(get("f.txt").size, 5);
        assert!(get("dir_link").is_dir && get("dir_link").is_symlink);
        assert!(!get("file_link").is_dir && get("file_link").is_symlink);
        assert_eq!(get("file_link").size, 5, "the target's size");
        assert!(!get("broken").is_dir && get("broken").is_symlink);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
