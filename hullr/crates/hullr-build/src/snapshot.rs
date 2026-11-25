//! Filesystem metadata snapshots and layer tars built from the difference of
//! two snapshots.
//!
//! A [`Snapshot`] records `(type, size, mtime, mode, uid, gid, link target)`
//! for every path under a root. [`write_layer_tar`] turns "before" and
//! "after" snapshots into an OCI layer: added or changed entries plus
//! `.wh.<name>` whiteouts for deletions (one whiteout per deleted subtree).

use hullr_core::{Context, Error, Result};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Fifo,
    Char,
    Block,
    Socket,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: EntryKind,
    pub size: u64,
    pub mtime_ns: i128,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    pub link: Option<PathBuf>,
}

impl Entry {
    /// Has `self` (before) changed into `other` (after)? Directory mtimes
    /// are ignored: they change whenever a child is added.
    fn differs(&self, other: &Entry) -> bool {
        if self.kind != other.kind
            || self.mode != other.mode
            || self.uid != other.uid
            || self.gid != other.gid
            || self.link != other.link
            || self.rdev != other.rdev
        {
            return true;
        }
        match self.kind {
            EntryKind::Dir => false,
            _ => self.size != other.size || self.mtime_ns != other.mtime_ns,
        }
    }
}

/// Relative `/`-joined path -> metadata.
pub type Snapshot = BTreeMap<String, Entry>;

pub fn snapshot(root: &Path) -> Result<Snapshot> {
    let mut snap = Snapshot::new();
    walk(root, "", &mut snap)?;
    Ok(snap)
}

fn walk(dir: &Path, prefix: &str, out: &mut Snapshot) -> Result<()> {
    let rd = std::fs::read_dir(dir).context(format!("read {}", dir.display()))?;
    for e in rd {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        let path = e.path();
        let md = std::fs::symlink_metadata(&path).context(format!("stat {}", path.display()))?;
        let ft = md.file_type();
        let kind = if ft.is_symlink() {
            EntryKind::Symlink
        } else if ft.is_dir() {
            EntryKind::Dir
        } else if ft.is_file() {
            EntryKind::File
        } else if ft.is_fifo() {
            EntryKind::Fifo
        } else if ft.is_char_device() {
            EntryKind::Char
        } else if ft.is_block_device() {
            EntryKind::Block
        } else {
            EntryKind::Socket
        };
        let link = if kind == EntryKind::Symlink { Some(std::fs::read_link(&path)?) } else { None };
        out.insert(
            rel.clone(),
            Entry {
                kind,
                size: if kind == EntryKind::File { md.size() } else { 0 },
                mtime_ns: md.mtime() as i128 * 1_000_000_000 + md.mtime_nsec() as i128,
                mode: md.mode() & 0o7777,
                uid: md.uid(),
                gid: md.gid(),
                rdev: md.rdev(),
                dev: md.dev(),
                ino: md.ino(),
                nlink: md.nlink(),
                link,
            },
        );
        if kind == EntryKind::Dir {
            walk(&path, &rel, out)?;
        }
    }
    Ok(())
}

/// The paths a layer must contain: `(added_or_changed, deleted_roots)`.
pub fn diff(before: &Snapshot, after: &Snapshot) -> (Vec<String>, Vec<String>) {
    let mut changed = Vec::new();
    for (p, a) in after {
        match before.get(p) {
            Some(b) if !b.differs(a) => {}
            _ => changed.push(p.clone()),
        }
    }
    let mut removed: Vec<String> = Vec::new();
    // directories whose type changed: children whiteouts are redundant
    let mut replaced_dirs: Vec<String> = Vec::new();
    for (p, b) in before {
        if let Some(a) = after.get(p) {
            if b.kind == EntryKind::Dir && a.kind != EntryKind::Dir {
                replaced_dirs.push(p.clone());
            }
        }
    }
    for p in before.keys() {
        if after.contains_key(p) {
            continue;
        }
        let under = |anc: &String| p.len() > anc.len() && p.starts_with(anc.as_str()) && p.as_bytes()[anc.len()] == b'/';
        if removed.last().map(under).unwrap_or(false) || replaced_dirs.iter().any(under) {
            continue;
        }
        removed.push(p.clone());
    }
    (changed, removed)
}

