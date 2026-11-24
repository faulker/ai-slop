//! Named shared network sandboxes (compose projects use one per project).
//!
//! `__sandbox <dir>` starts detached and forks a holder process H that
//! enters a user namespace (rootless only) and a new network namespace,
//! brings up `lo`, writes `holder.pid` and then sleeps forever. The parent
//! S supervises H: it runs pasta (or slirp4netns) against H with the ports
//! in `ports.json`, restarts it when that file changes (union of all
//! members' ports), and exits, killing the helper, once H is gone.
//! Containers in `NetworkMode::Sandbox` setns into H's user + net
//! namespaces.

use crate::util::{read_line, rt_err, write_all_fd};
use hullr_core::{PortMapping, Result};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn holder_pid(dir: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(dir.join("holder.pid")).ok()?.trim().parse().ok()?;
    crate::state::is_pid_alive(pid).then_some(pid)
}

fn read_ports(dir: &Path) -> Vec<PortMapping> {
    std::fs::read(dir.join("ports.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn main(dir: &Path) -> i32 {
    let start = crate::util::take_handoff_fd();
    crate::util::daemonize();
    crate::util::redirect_stdio(Some(&dir.join("sandbox.log")));
    let report = |m: &str| {
        if let Some(f) = &start {
            write_all_fd(f.as_raw_fd(), format!("{m}\n").as_bytes());
        }
    };
    let (ready_r, ready_w) = match nix::unistd::pipe() {
        Ok(p) => p,
        Err(e) => {
            report(&format!("ERR pipe: {e}"));
            return 1;
        }
    };
    // SAFETY: single-threaded (fresh re-exec), so forking is safe.
    let h = unsafe { libc::fork() };
    if h == 0 {
        drop(ready_r);
        let res = (|| -> Result<()> {
            if crate::util::euid() != 0 {
                crate::ns::enter_userns(&crate::ns::IdMapping::detect())?;
            }
            nix::sched::unshare(nix::sched::CloneFlags::CLONE_NEWNET).map_err(|e| rt_err(format!("unshare net: {e}")))?;
            crate::net::loopback_up().map_err(|e| rt_err(format!("lo up: {e}")))?;
            hullr_core::paths::write_atomic(&dir.join("holder.pid"), std::process::id().to_string().as_bytes())?;
            Ok(())
        })();
        let msg = match res {
            Ok(()) => "OK\n".to_string(),
            Err(e) => format!("ERR {}\n", crate::init::strip(e)),
        };
        write_all_fd(ready_w.as_raw_fd(), msg.as_bytes());
        drop(ready_w);
        if msg != "OK\n" {
            // SAFETY: exit the failed holder without unwinding.
            unsafe { libc::_exit(1) };
        }
        loop {
            // SAFETY: pause just sleeps until a signal arrives.
            unsafe { libc::pause() };
        }
    }
    drop(ready_w);
    let line = read_line(ready_r.as_raw_fd()).unwrap_or_else(|| "ERR holder died".into());
    if line != "OK" {
        report(&line);
        return 1;
    }
    let _ = std::fs::write(dir.join("sandbox.pid"), std::process::id().to_string());
    let tool = crate::net::detect_user_net();
    let mut helper = None;
    let mut applied: Option<Vec<PortMapping>> = None;
    let mut reported = false;
    loop {
        if !crate::state::is_pid_alive(h as u32) {
            break;
        }
        let ports = read_ports(dir);
        if applied.as_ref() != Some(&ports) {
            drop(helper.take());
            if let Some(t) = &tool {
                match crate::net::attach(t, h as u32, &ports, dir) {
                    Ok(x) => helper = Some(x),
                    Err(e) => eprintln!("hullr sandbox: network helper: {e}"),
                }
            }
            let _ = std::fs::write(dir.join("ports.applied"), serde_json::to_vec(&ports).unwrap_or_default());
            applied = Some(ports);
        }
        if !reported {
            report("OK");
            reported = true;
        }
        // Reap the holder if it died (we are its parent).
        let mut st = 0;
        // SAFETY: non-blocking waitpid on our own child.
        unsafe { libc::waitpid(h, &mut st, libc::WNOHANG) };
        std::thread::sleep(Duration::from_millis(250));
    }
    drop(helper);
    0
}

/// Create the sandbox if needed and make sure `ports` are published.
pub fn ensure(exe: &Path, dir: &Path, sandboxes: &Path, ports: &[PortMapping]) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut all = read_ports(dir);
    for p in ports {
        if !all.contains(p) {
            all.push(p.clone());
        }
    }
    let changed = all != read_ports(dir) || !dir.join("ports.json").exists();
    if changed {
        hullr_core::paths::write_atomic(&dir.join("ports.json"), &serde_json::to_vec(&all)?)?;
    }
    if holder_pid(dir).is_some() && sup_alive(dir) {
        if changed {
            let want = serde_json::to_vec(&all)?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if std::fs::read(dir.join("ports.applied")).ok().as_deref() == Some(&want[..]) {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            return Err(rt_err("sandbox did not apply the new ports in time"));
        }
        return Ok(());
    }
    let _ = sandboxes;
    let _ = std::fs::remove_file(dir.join("holder.pid"));
    let line = crate::util::spawn_detached(exe, &["__sandbox".as_ref(), dir.as_os_str()], &[], Duration::from_secs(15))?;
    match line.as_str() {
        "OK" => Ok(()),
        l => Err(rt_err(format!("start sandbox: {}", l.strip_prefix("ERR ").unwrap_or(l)))),
    }
}

fn sup_alive(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("sandbox.pid"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .map(crate::state::is_pid_alive)
        .unwrap_or(false)
}

pub fn remove(dir: &Path) -> Result<()> {
    if let Some(h) = holder_pid(dir) {
        // SAFETY: signalling a pid we recorded.
        unsafe { libc::kill(h as i32, libc::SIGKILL) };
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while sup_alive(dir) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn dir_for(sandboxes: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return Err(hullr_core::Error::Invalid(format!("bad sandbox name '{name}'")));
    }
    Ok(sandboxes.join(name))
}
