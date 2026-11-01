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
            Ok(m) => (path.is_dir(), m.len(), m.modified().ok().map(epoch_millis)),
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

/// True when the file lives on a network mount. Same deadline guard as
/// statfs_timeout; an unreachable mount counts as network, which is the
/// safe answer for the callers.
pub fn is_network_volume(path: &Path) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(statfs_is_network(&p));
    });
    rx.recv_timeout(std::time::Duration::from_millis(800)).unwrap_or(true)
}

/// statfs MNT_LOCAL unset → network.
#[cfg(target_os = "macos")]
fn statfs_is_network(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let local = CString::new(path.as_os_str().as_bytes()).ok().is_some_and(|c| {
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::statfs(c.as_ptr(), &mut stat) } == 0;
        ok && (stat.f_flags & libc::MNT_LOCAL as u32) != 0
    });
    !local
}

/// Linux has no MNT_LOCAL; go by the filesystem magic instead. FUSE counts
/// as network: that is where gvfs (SMB) and sshfs mounts live, and the
/// callers want the conservative answer for anything FUSE-backed.
#[cfg(target_os = "linux")]
fn statfs_is_network(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = CString::new(path.as_os_str().as_bytes()) else {
        return true;
    };
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c.as_ptr(), &mut stat) } != 0 {
        return true;
    }
    linux_volumes::is_network_fstype(stat.f_type as u64)
}

/// statfs with a deadline: a sleeping disk or a dead network mount can hang
/// the syscall for tens of seconds, which must not stall the drives menu.
/// On timeout the stuck thread finishes (and is discarded) on its own.
fn statfs_timeout(path: &Path, ms: u64) -> Option<(u64, u64)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(statfs(&p));
    });
    rx.recv_timeout(std::time::Duration::from_millis(ms)).ok()?
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
            let stat = statfs_timeout(&path, 800);
            let ready = stat.is_some();
            let (free, total) = stat.unwrap_or((0, 0));
            out.push(VolumeInfo {
                name,
                path,
                free,
                total,
                ready,
                is_system,
            });
        }
    }
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
        .map(|(name, path, is_system)| {
            let stat = statfs_timeout(&path, 800);
            let ready = stat.is_some();
            let (free, total) = stat.unwrap_or((0, 0));
            VolumeInfo { name, path, free, total, ready, is_system }
        })
        .collect();
    // The root filesystem first, then the rest alphabetical.
    out.sort_by_key(|v| (!v.is_system, v.name.to_lowercase()));
    out
}

/// GVfs FUSE mounts (SMB shares mounted by us or GNOME Files) live under
/// $XDG_RUNTIME_DIR/gvfs, one directory per mount.
#[cfg(target_os = "linux")]
fn gvfs_mounts() -> Vec<(String, PathBuf, bool)> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })))
        .join("gvfs");
    let Ok(read) = std::fs::read_dir(&base) else {
        return vec![];
    };
    read.flatten()
        .map(|entry| {
            let dir = entry.file_name().to_string_lossy().to_string();
            let name = linux_volumes::gvfs_display_name(&dir).unwrap_or_else(|| dir.clone());
            (name, entry.path(), false)
        })
        .collect()
}

/// Pure parsing helpers for the Linux volume list; compiled under test on
/// every platform so the fixtures run with the regular macOS `cargo test`.
#[cfg(any(target_os = "linux", test))]
mod linux_volumes {
    use std::path::PathBuf;

    /// Filesystem magics that mean "network" (plus FUSE, see caller).
    pub fn is_network_fstype(f_type: u64) -> bool {
        matches!(
            f_type,
            0xFF53_4D42 // CIFS
            | 0xFE53_4D42 // SMB2
            | 0x6969 // NFS
            | 0x6573_5546 // FUSE (gvfs, sshfs)
        )
    }

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

    /// "smb-share:server=nas,share=media" → "media on nas".
    pub fn gvfs_display_name(dir: &str) -> Option<String> {
        let rest = dir.strip_prefix("smb-share:")?;
        let mut server = None;
        let mut share = None;
        for kv in rest.split(',') {
            match kv.split_once('=') {
                Some(("server", v)) => server = Some(v),
                Some(("share", v)) => share = Some(v),
                _ => {}
            }
        }
        Some(format!("{} on {}", share?, server?))
    }
}

#[cfg(test)]
mod tests {
    use super::linux_volumes::{gvfs_display_name, is_network_fstype, parse_mountinfo};
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
    fn gvfs_names() {
        assert_eq!(
            gvfs_display_name("smb-share:server=nas,share=media"),
            Some("media on nas".to_string())
        );
        assert_eq!(
            gvfs_display_name("smb-share:server=nas,share=media,user=kigi"),
            Some("media on nas".to_string())
        );
        assert_eq!(gvfs_display_name("sftp:host=box"), None);
        assert_eq!(gvfs_display_name("smb-share:server=nas"), None);
    }

    #[test]
    fn network_fstypes() {
        assert!(is_network_fstype(0xFF53_4D42)); // cifs
        assert!(is_network_fstype(0x6969)); // nfs
        assert!(is_network_fstype(0x6573_5546)); // fuse
        assert!(!is_network_fstype(0xEF53)); // ext4
    }
}
