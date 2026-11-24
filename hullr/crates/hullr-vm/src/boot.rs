//! Everything the VM needs before it boots that is not Virtualization.framework
//! specific: finding the kernel and initramfs, the sparse data disk, the
//! directory share plan and the kernel command line.
//!
//! The guest `/init` (see `vm/initramfs/init`) reads two custom kernel
//! parameters:
//!
//! * `hullr.shares=s0:/Users/me,s1:/tmp:/private/tmp` maps each entry (by
//!   name) of the `hullr-host` virtiofs multiple-directory share to its
//!   absolute path in the guest, which is the same path as on the host. A
//!   third field is the canonical host path when it differs; the guest links
//!   it to the mount so both spellings work.
//! * `hullr.rosetta=1` says the `rosetta` virtiofs tag is attached.

use crate::config::VmConfig;
use crate::state::VmFiles;
use hullr_core::{Error, Result};
use std::path::{Path, PathBuf};

/// Base kernel command line. `root=` is ignored by the kernel because the
/// initramfs provides `/init`; it documents where the data disk lives.
pub const BASE_CMDLINE: &str = "console=hvc0 root=/dev/vda rw quiet";

/// The virtiofs tag of the multiple-directory share of host paths.
pub const HOST_SHARE_TAG: &str = "hullr-host";
/// The virtiofs tag of the Rosetta share.
pub const ROSETTA_TAG: &str = "rosetta";

/// Locate the kernel and initramfs: explicit config, then `<root>/vm/`, then
/// `<exe dir>/../share/hullr/vm/` for packaged installs.
pub fn resolve_boot_files(files: &VmFiles, cfg: &VmConfig, exe: &Path) -> Result<(PathBuf, PathBuf)> {
    let packaged = exe
        .parent()
        .map(|d| d.join("../share/hullr/vm"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let pick = |explicit: &Option<PathBuf>, name: &str, installed: PathBuf| -> Result<PathBuf> {
        if let Some(p) = explicit {
            return if p.is_file() {
                Ok(p.clone())
            } else {
                Err(Error::NotFound(format!("vm {name} '{}' (from vm/config.json)", p.display())))
            };
        }
        let candidates = [installed, packaged.join(name)];
        candidates.iter().find(|p| p.is_file()).cloned().ok_or_else(|| {
            Error::NotFound(format!(
                "vm {name}: looked in {} and {}; build and install it with `vm/build.sh --install` (or `hullr vm install`)",
                candidates[0].display(),
                candidates[1].display()
            ))
        })
    };
    Ok((pick(&cfg.kernel, "kernel", files.kernel())?, pick(&cfg.initrd, "initrd", files.initrd())?))
}

/// Create the sparse raw disk image if missing, or grow it to `gib`. The guest
/// formats an empty disk as ext4 on first boot and runs resize2fs after a
/// grow. Never shrinks.
pub fn ensure_disk(path: &Path, gib: u64) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let want = gib.checked_mul(1 << 30).ok_or_else(|| Error::Invalid("disk_gib too large".into()))?;
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
    // set_len is ftruncate: it extends without allocating blocks.
    if file.metadata()?.len() < want {
        file.set_len(want)?;
    }
    Ok(())
}

/// One host directory exposed to the guest at the same path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestShare {
    /// Entry name inside the `hullr-host` multiple-directory share.
    pub name: String,
    /// Path as configured; also the mount point inside the guest.
    pub guest_path: PathBuf,
    /// Canonical host path handed to Virtualization.framework (on macOS `/tmp`
    /// resolves to `/private/tmp`).
    pub host_path: PathBuf,
}

/// Turn configured share paths into a share plan. Paths that do not exist,
/// are not directories, or contain characters the kernel command line cannot
/// carry (whitespace, `,`, `:`) are skipped with a warning. Duplicates and
/// paths nested inside an earlier share are dropped, since they are already
/// visible.
pub fn plan_shares(configured: &[PathBuf]) -> Vec<GuestShare> {
    let mut out: Vec<GuestShare> = Vec::new();
    for p in configured {
        let s = p.to_string_lossy();
        if !p.is_absolute() || s.chars().any(|c| c.is_whitespace() || c == ',' || c == ':') {
            eprintln!("hullr: skipping vm share '{s}': must be absolute without spaces, ',' or ':'");
            continue;
        }
        let Ok(host_path) = std::fs::canonicalize(p) else {
            continue;
        };
        if !host_path.is_dir() {
            continue;
        }
        if out.iter().any(|o| p.starts_with(&o.guest_path) || host_path.starts_with(&o.host_path)) {
            continue;
        }
        out.push(GuestShare { name: format!("s{}", out.len()), guest_path: p.clone(), host_path });
    }
    out
}

