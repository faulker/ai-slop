//! Flatten extracted layer directories into one root filesystem.
//!
//! Layers are merged in order into a destination directory. OCI whiteouts are
//! honoured and never appear in the result: `.wh.<name>` removes `<name>` that
//! came from lower layers, and `.wh..wh..opq` empties the directory's lower
//! content before the layer's own entries are added. Files are hardlinked
//! from the layer directories when possible (same filesystem) and copied
//! otherwise, so the result must be treated as read-only.

use crate::fsutil;
use hullr_core::Result;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const WH_PREFIX: &str = ".wh.";
pub const WH_OPAQUE: &str = ".wh..wh..opq";

#[derive(Clone, Copy)]
struct DirAttr {
    mode: u32,
    mtime: i64,
    uid: u32,
    gid: u32,
}

/// Merges layers into one destination directory.
pub struct Flattener {
    dest: PathBuf,
    root: bool,
    /// Final attributes for directories, applied after all layers.
    dirs: HashMap<PathBuf, DirAttr>,
}

impl Flattener {
    pub fn new(dest: &Path) -> Self {
        Self { dest: dest.to_path_buf(), root: fsutil::is_root(), dirs: HashMap::new() }
    }

    /// Apply one extracted layer directory on top of what is already there.
    pub fn apply_layer(&mut self, layer_dir: &Path) -> Result<()> {
        let dest = self.dest.clone();
        self.merge_dir(layer_dir, &dest)?;
        Ok(())
    }

    /// Set directory modes, owners and mtimes (deepest first) and finish.
    pub fn finish(self) -> Result<()> {
        let mut dirs: Vec<(PathBuf, DirAttr)> = self.dirs.into_iter().collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.0.components().count()));
        for (p, a) in dirs {
            // Skip paths that were removed or replaced by a non-directory.
            match fs::symlink_metadata(&p) {
                Ok(md) if md.is_dir() => {}
                _ => continue,
            }
            if self.root {
                fsutil::lchown_quiet(&p, a.uid, a.gid);
            }
            fsutil::chmod(&p, a.mode)?;
            fsutil::set_mtime(&p, a.mtime, false);
        }
        // The destination root itself is writable until here; nothing to do.
        Ok(())
    }

    fn merge_dir(&mut self, src: &Path, dst: &Path) -> Result<()> {
        let mut names: Vec<std::ffi::OsString> = Vec::new();
        for e in fs::read_dir(src)? {
            names.push(e?.file_name());
        }
        names.sort();

        if names.iter().any(|n| n == WH_OPAQUE) && dst.is_dir() {
            fsutil::clear_dir(dst)?;
            self.forget_below(dst);
        }
        // Whiteouts first: they only affect content from lower layers.
        for n in &names {
            let s = n.to_string_lossy();
            if s == WH_OPAQUE {
                continue;
            }
            if let Some(victim) = s.strip_prefix(WH_PREFIX) {
                let target = dst.join(victim);
                fsutil::remove_tree(&target)?;
                self.forget_below(&target);
                self.dirs.remove(&target);
            }
        }
        for n in &names {
            let s = n.to_string_lossy();
            if s == WH_OPAQUE || s.starts_with(WH_PREFIX) {
                continue;
            }
            self.merge_entry(&src.join(n), &dst.join(n))?;
        }
        Ok(())
    }

    fn forget_below(&mut self, p: &Path) {
        self.dirs.retain(|k, _| !k.starts_with(p) || k == p);
    }

    fn merge_entry(&mut self, src: &Path, dst: &Path) -> Result<()> {
        let md = fs::symlink_metadata(src)?;
        let ft = md.file_type();
        let existing = fs::symlink_metadata(dst).ok();

        if ft.is_dir() {
            match &existing {
                Some(e) if e.is_dir() => {}
                Some(_) => {
                    fsutil::remove_tree(dst)?;
                    fs::create_dir(dst)?;
                }
                None => fs::create_dir(dst)?,
            }
            // Writable while populating; real attributes are applied in finish().
            fsutil::chmod(dst, md.mode() | 0o700)?;
            self.dirs.insert(
                dst.to_path_buf(),
                DirAttr { mode: md.mode(), mtime: md.mtime(), uid: md.uid(), gid: md.gid() },
            );
            return self.merge_dir(src, dst);
        }

        if let Some(e) = &existing {
            if e.is_dir() {
                self.forget_below(dst);
                self.dirs.remove(dst);
            }
            fsutil::remove_tree(dst)?;
        }

        if ft.is_symlink() {
            let t = fs::read_link(src)?;
            std::os::unix::fs::symlink(t, dst)?;
            if self.root {
                fsutil::lchown_quiet(dst, md.uid(), md.gid());
            }
            fsutil::set_mtime(dst, md.mtime(), true);
        } else if ft.is_file() {
            link_or_copy(src, dst, &md, self.root)?;
        } else if ft.is_fifo() {
            fsutil::mkfifo(dst, md.mode() & 0o7777)?;
            if self.root {
                fsutil::lchown_quiet(dst, md.uid(), md.gid());
            }
        } else if (ft.is_char_device() || ft.is_block_device()) && self.root {
            let _ = fsutil::mknod(dst, md.mode(), md.rdev());
        }
        Ok(())
    }
}

