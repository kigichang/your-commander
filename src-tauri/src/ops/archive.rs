use super::queue::{cancelled_err, keep_both_path, OpCtx};
use super::safe_path::{create_dir_inside, create_file_nofollow, link_inside, symlink_inside};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const CHUNK: usize = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    TarGz,
    Gz,
}

/// macOS noise that must never end up inside an archive.
pub fn excluded(name: &str) -> bool {
    name == ".DS_Store" || name.starts_with("._")
}

/// What `extract_op` can unpack: every `ArchiveFormat`, plus 7z.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extractable {
    Zip,
    SevenZ,
    Tar,
    TarGz,
    Gz,
}

/// Every archive suffix the backend recognises. ".tar.gz" must come before
/// ".gz", which is its tail. The frontend mirrors this list in ARCHIVE_RE
/// and BROWSABLE_ARCHIVE_RE (src/keymap/actions.ts).
const SUFFIXES: [(&str, Extractable); 6] = [
    (".tar.gz", Extractable::TarGz),
    (".tgz", Extractable::TarGz),
    (".tar", Extractable::Tar),
    (".zip", Extractable::Zip),
    (".7z", Extractable::SevenZ),
    (".gz", Extractable::Gz),
];

/// The archive kind and the name with its suffix stripped, or None when the
/// name is not a known archive.
pub fn classify(name: &str) -> Option<(Extractable, &str)> {
    let lower = name.to_lowercase();
    SUFFIXES
        .iter()
        .find(|(suffix, _)| lower.ends_with(suffix))
        .map(|(suffix, kind)| (*kind, &name[..name.len() - suffix.len()]))
}

/// (relative name with forward slashes, absolute path, is_dir), exclusions applied.
fn collect_tree(sources: &[PathBuf]) -> Result<Vec<(String, PathBuf, bool)>, String> {
    fn walk(
        out: &mut Vec<(String, PathBuf, bool)>,
        rel: &str,
        path: &Path,
    ) -> Result<(), String> {
        // lstat: a link to a folder is stored as a link, not walked.
        if path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            out.push((rel.to_string(), path.to_path_buf(), true));
            let entries =
                std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if excluded(&name) {
                    continue;
                }
                walk(out, &format!("{rel}/{name}"), &entry.path())?;
            }
        } else {
            out.push((rel.to_string(), path.to_path_buf(), false));
        }
        Ok(())
    }

    let mut out = Vec::new();
    for src in sources {
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .ok_or_else(|| format!("bad path: {}", src.display()))?;
        walk(&mut out, &name, src)?;
    }
    Ok(out)
}

/// Returns the created archive's file name.
pub fn compress_op(
    ctx: &mut OpCtx,
    sources: Vec<PathBuf>,
    dest: PathBuf,
    format: ArchiveFormat,
) -> Result<String, String> {
    if dest.exists() {
        return Err(format!("\"{}\" already exists", dest.to_string_lossy()));
    }

    if format == ArchiveFormat::Gz {
        if sources.len() != 1 || sources[0].is_dir() {
            return Err(".gz can only compress a single file".to_string());
        }
        return gz_file(ctx, &sources[0], &dest);
    }

    let tree = collect_tree(&sources)?;
    ctx.total_bytes = tree
        .iter()
        .filter(|(_, _, is_dir)| !is_dir)
        .filter_map(|(_, p, _)| p.symlink_metadata().ok())
        .filter(|m| !m.is_symlink())
        .map(|m| m.len())
        .sum();

    let result = match format {
        ArchiveFormat::Zip => write_zip(ctx, &tree, &dest),
        ArchiveFormat::Tar => {
            let file = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
            write_tar(ctx, &tree, file)
        }
        ArchiveFormat::TarGz => {
            let file = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            write_tar(ctx, &tree, enc)
        }
        ArchiveFormat::Gz => unreachable!(),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(&dest);
    }
    result?;
    Ok(dest
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string())
}

