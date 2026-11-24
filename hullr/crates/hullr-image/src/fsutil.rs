//! Small filesystem helpers shared by layer extraction and rootfs flattening.

use filetime::FileTime;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// True when running as uid 0 (chown and device nodes are possible).
pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

fn cpath(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "nul in path"))
}

/// `lchown`, ignoring failures (user namespaces, unmapped ids, no privilege).
pub fn lchown_quiet(p: &Path, uid: u32, gid: u32) {
    if let Ok(c) = cpath(p) {
        // SAFETY: c is a valid NUL-terminated path.
        unsafe {
            libc::lchown(c.as_ptr(), uid, gid);
        }
    }
}

pub fn mkfifo(p: &Path, mode: u32) -> io::Result<()> {
    let c = cpath(p)?;
    // SAFETY: c is a valid NUL-terminated path.
    let r = unsafe { libc::mkfifo(c.as_ptr(), mode as libc::mode_t) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Create a device node (needs root; callers skip it otherwise).
pub fn mknod(p: &Path, mode: u32, rdev: u64) -> io::Result<()> {
    let c = cpath(p)?;
    // SAFETY: c is a valid NUL-terminated path.
    let r = unsafe { libc::mknod(c.as_ptr(), mode as libc::mode_t, rdev as libc::dev_t) };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn set_mtime(p: &Path, mtime: i64, symlink: bool) {
    let t = FileTime::from_unix_time(mtime, 0);
    let _ = if symlink { filetime::set_symlink_file_times(p, t, t) } else { filetime::set_file_mtime(p, t) };
}

pub fn chmod(p: &Path, mode: u32) -> io::Result<()> {
    fs::set_permissions(p, fs::Permissions::from_mode(mode & 0o7777))
}

fn make_writable(p: &Path) {
    if let Ok(md) = fs::symlink_metadata(p) {
        if md.is_dir() {
            let _ = fs::set_permissions(p, fs::Permissions::from_mode(0o700));
            if let Ok(rd) = fs::read_dir(p) {
                for e in rd.flatten() {
                    make_writable(&e.path());
                }
            }
        }
    }
}

/// Remove a file, symlink or directory tree, tolerating read-only
/// directories (as found in image layers when running unprivileged).
pub fn remove_tree(p: &Path) -> io::Result<()> {
    match fs::symlink_metadata(p) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(md) if !md.is_dir() => fs::remove_file(p),
        Ok(_) => match fs::remove_dir_all(p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                make_writable(p);
                fs::remove_dir_all(p)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        },
    }
}

/// Remove everything inside `dir`, keeping `dir` itself.
pub fn clear_dir(dir: &Path) -> io::Result<()> {
    make_writable_dir_only(dir);
    for e in fs::read_dir(dir)? {
        remove_tree(&e?.path())?;
    }
    Ok(())
}

fn make_writable_dir_only(dir: &Path) {
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
}