fn link_or_copy(src: &Path, dst: &Path, md: &fs::Metadata, root: bool) -> io::Result<()> {
    if fs::hard_link(src, dst).is_ok() {
        return Ok(());
    }
    fs::copy(src, dst)?;
    fs::set_permissions(dst, fs::Permissions::from_mode(md.mode() & 0o7777))?;
    if root {
        fsutil::lchown_quiet(dst, md.uid(), md.gid());
        fs::set_permissions(dst, fs::Permissions::from_mode(md.mode() & 0o7777))?;
    }
    fsutil::set_mtime(dst, md.mtime(), false);
    Ok(())
}

/// Flatten `layers` (extracted directories, base first) into `dest`, which
/// must already exist and be empty.
pub fn flatten(layers: &[PathBuf], dest: &Path) -> Result<()> {
    let mut f = Flattener::new(dest);
    for l in layers {
        f.apply_layer(l)?;
    }
    f.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{extract_tar, testutil::*};

    fn layer(entries: &[E]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        extract_tar(&build_tar(entries)[..], d.path()).unwrap();
        d
    }

    fn all_paths(root: &Path) -> Vec<String> {
        fn walk(p: &Path, root: &Path, out: &mut Vec<String>) {
            for e in fs::read_dir(p).unwrap().flatten() {
                let rel = e.path().strip_prefix(root).unwrap().to_string_lossy().to_string();
                out.push(rel);
                if fs::symlink_metadata(e.path()).unwrap().is_dir() {
                    walk(&e.path(), root, out);
                }
            }
        }
        let mut v = Vec::new();
        walk(root, root, &mut v);
        v.sort();
        v
    }

    #[test]
    fn whiteout_deletes_lower_file_and_leaves_no_markers() {
        let l1 = layer(&[
            E::Dir("etc/", 0o755),
            E::File("etc/a", b"a", 0o644),
            E::File("etc/b", b"b", 0o644),
            E::File("keep", b"k", 0o644),
        ]);
        let l2 = layer(&[E::Dir("etc/", 0o755), E::File("etc/.wh.a", b"", 0o644), E::File("etc/c", b"c", 0o644)]);
        let out = tempfile::tempdir().unwrap();
        flatten(&[l1.path().into(), l2.path().into()], out.path()).unwrap();
        assert_eq!(all_paths(out.path()), vec!["etc", "etc/b", "etc/c", "keep"]);
    }

    #[test]
    fn opaque_dir_clears_lower_content_but_keeps_own() {
        let l1 = layer(&[
            E::Dir("d/", 0o755),
            E::File("d/old1", b"1", 0o644),
            E::Dir("d/sub/", 0o755),
            E::File("d/sub/old2", b"2", 0o644),
            E::File("other", b"o", 0o644),
        ]);
        let l2 = layer(&[E::Dir("d/", 0o755), E::File("d/.wh..wh..opq", b"", 0o644), E::File("d/new", b"n", 0o644)]);
        let out = tempfile::tempdir().unwrap();
        flatten(&[l1.path().into(), l2.path().into()], out.path()).unwrap();
        assert_eq!(all_paths(out.path()), vec!["d", "d/new", "other"]);
    }

    #[test]
    fn whiteout_of_directory_removes_tree() {
        let l1 = layer(&[E::Dir("d/", 0o755), E::File("d/x", b"1", 0o644)]);
        let l2 = layer(&[E::File(".wh.d", b"", 0o644)]);
        let out = tempfile::tempdir().unwrap();
        flatten(&[l1.path().into(), l2.path().into()], out.path()).unwrap();
        assert!(all_paths(out.path()).is_empty());
    }

    #[test]
    fn upper_replaces_type_and_overrides_content() {
        let l1 = layer(&[E::Symlink("bin", "usr/bin"), E::File("f", b"old", 0o644), E::Dir("g/", 0o755), E::File("g/x", b"x", 0o644)]);
        let l2 = layer(&[E::Dir("bin/", 0o755), E::File("bin/sh", b"sh", 0o755), E::File("f", b"new", 0o600), E::File("g", b"now a file", 0o644)]);
        let out = tempfile::tempdir().unwrap();
        flatten(&[l1.path().into(), l2.path().into()], out.path()).unwrap();
        assert!(fs::symlink_metadata(out.path().join("bin")).unwrap().is_dir());
        assert_eq!(fs::read(out.path().join("bin/sh")).unwrap(), b"sh");
        assert_eq!(fs::read(out.path().join("f")).unwrap(), b"new");
        assert_eq!(fs::read(out.path().join("g")).unwrap(), b"now a file");
    }

    #[test]
    fn directory_modes_are_applied_at_the_end() {
        let l1 = layer(&[E::Dir("ro/", 0o555), E::File("ro/a", b"a", 0o644)]);
        let l2 = layer(&[E::Dir("ro/", 0o555), E::File("ro/b", b"b", 0o644)]);
        let out = tempfile::tempdir().unwrap();
        flatten(&[l1.path().into(), l2.path().into()], out.path()).unwrap();
        let m = fs::metadata(out.path().join("ro")).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o555);
        assert!(out.path().join("ro/a").exists() && out.path().join("ro/b").exists());
        fsutil::remove_tree(&out.path().join("ro")).unwrap();
    }
}
