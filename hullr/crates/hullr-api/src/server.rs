//! Serves any `Api` (normally the engine) over a socket.

use crate::{read_line_raw, Reply, Request};
use futures::StreamExt;
use hullr_core::api::{AttachStream, ProgressFn};
use hullr_core::{Api, Error, Result};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listen {
    Unix(PathBuf),
    Vsock(u32),
}

impl std::str::FromStr for Listen {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        if let Some(p) = s.strip_prefix("vsock:") {
            return p.parse().map(Listen::Vsock).map_err(|_| Error::Invalid(format!("bad vsock port '{p}'")));
        }
        let p = s.strip_prefix("unix:").unwrap_or(s);
        if p.is_empty() {
            return Err(Error::Invalid("empty listen address".into()));
        }
        Ok(Listen::Unix(p.into()))
    }
}

pub async fn serve(api: Arc<dyn Api>, listen: Listen) -> Result<()> {
    match listen {
        Listen::Unix(path) => {
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d)?;
            }
            // A leftover socket from a crashed daemon blocks bind.
            if UnixStream::connect(&path).await.is_ok() {
                return Err(Error::Conflict(format!("a daemon is already listening on {}", path.display())));
            }
            let _ = std::fs::remove_file(&path);
            let l = UnixListener::bind(&path).map_err(|e| Error::Io(format!("bind {}: {e}", path.display())))?;
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o660));
            }
            serve_listener(api, l).await
        }
        Listen::Vsock(port) => serve_vsock(api, port).await,
    }
}

pub async fn serve_listener(api: Arc<dyn Api>, l: UnixListener) -> Result<()> {
    loop {
        let (conn, _) = l.accept().await?;
        let api = api.clone();
        tokio::spawn(async move { handle(api, conn).await });
    }
}

