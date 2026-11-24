//! Filesystem helpers: rootfs-confined path resolution, tree copy, cleanup,
//! passwd/group lookup.

use hullr_core::{Context, Error, Result};
use std::path::{Component, Path, PathBuf};

/// Resolve `path` (absolute or relative, interpreted inside `root`) to a host
/// path, following symlinks as a chroot would (absolute targets are rebased
/// on `root`, `..` cannot climb above it). The last component is followed
/// only when `follow_last`. Missing components are appended verbatim.
pub fn resolve_in_root(root: &Path, path: &str, follow_last: bool) -> Result<PathBuf> {
    let mut pending: Vec<String> = path
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .map(str::to_string)
        .rev()
        .collect();
    let mut resolved: Vec<String> = Vec::new();
    let mut hops = 0;
    while let Some(comp) = pending.pop() {
        if comp == ".." {
            resolved.pop();
            continue;
        }
        let candidate = join(root, &resolved).join(&comp);
        let is_last = pending.is_empty();
        match std::fs::symlink_metadata(&candidate) {
            Ok(md) if md.file_type().is_symlink() && (!is_last || follow_last) => {
                hops += 1;
                if hops > 40 {
                    return Err(Error::Build(format!("too many symlinks resolving '{path}'")));
                }
                let target = std::fs::read_link(&candidate)?;
                let t = target.to_string_lossy().into_owned();
                if t.starts_with('/') {
                    resolved.clear();
                }
                for c in t.split('/').filter(|c| !c.is_empty() && *c != ".").rev() {
                    pending.push(c.to_string());
                }
            }
            _ => resolved.push(comp),
        }
    }
    Ok(join(root, &resolved))
}

fn join(root: &Path, comps: &[String]) -> PathBuf {
    let mut p = root.to_path_buf();
    for c in comps {
        p.push(c);
    }
    p
}

/// Lexically normalise an absolute container path.
pub fn clean_abs(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    format!("/{}", out.join("/"))
}

/// Copy a rootfs tree preserving everything (`cp -a --reflink=auto`), with a
/// slow Rust fallback if `cp` is unavailable or fails.
pub fn copy_tree(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).context("create rootfs dir")?;
    let src_dot = src.join(".");
    let ok = std::process::Command::new("cp")
        .arg("-a")
        .arg("--reflink=auto")
        .arg(&src_dot)
        .arg(dst)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok {
        return Ok(());
    }
    copy_tree_rust(src, dst)
}

pub fn copy_tree_rust(src: &Path, dst: &Path) -> Result<()> {
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src).context(format!("read {}", src.display()))? {
        let e = e?;
        let from = e.path();
        let to = dst.join(e.file_name());
        let md = std::fs::symlink_metadata(&from)?;
        let ft = md.file_type();
        if ft.is_symlink() {
            let _ = std::fs::remove_file(&to);
            symlink(std::fs::read_link(&from)?, &to)?;
        } else if ft.is_dir() {
            copy_tree_rust(&from, &to)?;
            let _ = std::fs::set_permissions(&to, std::fs::Permissions::from_mode(md.mode() & 0o7777));
        } else if ft.is_file() {
            std::fs::copy(&from, &to)?;
        }
        if !ft.is_symlink() {
            // best effort: only root can keep foreign ownership
            let _ = unsafe_lchown(&to, md.uid(), md.gid());
        }
    }
    Ok(())
}

/// `lchown` wrapper; errors are returned, not fatal by themselves.
pub fn unsafe_lchown(path: &Path, uid: u32, gid: u32) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "nul in path"))?;
    // SAFETY: c is a valid NUL-terminated path for the duration of the call.
    let r = unsafe { libc::lchown(c.as_ptr(), uid, gid) };
    if r == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Remove a tree even if it contains read-only directories.
pub fn rm_rf(path: &Path) {
    if std::fs::remove_dir_all(path).is_ok() || !path.exists() {
        return;
    }
    fn open_up(p: &Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(md) = std::fs::symlink_metadata(p) {
            if md.is_dir() {
                let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700));
                if let Ok(rd) = std::fs::read_dir(p) {
                    for e in rd.flatten() {
                        open_up(&e.path());
                    }
                }
            }
        }
    }
    open_up(path);
    let _ = std::fs::remove_dir_all(path);
}

