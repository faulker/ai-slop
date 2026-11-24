//! Building the container's filesystem view inside its own mount
//! namespace: overlay (or copy) root, /proc, /sys, a minimal /dev, user
//! mounts, the generated /etc files, masked paths, then pivot_root.

use crate::state::InitConfig;
use crate::util::{rt_err, secure_join};
use hullr_core::{MountKind, Result};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use std::path::{Path, PathBuf};

fn mnt(src: Option<&str>, target: &Path, fstype: Option<&str>, flags: MsFlags, data: Option<&str>) -> Result<()> {
    mount(src, target, fstype, flags, data).map_err(|e| {
        rt_err(format!("mount {} on {}: {e}", fstype.or(src).unwrap_or("?"), target.display()))
    })
}

fn bind(src: &Path, target: &Path, recursive: bool) -> Result<()> {
    let mut f = MsFlags::MS_BIND;
    if recursive {
        f |= MsFlags::MS_REC;
    }
    mount(Some(src), target, None::<&str>, f, None::<&str>)
        .map_err(|e| rt_err(format!("bind {} on {}: {e}", src.display(), target.display())))
}

/// Remount a bind mount read-only, keeping the flags the kernel locked on
/// it (nosuid/nodev/noexec must be preserved inside a user namespace).
fn remount_ro(target: &Path) -> Result<()> {
    let st = nix::sys::statvfs::statvfs(target).map_err(|e| rt_err(format!("statvfs {}: {e}", target.display())))?;
    use nix::sys::statvfs::FsFlags;
    let mut f = MsFlags::MS_BIND | MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY;
    let fl = st.flags();
    for (a, b) in [
        (FsFlags::ST_NOSUID, MsFlags::MS_NOSUID),
        (FsFlags::ST_NODEV, MsFlags::MS_NODEV),
        (FsFlags::ST_NOEXEC, MsFlags::MS_NOEXEC),
        (FsFlags::ST_NOATIME, MsFlags::MS_NOATIME),
        (FsFlags::ST_NODIRATIME, MsFlags::MS_NODIRATIME),
        (FsFlags::ST_RELATIME, MsFlags::MS_RELATIME),
    ] {
        if fl.contains(a) {
            f |= b;
        }
    }
    mount(None::<&str>, target, None::<&str>, f, None::<&str>)
        .map_err(|e| rt_err(format!("remount {} read-only: {e}", target.display())))
}

/// Create `target` as a dir or an empty file to match `like`.
fn ensure_target(target: &Path, dir: bool) -> Result<()> {
    if dir {
        std::fs::create_dir_all(target).map_err(|e| rt_err(format!("mkdir {}: {e}", target.display())))?;
    } else if !target.exists() {
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(|e| rt_err(format!("mkdir {}: {e}", p.display())))?;
        }
        std::fs::File::create(target).map_err(|e| rt_err(format!("create {}: {e}", target.display())))?;
    }
    Ok(())
}

fn in_userns() -> bool {
    std::fs::read_to_string("/proc/self/uid_map")
        .map(|m| !m.split_whitespace().collect::<Vec<_>>().starts_with(&["0", "0", "4294967295"]))
        .unwrap_or(false)
}

/// Mount the root filesystem at `cfg.rootfs`: overlay when possible, else a
/// one-time copy of the lower dir. Returns a note if it fell back.
fn mount_root(cfg: &InitConfig) -> Result<Option<String>> {
    let root = &cfg.rootfs;
    std::fs::create_dir_all(root).map_err(|e| rt_err(format!("mkdir rootfs: {e}")))?;
    let Some((upper, work)) = &cfg.overlay else {
        bind(root, root, true)?;
        return Ok(None);
    };
    let marker = cfg.copied_marker.as_ref();
    let copied = marker.map(|m| m.exists()).unwrap_or(false);
    if !copied && std::env::var_os("HULLR_NO_OVERLAY").is_none() {
        for p in [upper, work] {
            std::fs::create_dir_all(p).map_err(|e| rt_err(format!("mkdir {}: {e}", p.display())))?;
        }
        let base = format!("lowerdir={},upperdir={},workdir={}", cfg.spec.lower.display(), upper.display(), work.display());
        let mut attempts = vec![];
        if in_userns() {
            attempts.push(format!("{base},userxattr"));
        }
        attempts.push(base);
        let mut last = String::new();
        for opts in &attempts {
            match mount(Some("overlay"), root.as_path(), Some("overlay"), MsFlags::empty(), Some(opts.as_str())) {
                Ok(()) => return Ok(None),
                Err(e) => last = e.to_string(),
            }
        }
        // Fall through to the copy fallback.
        let note = format!("overlay mount failed ({last}); using a full copy of the image");
        copy_root(cfg)?;
        return Ok(Some(note));
    }
    if !copied {
        copy_root(cfg)?;
    }
    bind(root, root, true)?;
    Ok(None)
}

