use super::queue::{cancelled_err, keep_both_path, OpCtx};
use super::safe_path::{
    create_dir_inside, create_file_nofollow, create_new_file, link_inside, symlink_inside,
};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const CHUNK: usize = 1024 * 1024;
/// Longest symlink target read out of an archive (PATH_MAX on Linux).
const LINK_TARGET_MAX: u64 = 4096;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    TarGz,
    Gz,
}

/// macOS noise that must never end up inside an archive.
fn excluded(name: &str) -> bool {
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
    // Compared as ASCII on the name's own bytes: lowercasing the whole
    // name can change its length, and the stem is cut by suffix length.
    let bytes = name.as_bytes();
    SUFFIXES
        .iter()
        .find(|(suffix, _)| {
            bytes.len() >= suffix.len()
                && bytes[bytes.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
        })
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
    // exists() follows links: a dangling one would raise no conflict, and the
    // archive would then be written to wherever it points.
    if dest.exists() || dest.is_symlink() {
        return Err(format!("\"{}\" already exists", dest.to_string_lossy()));
    }
    if format == ArchiveFormat::Gz && (sources.len() != 1 || sources[0].is_dir()) {
        return Err(".gz can only compress a single file".to_string());
    }
    // What goes in is listed before the archive file exists. The
    // destination may lie inside a source folder (the other panel standing
    // in a subfolder of the selection): listed afterwards, the archive
    // would be among its own contents, reading itself as it writes itself
    // until the disk is full. A listing that fails leaves no file behind.
    let tree = match format {
        ArchiveFormat::Gz => Vec::new(),
        _ => collect_tree(&sources)?,
    };
    // Created here, exclusively: whatever takes the name after the check
    // above fails the open instead of being truncated, and from here on
    // the file is ours to remove if the archive cannot be finished.
    let file = create_new_file(&dest)?;

    if format == ArchiveFormat::Gz {
        let result = gz_file(ctx, &sources[0], file);
        if result.is_err() {
            let _ = std::fs::remove_file(&dest);
        }
        result?;
        return Ok(dest
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string());
    }

    ctx.total_bytes = tree
        .iter()
        .filter(|(_, _, is_dir)| !is_dir)
        .filter_map(|(_, p, _)| p.symlink_metadata().ok())
        .filter(|m| !m.is_symlink())
        .map(|m| m.len())
        .sum();

    let result = match format {
        ArchiveFormat::Zip => write_zip(ctx, &tree, file),
        ArchiveFormat::Tar => write_tar(ctx, &tree, file).map(drop),
        ArchiveFormat::TarGz => {
            let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            // finish() writes the gzip trailer. Dropping the encoder would
            // try the same and throw the error away: a full disk would
            // leave a cut-off archive reported as done.
            write_tar(ctx, &tree, enc)
                .and_then(|enc| enc.finish().map(drop).map_err(|e| e.to_string()))
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
    file: std::fs::File,
) -> Result<(), String> {
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    // One buffer for the whole archive, not one per file.
    let mut buf = vec![0u8; CHUNK];

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

/// Returns the writer once the tar's end has been written to it, for a
/// compressing writer to be finished by the caller.
fn write_tar<W: Write>(
    ctx: &mut OpCtx,
    tree: &[(String, PathBuf, bool)],
    writer: W,
) -> Result<W, String> {
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
    tar.into_inner().map_err(|e| e.to_string())
}

/// Gzip `src` into `file`. The caller removes the file on any error.
fn gz_file(ctx: &mut OpCtx, src: &Path, file: std::fs::File) -> Result<(), String> {
    ctx.total_bytes = src.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let mut enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut buf = vec![0u8; CHUNK];
    loop {
        if ctx.cancelled() {
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
    Ok(())
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
        // An archive named just ".zip", "..zip" or "...zip" leaves no
        // usable folder name ("", "." and ".." all name the destination or
        // its parent, not something in it).
        let stem = if super::safe_path::is_listing_name(stem) { stem } else { "archive" };
        let mut target = dest_dir.join(stem);
        // lstat: a dangling link takes the name too.
        if target.symlink_metadata().is_ok() {
            target = keep_both_path(&target);
        }

        ctx.report(&name);
        let detail = Rc::new(RefCell::new(name.clone()));
        let quarantine = Quarantine::of(archive);
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
        // `target` is this archive's own fresh folder. Flagged whether or
        // not extraction got to the end: what was written stays behind.
        quarantine.mark_tree(&target);
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
    // The sizes are the archive's own claims: add them without overflow.
    let total = (0..zip.len())
        .filter_map(|i| zip.by_index_data(i).ok().map(|e| e.size()))
        .fold(0u64, u64::saturating_add);
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
            // No real target is longer than a path can be; an entry that
            // claims to be a link can still deflate to gigabytes.
            let mut target = String::new();
            entry
                .by_ref()
                .take(LINK_TARGET_MAX + 1)
                .read_to_string(&mut target)
                .map_err(|e| e.to_string())?;
            if target.len() as u64 > LINK_TARGET_MAX {
                return Err(format!("{shown}: symlink target too long"));
            }
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
        apply_mode(&writer, mode);
    }
    Ok(())
}

/// Give a file just written from an archive the permission bits the
/// archive recorded (the executable bit of a script, say). Only rwx: an
/// archive's setuid/setgid/sticky bits are dropped, as tar's unpack does
/// by default. Used by full extraction and by F5 copy-out of entries.
/// Set on the open file, not by its path: the name could be swapped for
/// a link to something else in between.
pub(crate) fn apply_mode(file: &std::fs::File, mode: Option<u32>) {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(mode & 0o777));
    }
}

/// The macOS quarantine flag of an archive (set by the browser or mail
/// client that downloaded it), handed on to whatever is taken out of it, as
/// Archive Utility does. Without it an extracted app or script would skip
/// the Gatekeeper check the download itself was subject to. Best effort,
/// and nothing at all where the archive carries no flag (always on Linux).
pub(crate) struct Quarantine(Option<Vec<u8>>);

impl Quarantine {
    const ATTR: &str = "com.apple.quarantine";

    pub fn of(archive: &Path) -> Self {
        Self(xattr::get(archive, Self::ATTR).ok().flatten())
    }

    /// Flag one file or folder. The system refuses an attribute on what
    /// the owner cannot write, and an archive chooses its entries' modes:
    /// a read-only app or script would otherwise come out unflagged. Such
    /// an entry is made writable for the moment it takes.
    fn mark(&self, path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let Some(value) = &self.0 else { return };
        if xattr::set(path, Self::ATTR, value).is_ok() {
            return;
        }
        let Ok(meta) = path.symlink_metadata() else {
            return;
        };
        if meta.is_symlink() {
            return;
        }
        let mode = meta.permissions().mode();
        let writable = std::fs::Permissions::from_mode(mode | 0o200);
        if std::fs::set_permissions(path, writable).is_ok() {
            let _ = xattr::set(path, Self::ATTR, value);
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
        }
    }

    /// Flag `path` and everything under it. Links are left alone: setting
    /// the attribute would follow them. A folder the owner cannot list or
    /// enter is opened up while its contents are flagged, then put back.
    pub fn mark_tree(&self, path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let Ok(meta) = path.symlink_metadata() else {
            return;
        };
        if self.0.is_none() || meta.is_symlink() {
            return;
        }
        self.mark(path);
        if meta.is_dir() {
            let mode = meta.permissions().mode();
            let closed = mode & 0o700 != 0o700;
            if closed {
                let _ = std::fs::set_permissions(
                    path,
                    std::fs::Permissions::from_mode(mode | 0o700),
                );
            }
            for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
                self.mark_tree(&entry.path());
            }
            if closed {
                let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
            }
        }
    }

    /// Flag `path` and the folders leading to it below `root` (an app is a
    /// folder, and its flag is the one that counts).
    pub fn mark_below(&self, root: &Path, path: &Path) {
        if self.0.is_none() {
            return;
        }
        for p in path.ancestors().take_while(|p| *p != root && p.starts_with(root)) {
            self.mark(p);
        }
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
    let total = sevenz_rust2::Archive::open(archive)
        .map_err(|e| e.to_string())?
        .files
        .iter()
        .map(|f| f.size)
        .fold(0u64, u64::saturating_add);
    let mut read = 0u64;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let root = dest.canonicalize().map_err(|e| e.to_string())?;
    let refuse = |e: String| sevenz_rust2::Error::Other(e.into());
    // The crate joins each entry name safely, but its default writer follows
    // a link at the destination: as with zip, write in the entry's real
    // folder, checked to be inside root, and never through a link.
    sevenz_rust2::decompress_file_with_extract_fn(archive, dest, |entry, reader, path| {
        // Empty files and folders never reach the metered reader, which is
        // where a cancel is otherwise noticed.
        if ctx.cancelled() {
            return Err(refuse(cancelled_err()));
        }
        if entry.is_directory() {
            create_dir_inside(&root, path).map_err(refuse)?;
            return Ok(true);
        }
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return Ok(true);
        };
        let out = create_dir_inside(&root, parent).map_err(refuse)?.join(name);
        let mut writer = create_file_nofollow(&out).map_err(refuse)?;
        if entry.size() > 0 {
            let mut meter = Meter {
                inner: reader,
                ctx: &mut *ctx,
                base,
                len,
                read,
                total,
                detail: Rc::new(RefCell::new(entry.name().to_string())),
            };
            std::io::copy(&mut meter, &mut writer)?;
            read = meter.read;
            let times = std::fs::FileTimes::new()
                .set_accessed(entry.access_date().into())
                .set_modified(entry.last_modified_date().into());
            #[cfg(target_os = "macos")]
            let times = {
                use std::os::macos::fs::FileTimesExt;
                times.set_created(entry.creation_date().into())
            };
            let _ = writer.set_times(times);
        }
        Ok(true)
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
        // Files, folders and (checked below) symlinks only. A tar can also
        // carry FIFOs and device nodes, which unpack_in would create: a
        // FIFO in the tree blocks whatever reads it next (preview, search,
        // compress). These are left out; tar's own bookkeeping entries
        // pass through.
        let kind = entry.header().entry_type();
        if matches!(
            kind,
            tar::EntryType::Fifo | tar::EntryType::Char | tar::EntryType::Block
        ) {
            continue;
        }
        // A hard link is a second name for a file the archive already
        // unpacked, named from the archive's root. One whose target is not
        // a file inside `dest` is an error, never a silent skip: it would
        // give something outside (or nothing at all) a name in here.
        // unpack_in checks this as well (tar 0.4.46, covered by a test);
        // this check does not depend on it.
        if kind == tar::EntryType::Link {
            let target = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let inside = dest
                .join(&target)
                .canonicalize()
                .is_ok_and(|real| real.starts_with(&root));
            if !inside {
                return Err(format!(
                    "{}: hard link to {} does not lead to a file in the destination folder",
                    detail.borrow(),
                    target.display()
                ));
            }
        }
        // unpack_in refuses paths that escape `dest` and resolves the real
        // parent before writing, but creates symlinks with any target:
        // refuse those that point outside `dest`.
        if kind.is_symlink() {
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

    /// A read-only entry, or one inside a folder the owner cannot open,
    /// still gets the archive's quarantine flag, and keeps its mode.
    #[cfg(target_os = "macos")]
    #[test]
    fn quarantine_reaches_read_only_entries() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmp("quarantine_ro");
        let closed = dir.join("closed");
        std::fs::create_dir(&closed).unwrap();
        let file = closed.join("run.sh");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        let set = |p: &Path, m| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m));
        set(&file, 0o555).unwrap();
        set(&closed, 0o111).unwrap();

        let flag = b"0081;00000000;Safari;".to_vec();
        Quarantine(Some(flag.clone())).mark_tree(&dir);

        let mode = |p: &Path| p.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&closed), 0o111);
        set(&closed, 0o755).unwrap();
        assert_eq!(mode(&file), 0o555);
        assert_eq!(xattr::get(&file, Quarantine::ATTR).unwrap(), Some(flag.clone()));
        assert_eq!(xattr::get(&closed, Quarantine::ATTR).unwrap(), Some(flag));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn archive_with_no_stem_extracts_inside_the_destination() {
        let dir = tmp("no_stem");
        let src = dir.join("src");
        let dest = dir.join("dest");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("a.txt"), "a").unwrap();
        for name in [".zip", "..zip", "...zip"] {
            let archive = dir.join(name);
            compress_op(&mut test_ctx(), vec![src.join("a.txt")], archive.clone(), ArchiveFormat::Zip)
                .unwrap();
            extract_op(&mut test_ctx(), vec![archive], dest.clone()).unwrap();
        }
        // Everything landed under dest; nothing beside it.
        let beside: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| !n.ends_with("zip") && n != "src" && n != "dest")
            .collect();
        assert!(beside.is_empty(), "{beside:?}");
        assert!(dest.join("archive/a.txt").is_file());
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// One entry of a hand-built tar: a regular file with `data`, or with
    /// `link` a FIFO / hard link pointing at it.
    fn tar_entry<W: Write>(
        tar: &mut tar::Builder<W>,
        name: &str,
        kind: tar::EntryType,
        link: Option<&str>,
        data: &[u8],
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        if let Some(link) = link {
            header.set_link_name(link).unwrap();
        }
        header.set_path(name).unwrap();
        header.set_cksum();
        tar.append(&header, data).unwrap();
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn tar_special_entries_are_not_created() {
        let dir = tmp("tar_special");
        let archive = dir.join("odd.tar");
        let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
        tar_entry(&mut tar, "pipe", tar::EntryType::Fifo, None, b"");
        tar_entry(&mut tar, "ok.txt", tar::EntryType::Regular, None, b"ok");
        tar.into_inner().unwrap();

        let dest = dir.join("out");
        std::fs::create_dir(&dest).unwrap();
        extract_op(&mut test_ctx(), vec![archive], dest.clone()).unwrap();
        assert_eq!(names_in(&dest.join("odd")), ["ok.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A hard link to a file of the same archive is a file like any other:
    /// leaving it out would report success with files missing.
    #[test]
    fn tar_hard_links_inside_the_archive_are_extracted() {
        let dir = tmp("tar_hardlink");
        let archive = dir.join("linked.tar");
        let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
        tar_entry(&mut tar, "docs/a.txt", tar::EntryType::Regular, None, b"same bytes");
        // A hard link names its target from the archive's root.
        tar_entry(&mut tar, "docs/b.txt", tar::EntryType::Link, Some("docs/a.txt"), b"");
        tar.into_inner().unwrap();

        let dest = dir.join("out");
        std::fs::create_dir(&dest).unwrap();
        extract_op(&mut test_ctx(), vec![archive], dest.clone()).unwrap();
        let docs = dest.join("linked/docs");
        assert_eq!(names_in(&docs), ["a.txt", "b.txt"]);
        assert_eq!(std::fs::read(docs.join("a.txt")).unwrap(), b"same bytes");
        assert_eq!(std::fs::read(docs.join("b.txt")).unwrap(), b"same bytes");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A hard link to something outside the destination is refused with an
    /// error, not skipped: nothing outside gets a second name inside.
    #[test]
    fn tar_hard_links_leaving_the_destination_are_refused() {
        let dir = tmp("tar_hardlink_out");
        let outside = dir.join("outside");
        std::fs::write(&outside, b"not yours").unwrap();
        let dest = dir.join("out");
        std::fs::create_dir(&dest).unwrap();
        // The archive unpacks to out/<name>/, so "../../outside" is `outside`.
        for (i, target) in ["/etc/hosts", "../../outside", outside.to_str().unwrap()]
            .into_iter()
            .enumerate()
        {
            let archive = dir.join(format!("evil{i}.tar"));
            let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
            tar_entry(&mut tar, "grab", tar::EntryType::Link, Some(target), b"");
            tar.into_inner().unwrap();
            let result = extract_op(&mut test_ctx(), vec![archive], dest.clone());
            assert!(result.is_err(), "{target}: extracted without an error");
            let grab = dest.join(format!("evil{i}/grab"));
            assert!(grab.symlink_metadata().is_err(), "{target}: link was created");
        }
        use std::os::unix::fs::MetadataExt;
        assert_eq!(outside.metadata().unwrap().nlink(), 1, "outside got another name");
        assert_eq!(names_in(&dir), ["evil0.tar", "evil1.tar", "evil2.tar", "out", "outside"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compress_never_replaces_or_removes_what_holds_the_name() {
        let dir = tmp("compress_excl");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        let victim = dir.join("victim");
        let dest = dir.join("out.zip");
        std::os::unix::fs::symlink(&victim, &dest).unwrap();
        for format in [ArchiveFormat::Zip, ArchiveFormat::TarGz, ArchiveFormat::Gz] {
            let err = compress_op(&mut test_ctx(), vec![dir.join("a.txt")], dest.clone(), format);
            assert!(err.is_err());
            assert!(dest.is_symlink(), "the link is still there");
            assert!(!victim.exists(), "nothing was written through it");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The archive is written into a subfolder of what it archives (the
    /// other panel standing inside the selected folder). It must not find
    /// itself in the list of things to pack: reading itself while writing
    /// itself, it would grow until the disk was full.
    fn compress_into_own_source(format: ArchiveFormat, file_name: &str) {
        const SIZE: usize = 4 * 1024 * 1024;
        let dir = tmp(&format!("self_{file_name}"));
        let proj = dir.join("proj");
        std::fs::create_dir_all(proj.join("sub")).unwrap();
        // Incompressible, so the archive is about as big as the data.
        let mut x = 0x2545_F491_4F6C_DD1Du64;
        let noise: Vec<u8> = (0..SIZE)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect();
        std::fs::write(proj.join("data.bin"), &noise).unwrap();
        let dest = proj.join("sub").join(file_name);

        // A runaway is cancelled here instead of filling the disk.
        let (mut ctx, seen) = OpCtx::recording(Some(16 * SIZE as u64));
        compress_op(&mut ctx, vec![proj.clone()], dest.clone(), format).unwrap();
        let most = seen.lock().unwrap().iter().map(|(done, _)| *done).max().unwrap_or(0);
        assert!(most <= SIZE as u64, "processed {most} bytes of a {SIZE}-byte source");

        let names: Vec<String> = match format {
            ArchiveFormat::Zip => {
                let zip = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
                zip.file_names().map(|n| n.unwrap().into_owned()).collect()
            }
            _ => {
                let gz = flate2::read::GzDecoder::new(std::fs::File::open(&dest).unwrap());
                tar::Archive::new(gz)
                    .entries()
                    .unwrap()
                    .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
                    .collect()
            }
        };
        assert!(names.iter().any(|n| n.ends_with("data.bin")), "{names:?}");
        assert!(!names.iter().any(|n| n.ends_with(file_name)), "{names:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_source_that_cannot_be_listed_leaves_no_archive_behind() {
        let dir = tmp("unlistable");
        let dest = dir.join("out.zip");
        let err = compress_op(&mut test_ctx(), vec![PathBuf::from("/")], dest.clone(), ArchiveFormat::Zip);
        assert!(err.is_err());
        assert!(dest.symlink_metadata().is_err(), "an empty archive was left behind");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zip_written_inside_its_source_does_not_pack_itself() {
        compress_into_own_source(ArchiveFormat::Zip, "proj.zip");
    }

    #[test]
    fn tar_gz_written_inside_its_source_does_not_pack_itself() {
        compress_into_own_source(ArchiveFormat::TarGz, "proj.tar.gz");
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

    #[test]
    fn compress_refuses_a_name_held_by_a_dangling_link() {
        let work = tmp("dangling");
        let src = work.join("a.bin");
        std::fs::write(&src, b"data").unwrap();
        for (i, fmt) in [
            ArchiveFormat::Zip,
            ArchiveFormat::Tar,
            ArchiveFormat::TarGz,
            ArchiveFormat::Gz,
        ]
        .into_iter()
        .enumerate()
        {
            let victim = work.join(format!("victim{i}"));
            let dest = work.join(format!("out{i}"));
            std::os::unix::fs::symlink(&victim, &dest).unwrap();
            let err = compress_op(&mut test_ctx(), vec![src.clone()], dest.clone(), fmt)
                .unwrap_err();
            assert!(err.contains("already exists"), "{fmt:?}: {err}");
            assert!(!victim.exists(), "{fmt:?}: nothing created through the link");
            assert!(dest.is_symlink(), "{fmt:?}: the link is left alone");
        }
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn extract_7z_does_not_write_through_links() {
        let work = tmp("7z_links");
        let src = work.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"archive").unwrap();
        std::fs::write(src.join("sub/b.txt"), b"archive").unwrap();
        let archive = work.join("t.7z");
        sevenz_rust2::compress_to_path(&src, &archive).unwrap();
        let len = archive.metadata().unwrap().len();

        // A file link in the target: refused, its target untouched.
        let dest = work.join("dest1");
        std::fs::create_dir_all(&dest).unwrap();
        let victim = work.join("victim.txt");
        std::fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(&victim, dest.join("a.txt")).unwrap();
        assert!(extract_7z(&mut test_ctx(), &archive, &dest, 0, len).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");

        // A folder link leaving the target: refused, nothing written out there.
        let dest = work.join("dest2");
        let outside = work.join("outside");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, dest.join("sub")).unwrap();
        let err = extract_7z(&mut test_ctx(), &archive, &dest, 0, len).unwrap_err();
        assert!(err.contains("escapes"), "{err}");
        assert!(!outside.join("b.txt").exists());

        // A plain extract still restores contents.
        let dest = work.join("dest3");
        std::fs::create_dir_all(&dest).unwrap();
        extract_7z(&mut test_ctx(), &archive, &dest, 0, len).unwrap();
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"archive");
        assert_eq!(std::fs::read(dest.join("sub/b.txt")).unwrap(), b"archive");
        let _ = std::fs::remove_dir_all(&work);
    }
}