/// Look up `name` in an `/etc/passwd`-style file: returns `(uid, gid)`.
pub fn lookup_user(root: &Path, name: &str) -> Option<(u32, u32)> {
    let text = std::fs::read_to_string(resolve_in_root(root, "/etc/passwd", true).ok()?).ok()?;
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 4 && f[0] == name).then(|| Some((f[2].parse().ok()?, f[3].parse().ok()?)))?
    })
}

pub fn lookup_group(root: &Path, name: &str) -> Option<u32> {
    let text = std::fs::read_to_string(resolve_in_root(root, "/etc/group", true).ok()?).ok()?;
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.split(':').collect();
        (f.len() >= 3 && f[0] == name).then(|| f[2].parse().ok())?
    })
}

/// Resolve `user[:group]` (names or numbers) against the rootfs.
pub fn resolve_owner(root: &Path, spec: &str) -> Result<(u32, u32)> {
    let (u, g) = match spec.split_once(':') {
        Some((u, g)) => (u, Some(g)),
        None => (spec, None),
    };
    let missing = |what: &str, n: &str| {
        Error::Build(format!("unable to resolve {what} '{n}' (no entry in the image's /etc/passwd or /etc/group)"))
    };
    let (uid, user_gid) = match u.parse::<u32>() {
        Ok(n) => (n, n),
        Err(_) => lookup_user(root, u).ok_or_else(|| missing("user", u))?,
    };
    let gid = match g {
        None => user_gid,
        Some(g) => match g.parse::<u32>() {
            Ok(n) => n,
            Err(_) => lookup_group(root, g).ok_or_else(|| missing("group", g))?,
        },
    };
    Ok((uid, gid))
}

/// Relative path to a `/`-joined string.
pub fn rel_string(p: &Path) -> String {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn resolve_follows_symlinks_inside_root() {
        let td = tempfile::tempdir().unwrap();
        let r = td.path();
        std::fs::create_dir_all(r.join("run/x")).unwrap();
        std::fs::create_dir_all(r.join("var")).unwrap();
        symlink("/run", r.join("var/run")).unwrap();
        symlink("../../../../etc", r.join("var/up")).unwrap();
        assert_eq!(resolve_in_root(r, "/var/run/x", true).unwrap(), r.join("run/x"));
        assert_eq!(resolve_in_root(r, "/var/run", false).unwrap(), r.join("var/run"));
        assert_eq!(resolve_in_root(r, "/var/run", true).unwrap(), r.join("run"));
        assert_eq!(resolve_in_root(r, "/var/up/passwd", true).unwrap(), r.join("etc/passwd"));
        assert_eq!(resolve_in_root(r, "/a/../../b", true).unwrap(), r.join("b"));
        symlink("loop", r.join("loop")).unwrap();
        assert!(resolve_in_root(r, "/loop/x", true).is_err());
    }

    #[test]
    fn clean_paths() {
        assert_eq!(clean_abs("/a/./b/../c/"), "/a/c");
        assert_eq!(clean_abs("/../.."), "/");
    }

    #[test]
    fn owners() {
        let td = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(td.path().join("etc")).unwrap();
        std::fs::write(td.path().join("etc/passwd"), "root:x:0:0::/root:/bin/sh\napp:x:1000:1001::/home/app:/bin/sh\n").unwrap();
        std::fs::write(td.path().join("etc/group"), "root:x:0:\nstaff:x:50:\n").unwrap();
        assert_eq!(resolve_owner(td.path(), "app").unwrap(), (1000, 1001));
        assert_eq!(resolve_owner(td.path(), "app:staff").unwrap(), (1000, 50));
        assert_eq!(resolve_owner(td.path(), "5").unwrap(), (5, 5));
        assert_eq!(resolve_owner(td.path(), "5:6").unwrap(), (5, 6));
        assert!(resolve_owner(td.path(), "ghost").is_err());
    }

    #[test]
    fn copy_tree_keeps_symlinks() {
        let td = tempfile::tempdir().unwrap();
        let src = td.path().join("s");
        std::fs::create_dir_all(src.join("d")).unwrap();
        std::fs::write(src.join("d/f"), "hi").unwrap();
        symlink("d/f", src.join("l")).unwrap();
        let dst = td.path().join("t");
        copy_tree(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(dst.join("d/f")).unwrap(), "hi");
        assert_eq!(std::fs::read_link(dst.join("l")).unwrap(), PathBuf::from("d/f"));
        let dst2 = td.path().join("t2");
        copy_tree_rust(&src, &dst2).unwrap();
        assert_eq!(std::fs::read_link(dst2.join("l")).unwrap(), PathBuf::from("d/f"));
    }
}