/// Write the layer for `before` -> `after` to `out`. Returns the number of
/// tar entries; when it is 0 no file is created.
///
/// `owner_override` maps paths to container uid/gid (used when the builder
/// cannot chown on disk); `map_ids` converts host ids to container ids.
pub fn write_layer_tar(
    root: &Path,
    before: &Snapshot,
    after: &Snapshot,
    owner_override: &HashMap<String, (u32, u32)>,
    map_ids: &dyn Fn(u32, u32) -> (u32, u32),
    out: &Path,
) -> Result<usize> {
    let (changed, removed) = diff(before, after);
    if changed.is_empty() && removed.is_empty() {
        return Ok(0);
    }
    let file = std::fs::File::create(out).context("create layer tar")?;
    let mut b = tar::Builder::new(std::io::BufWriter::new(file));
    let mut count = 0;
    let mut inodes: HashMap<(u64, u64), String> = HashMap::new();

    for p in &changed {
        let e = &after[p];
        let abs = root.join(p);
        let mut h = tar::Header::new_gnu();
        h.set_mode(e.mode);
        let (uid, gid) = owner_override.get(p).copied().unwrap_or_else(|| map_ids(e.uid, e.gid));
        h.set_uid(uid as u64);
        h.set_gid(gid as u64);
        h.set_mtime((e.mtime_ns / 1_000_000_000).max(0) as u64);
        h.set_size(0);
        let werr = |e: std::io::Error| Error::Io(format!("write layer entry {p}: {e}"));
        match e.kind {
            EntryKind::Dir => {
                h.set_entry_type(tar::EntryType::Directory);
                b.append_data(&mut h, format!("{p}/"), std::io::empty()).map_err(werr)?;
            }
            EntryKind::File => {
                if e.nlink > 1 {
                    if let Some(first) = inodes.get(&(e.dev, e.ino)) {
                        h.set_entry_type(tar::EntryType::Link);
                        b.append_link(&mut h, p, first).map_err(werr)?;
                        count += 1;
                        continue;
                    }
                    inodes.insert((e.dev, e.ino), p.clone());
                }
                h.set_entry_type(tar::EntryType::Regular);
                h.set_size(e.size);
                let f = std::fs::File::open(&abs).context(format!("read {}", abs.display()))?;
                b.append_data(&mut h, p, std::io::Read::take(f, e.size)).map_err(werr)?;
            }
            EntryKind::Symlink => {
                h.set_entry_type(tar::EntryType::Symlink);
                b.append_link(&mut h, p, e.link.as_ref().unwrap()).map_err(werr)?;
            }
            EntryKind::Fifo => {
                h.set_entry_type(tar::EntryType::Fifo);
                b.append_data(&mut h, p, std::io::empty()).map_err(werr)?;
            }
            EntryKind::Char | EntryKind::Block => {
                h.set_entry_type(if e.kind == EntryKind::Char { tar::EntryType::Char } else { tar::EntryType::Block });
                h.set_device_major(libc::major(e.rdev)).ok();
                h.set_device_minor(libc::minor(e.rdev)).ok();
                b.append_data(&mut h, p, std::io::empty()).map_err(werr)?;
            }
            EntryKind::Socket => continue,
        }
        count += 1;
    }

    for p in &removed {
        let (parent, name) = match p.rsplit_once('/') {
            Some((d, n)) => (format!("{d}/"), n),
            None => (String::new(), p.as_str()),
        };
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o644);
        h.set_size(0);
        b.append_data(&mut h, format!("{parent}.wh.{name}"), std::io::empty())
            .map_err(|e| Error::Io(format!("write whiteout {p}: {e}")))?;
        count += 1;
    }
    let mut w = b.into_inner().map_err(|e| Error::Io(e.to_string()))?;
    w.flush()?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    /// Entries of a tar as (path, type, content).
    fn read_tar(p: &Path) -> Vec<(String, tar::EntryType, String)> {
        let mut a = tar::Archive::new(std::fs::File::open(p).unwrap());
        a.entries()
            .unwrap()
            .map(|e| {
                let mut e = e.unwrap();
                let path = e.path().unwrap().to_string_lossy().into_owned();
                let ty = e.header().entry_type();
                let mut s = String::new();
                if ty == tar::EntryType::Symlink || ty == tar::EntryType::Link {
                    s = e.link_name().unwrap().unwrap().to_string_lossy().into_owned();
                } else {
                    std::io::Read::read_to_string(&mut e, &mut s).unwrap();
                }
                (path, ty, s)
            })
            .collect()
    }

    fn layer(root: &Path, before: &Snapshot, out: &Path) -> (usize, Vec<(String, tar::EntryType, String)>) {
        let after = snapshot(root).unwrap();
        let n = write_layer_tar(root, before, &after, &HashMap::new(), &|u, g| (u, g), out).unwrap();
        (n, if n > 0 { read_tar(out) } else { vec![] })
    }

    #[test]
    fn added_modified_deleted() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("root");
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(root.join("etc/keep"), "k").unwrap();
        std::fs::write(root.join("etc/mod"), "old").unwrap();
        std::fs::write(root.join("etc/gone"), "g").unwrap();
        let before = snapshot(&root).unwrap();

        std::fs::write(root.join("etc/mod"), "newer").unwrap();
        std::fs::write(root.join("etc/added"), "a").unwrap();
        std::fs::remove_file(root.join("etc/gone")).unwrap();
        let (n, ents) = layer(&root, &before, &td.path().join("l.tar"));
        let names: Vec<&str> = ents.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(n, 3, "{names:?}");
        assert!(names.contains(&"etc/added") && names.contains(&"etc/mod") && names.contains(&"etc/.wh.gone"));
        assert!(!names.contains(&"etc/keep"));
        assert_eq!(ents.iter().find(|e| e.0 == "etc/mod").unwrap().2, "newer");
        let wh = ents.iter().find(|e| e.0 == "etc/.wh.gone").unwrap();
        assert_eq!(wh.1, tar::EntryType::Regular);
        assert_eq!(wh.2, "");
    }

    #[test]
    fn deleted_dir_single_whiteout() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("root");
        std::fs::create_dir_all(root.join("opt/app/sub")).unwrap();
        std::fs::write(root.join("opt/app/sub/f"), "x").unwrap();
        std::fs::write(root.join("opt/app/g"), "x").unwrap();
        std::fs::write(root.join("top"), "x").unwrap();
        let before = snapshot(&root).unwrap();
        std::fs::remove_dir_all(root.join("opt/app")).unwrap();
        std::fs::remove_file(root.join("top")).unwrap();
        let (_, ents) = layer(&root, &before, &td.path().join("l.tar"));
        let names: Vec<&str> = ents.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(names, ["opt/.wh.app", ".wh.top"], "{names:?}");
    }

    #[test]
    fn symlink_and_dirs() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let before = snapshot(&root).unwrap();
        std::fs::create_dir_all(root.join("d/e")).unwrap();
        std::fs::write(root.join("d/e/f"), "x").unwrap();
        symlink("d/e/f", root.join("lnk")).unwrap();
        symlink("/dangling", root.join("dang")).unwrap();
        let (_, ents) = layer(&root, &before, &td.path().join("l.tar"));
        let get = |n: &str| ents.iter().find(|e| e.0 == n).unwrap_or_else(|| panic!("missing {n}: {ents:?}"));
        assert_eq!(get("lnk").1, tar::EntryType::Symlink);
        assert_eq!(get("lnk").2, "d/e/f");
        assert_eq!(get("dang").2, "/dangling");
        assert_eq!(get("d/").1, tar::EntryType::Directory);
        assert_eq!(get("d/e/").1, tar::EntryType::Directory);
        assert_eq!(get("d/e/f").2, "x");
        // parents come before children
        let pos = |n: &str| ents.iter().position(|e| e.0 == n).unwrap();
        assert!(pos("d/") < pos("d/e/") && pos("d/e/") < pos("d/e/f"));
    }

    #[test]
    fn symlink_retarget_and_type_change() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("root");
        std::fs::create_dir_all(root.join("dir")).unwrap();
        std::fs::write(root.join("dir/inner"), "i").unwrap();
        symlink("a", root.join("l")).unwrap();
        let before = snapshot(&root).unwrap();
        std::fs::remove_file(root.join("l")).unwrap();
        symlink("b", root.join("l")).unwrap();
        std::fs::remove_dir_all(root.join("dir")).unwrap();
        std::fs::write(root.join("dir"), "now a file").unwrap();
        let (_, ents) = layer(&root, &before, &td.path().join("l.tar"));
        let names: Vec<&str> = ents.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(ents.iter().find(|e| e.0 == "l").unwrap().2, "b");
        assert_eq!(ents.iter().find(|e| e.0 == "dir").unwrap().2, "now a file");
        assert!(!names.iter().any(|n| n.contains(".wh.")), "{names:?}");
    }

    #[test]
    fn no_changes_no_tar() {
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("root");
        std::fs::create_dir_all(root.join("d")).unwrap();
        std::fs::write(root.join("d/f"), "x").unwrap();
        let before = snapshot(&root).unwrap();
        let out = td.path().join("l.tar");
        let (n, _) = layer(&root, &before, &out);
        assert_eq!(n, 0);
        assert!(!out.exists());
    }

    #[test]
    fn mode_change_and_hardlink_and_override() {
        use std::os::unix::fs::PermissionsExt;
        let td = tempfile::tempdir().unwrap();
        let root = td.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("x"), "x").unwrap();
        let before = snapshot(&root).unwrap();
        std::fs::set_permissions(root.join("x"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(root.join("h1"), "same").unwrap();
        std::fs::hard_link(root.join("h1"), root.join("h2")).unwrap();
        let after = snapshot(&root).unwrap();
        let mut ov = HashMap::new();
        ov.insert("h1".to_string(), (42u32, 43u32));
        let out = td.path().join("l.tar");
        write_layer_tar(&root, &before, &after, &ov, &|u, g| (u, g), &out).unwrap();
        let ents = read_tar(&out);
        assert!(ents.iter().any(|e| e.0 == "x"));
        assert_eq!(ents.iter().find(|e| e.0 == "h1").unwrap().1, tar::EntryType::Regular);
        let h2 = ents.iter().find(|e| e.0 == "h2").unwrap();
        assert_eq!((h2.1, h2.2.as_str()), (tar::EntryType::Link, "h1"));
        let mut a = tar::Archive::new(std::fs::File::open(&out).unwrap());
        for e in a.entries().unwrap() {
            let e = e.unwrap();
            if e.path().unwrap().to_str() == Some("h1") {
                assert_eq!((e.header().uid().unwrap(), e.header().gid().unwrap()), (42, 43));
            }
        }
    }
}
