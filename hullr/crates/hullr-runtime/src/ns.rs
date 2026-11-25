//! Namespace plumbing: entering a rootless user namespace (single mapping,
//! or the full subuid/subgid range through newuidmap/newgidmap), and joining
//! the namespaces of an existing process.
//!
//! Everything here runs in a freshly re-exec'd, single-threaded helper, so
//! forking is safe.

use crate::user::{find_subid, SubIdRange};
use crate::util::rt_err;
use hullr_core::Result;
use nix::sched::{setns, unshare, CloneFlags};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::process::Command;

/// The id mapping a rootless user namespace gets.
#[derive(Debug, Clone)]
pub struct IdMapping {
    pub uid: u32,
    pub gid: u32,
    pub sub_uid: Option<SubIdRange>,
    pub sub_gid: Option<SubIdRange>,
}

impl IdMapping {
    pub fn detect() -> Self {
        // SAFETY: getuid/getgid have no preconditions.
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        let name = nix::unistd::User::from_uid(uid.into()).ok().flatten().map(|u| u.name).unwrap_or_default();
        let tools = crate::net::find_on_path("newuidmap").is_some() && crate::net::find_on_path("newgidmap").is_some();
        let lookup = |f: &str| {
            if !tools || std::env::var_os("HULLR_NO_SUBUID").is_some() {
                return None;
            }
            find_subid(&std::fs::read_to_string(f).ok()?, &name, uid)
        };
        let sub_uid = lookup("/etc/subuid");
        let sub_gid = lookup("/etc/subgid");
        // Both or neither: a half mapping is more confusing than useful.
        if sub_uid.is_some() && sub_gid.is_some() {
            Self { uid, gid, sub_uid, sub_gid }
        } else {
            Self { uid, gid, sub_uid: None, sub_gid: None }
        }
    }

    pub fn full(&self) -> bool {
        self.sub_uid.is_some()
    }
}

/// unshare(CLONE_NEWUSER) and install the mapping: container 0 maps to the
/// caller's uid/gid, plus 1..=count to the subordinate range if available.
pub fn enter_userns(map: &IdMapping) -> Result<()> {
    if !map.full() {
        unshare(CloneFlags::CLONE_NEWUSER).map_err(|e| rt_err(format!("unshare user namespace: {e}")))?;
        let w = |f: &str, s: String| std::fs::write(f, s).map_err(|e| rt_err(format!("write {f}: {e}")));
        w("/proc/self/setgroups", "deny".into())?;
        w("/proc/self/uid_map", format!("0 {} 1\n", map.uid))?;
        w("/proc/self/gid_map", format!("0 {} 1\n", map.gid))?;
        return Ok(());
    }
    // newuidmap must run outside the new namespace: fork a helper first,
    // it waits until we unshared, maps us, and reports back.
    let (to_helper_r, to_helper_w) = nix::unistd::pipe().map_err(|e| rt_err(format!("pipe: {e}")))?;
    let (from_helper_r, from_helper_w) = nix::unistd::pipe().map_err(|e| rt_err(format!("pipe: {e}")))?;
    let me = std::process::id();
    // SAFETY: single-threaded process (fresh re-exec), see module docs.
    let child = unsafe { libc::fork() };
    if child < 0 {
        return Err(rt_err("fork id-map helper"));
    }
    if child == 0 {
        drop(to_helper_w);
        drop(from_helper_r);
        let _ = crate::util::read_line(to_helper_r.as_raw_fd());
        let su = map.sub_uid.unwrap();
        let sg = map.sub_gid.unwrap();
        let run = |bin: &str, id: u32, r: SubIdRange| -> std::result::Result<(), String> {
            let out = Command::new(bin)
                .args([me.to_string(), "0".into(), id.to_string(), "1".into(), "1".into(), r.start.to_string(), r.count.to_string()])
                .output()
                .map_err(|e| format!("{bin}: {e}"))?;
            if out.status.success() {
                Ok(())
            } else {
                Err(format!("{bin}: {}", String::from_utf8_lossy(&out.stderr).trim()))
            }
        };
        let msg = match run("newuidmap", map.uid, su).and_then(|_| run("newgidmap", map.gid, sg)) {
            Ok(()) => "OK\n".to_string(),
            Err(e) => format!("ERR {e}\n"),
        };
        crate::util::write_all_fd(from_helper_w.as_raw_fd(), msg.as_bytes());
        // SAFETY: leave the forked helper without running destructors.
        unsafe { libc::_exit(0) };
    }
    drop(to_helper_r);
    drop(from_helper_w);
    let res = unshare(CloneFlags::CLONE_NEWUSER);
    crate::util::write_all_fd(to_helper_w.as_raw_fd(), b"go\n");
    let reply = crate::util::read_line(from_helper_r.as_raw_fd()).unwrap_or_default();
    crate::util::wait_pid(child);
    res.map_err(|e| rt_err(format!("unshare user namespace: {e}")))?;
    if let Some(e) = reply.strip_prefix("ERR ") {
        return Err(rt_err(format!("id mapping: {e}")));
    }
    if reply != "OK" {
        return Err(rt_err("id mapping helper failed"));
    }
    Ok(())
}

/// Namespace identity (`user:[4026531837]`) of `pid` (or `self`).
pub fn ns_id(pid: &str, ns: &str) -> Option<String> {
    std::fs::read_link(format!("/proc/{pid}/ns/{ns}")).ok().map(|p| p.to_string_lossy().into_owned())
}

/// Join the user namespace of `pid` if it differs from ours.
pub fn join_userns_of(pid: u32) -> Result<bool> {
    if ns_id(&pid.to_string(), "user") == ns_id("self", "user") {
        return Ok(false);
    }
    join(pid, "user", CloneFlags::CLONE_NEWUSER)?;
    Ok(true)
}

pub fn join(pid: u32, ns: &str, flag: CloneFlags) -> Result<()> {
    let f = File::open(format!("/proc/{pid}/ns/{ns}")).map_err(|e| rt_err(format!("open {ns} namespace of {pid}: {e}")))?;
    setns(&f, flag).map_err(|e| rt_err(format!("join {ns} namespace of {pid}: {e}")))
}

/// Become root (uid/gid 0) in the current user namespace.
pub fn become_root() {
    // SAFETY: plain credential syscalls; failures are ignored since we may
    // already be uid 0.
    unsafe {
        libc::setresgid(0, 0, 0);
        libc::setresuid(0, 0, 0);
    }
}