fn write_zip(
    ctx: &mut OpCtx,
    tree: &[(String, PathBuf, bool)],
    dest: &Path,
) -> Result<(), String> {
    let file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    for (rel, path, is_dir) in tree {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        ctx.report(rel);
        if *is_dir {
            zip.add_directory(rel.clone(), options)
                .map_err(|e| e.to_string())?;
            continue;
        }
        if path.is_symlink() {
            let target =
                std::fs::read_link(path).map_err(|e| format!("{}: {e}", path.display()))?;
            zip.add_symlink(rel.clone(), target.to_string_lossy(), options)
                .map_err(|e| e.to_string())?;
            continue;
        }
        zip.start_file(rel.clone(), options)
            .map_err(|e| e.to_string())?;
        let mut reader =
            std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            if ctx.cancelled() {
                return Err(cancelled_err());
            }
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            zip.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            ctx.done_bytes += n as u64;
            ctx.report(rel);
        }
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn write_tar<W: Write>(
    ctx: &mut OpCtx,
    tree: &[(String, PathBuf, bool)],
    writer: W,
) -> Result<(), String> {
    let mut tar = tar::Builder::new(writer);
    tar.follow_symlinks(false);
    for (rel, path, is_dir) in tree {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        ctx.report(rel);
        if *is_dir {
            tar.append_dir(rel, path).map_err(|e| e.to_string())?;
        } else {
            tar.append_path_with_name(path, rel)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if !path.is_symlink() {
                ctx.done_bytes += path.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    let inner = tar.into_inner().map_err(|e| e.to_string())?;
    drop(inner);
    Ok(())
}

fn gz_file(ctx: &mut OpCtx, src: &Path, dest: &Path) -> Result<String, String> {
    ctx.total_bytes = src.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut buf = vec![0u8; CHUNK];
    loop {
        if ctx.cancelled() {
            drop(enc);
            let _ = std::fs::remove_file(dest);
            return Err(cancelled_err());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        enc.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        ctx.done_bytes += n as u64;
        ctx.report(&src.to_string_lossy());
    }
    enc.finish().map_err(|e| e.to_string())?;
    Ok(dest
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string())
}

/// Progress for reading one archive, fed from inside the decoder: bytes
/// pulled through it advance `ctx.done_bytes`, scaled into this archive's
/// slice `[base, base + len)` of the op total, and a cancel turns into a
/// read error. Units inside the archive are whatever `total` counts
/// (compressed bytes for tar/gz streams, uncompressed bytes for zip and
/// 7z entries); the op total is always on-disk archive bytes, so rate and
/// ETA stay meaningful across several archives of mixed formats.
pub(crate) struct Meter<'a, R> {
    pub inner: R,
    pub ctx: &'a mut OpCtx,
    pub base: u64,
    pub len: u64,
    pub read: u64,
    pub total: u64,
    /// What the progress line shows (current entry or archive name).
    pub detail: Rc<RefCell<String>>,
}

impl<R: Read> Read for Meter<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.ctx.cancelled() {
            return Err(std::io::Error::other(cancelled_err()));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        let part = if self.total > 0 {
            (self.len as u128 * self.read.min(self.total) as u128 / self.total as u128) as u64
        } else {
            0
        };
        self.ctx.done_bytes = self.base + part;
        self.ctx.report(&self.detail.borrow());
        Ok(n)
    }
}

/// Extract each archive into its own folder under `dest_dir`. Returns the
/// number of archives extracted.
pub fn extract_op(
    ctx: &mut OpCtx,
    archives: Vec<PathBuf>,
    dest_dir: PathBuf,
) -> Result<usize, String> {
    let lens: Vec<u64> = archives
        .iter()
        .map(|a| a.metadata().map(|m| m.len()).unwrap_or(0))
        .collect();
    ctx.total_bytes = lens.iter().sum();
    ctx.done_bytes = 0;
    let mut base = 0u64;
    let mut count = 0usize;
    for (archive, &len) in archives.iter().zip(&lens) {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let name = archive
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some((kind, stem)) = classify(&name) else {
            return Err(format!("Unknown archive format: {name}"));
        };
        let mut target = dest_dir.join(stem);
        if target.exists() {
            target = keep_both_path(&target);
        }

        ctx.report(&name);
        let detail = Rc::new(RefCell::new(name.clone()));
        let result = match kind {
            Extractable::Zip => extract_zip(ctx, archive, &target, base, len),
            Extractable::SevenZ => extract_7z(ctx, archive, &target, base, len),
            Extractable::TarGz | Extractable::Tar => (|| {
                let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
                let meter = Meter {
                    inner: file,
                    ctx: &mut *ctx,
                    base,
                    len,
                    read: 0,
                    total: len,
                    detail: detail.clone(),
                };
                if kind == Extractable::TarGz {
                    extract_tar(flate2::read::GzDecoder::new(meter), &target, &detail)
                } else {
                    extract_tar(meter, &target, &detail)
                }
            })(),
            Extractable::Gz => (|| {
                // Single gzip-compressed file: decompress next to the others.
                std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                let out = target.join(stem);
                let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
                let mut dec = flate2::read::GzDecoder::new(Meter {
                    inner: file,
                    ctx: &mut *ctx,
                    base,
                    len,
                    read: 0,
                    total: len,
                    detail: detail.clone(),
                });
                // `target` is a fresh folder, but never write through a link.
                let mut writer = create_file_nofollow(&out)?;
                std::io::copy(&mut dec, &mut writer).map_err(|e| e.to_string())?;
                Ok(())
            })(),
        };
        // A cancel inside a decoder surfaces as an I/O error; report it as
        // the cancel it was.
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        result?;
        base += len;
        ctx.done_bytes = base;
        ctx.report(&name);
        count += 1;
    }
    Ok(count)
}

fn extract_zip(
    ctx: &mut OpCtx,
    archive: &Path,
    dest: &Path,
    base: u64,
    len: u64,
) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    // Progress inside this archive counts uncompressed bytes written.
    let total: u64 = (0..zip.len())
        .filter_map(|i| zip.by_index_data(i).ok().map(|e| e.size()))
        .sum();
    let mut read = 0u64;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    // Directories are created and files written only where the real
    // (symlink-resolved) path is still under this root.
    let root = dest.canonicalize().map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name guards against zip-slip path escapes.
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(&rel);
        let shown = rel.to_string_lossy().to_string();
        ctx.report(&shown);
        if entry.is_dir() {
            create_dir_inside(&root, &out)?;
            continue;
        }
        // From here on, work in the entry's real folder: the one checked
        // to be inside root, whatever links lead to it.
        let (Some(parent), Some(file_name)) = (out.parent(), out.file_name()) else {
            continue;
        };
        let out = create_dir_inside(&root, parent)?.join(file_name);
        if entry.is_symlink() {
            // The entry's data is the link target, verbatim. A target that
            // leaves `dest` is refused so later entries cannot write
            // through it.
            let mut target = String::new();
            entry
                .read_to_string(&mut target)
                .map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&out);
            symlink_inside(&root, &out, &target)?;
            continue;
        }
        let mut writer = create_file_nofollow(&out)?;
        let mode = entry.unix_mode();
        let mut meter = Meter {
            inner: &mut entry,
            ctx: &mut *ctx,
            base,
            len,
            read,
            total,
            detail: Rc::new(RefCell::new(shown)),
        };
        std::io::copy(&mut meter, &mut writer).map_err(|e| e.to_string())?;
        read = meter.read;
        apply_mode(&out, mode);
    }
    Ok(())
}

