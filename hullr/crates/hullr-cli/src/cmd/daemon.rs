//! `hullr daemon ...`: optional background service.
//!
//! The daemon is a convenience, not a requirement: containers are owned by
//! their shims and outlive both the CLI and the daemon. What the daemon adds
//! is a shared API socket (TUI/GUI/remote), healthchecks, cleanup of `--rm`
//! containers and restoring restart policies at boot.

use crate::cli::{DaemonCmd, Global};
use crate::connect::exe;
use anyhow::{bail, Context, Result};
use hullr_core::paths::Paths;
use std::path::PathBuf;
use std::time::Duration;

fn pid_file(p: &Paths) -> PathBuf {
    p.root.join("hullrd.pid")
}

fn socket(g: &Global, p: &Paths) -> PathBuf {
    g.socket.clone().unwrap_or_else(|| p.socket())
}

pub async fn run(c: DaemonCmd, g: &Global) -> Result<i32> {
    let paths = Paths::from_env();
    match c {
        DaemonCmd::Run { listen } => run_foreground(&paths, g, listen).await,
        DaemonCmd::Start => start(&paths, g).await,
        DaemonCmd::Stop => stop(&paths, g).await,
        DaemonCmd::Status => {
            let up = hullr_api::Client::new(socket(g, &paths)).ping().await;
            println!("{}", if up { "running" } else { "not running" });
            Ok(if up { 0 } else { 3 })
        }
        DaemonCmd::Install { system } => install(system),
        DaemonCmd::Uninstall { system } => uninstall(system),
    }
}

#[cfg(target_os = "macos")]
async fn run_foreground(_paths: &Paths, _g: &Global, _listen: Vec<String>) -> Result<i32> {
    bail!("on macOS the daemon runs inside the hullr VM; use `hullr daemon start` or `hullr vm start`")
}

#[cfg(not(target_os = "macos"))]
async fn run_foreground(paths: &Paths, g: &Global, listen: Vec<String>) -> Result<i32> {
    use std::sync::Arc;
    let listen: Vec<hullr_api::Listen> = if listen.is_empty() {
        vec![hullr_api::Listen::Unix(socket(g, paths))]
    } else {
        listen.iter().map(|l| l.parse()).collect::<hullr_core::Result<_>>()?
    };
    let engine = hullr_engine::Engine::open(paths.clone(), exe(), "daemon")?;
    hullr_core::paths::write_atomic(&pid_file(paths), std::process::id().to_string().as_bytes())?;
    engine.restore_restart_policies().await;
    tokio::spawn(hullr_engine::health_loop(engine.clone()));
    let api: Arc<dyn hullr_core::Api> = engine;
    let mut tasks = Vec::new();
    for l in listen {
        eprintln!("hullr daemon listening on {l:?}");
        let api = api.clone();
        tasks.push(tokio::spawn(async move { hullr_api::serve(api, l).await }));
    }
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let result = tokio::select! {
        _ = term.recv() => Ok(0),
        _ = tokio::signal::ctrl_c() => Ok(0),
        r = futures::future::select_all(tasks) => match r.0 {
            Ok(Err(e)) => Err(anyhow::Error::from(e)),
            Ok(Ok(())) => Ok(0),
            Err(e) => Err(anyhow::anyhow!("listener task failed: {e}")),
        },
    };
    let _ = std::fs::remove_file(socket(g, paths));
    let _ = std::fs::remove_file(pid_file(paths));
    eprintln!("hullr daemon stopped (containers keep running)");
    result
}

#[cfg(target_os = "macos")]
async fn start(paths: &Paths, _g: &Global) -> Result<i32> {
    let sock = hullr_vm::ensure_running(paths, &exe()).await?;
    println!("hullr VM running; API at {}", sock.display());
    Ok(0)
}