fn copy_root(cfg: &InitConfig) -> Result<()> {
    crate::util::copy_tree(&cfg.spec.lower, &cfg.rootfs)?;
    if let Some(m) = &cfg.copied_marker {
        let _ = std::fs::write(m, "");
    }
    bind(&cfg.rootfs, &cfg.rootfs, true)
}

fn mount_proc(root: &Path) -> Result<()> {
    let t = root.join("proc");
    ensure_target(&t, true)?;
    let f = MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC;
    if mount(Some("proc"), &t, Some("proc"), f, None::<&str>).is_ok() {
        return Ok(());
    }
    // Some sandboxes (masked host /proc) refuse a fresh proc mount; a
    // recursive bind of the host's /proc is the last resort.
    bind(Path::new("/proc"), &t, true)
}

fn mount_sys(root: &Path, own_netns: bool) -> Result<()> {
    let t = root.join("sys");
    ensure_target(&t, true)?;
    let f = MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC | MsFlags::MS_RDONLY;
    if own_netns && mount(Some("sysfs"), &t, Some("sysfs"), f, None::<&str>).is_ok() {
        return Ok(());
    }
    // sysfs needs an owned netns in a user namespace: bind the host's.
    bind(Path::new("/sys"), &t, true)?;
    let _ = remount_ro(&t);
    Ok(())
}

fn mount_dev(root: &Path) -> Result<()> {
    let dev = root.join("dev");
    ensure_target(&dev, true)?;
    mnt(Some("tmpfs"), &dev, Some("tmpfs"), MsFlags::MS_NOSUID | MsFlags::MS_STRICTATIME, Some("mode=755,size=65536k"))?;
    for name in ["null", "zero", "full", "random", "urandom", "tty"] {
        let t = dev.join(name);
        ensure_target(&t, false)?;
        bind(&Path::new("/dev").join(name), &t, false)?;
    }
    let pts = dev.join("pts");
    ensure_target(&pts, true)?;
    let pf = MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC;
    if mount(Some("devpts"), &pts, Some("devpts"), pf, Some("newinstance,ptmxmode=0666,mode=0620,gid=5")).is_err() {
        // gid 5 (tty) is unmapped in a single-id user namespace.
        mnt(Some("devpts"), &pts, Some("devpts"), pf, Some("newinstance,ptmxmode=0666,mode=0620"))?;
    }
    let shm = dev.join("shm");
    ensure_target(&shm, true)?;
    mnt(
        Some("shm"),
        &shm,
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        Some("mode=1777,size=65536k"),
    )?;
    let mq = dev.join("mqueue");
    if ensure_target(&mq, true).is_ok()
        && mount(Some("mqueue"), &mq, Some("mqueue"), MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC, None::<&str>)
            .is_err()
    {
        let _ = std::fs::remove_dir(&mq);
    }
    for (link, target) in [
        ("fd", "/proc/self/fd"),
        ("stdin", "/proc/self/fd/0"),
        ("stdout", "/proc/self/fd/1"),
        ("stderr", "/proc/self/fd/2"),
        ("ptmx", "pts/ptmx"),
        ("core", "/proc/kcore"),
    ] {
        let _ = std::os::unix::fs::symlink(target, dev.join(link));
    }
    Ok(())
}