/// `name:guest_path`, plus `:host_path` when the canonical host path differs
/// (the guest then symlinks the host spelling to the guest path).
fn share_arg(s: &GuestShare) -> String {
    if s.host_path == s.guest_path {
        format!("{}:{}", s.name, s.guest_path.display())
    } else {
        format!("{}:{}:{}", s.name, s.guest_path.display(), s.host_path.display())
    }
}

/// The full kernel command line for a share plan.
pub fn kernel_cmdline(shares: &[GuestShare], rosetta: bool) -> String {
    let mut cmd = String::from(BASE_CMDLINE);
    if !shares.is_empty() {
        let list: Vec<String> =
            shares.iter().map(share_arg).collect();
        cmd.push_str(" hullr.shares=");
        cmd.push_str(&list.join(","));
    }
    if rosetta {
        cmd.push_str(" hullr.rosetta=1");
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use hullr_core::paths::Paths;

    #[test]
    fn boot_files_lookup_order() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("root"));
        let files = VmFiles::new(&paths);
        let exe = dir.path().join("pkg/bin/hullr");
        let cfg = VmConfig::default();

        let err = resolve_boot_files(&files, &cfg, &exe).unwrap_err();
        assert!(err.to_string().contains("build.sh --install"), "{err}");

        // Packaged layout next to the executable.
        let share = dir.path().join("pkg/bin/../share/hullr/vm");
        std::fs::create_dir_all(&share).unwrap();
        std::fs::write(share.join("kernel"), b"k").unwrap();
        std::fs::write(share.join("initrd"), b"i").unwrap();
        let (k, _) = resolve_boot_files(&files, &cfg, &exe).unwrap();
        assert!(k.ends_with("share/hullr/vm/kernel"));

        // Installed files win over packaged ones.
        std::fs::create_dir_all(&files.dir).unwrap();
        std::fs::write(files.kernel(), b"k").unwrap();
        let (k, i) = resolve_boot_files(&files, &cfg, &exe).unwrap();
        assert_eq!(k, files.kernel());
        assert!(i.ends_with("share/hullr/vm/initrd"));

        // An explicit path that does not exist is an error, not a fallback.
        let cfg = VmConfig { kernel: Some(dir.path().join("nope")), ..VmConfig::default() };
        assert!(resolve_boot_files(&files, &cfg, &exe).is_err());
    }

    #[test]
    fn disk_is_sparse_and_only_grows() {
        let dir = tempfile::tempdir().unwrap();
        let disk = dir.path().join("vm/disk.img");
        ensure_disk(&disk, 2).unwrap();
        assert_eq!(std::fs::metadata(&disk).unwrap().len(), 2 << 30);
        ensure_disk(&disk, 3).unwrap();
        assert_eq!(std::fs::metadata(&disk).unwrap().len(), 3 << 30);
        ensure_disk(&disk, 1).unwrap();
        assert_eq!(std::fs::metadata(&disk).unwrap().len(), 3 << 30);
        use std::os::unix::fs::MetadataExt;
        // Allocated blocks stay far below the apparent size.
        assert!(std::fs::metadata(&disk).unwrap().blocks() * 512 < 1 << 20);
    }

    #[test]
    fn share_plan_and_cmdline() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let nested = a.join("nested");
        let spaced = dir.path().join("has space");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&spaced).unwrap();
        let plan = plan_shares(&[
            a.clone(),
            nested.clone(),
            spaced,
            dir.path().join("missing"),
            a.clone(),
            "relative".into(),
        ]);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].name, "s0");
        assert_eq!(plan[0].guest_path, a);

        let cmd = kernel_cmdline(&plan, true);
        assert!(cmd.starts_with(BASE_CMDLINE));
        let canon = std::fs::canonicalize(&a).unwrap();
        let want = if canon == a { format!("s0:{}", a.display()) } else { format!("s0:{}:{}", a.display(), canon.display()) };
        assert!(cmd.contains(&format!(" hullr.shares={want}")), "{cmd}");
        assert!(cmd.ends_with(" hullr.rosetta=1"));
        assert_eq!(kernel_cmdline(&[], false), BASE_CMDLINE);
    }
}