/// Give a file just written from an archive the permission bits the
/// archive recorded (the executable bit of a script, say). Only rwx: an
/// archive's setuid/setgid/sticky bits are dropped, as tar's unpack does
/// by default. Used by full extraction and by F5 copy-out of entries.
pub(crate) fn apply_mode(path: &Path, mode: Option<u32>) {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777));
    }
}

fn extract_7z(
    ctx: &mut OpCtx,
    archive: &Path,
    dest: &Path,
    base: u64,
    len: u64,
) -> Result<(), String> {
    // Progress inside this archive counts uncompressed bytes written.
    let total: u64 = sevenz_rust2::Archive::open(archive)
        .map_err(|e| e.to_string())?
        .files
        .iter()
        .map(|f| f.size)
        .sum();
    let mut read = 0u64;
    sevenz_rust2::decompress_file_with_extract_fn(archive, dest, |entry, reader, path| {
        let mut meter = Meter {
            inner: reader,
            ctx: &mut *ctx,
            base,
            len,
            read,
            total,
            detail: Rc::new(RefCell::new(entry.name().to_string())),
        };
        let more = sevenz_rust2::default_entry_extract_fn(entry, &mut meter, path)?;
        read = meter.read;
        Ok(more)
    })
    .map_err(|e| e.to_string())
}