fn user_mounts(cfg: &InitConfig) -> Result<()> {
    let root = &cfg.rootfs;
    for m in &cfg.spec.mounts {
        let target = secure_join(root, &m.target);
        match m.kind {
            MountKind::Tmpfs => {
                ensure_target(&target, true)?;
                let mut f = MsFlags::MS_NOSUID | MsFlags::MS_NODEV;
                if m.read_only {
                    f |= MsFlags::MS_RDONLY;
                }
                mnt(Some("tmpfs"), &target, Some("tmpfs"), f, Some("mode=1777"))?;
            }
            MountKind::Bind | MountKind::Volume => {
                let src = &m.source;
                if m.kind == MountKind::Bind && !src.exists() {
                    return Err(rt_err(format!("bind source path does not exist: {}", src.display())));
                }
                if m.kind == MountKind::Volume {
                    std::fs::create_dir_all(src).map_err(|e| rt_err(format!("volume dir {}: {e}", src.display())))?;
                    seed_volume(src, &target);
                }
                ensure_target(&target, src.is_dir())?;
                bind(src, &target, true)?;
                if m.read_only {
                    remount_ro(&target)?;
                }
            }
        }
    }
    Ok(())
}

/// Docker copies image content at the mount point into an empty volume.
fn seed_volume(vol: &Path, image_dir: &Path) {
    let empty = std::fs::read_dir(vol).map(|mut r| r.next().is_none()).unwrap_or(false);
    let has = std::fs::read_dir(image_dir).map(|mut r| r.next().is_some()).unwrap_or(false);
    if empty && has {
        let _ = crate::util::copy_tree(image_dir, vol);
    }
}

fn etc_files(cfg: &InitConfig) -> Result<()> {
    for (src, target) in &cfg.etc_files {
        let t = secure_join(&cfg.rootfs, target);
        // A dangling symlink (e.g. resolv.conf -> ../run/...) is replaced.
        if t.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            let _ = std::fs::remove_file(&t);
        }
        ensure_target(&t, false)?;
        bind(src, &t, false)?;
    }
    Ok(())
}

/// Hide host information the way Docker's default profile does. Best effort.
fn mask_paths(root: &Path) {
    for p in ["proc/kcore", "proc/keys", "proc/timer_list", "proc/sched_debug", "sys/firmware", "proc/scsi", "proc/acpi"] {
        let t = root.join(p);
        let Ok(md) = t.metadata() else { continue };
        if md.is_dir() {
            let _ = mount(Some("tmpfs"), &t, Some("tmpfs"), MsFlags::MS_RDONLY, None::<&str>);
        } else {
            let _ = bind(Path::new("/dev/null"), &t, false);
        }
    }
    for p in ["proc/bus", "proc/fs", "proc/irq", "proc/sys", "proc/sysrq-trigger"] {
        let t = root.join(p);
        if t.exists() && bind(&t, &t, true).is_ok() {
            let _ = remount_ro(&t);
        }
    }
}

/// pivot_root into `root` and drop the old root.
fn pivot(root: &Path) -> Result<()> {
    nix::unistd::chdir(root).map_err(|e| rt_err(format!("chdir rootfs: {e}")))?;
    nix::unistd::pivot_root(".", ".").map_err(|e| rt_err(format!("pivot_root: {e}")))?;
    // The old root is now stacked under "."; detach it.
    umount2(".", MntFlags::MNT_DETACH).map_err(|e| rt_err(format!("umount old root: {e}")))?;
    nix::unistd::chdir("/").map_err(|e| rt_err(format!("chdir /: {e}")))?;
    Ok(())
}

/// Full filesystem setup; on return the process is inside the new root.
/// Returns warnings worth logging.
pub fn setup(cfg: &InitConfig, own_netns: bool) -> Result<Vec<String>> {
    let mut notes = vec![];
    mnt(None, Path::new("/"), None, MsFlags::MS_REC | MsFlags::MS_PRIVATE, None)?;
    if let Some(n) = mount_root(cfg)? {
        notes.push(n);
    }
    let root: PathBuf = cfg.rootfs.clone();
    mount_proc(&root)?;
    mount_sys(&root, own_netns)?;
    mount_dev(&root)?;
    user_mounts(cfg)?;
    etc_files(cfg)?;
    mask_paths(&root);
    pivot(&root)?;
    Ok(notes)
}
