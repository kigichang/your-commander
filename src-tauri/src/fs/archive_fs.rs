//! Browse archives like folders (design screen 2d). Read-only in v1:
//! listing, previewing, and extracting selected entries out of local
//! zip / tar / tar.gz / tgz files. Inner paths use forward slashes and
//! no leading slash ("" is the archive root).

use crate::fs::provider::FsEntry;
use crate::ops::archive::{apply_mode, classify, Extractable, Quarantine};
use crate::ops::queue::{cancelled_err, keep_both_path, ConflictChoice, OpCtx};
use crate::ops::safe_path::{create_dir_inside, replace_file_nofollow};
use crate::lru::Lru;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Read;
use std::rc::Rc;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

const CHUNK: usize = 1024 * 1024;

#[derive(Clone, Copy)]
enum Kind {
    Zip,
    Tar,
    TarGz,
}

fn kind_of(archive: &Path) -> Result<Kind, String> {
    let name = archive.file_name().unwrap_or_default().to_string_lossy();
    match classify(&name) {
        Some((Extractable::Zip, _)) => Ok(Kind::Zip),
        Some((Extractable::TarGz, _)) => Ok(Kind::TarGz),
        Some((Extractable::Tar, _)) => Ok(Kind::Tar),
        _ => Err(format!("Cannot browse this archive type: {}", name.to_lowercase())),
    }
}

/// Unix epoch ms from a civil date/time (Howard Hinnant's days algorithm).
fn civil_epoch_ms(y: i64, m: u64, d: u64, h: u64, min: u64, s: u64) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    if days < 0 {
        return 0;
    }
    ((days as u64 * 86_400) + h * 3600 + min * 60 + s) * 1000
}

fn zip_epoch_ms(t: zip::DateTime) -> u64 {
    civil_epoch_ms(
        t.year() as i64,
        t.month() as u64,
        t.day() as u64,
        t.hour() as u64,
        t.minute() as u64,
        t.second() as u64,
    )
}

/// Visit every entry: `(inner_path, is_dir, size, modified_ms)`. An error
/// from `visit` ends the scan and is returned.
fn for_each_entry(
    archive: &Path,
    mut visit: impl FnMut(&str, bool, u64, Option<u64>) -> Result<(), String>,
) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", archive.display());
    let kind = kind_of(archive)?;
    match kind {
        Kind::Zip => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let zip = zip::ZipArchive::new(file).map_err(|e| err(&e))?;
            for i in 0..zip.len() {
                // From the central directory already in memory; by_index_raw
                // would seek to and read each entry's local header.
                let entry = zip.by_index_data(i).map_err(|e| err(&e))?;
                let Ok(raw) = entry.name() else { continue };
                let name = raw.trim_matches('/').to_string();
                if name.is_empty() {
                    continue;
                }
                let modified = entry.last_modified().map(zip_epoch_ms);
                visit(&name, entry.is_dir(), entry.size(), modified)?;
            }
        }
        Kind::Tar | Kind::TarGz => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let reader: Box<dyn Read> = match kind {
                Kind::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
                _ => Box::new(file),
            };
            let mut tar = tar::Archive::new(reader);
            for entry in tar.entries().map_err(|e| err(&e))?.flatten() {
                let Ok(path) = entry.path() else { continue };
                let name = path.to_string_lossy().trim_matches('/').to_string();
                if name.is_empty() || name == "." {
                    continue;
                }
                let header = entry.header();
                let modified = header.mtime().ok().map(|s| s * 1000);
                visit(
                    name.trim_start_matches("./"),
                    header.entry_type().is_dir(),
                    header.size().unwrap_or(0),
                    modified,
                )?;
            }
        }
    }
    Ok(())
}

/// One entry of an archive's flat index.
struct Item {
    name: String,
    is_dir: bool,
    size: u64,
    modified: Option<u64>,
}

/// An archive as it was indexed: its path, size and mtime. Rewriting the
/// file changes the key, so a stale index is never served.
type IndexKey = (PathBuf, u64, Option<SystemTime>);

