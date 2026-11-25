//! Layer tar extraction.
//!
//! Layers are unpacked verbatim into a directory (whiteout marker files such
//! as `.wh.foo` and `.wh..wh..opq` are kept as plain empty files; they are
//! interpreted later when the rootfs is flattened). Extraction is
//! rootless-safe: directory modes are applied last so read-only directories do
//! not block their children, chown is only attempted as root, and device
//! nodes are skipped unless running as root.
//!
//! Safety: entry paths may not contain `..` or be absolute, and no parent
//! component of an entry may be a symlink, so a layer cannot write outside
//! the destination directory.

use crate::fsutil;
use hullr_core::{Error, Result};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use tar::EntryType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Zstd,
}

/// Decide the compression of a layer. The media type (`...tar+gzip`,
/// `...tar+zstd`, `...tar`) is the expected answer, but the magic bytes are
/// authoritative because registries sometimes mislabel layers; data with no
/// recognizable magic is treated as a plain tar.
pub fn detect_compression(_media_type: &str, magic: &[u8]) -> Compression {
    if magic.starts_with(&[0x1f, 0x8b]) {
        return Compression::Gzip;
    }
    if magic.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Compression::Zstd;
    }
    Compression::None
}

/// Open `path` and return a reader over the uncompressed tar stream.
pub fn open_uncompressed(path: &Path, media_type: &str) -> Result<Box<dyn Read + Send>> {
    let f = fs::File::open(path)?;
    let mut br = BufReader::with_capacity(256 * 1024, f);
    let magic = br.fill_buf()?.iter().take(4).copied().collect::<Vec<u8>>();
    Ok(match detect_compression(media_type, &magic) {
        Compression::Gzip => Box::new(flate2::bufread::MultiGzDecoder::new(br)),
        Compression::Zstd => Box::new(zstd::stream::read::Decoder::with_buffer(br)?),
        Compression::None => Box::new(br),
    })
}

/// Reader that hashes (and optionally copies) everything passing through.
struct Tap<'a, R> {
    inner: R,
    hasher: Sha256,
    tee: Option<&'a mut dyn Write>,
}

impl<R: Read> Read for Tap<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        if let Some(t) = self.tee.as_mut() {
            t.write_all(&buf[..n])?;
        }
        Ok(n)
    }
}

/// Result of extracting a layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// `sha256:<hex>` over the uncompressed tar stream.
    pub diff_id: String,
    /// Sum of regular file sizes written.
    pub unpacked: u64,
}

/// Extract the layer at `src` into the (existing, empty) directory `dest`,
/// computing the diff_id over the whole uncompressed stream. When `tee` is
/// given, the uncompressed bytes are also written to it (used to compress a
/// layer while importing it).
pub fn extract_layer(src: &Path, media_type: &str, dest: &Path, tee: Option<&mut dyn Write>) -> Result<Extracted> {
    let reader = open_uncompressed(src, media_type)?;
    let mut tap = Tap { inner: reader, hasher: Sha256::new(), tee };
    let unpacked = extract_tar(&mut tap, dest)?;
    // The tar reader stops at the end-of-archive marker; hash the padding too.
    io::copy(&mut tap, &mut io::sink())?;
    Ok(Extracted { diff_id: format!("sha256:{}", hex::encode(tap.hasher.finalize())), unpacked })
}

/// Convert an archive path to a safe relative path: only normal components,
/// `.` ignored, `..` and absolute paths rejected.
pub fn safe_rel_path(p: &Path) -> Result<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(Error::Invalid(format!("unsafe path in layer (parent reference): {}", p.display())))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::Invalid(format!("unsafe path in layer (absolute): {}", p.display())))
            }
        }
    }
    Ok(out)
}

