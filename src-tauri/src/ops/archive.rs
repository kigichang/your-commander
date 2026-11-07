use super::queue::{cancelled_err, keep_both_path, OpCtx};
use super::safe_path::{create_dir_inside, create_file_nofollow, link_inside, symlink_inside};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

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
        if path.is_dir() && !path.is_symlink() {
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
        .filter(|(_, p, is_dir)| !is_dir && !p.is_symlink())
        .map(|(_, p, _)| p.metadata().map(|m| m.len()).unwrap_or(0))
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

/// Extract each archive into its own folder under `dest_dir`. Returns the
/// number of archives extracted.
pub fn extract_op(
    ctx: &mut OpCtx,
    archives: Vec<PathBuf>,
    dest_dir: PathBuf,
) -> Result<usize, String> {
    let mut count = 0usize;
    for archive in &archives {
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
        match kind {
            Extractable::Zip => extract_zip(ctx, archive, &target)?,
            Extractable::SevenZ => {
                sevenz_rust2::decompress_file(archive, &target).map_err(|e| e.to_string())?;
            }
            Extractable::TarGz => {
                let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
                let dec = flate2::read::GzDecoder::new(file);
                extract_tar(ctx, dec, &target)?;
            }
            Extractable::Tar => {
                let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
                extract_tar(ctx, file, &target)?;
            }
            Extractable::Gz => {
                // Single gzip-compressed file: decompress next to the others.
                std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
                let out = target.join(stem);
                let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
                let mut dec = flate2::read::GzDecoder::new(file);
                let mut writer = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; CHUNK];
                loop {
                    if ctx.cancelled() {
                        return Err(cancelled_err());
                    }
                    let n = dec.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    writer.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                }
            }
        }
        count += 1;
    }
    Ok(count)
}

fn extract_zip(ctx: &mut OpCtx, archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let total = zip.len();
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    // Directories are created and files written only where the real
    // (symlink-resolved) path is still under this root.
    let root = dest.canonicalize().map_err(|e| e.to_string())?;
    for i in 0..total {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name guards against zip-slip path escapes.
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let out = dest.join(&rel);
        ctx.done_bytes = i as u64;
        ctx.total_bytes = total as u64;
        ctx.report(&rel.to_string_lossy());
        if entry.is_dir() {
            create_dir_inside(&root, &out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            create_dir_inside(&root, parent)?;
        }
        if entry.is_symlink() {
            // The entry's data is the link target, verbatim. A target that
            // leaves `dest` is refused so later entries cannot write
            // through it.
            let mut target = String::new();
            entry
                .read_to_string(&mut target)
                .map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&out);
            symlink_inside(dest, &out, &target)?;
            continue;
        }
        let mut writer = create_file_nofollow(&out)?;
        std::io::copy(&mut entry, &mut writer).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

fn extract_tar<R: Read>(ctx: &mut OpCtx, reader: R, dest: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(reader);
    let entries = tar.entries().map_err(|e| e.to_string())?;
    for entry in entries {
        if ctx.cancelled() {
            return Err(cancelled_err());
        }
        let mut entry = entry.map_err(|e| e.to_string())?;
        if let Ok(path) = entry.path() {
            ctx.report(&path.to_string_lossy());
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
            if link_inside(dest, &link, &target.to_string_lossy()).is_none() {
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
}
