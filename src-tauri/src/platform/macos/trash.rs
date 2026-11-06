//! Finder's "Put Back" records. When Finder trashes an item (the trash
//! crate's default method does it through Finder), it stores the original
//! folder (`ptbL`, relative to the volume root) and name (`ptbN`) under the
//! item's Trash name in the Trash's .DS_Store. This is a read-only parser
//! of that file's B-tree ("Bud1" buddy-allocator format).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

struct Reader<'a> {
    d: &'a [u8],
}

impl<'a> Reader<'a> {
    fn bytes(&self, at: usize, n: usize) -> Option<&'a [u8]> {
        self.d.get(at..at.checked_add(n)?)
    }
    fn u32(&self, at: usize) -> Option<u32> {
        Some(u32::from_be_bytes(self.bytes(at, 4)?.try_into().ok()?))
    }
    fn utf16(&self, at: usize, chars: usize) -> Option<String> {
        let raw = self.bytes(at, chars.checked_mul(2)?)?;
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&units).ok()
    }
}

/// One record: (file name, code, value if it is a `ustr`), and the offset
/// just past it.
fn record(r: &Reader, at: usize) -> Option<(String, [u8; 4], Option<String>, usize)> {
    let len = r.u32(at)? as usize;
    let name = r.utf16(at + 4, len)?;
    let mut o = at + 4 + len * 2;
    let code: [u8; 4] = r.bytes(o, 4)?.try_into().ok()?;
    let kind = r.bytes(o + 4, 4)?;
    o += 8;
    let mut text = None;
    match kind {
        b"long" | b"shor" | b"type" => o += 4,
        b"bool" => o += 1,
        b"comp" | b"dutc" => o += 8,
        b"blob" => o += 4 + r.u32(o)? as usize,
        b"ustr" => {
            let n = r.u32(o)? as usize;
            text = Some(r.utf16(o + 4, n)?);
            o += 4 + n * 2;
        }
        _ => return None,
    }
    Some((name, code, text, o))
}

/// Trash name → (ptbL, ptbN) for every record in a .DS_Store. None when
/// the file isn't one or is damaged.
fn put_back_records(data: &[u8]) -> Option<HashMap<String, (Option<String>, Option<String>)>> {
    let r = Reader { d: data };
    if r.bytes(4, 4)? != b"Bud1" {
        return None;
    }
    // Every offset in the file is relative to byte 4.
    let root = 4 + r.u32(8)? as usize;
    let nblocks = r.u32(root)? as usize;
    let addr = |i: usize| -> Option<usize> {
        if i >= nblocks {
            return None;
        }
        Some(4 + (r.u32(root + 8 + 4 * i)? & !0x1f) as usize)
    };
    // Block addresses are stored in pages of 256.
    let mut o = root + 8 + 4 * nblocks.div_ceil(256) * 256;
    let ndirs = r.u32(o)?;
    o += 4;
    let mut dsdb = None;
    for _ in 0..ndirs {
        let len = *r.d.get(o)? as usize;
        let name = r.bytes(o + 1, len)?;
        let block = r.u32(o + 1 + len)? as usize;
        if name == b"DSDB" {
            dsdb = Some(block);
        }
        o += 1 + len + 4;
    }
    let root_node = r.u32(addr(dsdb?)?)? as usize;

    let mut out: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    // Depth-first over the B-tree; a node visited twice means a loop.
    let mut stack = vec![root_node];
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            return None;
        }
        let mut o = addr(node)?;
        let rightmost = r.u32(o)? as usize;
        let count = r.u32(o + 4)?;
        o += 8;
        for _ in 0..count {
            if rightmost != 0 {
                stack.push(r.u32(o)? as usize);
                o += 4;
            }
            let (name, code, text, next) = record(&r, o)?;
            o = next;
            match &code {
                b"ptbL" => out.entry(name).or_default().0 = text,
                b"ptbN" => out.entry(name).or_default().1 = text,
                _ => {}
            }
        }
        if rightmost != 0 {
            stack.push(rightmost);
        }
    }
    Some(out)
}

/// `ptbL` is relative to the volume root, and on the boot volume that root
/// is the Data volume ("System/Volumes/Data/Users/…"). Map it back to the
/// firmlinked path users know ("/Users/…") when that one exists.
fn from_volume_root(folder: &str) -> PathBuf {
    let rel = folder.trim_start_matches('/');
    if let Some(rest) = rel.strip_prefix("System/Volumes/Data/") {
        let short = Path::new("/").join(rest);
        let top = rest.split('/').next().unwrap_or("");
        if !top.is_empty() && Path::new("/").join(top).exists() {
            return short;
        }
    }
    Path::new("/").join(rel)
}

