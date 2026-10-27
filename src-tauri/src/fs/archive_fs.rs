//! Browse archives like folders (design screen 2d). Read-only in v1:
//! listing, previewing, and extracting selected entries out of local
//! zip / tar / tar.gz / tgz files. Inner paths use forward slashes and
//! no leading slash ("" is the archive root).

use crate::fs::provider::FsEntry;
use crate::ops::queue::{cancelled_err, OpCtx};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

const CHUNK: usize = 1024 * 1024;

enum Kind {
    Zip,
    Tar,
    TarGz,
}

fn kind_of(archive: &Path) -> Result<Kind, String> {
    let name = archive
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if name.ends_with(".zip") {
        Ok(Kind::Zip)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Ok(Kind::TarGz)
    } else if name.ends_with(".tar") {
        Ok(Kind::Tar)
    } else {
        Err(format!("Cannot browse this archive type: {name}"))
    }
}

/// Can the panel step into this file like a folder?
pub fn can_browse(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".zip")
        || lower.ends_with(".tar")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
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

/// Visit every entry: `(inner_path, is_dir, size, modified_ms)`.
fn for_each_entry(
    archive: &Path,
    mut visit: impl FnMut(&str, bool, u64, Option<u64>),
) -> Result<(), String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", archive.display());
    match kind_of(archive)? {
        Kind::Zip => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| err(&e))?;
            for i in 0..zip.len() {
                let entry = zip.by_index_raw(i).map_err(|e| err(&e))?;
                let Ok(raw) = entry.name() else { continue };
                let name = raw.trim_matches('/').to_string();
                if name.is_empty() {
                    continue;
                }
                let modified = entry.last_modified().map(|t| {
                    civil_epoch_ms(
                        t.year() as i64,
                        t.month() as u64,
                        t.day() as u64,
                        t.hour() as u64,
                        t.minute() as u64,
                        t.second() as u64,
                    )
                });
                visit(&name, entry.is_dir(), entry.size(), modified);
            }
        }
        Kind::Tar | Kind::TarGz => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let reader: Box<dyn Read> = match kind_of(archive)? {
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
                );
            }
        }
    }
    Ok(())
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
    for_each_entry(archive, |name, is_dir, size, modified| {
        let Some(rest) = name.strip_prefix(&prefix) else {
            return;
        };
        if rest.is_empty() {
            return;
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
    })?;
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
    match kind_of(archive)? {
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
            let reader: Box<dyn Read> = match kind_of(archive)? {
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

/// Extract the selected inner paths (files or folders) into `dest_dir`,
/// each rooted at its own name — F5 out of an archive.
pub fn extract_entries(
    ctx: &mut OpCtx,
    archive: &Path,
    inners: Vec<String>,
    dest_dir: &Path,
) -> Result<usize, String> {
    // (inner prefix without the selection's parent) for output paths.
    let wanted: Vec<(String, usize)> = inners
        .iter()
        .map(|p| {
            let parent_len = p.rfind('/').map(|i| i + 1).unwrap_or(0);
            (p.clone(), parent_len)
        })
        .collect();
    let out_path = |name: &str| -> Option<PathBuf> {
        for (sel, parent_len) in &wanted {
            if name == sel || name.starts_with(&format!("{sel}/")) {
                let rel = &name[*parent_len..];
                // Zip-slip guard: no absolute or parent-escaping pieces.
                if rel.split('/').any(|c| c == ".." || c.is_empty()) {
                    return None;
                }
                return Some(dest_dir.join(rel));
            }
        }
        None
    };

    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", archive.display());
    let mut count = 0usize;
    match kind_of(archive)? {
        Kind::Zip => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| err(&e))?;
            // Pre-scan for a progress denominator.
            let mut total = 0u64;
            for i in 0..zip.len() {
                let Ok(entry) = zip.by_index_raw(i) else { continue };
                if entry.is_dir() {
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
                    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                    continue;
                }
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut writer = std::fs::File::create(&out)
                    .map_err(|e| format!("{}: {e}", out.display()))?;
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
                count += 1;
            }
        }
        Kind::Tar | Kind::TarGz => {
            let file = std::fs::File::open(archive).map_err(|e| err(&e))?;
            let reader: Box<dyn Read> = match kind_of(archive)? {
                Kind::TarGz => Box::new(flate2::read::GzDecoder::new(file)),
                _ => Box::new(file),
            };
            let mut tar = tar::Archive::new(reader);
            for entry in tar.entries().map_err(|e| err(&e))?.flatten() {
                if ctx.cancelled() {
                    return Err(cancelled_err());
                }
                let Ok(path) = entry.path() else { continue };
                let name = path
                    .to_string_lossy()
                    .trim_start_matches("./")
                    .trim_matches('/')
                    .to_string();
                let Some(out) = out_path(&name) else { continue };
                ctx.report(&name);
                let mut entry = entry;
                if entry.header().entry_type().is_dir() {
                    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
                    continue;
                }
                if !entry.header().entry_type().is_file() {
                    continue;
                }
                if let Some(parent) = out.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                let mut writer = std::fs::File::create(&out)
                    .map_err(|e| format!("{}: {e}", out.display()))?;
                std::io::copy(&mut entry, &mut writer).map_err(|e| e.to_string())?;
                count += 1;
            }
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
}
