//! Best-effort cgroup v2 limits. Used only when `/sys/fs/cgroup` is a
//! cgroup2 mount and we may create children: as root under
//! `/sys/fs/cgroup/hullr/`, rootless under our own (delegated) cgroup when
//! the memory and cpu controllers are already enabled for its children.
//! Anything else (cgroup v1, no delegation) means no limits and /proc-based
//! stats.

use std::path::{Path, PathBuf};

const ROOT: &str = "/sys/fs/cgroup";

pub fn is_v2() -> bool {
    nix::sys::statfs::statfs(ROOT)
            .map(|s| s.filesystem_type() == nix::sys::statfs::CGROUP2_SUPER_MAGIC)
            .unwrap_or(false)
}

fn own_cgroup() -> Option<PathBuf> {
    let s = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let rel = s.lines().find_map(|l| l.strip_prefix("0::"))?;
    Some(Path::new(ROOT).join(rel.trim_start_matches('/')))
}

/// Parent directory for container cgroups, if usable.
pub fn base() -> Option<PathBuf> {
    if !is_v2() {
        return None;
    }
    if crate::util::euid() == 0 {
        let b = Path::new(ROOT).join("hullr");
        std::fs::create_dir_all(&b).ok()?;
        let _ = std::fs::write(Path::new(ROOT).join("cgroup.subtree_control"), "+memory +cpu +pids");
        let _ = std::fs::write(b.join("cgroup.subtree_control"), "+memory +cpu +pids");
        return Some(b);
    }
    let own = own_cgroup()?;
    let ctl = std::fs::read_to_string(own.join("cgroup.subtree_control")).ok()?;
    let probe = own.join("cgroup.procs");
    let writable = nix::unistd::access(&probe, nix::unistd::AccessFlags::W_OK).is_ok();
    (writable && ctl.contains("memory") && ctl.contains("cpu")).then_some(own)
}

pub fn path_for(id: &str) -> Option<PathBuf> {
    let p = base()?.join(format!("hullr-{}", &id[..id.len().min(12)]));
    p.exists().then_some(p)
}

/// Create the container cgroup and apply limits. Returns its path.
pub fn create(id: &str, memory: Option<u64>, cpus: Option<f64>) -> Result<PathBuf, String> {
    let b = base().ok_or("cgroup v2 delegation not available")?;
    let p = b.join(format!("hullr-{}", &id[..id.len().min(12)]));
    std::fs::create_dir_all(&p).map_err(|e| format!("create cgroup: {e}"))?;
    if let Some(m) = memory {
        std::fs::write(p.join("memory.max"), m.to_string()).map_err(|e| format!("memory.max: {e}"))?;
        let _ = std::fs::write(p.join("memory.swap.max"), "0");
    }
    if let Some(c) = cpus {
        let period = 100_000u64;
        let quota = ((c * period as f64) as u64).max(1000);
        std::fs::write(p.join("cpu.max"), format!("{quota} {period}")).map_err(|e| format!("cpu.max: {e}"))?;
    }
    Ok(p)
}

pub fn add_pid(cg: &Path, pid: u32) -> Result<(), String> {
    std::fs::write(cg.join("cgroup.procs"), pid.to_string()).map_err(|e| format!("join cgroup: {e}"))
}

pub fn remove(id: &str) {
    if let Some(p) = path_for(id) {
        let _ = std::fs::remove_dir(p);
    }
}

/// (memory bytes, memory limit, pids, cpu usage in microseconds).
pub fn read_stats(cg: &Path) -> Option<(u64, Option<u64>, u32, u64)> {
    let read = |f: &str| std::fs::read_to_string(cg.join(f)).ok();
    let mem = read("memory.current")?.trim().parse().ok()?;
    let limit = read("memory.max").and_then(|s| s.trim().parse().ok());
    let pids = read("pids.current").and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    let cpu = read("cpu.stat")
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("usage_usec ").map(|v| v.trim().parse().ok()))?)
        .unwrap_or(0);
    Some((mem, limit, pids, cpu))
}