/// Where `item` (a file directly in ~/.Trash) was before Finder trashed it.
pub fn original_location(item: &Path) -> Result<PathBuf, String> {
    let trash = item.parent().ok_or("not in the Trash")?;
    let name = item
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("not in the Trash")?;
    let data = std::fs::read(trash.join(".DS_Store")).map_err(|_| no_record(name))?;
    let records = put_back_records(&data).ok_or_else(|| no_record(name))?;
    match records.get(name) {
        Some((Some(folder), orig)) => {
            Ok(from_volume_root(folder).join(orig.as_deref().unwrap_or(name)))
        }
        _ => Err(no_record(name)),
    }
}

fn no_record(name: &str) -> String {
    format!("{name}: original location unknown (it wasn't moved to the Trash by Finder)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(s: &str) -> Vec<u8> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut v = (units.len() as u32).to_be_bytes().to_vec();
        for u in units {
            v.extend(u.to_be_bytes());
        }
        v
    }

    fn ustr_record(name: &str, code: &[u8; 4], value: &str) -> Vec<u8> {
        let mut v = utf16(name);
        v.extend(code);
        v.extend(b"ustr");
        v.extend(utf16(value));
        v
    }

    /// A minimal one-leaf .DS_Store laid out the way Finder writes them.
    fn ds_store(records: &[Vec<u8>]) -> Vec<u8> {
        let mut f = vec![0u8; 0x1000 + 0x800];
        f[0..4].copy_from_slice(&1u32.to_be_bytes());
        f[4..8].copy_from_slice(b"Bud1");
        let root_off = 0x800u32; // relative to byte 4
        f[8..12].copy_from_slice(&root_off.to_be_bytes());
        f[12..16].copy_from_slice(&0x800u32.to_be_bytes());
        f[16..20].copy_from_slice(&root_off.to_be_bytes());
        // Blocks: 0 = DSDB header at 0x20, 1 = leaf at 0x1000 (size 2^12).
        let root = 4 + root_off as usize;
        let mut w = |at: usize, v: u32| f[at..at + 4].copy_from_slice(&v.to_be_bytes());
        w(root, 2);
        w(root + 8, 0x20 | 5);
        w(root + 12, 0x1000 | 12);
        let dirs = root + 8 + 4 * 256;
        w(dirs, 1);
        f[dirs + 4] = 4;
        f[dirs + 5..dirs + 9].copy_from_slice(b"DSDB");
        f[dirs + 9..dirs + 13].copy_from_slice(&1u32.to_be_bytes());
        // DSDB header → root node is block 1.
        f[4 + 0x20..4 + 0x24].copy_from_slice(&1u32.to_be_bytes());
        // Leaf node.
        let leaf = 4 + 0x1000;
        f.resize(leaf + 0x1000, 0);
        f[leaf + 4..leaf + 8].copy_from_slice(&(records.len() as u32).to_be_bytes());
        let mut o = leaf + 8;
        for r in records {
            f[o..o + r.len()].copy_from_slice(r);
            o += r.len();
        }
        f
    }

    #[test]
    fn reads_put_back_records() {
        let data = ds_store(&[
            ustr_record("a 12-00-01.txt", b"ptbL", "Users/u/Docs/"),
            ustr_record("a 12-00-01.txt", b"ptbN", "a.txt"),
            ustr_record("中文.zip", b"ptbL", "System/Volumes/Data/Users/u/"),
        ]);
        let recs = put_back_records(&data).unwrap();
        assert_eq!(
            recs["a 12-00-01.txt"],
            (Some("Users/u/Docs/".into()), Some("a.txt".into()))
        );
        assert_eq!(recs["中文.zip"].0.as_deref(), Some("System/Volumes/Data/Users/u/"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(put_back_records(b"not a ds_store file").is_none());
        let mut data = ds_store(&[ustr_record("a", b"ptbL", "x/")]);
        data.truncate(0x900);
        assert!(put_back_records(&data).is_none());
    }

    #[test]
    fn maps_data_volume_paths_to_firmlinks() {
        assert_eq!(
            from_volume_root("System/Volumes/Data/Users/u/"),
            PathBuf::from("/Users/u/")
        );
        assert_eq!(from_volume_root("Users/u/"), PathBuf::from("/Users/u/"));
        assert_eq!(
            from_volume_root("System/Volumes/Data/no-such-top/x/"),
            PathBuf::from("/System/Volumes/Data/no-such-top/x/")
        );
    }
}