#[cfg(not(target_os = "macos"))]
async fn start(paths: &Paths, g: &Global) -> Result<i32> {
    use std::os::unix::process::CommandExt;
    let sock = socket(g, paths);
    let client = hullr_api::Client::new(&sock);
    if client.ping().await {
        println!("already running");
        return Ok(0);
    }
    paths.ensure()?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(paths.root.join("daemon.log"))?;
    let mut cmd = std::process::Command::new(exe());
    cmd.args(["daemon", "run"]);
    if let Some(s) = &g.socket {
        cmd.arg("--socket").arg(s);
    }
    cmd.stdin(std::process::Stdio::null()).stdout(log.try_clone()?).stderr(log);
    // SAFETY: setsid in the child before exec detaches it from our session.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().context("spawning the daemon")?;
    for _ in 0..50 {
        if client.ping().await {
            println!("hullr daemon started");
            return Ok(0);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("the daemon did not come up; see {}", paths.root.join("daemon.log").display())
}

#[cfg(target_os = "macos")]
async fn stop(paths: &Paths, _g: &Global) -> Result<i32> {
    hullr_vm::stop(paths).await?;
    println!("hullr VM stopped");
    Ok(0)
}

#[cfg(not(target_os = "macos"))]
async fn stop(paths: &Paths, g: &Global) -> Result<i32> {
    let Ok(pid) = std::fs::read_to_string(pid_file(paths)) else {
        println!("not running");
        return Ok(0);
    };
    let pid: i32 = pid.trim().parse().context("bad pid file")?;
    // SAFETY: signalling a pid we recorded; ESRCH just means it is gone.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let client = hullr_api::Client::new(socket(g, paths));
    for _ in 0..50 {
        if !client.ping().await {
            println!("hullr daemon stopped");
            return Ok(0);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("daemon (pid {pid}) did not stop")
}

const UNIT: &str = "hullr.service";
const PLIST: &str = "dev.hullr.vm.plist";

/// Render the systemd unit. `KillMode=process` keeps container shims alive
/// when the daemon restarts or stops.
pub fn systemd_unit(exe: &std::path::Path) -> String {
    format!(
        "[Unit]\nDescription=hullr container daemon\nAfter=network-online.target\n\n\
         [Service]\nType=simple\nExecStart={} daemon run\nRestart=on-failure\nKillMode=process\n\n\
         [Install]\nWantedBy=default.target\n",
        exe.display()
    )
}

pub fn launchd_plist(exe: &std::path::Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.hullr.vm</string>
  <key>ProgramArguments</key><array><string>{}</string><string>__vm</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#,
        exe.display()
    )
}

fn sh(cmd: &str, args: &[&str]) -> Result<()> {
    let st = std::process::Command::new(cmd).args(args).status().with_context(|| format!("running {cmd}"))?;
    if !st.success() {
        bail!("`{cmd} {}` failed", args.join(" "));
    }
    Ok(())
}

fn install(system: bool) -> Result<i32> {
    let exe = std::fs::canonicalize(exe())?;
    if cfg!(target_os = "macos") {
        let dir = dirs::home_dir().context("no home dir")?.join("Library/LaunchAgents");
        std::fs::create_dir_all(&dir)?;
        let p = dir.join(PLIST);
        std::fs::write(&p, launchd_plist(&exe))?;
        let _ = sh("launchctl", &["unload", &p.to_string_lossy()]);
        sh("launchctl", &["load", "-w", &p.to_string_lossy()])?;
        println!("Installed launchd agent {} (the hullr VM starts at login)", p.display());
        return Ok(0);
    }
    let (dir, ctl): (PathBuf, Vec<&str>) = if system {
        (PathBuf::from("/etc/systemd/system"), vec![])
    } else {
        (dirs::config_dir().context("no config dir")?.join("systemd/user"), vec!["--user"])
    };
    std::fs::create_dir_all(&dir)?;
    let p = dir.join(UNIT);
    std::fs::write(&p, systemd_unit(&exe)).with_context(|| format!("writing {}", p.display()))?;
    let mut a = ctl.clone();
    a.push("daemon-reload");
    sh("systemctl", &a)?;
    let mut a = ctl.clone();
    a.extend(["enable", "--now", UNIT]);
    sh("systemctl", &a)?;
    println!("Installed and started {}", p.display());
    if !system {
        println!("Tip: `loginctl enable-linger $USER` keeps it running when you are logged out.");
    }
    Ok(0)
}

fn uninstall(system: bool) -> Result<i32> {
    if cfg!(target_os = "macos") {
        let p = dirs::home_dir().context("no home dir")?.join("Library/LaunchAgents").join(PLIST);
        let _ = sh("launchctl", &["unload", &p.to_string_lossy()]);
        let _ = std::fs::remove_file(&p);
        println!("Removed {}", p.display());
        return Ok(0);
    }
    let (dir, ctl): (PathBuf, Vec<&str>) = if system {
        (PathBuf::from("/etc/systemd/system"), vec![])
    } else {
        (dirs::config_dir().context("no config dir")?.join("systemd/user"), vec!["--user"])
    };
    let mut a = ctl.clone();
    a.extend(["disable", "--now", UNIT]);
    let _ = sh("systemctl", &a);
    let _ = std::fs::remove_file(dir.join(UNIT));
    let mut a = ctl;
    a.push("daemon-reload");
    let _ = sh("systemctl", &a);
    println!("Removed {UNIT}");
    Ok(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn unit_keeps_shims_alive() {
        let u = super::systemd_unit(std::path::Path::new("/usr/bin/hullr"));
        assert!(u.contains("ExecStart=/usr/bin/hullr daemon run"));
        assert!(u.contains("KillMode=process"));
        assert!(super::launchd_plist(std::path::Path::new("/x/hullr")).contains("<string>__vm</string>"));
    }
}