/// Archives larger than this are listed without being cached.
const INDEX_MAX_ENTRIES: usize = 500_000;
/// Archives past either limit are not browsed at all. A .tar.gz names its
/// entries only as it decompresses, so a few megabytes of gzip can expand
/// into millions of headers (or huge PAX path names), each kept in the
/// index: without a limit, entering such a file exhausts memory.
const BROWSE_MAX_ENTRIES: usize = 2_000_000;
const BROWSE_MAX_NAME_BYTES: usize = 256 * 1024 * 1024;

fn index_cache() -> &'static Mutex<Lru<IndexKey, Arc<Vec<Item>>, 4>> {
    static CACHE: OnceLock<Mutex<Lru<IndexKey, Arc<Vec<Item>>, 4>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Every entry of `archive`, scanned once and then served from a small
/// cache: entering a folder inside a .tar.gz would otherwise decompress
/// the whole archive again, and a plain tar read it end to end.
fn index(archive: &Path) -> Result<Arc<Vec<Item>>, String> {
    let meta = std::fs::metadata(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let key = (archive.to_path_buf(), meta.len(), meta.modified().ok());
    if let Some(hit) = index_cache().lock().unwrap().get(&key) {
        return Ok(hit.clone());
    }
    let items = Arc::new(scan(archive, BROWSE_MAX_ENTRIES, BROWSE_MAX_NAME_BYTES)?);
    if items.len() <= INDEX_MAX_ENTRIES {
        index_cache().lock().unwrap().insert(key, items.clone());
    }
    Ok(items)
}

/// Every entry of `archive`, or an error once there are more than
/// `max_entries` of them or their names add up past `max_name_bytes`.
fn scan(archive: &Path, max_entries: usize, max_name_bytes: usize) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    let mut name_bytes = 0usize;
    for_each_entry(archive, |name, is_dir, size, modified| {
        name_bytes += name.len();
        if items.len() >= max_entries || name_bytes > max_name_bytes {
            return Err(format!(
                "{}: too many entries to browse; extract it instead",
                archive.display()
            ));
        }
        items.push(Item {
            name: name.to_string(),
            is_dir,
            size,
            modified,
        });
        Ok(())
    })?;
    Ok(items)
}

/// Direct children of `inner` ("" = root), folders synthesized from paths.
pub fn list(archive: &Path, inner: &str) -> Result<Vec<FsEntry>, String> {
    let prefix = if inner.is_empty() {
        String::new()
    } else {
        format!("{inner}/")
    };
    struct Child {
        is_dir: bool,
        size: u64,
        modified: Option<u64>,
    }
    let mut children: BTreeMap<String, Child> = BTreeMap::new();
    for item in index(archive)?.iter() {
        let (is_dir, size, modified) = (item.is_dir, item.size, item.modified);
        let Some(rest) = item.name.strip_prefix(&prefix) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        match rest.split_once('/') {
            Some((first, _)) => {
                // Deeper entry: its first segment is a folder here.
                children.entry(first.to_string()).or_insert(Child {
                    is_dir: true,
                    size: 0,
                    modified: None,
                });
            }
            None => {
                let child = children.entry(rest.to_string()).or_insert(Child {
                    is_dir,
                    size: 0,
                    modified: None,
                });
                child.is_dir = is_dir;
                if !is_dir {
                    child.size = size;
                }
                child.modified = modified.or(child.modified);
            }
        }
    }
    Ok(children
        .into_iter()
        .map(|(name, c)| {
            let ext = if c.is_dir {
                String::new()
            } else {
                name.rsplit_once('.')
                    .map(|(_, e)| e.to_lowercase())
                    .unwrap_or_default()
            };
            FsEntry {
                is_hidden: name.starts_with('.'),
                is_cloud: false,
                path: PathBuf::from(format!("{prefix}{name}")),
                is_dir: c.is_dir,
                is_symlink: false,
                size: c.size,
                modified: c.modified,
                ext,
                name,
            }
        })
        .collect())
}