#[cfg(target_os = "linux")]
async fn serve_vsock(api: Arc<dyn Api>, port: u32) -> Result<()> {
    let l = crate::vsock::listen(port).map_err(|e| Error::Io(format!("vsock listen {port}: {e}")))?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || loop {
        match crate::vsock::accept(&l) {
            Ok(c) => {
                if tx.send(c).is_err() {
                    break;
                }
            }
            Err(e) => {
                eprintln!("vsock accept: {e}");
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    });
    while let Some(c) = rx.recv().await {
        c.set_nonblocking(true)?;
        let conn = UnixStream::from_std(c)?;
        let api = api.clone();
        tokio::spawn(async move { handle(api, conn).await });
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn serve_vsock(_api: Arc<dyn Api>, _port: u32) -> Result<()> {
    Err(Error::Unsupported("vsock listening is only available on Linux".into()))
}

enum Outcome {
    Value(Value),
    /// Stream already being forwarded as items by a task.
    Streaming(tokio::task::JoinHandle<()>),
    Upgrade(AttachStream),
}

async fn handle(api: Arc<dyn Api>, mut conn: UnixStream) {
    let line = match read_line_raw(&mut conn).await {
        Ok(Some(l)) => l,
        _ => return,
    };
    let (tx, mut rx) = mpsc::unbounded_channel::<Reply>();
    let (rd, mut wr) = conn.into_split();
    // All replies go through one writer task, so progress and items from
    // other tasks interleave safely. It hands the write half back at the end.
    let writer = tokio::spawn(async move {
        while let Some(r) = rx.recv().await {
            let mut b = match serde_json::to_vec(&r) {
                Ok(b) => b,
                Err(_) => continue,
            };
            b.push(b'\n');
            if wr.write_all(&b).await.is_err() {
                break;
            }
        }
        let _ = wr.flush().await;
        wr
    });

    let result = match serde_json::from_str::<Request>(&line) {
        Ok(req) => dispatch(api, req, tx.clone()).await,
        Err(e) => Err(Error::Invalid(format!("bad request: {e}"))),
    };
    match result {
        Ok(Outcome::Value(v)) => {
            let _ = tx.send(Reply::Ok(v));
            drop(tx);
            let _ = writer.await;
        }
        Ok(Outcome::Streaming(task)) => {
            drop(tx);
            let _ = task.await;
            let _ = writer.await;
        }
        Ok(Outcome::Upgrade(mut stream)) => {
            let _ = tx.send(Reply::Ok(Value::Null));
            drop(tx);
            let Ok(wr) = writer.await else { return };
            let Ok(mut conn) = rd.reunite(wr) else { return };
            let _ = tokio::io::copy_bidirectional(&mut conn, &mut stream).await;
        }
        Err(e) => {
            let _ = tx.send(Reply::Err(e));
            drop(tx);
            let _ = writer.await;
        }
    }
}

fn arg<T: DeserializeOwned>(p: &Value, key: &str) -> Result<T> {
    serde_json::from_value(p.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|e| Error::Invalid(format!("param '{key}': {e}")))
}

fn arg_or<T: DeserializeOwned>(p: &Value, key: &str, default: T) -> T {
    match p.get(key) {
        None | Some(Value::Null) => default,
        Some(v) => serde_json::from_value(v.clone()).unwrap_or(default),
    }
}

fn val<T: serde::Serialize>(t: T) -> Result<Outcome> {
    Ok(Outcome::Value(serde_json::to_value(t)?))
}

async fn dispatch(api: Arc<dyn Api>, req: Request, tx: mpsc::UnboundedSender<Reply>) -> Result<Outcome> {
    let p = &req.params;
    let ptx = tx.clone();
    let progress: ProgressFn = Arc::new(move |pr| {
        let _ = ptx.send(Reply::Progress(pr));
    });
    match req.method.as_str() {
        "ping" => val(json!("pong")),
        "info" => val(api.info().await?),
        "list_containers" => val(api.list_containers(arg_or(p, "all", false)).await?),
        "inspect_container" => val(api.inspect_container(&arg::<String>(p, "id")?).await?),
        "create_container" => val(api.create_container(arg(p, "spec")?, progress).await?),
        "start_container" => val(api.start_container(&arg::<String>(p, "id")?, arg_or(p, "wait_for_attach", false)).await?),
        "stop_container" => val(api.stop_container(&arg::<String>(p, "id")?, arg_or(p, "timeout", 10)).await?),
        "kill_container" => val(api.kill_container(&arg::<String>(p, "id")?, &arg_or(p, "signal", "KILL".to_string())).await?),
        "restart_container" => val(api.restart_container(&arg::<String>(p, "id")?, arg_or(p, "timeout", 10)).await?),
        "remove_container" => val(api.remove_container(&arg::<String>(p, "id")?, arg_or(p, "force", false)).await?),
        "wait_container" => val(api.wait_container(&arg::<String>(p, "id")?).await?),
        "logs" => {
            let mut s = api.logs(&arg::<String>(p, "id")?, arg_or(p, "opts", Default::default())).await?;
            let _ = tx.send(Reply::Ok(Value::Null));
            Ok(Outcome::Streaming(tokio::spawn(async move {
                while let Some(item) = s.next().await {
                    let r = match item {
                        Ok(l) => Reply::Item(serde_json::to_value(l).unwrap_or(Value::Null)),
                        Err(e) => Reply::Err(e),
                    };
                    if tx.send(r).is_err() {
                        break;
                    }
                }
            })))
        }
        "events" => {
            let mut s = api.events().await?;
            let _ = tx.send(Reply::Ok(Value::Null));
            Ok(Outcome::Streaming(tokio::spawn(async move {
                while let Some(item) = s.next().await {
                    let r = match item {
                        Ok(ev) => Reply::Item(serde_json::to_value(ev).unwrap_or(Value::Null)),
                        Err(e) => Reply::Err(e),
                    };
                    if tx.send(r).is_err() || tx.is_closed() {
                        break;
                    }
                }
            })))
        }
        "stats" => val(api.stats(&arg::<String>(p, "id")?).await?),
        "attach" => Ok(Outcome::Upgrade(api.attach(&arg::<String>(p, "id")?).await?)),
        "exec" => Ok(Outcome::Upgrade(api.exec(&arg::<String>(p, "id")?, arg(p, "spec")?).await?)),
        "list_images" => val(api.list_images().await?),
        "inspect_image" => val(api.inspect_image(&arg::<String>(p, "reference")?).await?),
        "pull_image" => val(api.pull_image(&arg::<String>(p, "reference")?, arg(p, "platform")?, progress).await?),
        "push_image" => val(api.push_image(&arg::<String>(p, "reference")?, progress).await?),
        "tag_image" => val(api.tag_image(&arg::<String>(p, "source")?, &arg::<String>(p, "target")?).await?),
        "remove_image" => val(api.remove_image(&arg::<String>(p, "reference")?, arg_or(p, "force", false)).await?),
        "build_image" => val(api.build_image(arg(p, "opts")?, progress).await?),
        "login" => val(
            api.login(&arg::<String>(p, "registry")?, &arg::<String>(p, "username")?, &arg::<String>(p, "password")?)
                .await?,
        ),
        "list_volumes" => val(api.list_volumes().await?),
        "create_volume" => val(api.create_volume(&arg::<String>(p, "name")?).await?),
        "remove_volume" => val(api.remove_volume(&arg::<String>(p, "name")?, arg_or(p, "force", false)).await?),
        "compose_up" => val(api.compose_up(arg(p, "opts")?, progress).await?),
        "compose_down" => val(api.compose_down(arg(p, "opts")?, progress).await?),
        "compose_ps" => val(api.compose_ps(arg(p, "opts")?).await?),
        "compose_list" => val(api.compose_list().await?),
        "prune" => val(api.prune(arg_or(p, "all_images", false), arg_or(p, "volumes", false)).await?),
        other => Err(Error::Invalid(format!("unknown method '{other}'"))),
    }
}
