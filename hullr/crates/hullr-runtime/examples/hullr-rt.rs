//! Tiny driver for the runtime, used by the integration tests and for
//! manual checks. `main` calls `hidden_entry()` first, exactly like the
//! real `hullr` binary.
//!
//! ```text
//! HULLR_RT_ROOT=/tmp/rt hullr-rt create <id> <lower> [opts] -- cmd...
//!   opts: -t -i --net host|none|private|sandbox:N|container:ID
//!         -v SRC:DST[:ro] --tmpfs DST -p H:C --restart P --user U
//!         --platform P -e K=V --memory BYTES --cpus N
//! hullr-rt start <id> [--attach] | attach <id> | exec [-t] [-i] <id> -- cmd...
//! hullr-rt stop <id> [secs] | kill <id> SIG | wait <id> | state <id> | rm <id>
//! hullr-rt logs <id> | stats <id> | caps | emulation-enable
//! hullr-rt run-rootfs <dir> [--platform P] -- cmd... | sandbox <name> [H:C...] | rmsandbox <name>
//! ```

use hullr_core::stream::{read_frame, write_frame, Frame};
use hullr_core::{ExecSpec, MountKind, NetworkMode, Platform, PortMapping, RestartPolicy};
use hullr_runtime::{ResolvedMount, RunSpec, Runtime, RuntimeConfig};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn main() {
    if let Some(code) = hullr_runtime::hidden_entry() {
        std::process::exit(code);
    }
    let rt = tokio::runtime::Runtime::new().unwrap();
    let code = rt.block_on(run()).unwrap_or_else(|e| {
        eprintln!("hullr-rt: {e}");
        1
    });
    std::process::exit(code);
}

fn runtime() -> Runtime {
    let root = PathBuf::from(std::env::var("HULLR_RT_ROOT").unwrap_or_else(|_| "/tmp/hullr-rt".into()));
    let cfg = RuntimeConfig {
        containers_dir: root.join("containers"),
        sandboxes_dir: root.join("sandboxes"),
        exe: std::env::current_exe().unwrap(),
    };
    std::fs::create_dir_all(&cfg.containers_dir).unwrap();
    std::fs::create_dir_all(&cfg.sandboxes_dir).unwrap();
    Runtime::new(cfg).unwrap()
}

fn split_dashdash(args: &[String]) -> (Vec<String>, Vec<String>) {
    match args.iter().position(|a| a == "--") {
        Some(i) => (args[..i].to_vec(), args[i + 1..].to_vec()),
        None => (args.to_vec(), vec![]),
    }
}