/// Up to `max` bytes of one file inside the archive, for Quick View.
pub fn read_entry(archive: &Path, inner: &str, max: u64) -> Result<Vec<u8>, String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", archive.display());
    let kind = kind_of(archive)?;
    match kind {
        Kind::Zip => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| err(&e))?;
            let mut entry = zip.by_name(inner).map_err(|e| err(&e))?;
            let mut out = Vec::new();
            entry
                .by_ref()
                .take(max)
                .read_to_end(&mut out)
                .map_err(|e| err(&e))?;
            Ok(out)
        }
        Kind::Tar | Kind::TarGz => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let reader: Box<dyn Read> = match kind {
                Kind::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
                _ => Box::new(file),
            };
            let mut tar = tar::Archive::new(reader);
            for entry in tar.entries().map_err(|e| err(&e))?.flatten() {
                let matches = entry
                    .path()
                    .map(|p| p.to_string_lossy().trim_start_matches("./").trim_matches('/') == inner)
                    .unwrap_or(false);
                if matches {
                    let mut out = Vec::new();
                    entry
                        .take(max)
                        .read_to_end(&mut out)
                        .map_err(|e| err(&e))?;
                    return Ok(out);
                }
            }
            Err(format!("{inner}: not found in archive"))
        }
    }
}

/// `out` in its real folder, created if missing and checked to be inside the
/// canonical `root`.
fn real_out(root: &Path, out: &Path) -> Result<PathBuf, String> {
    let (Some(parent), Some(name)) = (out.parent(), out.file_name()) else {
        return Err(format!("{}: not a file path", out.display()));
    };
    Ok(create_dir_inside(root, parent)?.join(name))
}

/// Where to write entry `name` when `out` may already be taken: `out`
/// itself, a free name beside it, or `None` to leave what is there. Asks
/// like any other copy; `modified_ms` is the entry's time, for "replace
/// older".
fn settle_existing(
    ctx: &mut OpCtx,
    name: &str,
    out: PathBuf,
    modified_ms: Option<u64>,
) -> Result<Option<PathBuf>, String> {
    // exists() follows links: a dangling one still takes the name.
    if !(out.exists() || out.is_symlink()) {
        return Ok(Some(out));
    }
    let newer = || {
        let there = out.metadata().ok()?.modified().ok()?;
        let there = there.duration_since(SystemTime::UNIX_EPOCH).ok()?;
        Some(u128::from(modified_ms?) > there.as_millis())
    };
    match ctx.ask_conflict_with(Path::new(name), &out, newer) {
        ConflictChoice::Overwrite => Ok(Some(out)),
        ConflictChoice::KeepBoth => Ok(Some(keep_both_path(&out))),
        ConflictChoice::CancelOp => Err(cancelled_err()),
        _ => Ok(None),
    }
}

/// The tar branch's progress and cancel point, fed from inside the decoder
/// like `ops::archive::Meter`. It shares the context instead of holding it:
/// between two reads the entry loop needs it to ask about a conflict.
struct TarMeter<'a, 'b> {
    inner: std::fs::File,
    ctx: &'a RefCell<&'b mut OpCtx>,
    read: u64,
    len: u64,
    detail: Rc<RefCell<String>>,
}

impl Read for TarMeter<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut ctx = self.ctx.borrow_mut();
        if ctx.cancelled() {
            return Err(std::io::Error::other(cancelled_err()));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        ctx.done_bytes = self.read.min(self.len);
        ctx.report(&self.detail.borrow());
        Ok(n)
    }
}