/// Unpack a tar stream into `dest`. Cancelling and progress happen in the
/// reader (a `Meter` under any decompressor); `detail` is set to each
/// entry's path so the progress line names it.
fn extract_tar<R: Read>(reader: R, dest: &Path, detail: &RefCell<String>) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let root = dest.canonicalize().map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(reader);
    let entries = tar.entries().map_err(|e| e.to_string())?;
    for entry in entries {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if let Ok(path) = entry.path() {
            *detail.borrow_mut() = path.to_string_lossy().to_string();
        }
        // unpack_in refuses paths that escape `dest` and resolves the real
        // parent before writing, but creates symlinks with any target:
        // refuse those that point outside `dest`.
        if entry.header().entry_type().is_symlink() {
            let link = entry.path().map_err(|e| e.to_string())?;
            let link = dest.join(link);
            let target = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            if link_inside(&root, &link, &target.to_string_lossy()).is_none() {
                return Err(format!(
                    "{}: symlink to {} escapes the destination folder",
                    link.display(),
                    target.display()
                ));
            }
        }
        entry.unpack_in(dest).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::queue::ConflictChoice;

    fn test_ctx() -> OpCtx {
        OpCtx::for_test(ConflictChoice::CancelOp)
    }

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yc_ar_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn excludes_macos_noise() {
        assert!(excluded(".DS_Store"));
        assert!(excluded("._resource_fork"));
        assert!(!excluded("normal.txt"));
        assert!(!excluded(".hidden_but_fine"));
    }

    #[test]
    fn classify_finds_kind_and_stem() {
        assert_eq!(classify("a.tar.gz"), Some((Extractable::TarGz, "a")));
        assert_eq!(classify("b.tgz"), Some((Extractable::TarGz, "b")));
        assert_eq!(classify("c.zip"), Some((Extractable::Zip, "c")));
        assert_eq!(classify("d.7z"), Some((Extractable::SevenZ, "d")));
        assert_eq!(classify("e.tar"), Some((Extractable::Tar, "e")));
        assert_eq!(classify("Pic.JPG.gz"), Some((Extractable::Gz, "Pic.JPG")));
        assert_eq!(classify("notes.txt"), None);
    }

    #[test]
    fn zip_roundtrip_excludes_ds_store() {
        let work = tmp("zip");
        let src = work.join("proj");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("keep.txt"), b"data").unwrap();
        std::fs::write(src.join(".DS_Store"), b"junk").unwrap();
        std::fs::write(src.join("sub/._apple"), b"junk").unwrap();
        std::fs::write(src.join("sub/also.txt"), b"ok").unwrap();

        let dest = work.join("out.zip");
        let mut ctx = test_ctx();
        compress_op(&mut ctx, vec![src.clone()], dest.clone(), ArchiveFormat::Zip).unwrap();

        let extract_to = work.join("x");
        let mut ctx2 = test_ctx();
        extract_op(&mut ctx2, vec![dest], extract_to.clone()).unwrap();
        let root = extract_to.join("out/proj");
        assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"data");
        assert_eq!(std::fs::read(root.join("sub/also.txt")).unwrap(), b"ok");
        assert!(!root.join(".DS_Store").exists());
        assert!(!root.join("sub/._apple").exists());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn targz_roundtrip() {
        let work = tmp("tgz");
        let src = work.join("stuff");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("f.txt"), b"hello tar").unwrap();
        std::os::unix::fs::symlink("f.txt", src.join("ln.txt")).unwrap();

        let dest = work.join("stuff.tar.gz");
        let mut ctx = test_ctx();
        compress_op(&mut ctx, vec![src], dest.clone(), ArchiveFormat::TarGz).unwrap();

        let extract_to = work.join("x");
        let mut ctx2 = test_ctx();
        extract_op(&mut ctx2, vec![dest], extract_to.clone()).unwrap();
        assert_eq!(
            std::fs::read(extract_to.join("stuff/stuff/f.txt")).unwrap(),
            b"hello tar"
        );
        let link = extract_to.join("stuff/stuff/ln.txt");
        assert!(link.is_symlink(), "tar must preserve symlinks");
        assert_eq!(std::fs::read(&link).unwrap(), b"hello tar");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn zip_roundtrip_preserves_symlinks() {
        let work = tmp("zipln");
        let src = work.join("proj");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("real.txt"), b"content").unwrap();
        std::os::unix::fs::symlink("real.txt", src.join("link.txt")).unwrap();

        let dest = work.join("out.zip");
        let mut ctx = test_ctx();
        compress_op(&mut ctx, vec![src], dest.clone(), ArchiveFormat::Zip).unwrap();

        let extract_to = work.join("x");
        let mut ctx2 = test_ctx();
        extract_op(&mut ctx2, vec![dest], extract_to.clone()).unwrap();
        let link = extract_to.join("out/proj/link.txt");
        assert!(link.is_symlink(), "zip must preserve symlinks");
        assert_eq!(std::fs::read_link(&link).unwrap(), PathBuf::from("real.txt"));
        assert_eq!(std::fs::read(&link).unwrap(), b"content");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn zip_extract_keeps_permission_bits() {
        use std::os::unix::fs::PermissionsExt;
        let work = tmp("zipmode");
        let archive = work.join("m.zip");
        {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("run.sh", opts.unix_permissions(0o755)).unwrap();
            zip.write_all(b"#!/bin/sh\n").unwrap();
            zip.finish().unwrap();
        }
        let mut ctx = test_ctx();
        extract_op(&mut ctx, vec![archive], work.join("x")).unwrap();
        let mode = work.join("x/m/run.sh").metadata().unwrap().permissions().mode();
        // (The zip writer itself drops setuid bits, so the rwx-only mask is
        // covered by the tar case in archive_fs's copy-out test.)
        assert_eq!(mode & 0o7777, 0o755);
        let _ = std::fs::remove_dir_all(&work);
    }

    /// A zip whose symlink points outside the extraction folder, followed
    /// by a file entry under that link.
    fn hostile_zip(path: &Path, target: &str) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        zip.add_symlink("esc", target, opts).unwrap();
        zip.start_file("esc/evil.txt", opts).unwrap();
        zip.write_all(b"owned").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn zip_symlink_escaping_dest_is_refused() {
        for (name, target) in [("abs", "/tmp"), ("rel", "../../..")] {
            let work = tmp(&format!("zipesc_{name}"));
            let archive = work.join("bad.zip");
            hostile_zip(&archive, target);
            let extract_to = work.join("x");
            let mut ctx = test_ctx();
            let err = extract_op(&mut ctx, vec![archive], extract_to.clone()).unwrap_err();
            assert!(err.contains("escapes"), "{err}");
            assert!(!extract_to.join("bad/esc").is_symlink(), "link not created");
            assert!(!work.join("evil.txt").exists() && !work.join("x/evil.txt").exists());
            let _ = std::fs::remove_dir_all(&work);
        }
    }

    #[test]
    fn zip_file_is_not_written_through_a_link_to_outside() {
        // Even if a link to outside existed under dest, a file entry
        // beneath it must not be written through it.
        let work = tmp("zipthru");
        let outside = work.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let archive = work.join("t.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("esc/evil.txt", opts).unwrap();
            zip.write_all(b"owned").unwrap();
            zip.finish().unwrap();
        }
        let extract_to = work.join("x");
        std::fs::create_dir_all(extract_to.join("t")).unwrap();
        std::os::unix::fs::symlink(&outside, extract_to.join("t/esc")).unwrap();
        let mut ctx = test_ctx();
        // "t" exists, so extraction lands in "t 2" — a fresh folder.
        extract_op(&mut ctx, vec![archive], extract_to.clone()).unwrap();
        assert!(!outside.join("evil.txt").exists());
        assert_eq!(
            std::fs::read(extract_to.join("t 2/esc/evil.txt")).unwrap(),
            b"owned"
        );
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn tar_symlink_escaping_dest_is_refused() {
        let work = tmp("taresc");
        let archive = work.join("bad.tar");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut tar = tar::Builder::new(file);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            tar.append_link(&mut header, "esc", "../../..").unwrap();
            tar.finish().unwrap();
        }
        let extract_to = work.join("x");
        let mut ctx = test_ctx();
        let err = extract_op(&mut ctx, vec![archive], extract_to.clone()).unwrap_err();
        assert!(err.contains("escapes"), "{err}");
        assert!(!extract_to.join("bad/esc").is_symlink());
        let _ = std::fs::remove_dir_all(&work);
    }

    /// Two incompressible files, big enough that progress lands between
    /// the start and the end of every format.
    fn noisy_tree(work: &Path) -> PathBuf {
        let src = work.join("proj");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        let mut x: u32 = 1;
        let mut noise = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    x as u8
                })
                .collect()
        };
        std::fs::write(src.join("a.bin"), noise(3 << 20)).unwrap();
        std::fs::write(src.join("sub/b.bin"), noise(2 << 20)).unwrap();
        src
    }

    fn archives(work: &Path) -> Vec<PathBuf> {
        let src = noisy_tree(work);
        let mut out = Vec::new();
        for (name, fmt) in [
            ("t.zip", ArchiveFormat::Zip),
            ("t.tar", ArchiveFormat::Tar),
            ("t.tar.gz", ArchiveFormat::TarGz),
        ] {
            let dest = work.join(name);
            compress_op(&mut test_ctx(), vec![src.clone()], dest.clone(), fmt).unwrap();
            out.push(dest);
        }
        let dest = work.join("a.bin.gz");
        compress_op(&mut test_ctx(), vec![src.join("a.bin")], dest.clone(), ArchiveFormat::Gz)
            .unwrap();
        out.push(dest);
        let dest = work.join("t.7z");
        sevenz_rust2::compress_to_path(&src, &dest).unwrap();
        out.push(dest);
        out
    }

    #[test]
    fn extract_progress_counts_archive_bytes_for_every_format() {
        let work = tmp("prog");
        for archive in archives(&work) {
            let len = archive.metadata().unwrap().len();
            let (mut ctx, seen) = OpCtx::recording(None);
            let out = work.join(format!("x_{}", archive.file_name().unwrap().to_string_lossy()));
            extract_op(&mut ctx, vec![archive.clone()], out).unwrap();
            let seen = seen.lock().unwrap();
            let name = archive.display();
            assert!(seen.iter().all(|&(_, t)| t == len), "{name}: total is the archive size");
            assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0), "{name}: never goes back");
            assert_eq!(seen.last().unwrap().0, len, "{name}: ends at 100%");
            assert!(
                seen.iter().any(|&(d, _)| d > 0 && d < len),
                "{name}: reports progress along the way"
            );
        }
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn extract_progress_spans_several_archives() {
        let work = tmp("prog_multi");
        let all = archives(&work);
        let lens: Vec<u64> = all.iter().map(|a| a.metadata().unwrap().len()).collect();
        let (mut ctx, seen) = OpCtx::recording(None);
        extract_op(&mut ctx, all, work.join("x")).unwrap();
        let seen = seen.lock().unwrap();
        let total: u64 = lens.iter().sum();
        assert!(seen.iter().all(|&(_, t)| t == total));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
        // Each archive boundary is reached exactly.
        let mut edge = 0;
        for len in lens {
            edge += len;
            assert!(seen.iter().any(|&(d, _)| d == edge), "boundary {edge}");
        }
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn extract_cancels_midway_in_every_format() {
        let work = tmp("prog_cancel");
        for archive in archives(&work) {
            let len = archive.metadata().unwrap().len();
            let (mut ctx, _) = OpCtx::recording(Some(len / 3));
            let out = work.join(format!("c_{}", archive.file_name().unwrap().to_string_lossy()));
            let err = extract_op(&mut ctx, vec![archive.clone()], out).unwrap_err();
            assert_eq!(err, cancelled_err(), "{}", archive.display());
        }
        let _ = std::fs::remove_dir_all(&work);
    }

    /// One big entry: progress must move, and a cancel must land, while
    /// unpack_in is still copying that single file.
    #[test]
    fn tar_progress_and_cancel_inside_one_big_entry() {
        let work = tmp("tar_one");
        let src = work.join("big.bin");
        let mut x: u32 = 7;
        let data: Vec<u8> = (0..(6 << 20))
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        std::fs::write(&src, &data).unwrap();
        for (name, fmt) in [("one.tar", ArchiveFormat::Tar), ("one.tar.gz", ArchiveFormat::TarGz)] {
            let archive = work.join(name);
            compress_op(&mut test_ctx(), vec![src.clone()], archive.clone(), fmt).unwrap();
            let len = archive.metadata().unwrap().len();

            let (mut ctx, seen) = OpCtx::recording(None);
            extract_op(&mut ctx, vec![archive.clone()], work.join(format!("ok_{name}"))).unwrap();
            let inside = seen
                .lock()
                .unwrap()
                .iter()
                .filter(|&&(d, _)| d > len / 10 && d < len * 9 / 10)
                .count();
            assert!(inside >= 3, "{name}: only {inside} reports inside the entry");

            let (mut ctx, _) = OpCtx::recording(Some(len / 2));
            let out = work.join(format!("cancel_{name}"));
            let err = extract_op(&mut ctx, vec![archive], out.clone()).unwrap_err();
            assert_eq!(err, cancelled_err(), "{name}");
            let written = std::fs::metadata(out.join("big.bin")).map(|m| m.len()).unwrap_or(0);
            assert!(written < data.len() as u64, "{name}: stopped mid-file ({written})");
        }
        let _ = std::fs::remove_dir_all(&work);
    }

    /// Every symlink under `root`, resolved by the OS, must land inside it.
    fn assert_no_link_escapes(root: &Path) {
        let canon = root.canonicalize().unwrap();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                let meta = std::fs::symlink_metadata(&p).unwrap();
                if meta.is_symlink() {
                    // Resolve the link's directory for real, then the target
                    // the way the kernel would (missing tails allowed).
                    let real_dir = p.parent().unwrap().canonicalize().unwrap();
                    let target = std::fs::read_link(&p).unwrap();
                    let mut at = real_dir.clone();
                    for c in target.components() {
                        match c {
                            std::path::Component::ParentDir => {
                                at.pop();
                            }
                            std::path::Component::Normal(n) => {
                                at.push(n);
                                if let Ok(r) = at.canonicalize() {
                                    at = r;
                                }
                            }
                            std::path::Component::RootDir => at = PathBuf::from("/"),
                            _ => {}
                        }
                    }
                    assert!(at.starts_with(&canon), "{} -> {} lands at {}", p.display(), target.display(), at.display());
                } else if meta.is_dir() {
                    stack.push(p);
                }
            }
        }
    }

    fn zip_of_links(path: &Path, links: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, target) in links {
            zip.add_symlink(*name, *target, opts).unwrap();
        }
        zip.finish().unwrap();
    }

    fn tar_of_links(path: &Path, links: &[(&str, &str)]) {
        let mut tar = tar::Builder::new(std::fs::File::create(path).unwrap());
        for (name, target) in links {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            tar.append_link(&mut header, name, target).unwrap();
        }
        tar.finish().unwrap();
    }

    /// Links that only look inside when resolved by text: one created
    /// through a link-to-parent folder, and a target that climbs after
    /// passing through such a link.
    #[test]
    fn chained_symlinks_cannot_escape() {
        let chains: [&[(&str, &str)]; 2] = [
            &[("a/b", ".."), ("a/b/esc", "..")],
            &[("a/b", ".."), ("x", "a/b/../..")],
        ];
        for (i, links) in chains.iter().enumerate() {
            let work = tmp(&format!("chain{i}"));
            for (kind, archive) in [("zip", work.join("t.zip")), ("tar", work.join("t.tar"))] {
                if kind == "zip" {
                    zip_of_links(&archive, links);
                } else {
                    tar_of_links(&archive, links);
                }
                let out = work.join(format!("x_{kind}"));
                let result = extract_op(&mut test_ctx(), vec![archive], out.clone());
                assert!(result.is_err(), "chain {i} {kind}: extraction should refuse");
                assert_no_link_escapes(&out);
            }
            let _ = std::fs::remove_dir_all(&work);
        }
    }

    #[test]
    fn ordinary_relative_links_still_extract() {
        let work = tmp("links_ok");
        let links: &[(&str, &str)] = &[
            ("lib/libfoo.so", "libfoo.so.1"),
            ("bin/tool", "../lib/libfoo.so"),
            ("share/doc/a/link", "../../../bin/tool"),
            ("self", "."),
        ];
        for (kind, archive) in [("zip", work.join("t.zip")), ("tar", work.join("t.tar"))] {
            if kind == "zip" {
                zip_of_links(&archive, links);
            } else {
                tar_of_links(&archive, links);
            }
            let out = work.join(format!("x_{kind}"));
            extract_op(&mut test_ctx(), vec![archive], out.clone()).unwrap();
            assert!(out.join("t/bin/tool").is_symlink(), "{kind}");
            assert!(out.join("t/share/doc/a/link").is_symlink(), "{kind}");
            assert_no_link_escapes(&out);
        }
        let _ = std::fs::remove_dir_all(&work);
    }
}
