use super::queue::{cancelled_err, keep_both_path, OpCtx};
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

/// Can this file be extracted by `extract_op`?
pub fn is_archive(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".zip")
        || lower.ends_with(".7z")
        || lower.ends_with(".tar")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".gz")
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
        let lower = name.to_lowercase();
        let stem = archive_stem(&name);
        let mut target = dest_dir.join(&stem);
        if target.exists() {
            target = keep_both_path(&target);
        }

        ctx.report(&name);
        if lower.ends_with(".zip") {
            extract_zip(ctx, archive, &target)?;
        } else if lower.ends_with(".7z") {
            sevenz_rust2::decompress_file(archive, &target).map_err(|e| e.to_string())?;
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
            let dec = flate2::read::GzDecoder::new(file);
            extract_tar(ctx, dec, &target)?;
        } else if lower.ends_with(".tar") {
            let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
            extract_tar(ctx, file, &target)?;
        } else if lower.ends_with(".gz") {
            // Single gzip-compressed file: decompress next to the others.
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            let out = target.join(&stem);
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
        } else {
            return Err(format!("Unknown archive format: {name}"));
        }
        count += 1;
    }
    Ok(count)
}

pub fn archive_stem(name: &str) -> String {
    let lower = name.to_lowercase();
    for suffix in [".tar.gz", ".tgz", ".tar", ".zip", ".7z", ".gz"] {
        if lower.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    name.to_string()
}

fn extract_zip(ctx: &mut OpCtx, archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let total = zip.len();
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
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
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut writer = std::fs::File::create(&out)
            .map_err(|e| format!("{}: {e}", out.display()))?;
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
        // unpack_in refuses paths that escape `dest`.
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
    fn archive_stem_strips_known_suffixes() {
        assert_eq!(archive_stem("a.tar.gz"), "a");
        assert_eq!(archive_stem("b.tgz"), "b");
        assert_eq!(archive_stem("c.zip"), "c");
        assert_eq!(archive_stem("d.7z"), "d");
        assert_eq!(archive_stem("Pic.JPG.gz"), "Pic.JPG");
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
        let _ = std::fs::remove_dir_all(&work);
    }
}
