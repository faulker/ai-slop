//! run, ps, logs, exec, attach, stats and friends.

use crate::cli::RunArgs;
use crate::output::{self, ago, table, truncate};
use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use hullr_core::*;
use std::future::Future;
use std::io::Write;

/// Run `f` for each container, printing each name like Docker; exit 1 if
/// any failed.
pub async fn each<'a, F, Fut>(ids: &'a [String], f: F) -> Result<i32>
where
    F: Fn(&'a str) -> Fut,
    Fut: Future<Output = hullr_core::Result<()>>,
{
    let mut code = 0;
    for id in ids {
        match f(id).await {
            Ok(()) => println!("{id}"),
            Err(e) => {
                output::error(&format!("{id}: {e}"));
                code = 1;
            }
        }
    }
    Ok(code)
}

pub fn parse_memory(s: &str) -> Result<u64> {
    let s = s.trim().to_ascii_lowercase();
    let s = s.trim_end_matches('b');
    let (num, mul) = match s.chars().last() {
        Some('k') => (&s[..s.len() - 1], 1u64 << 10),
        Some('m') => (&s[..s.len() - 1], 1 << 20),
        Some('g') => (&s[..s.len() - 1], 1 << 30),
        _ => (s, 1),
    };
    let n: f64 = num.parse().map_err(|_| anyhow!("bad memory size '{s}'"))?;
    Ok((n * mul as f64) as u64)
}

fn read_env_file(p: &std::path::Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(p).with_context(|| format!("reading env file {}", p.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}

pub fn spec_from_args(a: &RunArgs) -> Result<ContainerSpec> {
    let cwd = std::env::current_dir()?;
    let mut env = Vec::new();
    for f in &a.env_file {
        env.extend(read_env_file(f)?);
    }
    env.extend(a.env.iter().cloned());
    let mut mounts = a
        .volume
        .iter()
        .map(|v| Mount::parse(v, &cwd))
        .collect::<hullr_core::Result<Vec<_>>>()?;
    for t in &a.tmpfs {
        mounts.push(Mount { kind: MountKind::Tmpfs, source: String::new(), target: t.clone(), read_only: false });
    }
    let network = match a.network.as_deref() {
        None | Some("private") | Some("bridge") | Some("default") => NetworkMode::Private,
        Some("host") => NetworkMode::Host,
        Some("none") => NetworkMode::None,
        Some(s) if s.starts_with("container:") => NetworkMode::Container(s[10..].to_string()),
        Some(s) if s.starts_with("sandbox:") => NetworkMode::Sandbox(s[8..].to_string()),
        Some(s) => bail!("unknown network '{s}' (use private, host, none or container:NAME)"),
    };
    let mut labels = std::collections::BTreeMap::new();
    for l in &a.label {
        let (k, v) = l.split_once('=').unwrap_or((l, ""));
        labels.insert(k.to_string(), v.to_string());
    }
    let extra_hosts = a
        .add_host
        .iter()
        .map(|h| {
            h.split_once(':')
                .map(|(n, ip)| (n.to_string(), ip.to_string()))
                .ok_or_else(|| anyhow!("bad --add-host '{h}' (want HOST:IP)"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ContainerSpec {
        name: a.name.clone(),
        image: a.image.clone(),
        platform: a.platform.as_deref().map(Platform::parse).transpose()?,
        entrypoint: a.entrypoint.as_ref().map(|e| if e.is_empty() { vec![] } else { vec![e.clone()] }),
        cmd: if a.command.is_empty() { None } else { Some(a.command.clone()) },
        env,
        working_dir: a.workdir.clone(),
        user: a.user.clone(),
        hostname: a.hostname.clone(),
        ports: a.publish.iter().map(|p| PortMapping::parse(p)).collect::<hullr_core::Result<_>>()?,
        mounts,
        network,
        extra_hosts,
        labels,
        tty: a.tty,
        interactive: a.interactive,
        auto_remove: a.rm,
        restart: RestartPolicy::parse(&a.restart)?,
        healthcheck: None,
        pull: match a.pull.as_str() {
            "missing" => PullPolicy::Missing,
            "always" => PullPolicy::Always,
            "never" => PullPolicy::Never,
            o => bail!("bad --pull '{o}' (missing, always, never)"),
        },
        memory_limit: a.memory.as_deref().map(parse_memory).transpose()?,
        cpus: a.cpus,
    })
}

pub async fn run(api: &dyn Api, a: RunArgs, create_only: bool) -> Result<i32> {
    if a.rm && a.restart != "no" {
        bail!("--rm and --restart cannot be combined");
    }
    let spec = spec_from_args(&a)?;
    if let Some(p) = &spec.platform {
        if !p.is_native() {
            eprintln!("{}", output::dim(&format!("Running {p} under emulation")));
        }
    }
    let id = api.create_container(spec, output::progress_printer()).await?;
    if create_only {
        println!("{id}");
        return Ok(0);
    }
    if a.detach {
        if let Err(e) = api.start_container(&id, false).await {
            cleanup_failed(api, &id, a.rm).await;
            return Err(e.into());
        }
        println!("{id}");
        return Ok(0);
    }
    if let Err(e) = api.start_container(&id, true).await {
        cleanup_failed(api, &id, a.rm).await;
        return Err(e.into());
    }
    let code = attach_and_wait(api, &id, a.tty, a.interactive).await?;
    if a.rm && code != hullr_tui::term::DETACHED {
        let _ = api.wait_container(&id).await;
        let _ = api.remove_container(&id, true).await;
    }
    Ok(normalize_exit(code))
}

async fn cleanup_failed(api: &dyn Api, id: &str, rm: bool) {
    if rm {
        let _ = api.remove_container(id, true).await;
    }
}

fn normalize_exit(code: i32) -> i32 {
    if code == hullr_tui::term::DETACHED {
        0
    } else {
        code
    }
}

/// Attach the terminal; forward Ctrl-C as SIGINT when there is no tty.
async fn attach_and_wait(api: &dyn Api, id: &str, tty: bool, interactive: bool) -> Result<i32> {
    let stream = api.attach(id).await?;
    let code = if tty {
        hullr_tui::term::pump_with(stream, true, interactive).await?
    } else {
        let pump = hullr_tui::term::pump_with(stream, false, interactive);
        tokio::pin!(pump);
        loop {
            tokio::select! {
                r = &mut pump => break r?,
                _ = tokio::signal::ctrl_c() => {
                    let _ = api.kill_container(id, "INT").await;
                }
            }
        }
    };
    if code == hullr_tui::term::DETACHED {
        eprintln!("\nDetached from {}. It keeps running; reattach with `hullr attach {}`.", &id[..12.min(id.len())], &id[..12.min(id.len())]);
    }
    Ok(code)
}

pub async fn start(api: &dyn Api, ids: &[String], attach: bool, interactive: bool) -> Result<i32> {
    if attach || interactive {
        let [id] = ids else { bail!("you can only attach to one container at a time") };
        let info = api.inspect_container(id).await?;
        api.start_container(&info.id, true).await?;
        let code = attach_and_wait(api, &info.id, info.spec.tty, interactive).await?;
        return Ok(normalize_exit(code));
    }
    each(ids, |id| api.start_container(id, false)).await
}

pub async fn attach(api: &dyn Api, id: &str) -> Result<i32> {
    let info = api.inspect_container(id).await?;
    if info.state != ContainerState::Running {
        bail!("container {} is not running", info.name);
    }
    let code = attach_and_wait(api, &info.id, info.spec.tty, info.spec.interactive).await?;
    Ok(normalize_exit(code))
}

pub async fn ps(api: &dyn Api, all: bool, quiet: bool) -> Result<i32> {
    let list = api.list_containers(all).await?;
    if quiet {
        for c in &list {
            println!("{}", c.short_id());
        }
        return Ok(0);
    }
    if list.is_empty() {
        if all {
            eprintln!("No containers. Start one with: hullr run -d nginx");
        } else {
            eprintln!("No running containers. Use `hullr ps -a` to include stopped ones.");
        }
        return Ok(0);
    }
    let now = now_unix();
    let rows: Vec<Vec<String>> = list
        .iter()
        .map(|c| {
            vec![
                c.short_id().to_string(),
                truncate(&c.image, 30),
                format!("\"{}\"", truncate(&c.command.join(" "), 22)),
                ago(c.created),
                c.status_text(now),
                c.spec.ports.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
                c.name.clone(),
            ]
        })
        .collect();
    table(&["CONTAINER ID", "IMAGE", "COMMAND", "CREATED", "STATUS", "PORTS", "NAMES"], &rows);
    Ok(0)
}

pub async fn logs(api: &dyn Api, id: &str, follow: bool, tail: Option<usize>, timestamps: bool) -> Result<i32> {
    let mut s = api.logs(id, LogOptions { follow, tail, timestamps }).await?;
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    loop {
        tokio::select! {
            item = s.next() => {
                let Some(item) = item else { break };
                let l = item?;
                let line = if timestamps { format!("{} {}", fmt_ts(l.ts), l.line) } else { l.line };
                match l.stream {
                    LogStream::Stdout => { let _ = writeln!(out, "{line}"); }
                    LogStream::Stderr => { let _ = writeln!(err, "{line}"); }
                }
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(0)
}

/// RFC3339-ish UTC timestamp from unix millis, without a date crate.
pub fn fmt_ts(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms.rem_euclid(1000)
    )
}

#[allow(clippy::too_many_arguments)]
pub async fn exec(
    api: &dyn Api,
    id: &str,
    cmd: Vec<String>,
    interactive: bool,
    tty: bool,
    env: Vec<String>,
    workdir: Option<String>,
    user: Option<String>,
) -> Result<i32> {
    let spec = ExecSpec { cmd, env, working_dir: workdir, user, tty, interactive };
    let stream = api.exec(id, spec).await?;
    let code = hullr_tui::term::pump_with(stream, tty, interactive).await?;
    Ok(normalize_exit(code))
}

pub async fn inspect(api: &dyn Api, targets: &[String]) -> Result<i32> {
    let mut out = Vec::new();
    let mut code = 0;
    for t in targets {
        match api.inspect_container(t).await {
            Ok(c) => out.push(serde_json::to_value(c)?),
            Err(_) => match api.inspect_image(t).await {
                Ok(i) => out.push(serde_json::to_value(i)?),
                Err(e) => {
                    output::error(&format!("{t}: {e}"));
                    code = 1;
                }
            },
        }
    }
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(code)
}

pub async fn stats(api: &dyn Api, ids: &[String], no_stream: bool) -> Result<i32> {
    let tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
    loop {
        let list: Vec<ContainerInfo> = if ids.is_empty() {
            api.list_containers(false).await?
        } else {
            let mut v = Vec::new();
            for id in ids {
                v.push(api.inspect_container(id).await?);
            }
            v
        };
        let mut rows = Vec::new();
        for c in &list {
            let s = api.stats(&c.id).await.unwrap_or_default();
            let mem = match s.memory_limit {
                Some(l) => format!("{} / {}", human_bytes(s.memory_bytes), human_bytes(l)),
                None => human_bytes(s.memory_bytes),
            };
            rows.push(vec![
                c.short_id().to_string(),
                c.name.clone(),
                format!("{:.2}%", s.cpu_percent),
                mem,
                format!("{} / {}", human_bytes(s.net_rx_bytes), human_bytes(s.net_tx_bytes)),
                s.pids.to_string(),
            ]);
        }
        if tty && !no_stream {
            print!("\x1b[2J\x1b[H");
        }
        table(&["CONTAINER ID", "NAME", "CPU %", "MEM USAGE", "NET I/O", "PIDS"], &rows);
        if no_stream {
            return Ok(0);
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
            _ = tokio::signal::ctrl_c() => return Ok(0),
        }
    }
}

pub async fn wait(api: &dyn Api, ids: &[String]) -> Result<i32> {
    for id in ids {
        println!("{}", api.wait_container(id).await?);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> RunArgs {
        let mut v = vec!["hullr", "run"];
        v.extend_from_slice(args);
        match crate::cli::Cli::parse_from(v).command {
            crate::cli::Command::Run(a) => a,
            _ => unreachable!(),
        }
    }

    #[test]
    fn run_flags() {
        let a = parse(&["-d", "--rm", "-p", "8080:80", "-e", "A=1", "--platform", "linux/arm64", "nginx", "nginx", "-g", "daemon off;"]);
        let s = spec_from_args(&a).unwrap();
        assert!(a.detach && s.auto_remove);
        assert_eq!(s.ports[0].host_port, 8080);
        assert_eq!(s.platform.unwrap().architecture, "arm64");
        assert_eq!(s.cmd.unwrap(), vec!["nginx", "-g", "daemon off;"]);
        let a = parse(&["--network", "host", "-it", "alpine"]);
        let s = spec_from_args(&a).unwrap();
        assert_eq!(s.network, NetworkMode::Host);
        assert!(s.tty && s.interactive && s.cmd.is_none());
    }

    #[test]
    fn memory() {
        assert_eq!(parse_memory("512m").unwrap(), 512 << 20);
        assert_eq!(parse_memory("1g").unwrap(), 1 << 30);
        assert!(parse_memory("lots").is_err());
    }

    #[test]
    fn timestamps() {
        assert_eq!(fmt_ts(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(fmt_ts(1_700_000_000_123), "2023-11-14T22:13:20.123Z");
    }
}