/// Extract the selected inner paths (files or folders) into `dest_dir`,
/// each rooted at its own name — F5 out of an archive.
pub fn extract_entries(
    ctx: &mut OpCtx,
    archive: &Path,
    inners: Vec<String>,
    dest_dir: &Path,
) -> Result<usize, String> {
    // (selection, selection + "/", length of the selection's parent) for
    // matching entries and building output paths.
    let wanted: Vec<(String, String, usize)> = inners
        .iter()
        .map(|p| {
            let parent_len = p.rfind('/').map(|i| i + 1).unwrap_or(0);
            (p.clone(), format!("{p}/"), parent_len)
        })
        .collect();
    let out_path = |name: &str| -> Option<PathBuf> {
        for (sel, sel_dir, parent_len) in &wanted {
            if name == sel || name.starts_with(sel_dir.as_str()) {
                let rel = &name[*parent_len..];
                // Zip-slip guard: no absolute or parent-escaping pieces, and
                // no "." that would name the folder itself instead of an entry.
                if rel.split('/').any(|c| c == ".." || c == "." || c.is_empty()) {
                    return None;
                }
                return Some(dest_dir.join(rel));
            }
        }
        None
    };

    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", archive.display());
    let mut count = 0usize;
    let kind = kind_of(archive)?;
    let quarantine = Quarantine::of(archive);
    // Folders are made and files written only where the real (link-resolved)
    // path is still under this root: the destination may already hold a link
    // to somewhere else, and the archive picks the names that walk into it.
    std::fs::create_dir_all(dest_dir).map_err(|e| format!("{}: {e}", dest_dir.display()))?;
    let root = dest_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dest_dir.display()))?;
    match kind {
        Kind::Zip => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| err(&e))?;
            // Pre-scan for a progress denominator (central directory only).
            let mut total = 0u64;
            for i in 0..zip.len() {
                let Ok(entry) = zip.by_index_data(i) else { continue };
                if entry.is_dir() || entry.is_symlink() {
                    continue;
                }
                let Ok(name) = entry.name() else { continue };
                if out_path(name.trim_matches('/')).is_some() {
                    total += entry.size();
                }
            }
            ctx.total_bytes = total;
            for i in 0..zip.len() {
                if ctx.cancelled() {
                    return Err(cancelled_err());
                }
                let mut entry = zip.by_index(i).map_err(|e| err(&e))?;
                let Ok(raw) = entry.name() else { continue };
                let name = raw.trim_matches('/').to_string();
                let Some(out) = out_path(&name) else { continue };
                ctx.report(&name);
                if entry.is_dir() {
                    quarantine.mark_below(&root, &create_dir_inside(&root, &out)?);
                    continue;
                }
                // Like the tar branch: a link is not copied out at all. Its data
                // is only the target's path, and its mode is 777.
                if entry.is_symlink() {
                    continue;
                }
                let out = real_out(&root, &out)?;
                let modified = entry.last_modified().map(zip_epoch_ms);
                let Some(out) = settle_existing(ctx, &name, out, modified)? else {
                    ctx.done_bytes += entry.size();
                    continue;
                };
                // An overwrite replaces a link at `out` rather than writing
                // to wherever it points.
                let mut writer = replace_file_nofollow(&out)?;
                quarantine.mark_below(&root, &out);
                let mode = entry.unix_mode();
                let mut buf = vec![0u8; CHUNK];
                loop {
                    if ctx.cancelled() {
                        return Err(cancelled_err());
                    }
                    let n = entry.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    std::io::Write::write_all(&mut writer, &buf[..n])
                        .map_err(|e| e.to_string())?;
                    ctx.done_bytes += n as u64;
                    ctx.report(&name);
                }
                apply_mode(&out, mode);
                count += 1;
            }
        }
        Kind::Tar | Kind::TarGz => {
            // A tar stream has no index: progress is the share of the
            // archive file read so far, and the reader handles cancel.
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
            ctx.total_bytes = len;
            let detail = Rc::new(RefCell::new(String::new()));
            let shared = RefCell::new(&mut *ctx);
            let meter = TarMeter {
                inner: file,
                ctx: &shared,
                read: 0,
                len,
                detail: detail.clone(),
            };
            let reader: Box<dyn Read> = match kind {
                Kind::TarGz => Box::new(flate2::read::GzDecoder::new(meter)),
                _ => Box::new(meter),
            };
            let mut tar = tar::Archive::new(reader);
            let result = (|| {
                for entry in tar.entries().map_err(|e| err(&e))?.flatten() {
                    let Ok(path) = entry.path() else { continue };
                    let name = path
                        .to_string_lossy()
                        .trim_start_matches("./")
                        .trim_matches('/')
                        .to_string();
                    let Some(out) = out_path(&name) else { continue };
                    *detail.borrow_mut() = name.clone();
                    let mut entry = entry;
                    if entry.header().entry_type().is_dir() {
                        quarantine.mark_below(&root, &create_dir_inside(&root, &out)?);
                        continue;
                    }
                    if !entry.header().entry_type().is_file() {
                        continue;
                    }
                    let out = real_out(&root, &out)?;
                    let modified = entry.header().mtime().ok().map(|s| s * 1000);
                    let Some(out) =
                        settle_existing(&mut shared.borrow_mut(), &name, out, modified)?
                    else {
                        continue;
                    };
                    // An overwrite replaces a link at `out` rather than
                    // writing to wherever it points.
                    let mut writer = replace_file_nofollow(&out)?;
                    quarantine.mark_below(&root, &out);
                    std::io::copy(&mut entry, &mut writer).map_err(|e| e.to_string())?;
                    apply_mode(&out, entry.header().mode().ok());
                    count += 1;
                }
                Ok::<_, String>(())
            })();
            drop(tar);
            // A cancel inside the reader surfaces as an I/O error (or an
            // early end of entries); report it as the cancel it was.
            if ctx.cancelled() {
                return Err(cancelled_err());
            }
            result?;
            // The end-of-archive padding is never read: finish at 100%.
            ctx.done_bytes = len;
            ctx.report("");
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::queue::{ConflictChoice, OpCtx};

    fn make_zip(dir: &Path) -> PathBuf {
        use std::io::Write;
        let path = dir.join("t.zip");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        zip.add_directory("sub", opts).unwrap();
        zip.start_file("root.txt", opts).unwrap();
        zip.write_all(b"root").unwrap();
        zip.start_file("sub/inner.txt", opts).unwrap();
        zip.write_all(b"inner").unwrap();
        zip.finish().unwrap();
        path
    }

    #[test]
    fn zip_list_read_extract() {
        let dir = std::env::temp_dir().join(format!("yc_arfs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = make_zip(&dir);

        let root = list(&zip_path, "").unwrap();
        let names: Vec<&str> = root.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["root.txt", "sub"]);
        assert!(root.iter().find(|e| e.name == "sub").unwrap().is_dir);

        let sub = list(&zip_path, "sub").unwrap();
        assert_eq!(sub[0].name, "inner.txt");
        assert_eq!(sub[0].path, PathBuf::from("sub/inner.txt"));

        let bytes = read_entry(&zip_path, "sub/inner.txt", 1024).unwrap();
        assert_eq!(bytes, b"inner");

        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        let n = extract_entries(&mut ctx, &zip_path, vec!["sub".into()], &out).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read(out.join("sub/inner.txt")).unwrap(), b"inner");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_stops_past_its_limits() {
        let dir = std::env::temp_dir().join(format!("yc_arfs_cap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = make_zip(&dir); // sub, root.txt, sub/inner.txt
        assert_eq!(scan(&zip_path, 3, 1024).unwrap().len(), 3);
        assert!(scan(&zip_path, 2, 1024).is_err());
        assert!(scan(&zip_path, 3, 10).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tar_extract_entries_reports_progress_and_cancels() {
        let dir = std::env::temp_dir().join(format!("yc_arfs_tar_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("t.tar");
        {
            let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
            for name in ["a.bin", "b.bin"] {
                let data = vec![7u8; 1 << 20];
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                tar.append_data(&mut header, name, &data[..]).unwrap();
            }
            tar.finish().unwrap();
        }
        let len = archive.metadata().unwrap().len();
        let wanted = vec!["a.bin".to_string(), "b.bin".to_string()];

        let (mut ctx, seen) = OpCtx::recording(None);
        let n = extract_entries(&mut ctx, &archive, wanted.clone(), &dir.join("out")).unwrap();
        assert_eq!(n, 2);
        let seen = seen.lock().unwrap();
        assert!(seen.iter().all(|&(_, t)| t == len));
        assert!(seen.iter().any(|&(d, _)| d > 0 && d < len), "{seen:?}");
        assert_eq!(seen.last().unwrap().0, len);

        let (mut ctx, _) = OpCtx::recording(Some(len / 3));
        let err = extract_entries(&mut ctx, &archive, wanted, &dir.join("out2")).unwrap_err();
        assert_eq!(err, cancelled_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_out_keeps_permission_bits_but_not_setuid() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("yc_arfs_mode_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("m.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("run.sh", opts.unix_permissions(0o755)).unwrap();
            zip.write_all(b"#!/bin/sh\n").unwrap();
            zip.start_file("suid", opts.unix_permissions(0o4755)).unwrap();
            zip.write_all(b"x").unwrap();
            zip.finish().unwrap();
        }
        let tar_path = dir.join("m.tar");
        {
            let mut tar = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
            for (name, mode) in [("run.sh", 0o755), ("suid", 0o4755)] {
                let mut header = tar::Header::new_gnu();
                header.set_size(1);
                header.set_mode(mode);
                tar.append_data(&mut header, name, &b"x"[..]).unwrap();
            }
            tar.finish().unwrap();
        }
        for archive in [zip_path, tar_path] {
            let out = dir.join(format!("out_{}", archive.extension().unwrap().to_string_lossy()));
            let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
            let wanted = vec!["run.sh".to_string(), "suid".to_string()];
            assert_eq!(extract_entries(&mut ctx, &archive, wanted, &out).unwrap(), 2);
            let mode = |n: &str| out.join(n).metadata().unwrap().permissions().mode() & 0o7777;
            assert_eq!(mode("run.sh"), 0o755, "{}", archive.display());
            assert_eq!(mode("suid"), 0o755, "{}", archive.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dot_pieces_are_skipped_not_written_onto_the_folder() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("yc_arfs_dot_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("dot.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            // "a/." names the folder "a" itself; it must not abort the copy.
            zip.start_file("a/.", opts).unwrap();
            zip.write_all(b"junk").unwrap();
            zip.start_file("a/ok.txt", opts).unwrap();
            zip.write_all(b"ok").unwrap();
            zip.finish().unwrap();
        }
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        let n = extract_entries(&mut ctx, &zip_path, vec!["a".into()], &out).unwrap();
        assert_eq!(n, 1);
        assert_eq!(std::fs::read(out.join("a/ok.txt")).unwrap(), b"ok");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_out_replaces_links_instead_of_writing_through() {
        let dir = std::env::temp_dir().join(format!("yc_arfs_ln_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = make_zip(&dir);
        let tar_path = dir.join("t.tar");
        {
            let mut tar = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_mode(0o644);
            tar.append_data(&mut header, "root.txt", &b"root"[..]).unwrap();
            tar.finish().unwrap();
        }
        let victim = dir.join("victim.txt");
        for (n, archive) in [zip_path, tar_path].into_iter().enumerate() {
            std::fs::write(&victim, b"keep").unwrap();
            let out = dir.join(format!("out{n}"));
            std::fs::create_dir_all(&out).unwrap();
            std::os::unix::fs::symlink(&victim, out.join("root.txt")).unwrap();
            // The link takes the name: a conflict, answered with "overwrite".
            let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
            extract_entries(&mut ctx, &archive, vec!["root.txt".into()], &out).unwrap();
            assert_eq!(std::fs::read(&victim).unwrap(), b"keep", "{}", archive.display());
            let written = out.join("root.txt");
            assert!(!written.is_symlink(), "the link was replaced");
            assert_eq!(std::fs::read(&written).unwrap(), b"root");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_out_skips_zip_links_as_it_does_tar_links() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("yc_arfs_zl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("l.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("sub/inner.txt", opts).unwrap();
            zip.write_all(b"inner").unwrap();
            zip.add_symlink("sub/link", "a/rather/long/target/path", opts).unwrap();
            zip.finish().unwrap();
        }
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        let n = extract_entries(&mut ctx, &zip_path, vec!["sub".into()], &out).unwrap();
        assert_eq!(n, 1, "only the file counts");
        assert_eq!(std::fs::read(out.join("sub/inner.txt")).unwrap(), b"inner");
        assert!(out.join("sub/link").symlink_metadata().is_err(), "the link is skipped");
        // The progress total leaves it out too, so a copy still ends at 100%.
        assert_eq!((ctx.done_bytes, ctx.total_bytes), (5, 5));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn listing_reuses_the_index_until_the_archive_changes() {
        let dir = std::env::temp_dir().join(format!("yc_arfs_idx_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.tar");
        let write = |name: &str| {
            let mut tar = tar::Builder::new(std::fs::File::create(&path).unwrap());
            let mut header = tar::Header::new_gnu();
            header.set_size(1);
            header.set_mode(0o644);
            tar.append_data(&mut header, name, &b"x"[..]).unwrap();
            tar.finish().unwrap();
        };
        let names = || -> Vec<String> {
            list(&path, "sub").unwrap().into_iter().map(|e| e.name).collect()
        };
        write("sub/one.txt");
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(names(), vec!["one.txt"]);

        // Same size, same mtime, other contents: served from the index, so
        // the archive was not scanned again.
        write("sub/two.txt");
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(mtime).unwrap();
        assert_eq!(names(), vec!["one.txt"]);

        // A new mtime is a new archive.
        let later = mtime + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(later).unwrap();
        assert_eq!(names(), vec!["two.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn copy_out_does_not_write_through_a_link_leaving_the_destination() {
        let dir = std::env::temp_dir().join(format!("yc_arfs_esc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = make_zip(&dir);
        let tar_path = dir.join("t.tar");
        {
            let mut tar = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
            let mut header = tar::Header::new_gnu();
            header.set_size(5);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, "sub/inner.txt", &b"inner"[..]).unwrap();
            tar.finish().unwrap();
        }
        for (name, archive) in [("zip", &zip_path), ("tar", &tar_path)] {
            let outside = dir.join(format!("outside_{name}"));
            let out = dir.join(format!("out_{name}"));
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::create_dir_all(&out).unwrap();
            std::os::unix::fs::symlink(&outside, out.join("sub")).unwrap();

            let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
            let err = extract_entries(&mut ctx, archive, vec!["sub".into()], &out).unwrap_err();
            assert!(err.contains("escapes"), "{name}: {err}");
            assert!(!outside.join("inner.txt").exists(), "{name}: nothing written outside");
        }

        // A link that stays inside the destination is fine.
        let out = dir.join("out_inside");
        std::fs::create_dir_all(out.join("real")).unwrap();
        std::os::unix::fs::symlink("real", out.join("sub")).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
        extract_entries(&mut ctx, &zip_path, vec!["sub".into()], &out).unwrap();
        assert_eq!(std::fs::read(out.join("real/inner.txt")).unwrap(), b"inner");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F5 out of an archive onto files that are already there is a conflict
    /// like any other copy: asked about, and bound by the dialog's setting.
    #[test]
    fn copy_out_asks_before_replacing() {
        use crate::ops::queue::{CopyOptions, IfExists};
        let dir = std::env::temp_dir().join(format!("yc_arfs_conf_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = make_zip(&dir);
        let tar_path = dir.join("t.tar");
        {
            let mut tar = tar::Builder::new(std::fs::File::create(&tar_path).unwrap());
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_mode(0o644);
            header.set_mtime(1_000_000_000);
            tar.append_data(&mut header, "root.txt", &b"root"[..]).unwrap();
            tar.finish().unwrap();
        }
        let wanted = || vec!["root.txt".to_string()];
        for archive in [&zip_path, &tar_path] {
            let out = dir.join(format!("out_{}", archive.extension().unwrap().to_string_lossy()));
            std::fs::create_dir_all(&out).unwrap();
            let mine = out.join("root.txt");
            let read = |p: &Path| std::fs::read(p).unwrap();
            std::fs::write(&mine, b"mine").unwrap();

            let mut ctx = OpCtx::for_test(ConflictChoice::Skip);
            assert_eq!(extract_entries(&mut ctx, archive, wanted(), &out).unwrap(), 0);
            assert_eq!(read(&mine), b"mine", "skip keeps the file");

            // The dialog said skip: not even asked (the answer would overwrite).
            let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
            ctx.set_options(CopyOptions { if_exists: IfExists::Skip, ..Default::default() });
            assert_eq!(extract_entries(&mut ctx, archive, wanted(), &out).unwrap(), 0);
            assert_eq!(read(&mine), b"mine");

            // The entry is older than the file just written.
            let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
            ctx.set_options(CopyOptions { if_exists: IfExists::ReplaceOlder, ..Default::default() });
            assert_eq!(extract_entries(&mut ctx, archive, wanted(), &out).unwrap(), 0);
            assert_eq!(read(&mine), b"mine");

            let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
            let err = extract_entries(&mut ctx, archive, wanted(), &out).unwrap_err();
            assert_eq!(err, cancelled_err());
            assert_eq!(read(&mine), b"mine");

            let mut ctx = OpCtx::for_test(ConflictChoice::KeepBoth);
            assert_eq!(extract_entries(&mut ctx, archive, wanted(), &out).unwrap(), 1);
            assert_eq!(read(&mine), b"mine");
            assert_eq!(read(&out.join("root 2.txt")), b"root");

            let mut ctx = OpCtx::for_test(ConflictChoice::Overwrite);
            assert_eq!(extract_entries(&mut ctx, archive, wanted(), &out).unwrap(), 1);
            assert_eq!(read(&mine), b"root");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What comes out of a downloaded archive is as untrusted as the
    /// download: the quarantine flag goes with it, onto folders too.
    #[cfg(target_os = "macos")]
    #[test]
    fn quarantine_follows_the_contents() {
        const ATTR: &str = "com.apple.quarantine";
        let flag = b"0081;00000000;Test;".to_vec();
        let dir = std::env::temp_dir().join(format!("yc_arfs_q_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = make_zip(&dir);
        let get = |p: PathBuf| xattr::get(p, ATTR).unwrap();

        let plain = dir.join("plain");
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        extract_entries(&mut ctx, &zip_path, vec!["sub".into()], &plain).unwrap();
        assert_eq!(get(plain.join("sub/inner.txt")), None);

        xattr::set(&zip_path, ATTR, &flag).unwrap();
        let out = dir.join("out");
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        extract_entries(&mut ctx, &zip_path, vec!["sub".into()], &out).unwrap();
        assert_eq!(get(out.join("sub/inner.txt")), Some(flag.clone()));
        assert_eq!(get(out.join("sub")), Some(flag.clone()));
        assert_eq!(get(out.clone()), None, "the destination itself is the user's");

        let all = dir.join("all");
        std::fs::create_dir_all(&all).unwrap();
        let mut ctx = OpCtx::for_test(ConflictChoice::CancelOp);
        crate::ops::archive::extract_op(&mut ctx, vec![zip_path], all.clone()).unwrap();
        assert_eq!(get(all.join("t")), Some(flag.clone()));
        assert_eq!(get(all.join("t/sub/inner.txt")), Some(flag.clone()));
        assert_eq!(get(all.join("t/root.txt")), Some(flag));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