/// Make sure every parent directory of `rel` exists inside `dest` and is a
/// real directory (never a symlink). Returns the full target path.
fn prepare_parent(dest: &Path, rel: &Path) -> Result<PathBuf> {
    let mut cur = dest.to_path_buf();
    if let Some(parent) = rel.parent() {
        for c in parent.components() {
            cur.push(c);
            match fs::symlink_metadata(&cur) {
                Ok(md) if md.is_dir() => {}
                Ok(md) if md.file_type().is_symlink() => {
                    return Err(Error::Invalid(format!(
                        "unsafe layer: path {} traverses a symlink",
                        rel.display()
                    )))
                }
                Ok(_) => {
                    return Err(Error::Invalid(format!(
                        "unsafe layer: parent of {} is not a directory",
                        rel.display()
                    )))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&cur)?;
                    fsutil::chmod(&cur, 0o755)?;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    Ok(dest.join(rel))
}

struct DirMeta {
    path: PathBuf,
    depth: usize,
    mode: u32,
    mtime: i64,
    uid: u32,
    gid: u32,
}

/// Unpack an uncompressed tar stream into `dest`. Returns bytes of regular
/// file data written.
pub fn extract_tar<R: Read>(reader: R, dest: &Path) -> Result<u64> {
    let root = fsutil::is_root();
    let mut ar = tar::Archive::new(reader);
    let mut dirs: Vec<DirMeta> = Vec::new();
    let mut unpacked = 0u64;

    for entry in ar.entries()? {
        let mut entry = entry?;
        let raw_path = entry.path()?.into_owned();
        let rel = safe_rel_path(&raw_path)?;
        if rel.as_os_str().is_empty() {
            continue; // the "./" root entry
        }
        let full = prepare_parent(dest, &rel)?;
        let hdr = entry.header().clone();
        let mode = hdr.mode().unwrap_or(0o644);
        let mtime = hdr.mtime().unwrap_or(0) as i64;
        let uid = hdr.uid().unwrap_or(0) as u32;
        let gid = hdr.gid().unwrap_or(0) as u32;
        let ty = hdr.entry_type();

        let existing = fs::symlink_metadata(&full).ok();
        let replace = |existing: &Option<fs::Metadata>| -> Result<()> {
            if existing.is_some() {
                fsutil::remove_tree(&full)?;
            }
            Ok(())
        };

        match ty {
            EntryType::Directory => {
                match &existing {
                    Some(md) if md.is_dir() => {}
                    _ => {
                        replace(&existing)?;
                        fs::create_dir(&full)?;
                    }
                }
                // Stay writable until all children are in place.
                fsutil::chmod(&full, mode | 0o700)?;
                dirs.push(DirMeta { path: full, depth: rel.components().count(), mode, mtime, uid, gid });
            }
            EntryType::Regular | EntryType::Continuous | EntryType::GNUSparse => {
                replace(&existing)?;
                let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&full)?;
                unpacked += io::copy(&mut entry, &mut f)?;
                drop(f);
                fsutil::chmod(&full, mode)?;
                if root {
                    fsutil::lchown_quiet(&full, uid, gid);
                    // chown clears setuid/setgid bits; restore the mode.
                    fsutil::chmod(&full, mode)?;
                }
                fsutil::set_mtime(&full, mtime, false);
            }
            EntryType::Symlink => {
                replace(&existing)?;
                let target = entry
                    .link_name()?
                    .ok_or_else(|| Error::Invalid(format!("symlink {} without target", rel.display())))?
                    .into_owned();
                std::os::unix::fs::symlink(&target, &full)?;
                if root {
                    fsutil::lchown_quiet(&full, uid, gid);
                }
                fsutil::set_mtime(&full, mtime, true);
            }
            EntryType::Link => {
                let target = entry
                    .link_name()?
                    .ok_or_else(|| Error::Invalid(format!("hardlink {} without target", rel.display())))?
                    .into_owned();
                let target_rel = safe_rel_path(&target)?;
                let target_full = prepare_parent(dest, &target_rel)?;
                replace(&existing)?;
                fs::hard_link(&target_full, &full).map_err(|e| {
                    Error::Invalid(format!(
                        "hardlink {} -> {} failed: {e}",
                        rel.display(),
                        target_rel.display()
                    ))
                })?;
            }
            EntryType::Fifo => {
                replace(&existing)?;
                fsutil::mkfifo(&full, mode & 0o7777)?;
                if root {
                    fsutil::lchown_quiet(&full, uid, gid);
                }
                fsutil::set_mtime(&full, mtime, false);
            }
            EntryType::Char | EntryType::Block => {
                if !root {
                    continue;
                }
                replace(&existing)?;
                let major = hdr.device_major().ok().flatten().unwrap_or(0);
                let minor = hdr.device_minor().ok().flatten().unwrap_or(0);
                let kind = if ty == EntryType::Char { libc::S_IFCHR } else { libc::S_IFBLK };
                let rdev = libc::makedev(major, minor) as u64;
                // Failure (for example in a user namespace) is not fatal.
                if fsutil::mknod(&full, kind | (mode & 0o7777), rdev).is_ok() {
                    fsutil::lchown_quiet(&full, uid, gid);
                }
            }
            _ => {}
        }
    }

    // Apply directory metadata deepest first so parents stay accessible.
    dirs.sort_by_key(|d| std::cmp::Reverse(d.depth));
    for d in dirs {
        if root {
            fsutil::lchown_quiet(&d.path, d.uid, d.gid);
        }
        fs::set_permissions(&d.path, fs::Permissions::from_mode(d.mode & 0o7777))?;
        fsutil::set_mtime(&d.path, d.mtime, false);
    }
    Ok(unpacked)
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Helpers to build synthetic layer tars in tests.
    use std::io::Write;

    pub enum E<'a> {
        Dir(&'a str, u32),
        File(&'a str, &'a [u8], u32),
        Symlink(&'a str, &'a str),
        Hardlink(&'a str, &'a str),
    }

    pub fn build_tar(entries: &[E]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for e in entries {
            let mut h = tar::Header::new_gnu();
            h.set_mtime(1_700_000_000);
            h.set_uid(0);
            h.set_gid(0);
            match e {
                E::Dir(p, mode) => {
                    h.set_entry_type(tar::EntryType::Directory);
                    h.set_mode(*mode);
                    h.set_size(0);
                    b.append_data(&mut h, p, std::io::empty()).unwrap();
                }
                E::File(p, data, mode) => {
                    h.set_entry_type(tar::EntryType::Regular);
                    h.set_mode(*mode);
                    h.set_size(data.len() as u64);
                    b.append_data(&mut h, p, *data).unwrap();
                }
                E::Symlink(p, t) => {
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_mode(0o777);
                    h.set_size(0);
                    b.append_link(&mut h, p, t).unwrap();
                }
                E::Hardlink(p, t) => {
                    h.set_entry_type(tar::EntryType::Link);
                    h.set_mode(0o644);
                    h.set_size(0);
                    b.append_link(&mut h, p, t).unwrap();
                }
            }
        }
        b.into_inner().unwrap()
    }

    /// Write raw header bytes with an arbitrary (possibly unsafe) path,
    /// bypassing the builder's own sanitization.
    pub fn raw_file_tar(path: &str, data: &[u8]) -> Vec<u8> {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o644);
        h.set_size(data.len() as u64);
        {
            let name = &mut h.as_old_mut().name;
            name[..path.len()].copy_from_slice(path.as_bytes());
        }
        h.set_cksum();
        let mut out = Vec::new();
        out.write_all(h.as_bytes()).unwrap();
        out.write_all(data).unwrap();
        out.resize(out.len().div_ceil(512) * 512, 0);
        out.extend_from_slice(&[0u8; 1024]);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn extracts_files_dirs_links() {
        let tar = build_tar(&[
            E::Dir("etc/", 0o755),
            E::File("etc/hostname", b"box\n", 0o644),
            E::Symlink("etc/link", "hostname"),
            E::Hardlink("etc/hard", "etc/hostname"),
            E::Dir("ro/", 0o555),
            E::File("ro/file", b"x", 0o444),
        ]);
        let d = tempfile::tempdir().unwrap();
        let n = extract_tar(&tar[..], d.path()).unwrap();
        assert_eq!(n, 5);
        assert_eq!(fs::read(d.path().join("etc/hostname")).unwrap(), b"box\n");
        assert_eq!(fs::read_link(d.path().join("etc/link")).unwrap(), Path::new("hostname"));
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            fs::metadata(d.path().join("etc/hard")).unwrap().ino(),
            fs::metadata(d.path().join("etc/hostname")).unwrap().ino()
        );
        let m = fs::metadata(d.path().join("ro")).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o555);
        assert_eq!(m.mtime(), 1_700_000_000);
        fsutil::remove_tree(d.path().join("ro").as_path()).unwrap();
    }

    #[test]
    fn rejects_parent_reference() {
        let tar = raw_file_tar("../evil", b"x");
        let d = tempfile::tempdir().unwrap();
        let inner = d.path().join("inner");
        fs::create_dir(&inner).unwrap();
        assert!(matches!(extract_tar(&tar[..], &inner), Err(Error::Invalid(_))));
        assert!(!d.path().join("evil").exists());
    }

    #[test]
    fn rejects_absolute_path() {
        let tar = raw_file_tar("/etc/evil", b"x");
        let d = tempfile::tempdir().unwrap();
        assert!(extract_tar(&tar[..], d.path()).is_err());
    }

    #[test]
    fn rejects_write_through_symlink() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().to_str().unwrap().to_string();
        let tar = build_tar(&[E::Symlink("esc", &target), E::File("esc/pwned", b"x", 0o644)]);
        let d = tempfile::tempdir().unwrap();
        assert!(extract_tar(&tar[..], d.path()).is_err());
        assert!(!outside.path().join("pwned").exists());
    }

    #[test]
    fn later_entries_replace_earlier() {
        let tar = build_tar(&[E::File("a", b"1", 0o644), E::File("a", b"22", 0o644)]);
        let d = tempfile::tempdir().unwrap();
        extract_tar(&tar[..], d.path()).unwrap();
        assert_eq!(fs::read(d.path().join("a")).unwrap(), b"22");
    }

    #[test]
    fn compression_detection() {
        assert_eq!(detect_compression("", &[0x1f, 0x8b, 8]), Compression::Gzip);
        assert_eq!(detect_compression("", &[0x28, 0xb5, 0x2f, 0xfd]), Compression::Zstd);
        assert_eq!(detect_compression("application/vnd.oci.image.layer.v1.tar", b"usta"), Compression::None);
    }

    #[test]
    fn diff_id_is_over_uncompressed_stream_for_all_codecs() {
        let tar = build_tar(&[E::File("f", b"hello", 0o644)]);
        let expect = format!("sha256:{}", hex::encode(Sha256::digest(&tar)));
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("l.tar");
        fs::write(&plain, &tar).unwrap();
        let gz = dir.path().join("l.tgz");
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(&tar).unwrap();
        fs::write(&gz, e.finish().unwrap()).unwrap();
        let zs = dir.path().join("l.tzst");
        fs::write(&zs, zstd::encode_all(&tar[..], 1).unwrap()).unwrap();
        for (p, mt) in [(plain, ""), (gz, "application/vnd.oci.image.layer.v1.tar+gzip"), (zs, "")] {
            let out = tempfile::tempdir().unwrap();
            let x = extract_layer(&p, mt, out.path(), None).unwrap();
            assert_eq!(x.diff_id, expect);
            assert_eq!(fs::read(out.path().join("f")).unwrap(), b"hello");
        }
    }
}