async fn run() -> hullr_core::Result<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rt = runtime();
    let bad = || hullr_core::Error::Invalid("usage: see examples/hullr-rt.rs".into());
    let cmd = args.first().ok_or_else(bad)?.as_str();
    let rest = &args[1..];
    let id = || rest.iter().find(|a| !a.starts_with('-')).cloned().ok_or_else(bad);
    match cmd {
        "create" => {
            let (opts, argv) = split_dashdash(rest);
            let mut spec = RunSpec {
                id: opts.first().cloned().ok_or_else(bad)?,
                hostname: String::new(),
                lower: opts.get(1).map(PathBuf::from).ok_or_else(bad)?,
                args: argv,
                env: vec![],
                cwd: "/".into(),
                user: None,
                mounts: vec![],
                ports: vec![],
                network: NetworkMode::Private,
                extra_hosts: vec![],
                tty: false,
                interactive: false,
                platform: Platform::host(),
                restart: RestartPolicy::No,
                stop_signal: None,
                memory_limit: None,
                cpus: None,
            };
            spec.hostname = spec.id[..spec.id.len().min(12)].to_string();
            let mut it = opts[2..].iter();
            while let Some(o) = it.next() {
                let mut val = || it.next().cloned().ok_or_else(bad);
                match o.as_str() {
                    "-t" => spec.tty = true,
                    "-i" => spec.interactive = true,
                    "-w" => spec.cwd = val()?,
                    "--net" => {
                        let v = val()?;
                        spec.network = match v.as_str() {
                            "host" => NetworkMode::Host,
                            "none" => NetworkMode::None,
                            "private" => NetworkMode::Private,
                            s if s.starts_with("sandbox:") => NetworkMode::Sandbox(s[8..].into()),
                            s if s.starts_with("container:") => NetworkMode::Container(s[10..].into()),
                            _ => return Err(bad()),
                        }
                    }
                    "-v" => {
                        let v = val()?;
                        let p: Vec<&str> = v.split(':').collect();
                        spec.mounts.push(ResolvedMount {
                            kind: MountKind::Bind,
                            source: p[0].into(),
                            target: p.get(1).ok_or_else(bad)?.to_string(),
                            read_only: p.get(2) == Some(&"ro"),
                        });
                    }
                    "--tmpfs" => spec.mounts.push(ResolvedMount {
                        kind: MountKind::Tmpfs,
                        source: PathBuf::new(),
                        target: val()?,
                        read_only: false,
                    }),
                    "-p" => spec.ports.push(PortMapping::parse(&val()?)?),
                    "--restart" => spec.restart = RestartPolicy::parse(&val()?)?,
                    "--user" => spec.user = Some(val()?),
                    "--platform" => spec.platform = Platform::parse(&val()?)?,
                    "-e" => spec.env.push(val()?),
                    "--memory" => spec.memory_limit = val()?.parse().ok(),
                    "--cpus" => spec.cpus = val()?.parse().ok(),
                    "--add-host" => {
                        let v = val()?;
                        let (h, ip) = v.split_once(':').ok_or_else(bad)?;
                        spec.extra_hosts.push((h.into(), ip.into()));
                    }
                    _ => return Err(bad()),
                }
            }
            rt.create(&spec)?;
            Ok(0)
        }
        "start" => {
            let id = id()?;
            let att = rest.iter().any(|a| a == "--attach");
            rt.start(&id, att).await?;
            if att {
                return attach(rt.attach(&id).await?).await;
            }
            Ok(0)
        }
        "attach" => attach(rt.attach(&id()?).await?).await,
        "exec" => {
            let (opts, cmd) = split_dashdash(rest);
            let id = opts.iter().find(|a| !a.starts_with('-')).cloned().ok_or_else(bad)?;
            let spec = ExecSpec {
                cmd,
                tty: opts.iter().any(|a| a == "-t"),
                interactive: opts.iter().any(|a| a == "-i"),
                ..Default::default()
            };
            attach(rt.exec(&id, &spec).await?).await
        }
        "stop" => {
            let secs = rest.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
            rt.stop(&id()?, secs).await?;
            Ok(0)
        }
        "kill" => {
            rt.kill(&id()?, hullr_runtime::parse_signal(rest.get(1).map(String::as_str).unwrap_or("KILL"))?)?;
            Ok(0)
        }
        "wait" => {
            let c = rt.wait(&id()?).await?;
            println!("{c}");
            Ok(0)
        }
        "state" => {
            println!("{}", serde_json::to_string(&rt.state(&id()?)?).unwrap());
            Ok(0)
        }
        "rm" => {
            rt.remove(&id()?)?;
            Ok(0)
        }
        "logs" => {
            let t = std::fs::read_to_string(rt.log_path(&id()?)).unwrap_or_default();
            for l in t.lines() {
                let l: hullr_core::LogLine = serde_json::from_str(l)?;
                println!("{}", l.line);
            }
            Ok(0)
        }
        "stats" => {
            println!("{}", serde_json::to_string(&rt.stats(&id()?)?).unwrap());
            Ok(0)
        }
        "caps" => {
            println!("{}", serde_json::to_string_pretty(&rt.capabilities()).unwrap());
            Ok(0)
        }
        "emulation-enable" => {
            println!("{:?}", hullr_runtime::enable_emulation()?);
            Ok(0)
        }
        "sandbox" => {
            let ports = rest[1..].iter().map(|p| PortMapping::parse(p)).collect::<Result<Vec<_>, _>>()?;
            rt.ensure_sandbox(&id()?, &ports)?;
            Ok(0)
        }
        "rmsandbox" => {
            rt.remove_sandbox(&id()?)?;
            Ok(0)
        }
        "run-rootfs" => {
            let (opts, argv) = split_dashdash(rest);
            let dir = PathBuf::from(opts.first().ok_or_else(bad)?);
            let platform = match opts.iter().position(|o| o == "--platform") {
                Some(i) => Platform::parse(opts.get(i + 1).ok_or_else(bad)?)?,
                None => Platform::host(),
            };
            let out = |s: hullr_core::LogStream, d: &[u8]| {
                use std::io::Write;
                match s {
                    hullr_core::LogStream::Stdout => std::io::stdout().write_all(d),
                    hullr_core::LogStream::Stderr => std::io::stderr().write_all(d),
                }
                .unwrap();
            };
            rt.run_in_rootfs(&dir, argv, vec![], "/".into(), None, &platform, &out).await
        }
        _ => Err(bad()),
    }
}

/// Pump our stdin into the stream as frames; print output; return the exit code.
async fn attach(s: tokio::net::UnixStream) -> hullr_core::Result<i32> {
    let (mut r, mut w) = s.into_split();
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buf = vec![0u8; 4096];
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    let _ = write_frame(&mut w, &Frame::CloseStdin).await;
                    // Keep the write half open so the server keeps talking.
                    std::future::pending::<()>().await;
                }
                Ok(n) => {
                    if write_frame(&mut w, &Frame::Stdin(buf[..n].to_vec())).await.is_err() {
                        return;
                    }
                }
            }
        }
    });
    let mut out = tokio::io::stdout();
    let mut err = tokio::io::stderr();
    while let Some(f) = read_frame(&mut r).await? {
        match f {
            Frame::Stdout(d) => {
                out.write_all(&d).await?;
                out.flush().await?;
            }
            Frame::Stderr(d) => err.write_all(&d).await?,
            Frame::Error(e) => eprintln!("error: {e}"),
            Frame::Exit(c) => return Ok(c),
            _ => {}
        }
    }
    Ok(255)
}
