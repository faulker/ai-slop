//! Client <-> server round trips against a fake Api.

use futures::StreamExt;
use hullr_api::{serve_listener, Client};
use hullr_core::api::*;
use hullr_core::stream::{read_frame, write_frame, Frame};
use hullr_core::*;
use std::sync::Arc;

struct Fake;

fn uns<T>() -> Result<T> {
    Err(Error::Unsupported("fake".into()))
}

#[async_trait::async_trait]
impl Api for Fake {
    async fn info(&self) -> Result<SystemInfo> {
        uns()
    }
    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerInfo>> {
        if all { Ok(vec![]) } else { Err(Error::NotFound("only all".into())) }
    }
    async fn inspect_container(&self, _: &str) -> Result<ContainerInfo> { uns() }
    async fn create_container(&self, spec: ContainerSpec, progress: ProgressFn) -> Result<String> {
        progress(Progress::Status { message: "pulling".into() });
        Ok(format!("id-{}", spec.image))
    }
    async fn start_container(&self, _: &str, _: bool) -> Result<()> { Ok(()) }
    async fn stop_container(&self, _: &str, _: u32) -> Result<()> { uns() }
    async fn kill_container(&self, _: &str, _: &str) -> Result<()> { uns() }
    async fn restart_container(&self, _: &str, _: u32) -> Result<()> { uns() }
    async fn remove_container(&self, _: &str, _: bool) -> Result<()> { uns() }
    async fn wait_container(&self, _: &str) -> Result<i32> { Ok(3) }
    async fn logs(&self, id: &str, _: LogOptions) -> Result<LogLineStream> {
        if id == "missing" {
            return Err(Error::NotFound("no such container".into()));
        }
        let lines = (0..3).map(|i| Ok(LogLine { ts: i, stream: LogStream::Stdout, line: format!("l{i}") }));
        Ok(futures::stream::iter(lines).boxed())
    }
    async fn stats(&self, _: &str) -> Result<Stats> { uns() }
    async fn attach(&self, _: &str) -> Result<AttachStream> {
        // Echo server: Stdin -> Stdout, CloseStdin -> Exit(7).
        let (a, mut b) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            while let Ok(Some(f)) = read_frame(&mut b).await {
                match f {
                    Frame::Stdin(d) => write_frame(&mut b, &Frame::Stdout(d)).await.unwrap(),
                    Frame::CloseStdin => {
                        write_frame(&mut b, &Frame::Exit(7)).await.unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        });
        Ok(Box::new(a))
    }
    async fn exec(&self, _: &str, _: ExecSpec) -> Result<AttachStream> { uns() }
    async fn list_images(&self) -> Result<Vec<ImageInfo>> { Ok(vec![]) }
    async fn inspect_image(&self, _: &str) -> Result<ImageInfo> { uns() }
    async fn pull_image(&self, _: &str, _: Option<Platform>, _: ProgressFn) -> Result<ImageInfo> { uns() }
    async fn push_image(&self, _: &str, _: ProgressFn) -> Result<()> { uns() }
    async fn tag_image(&self, _: &str, _: &str) -> Result<()> { uns() }
    async fn remove_image(&self, _: &str, _: bool) -> Result<Vec<String>> { uns() }
    async fn build_image(&self, _: BuildOptions, _: ProgressFn) -> Result<ImageInfo> { uns() }
    async fn login(&self, _: &str, _: &str, _: &str) -> Result<()> { uns() }
    async fn list_volumes(&self) -> Result<Vec<VolumeInfo>> { uns() }
    async fn create_volume(&self, _: &str) -> Result<VolumeInfo> { uns() }
    async fn remove_volume(&self, _: &str, _: bool) -> Result<()> { uns() }
    async fn compose_up(&self, _: ComposeOptions, _: ProgressFn) -> Result<ComposeStatus> { uns() }
    async fn compose_down(&self, _: ComposeOptions, _: ProgressFn) -> Result<()> { uns() }
    async fn compose_ps(&self, _: ComposeOptions) -> Result<ComposeStatus> { uns() }
    async fn compose_list(&self) -> Result<Vec<ComposeStatus>> { uns() }
    async fn prune(&self, _: bool, _: bool) -> Result<PruneReport> { uns() }
    async fn events(&self) -> Result<EventStream> { uns() }
}

async fn setup() -> (tempfile::TempDir, Client) {
    let t = tempfile::tempdir().unwrap();
    let sock = t.path().join("s.sock");
    let l = tokio::net::UnixListener::bind(&sock).unwrap();
    tokio::spawn(serve_listener(Arc::new(Fake), l));
    (t, Client::new(sock))
}

#[tokio::test]
async fn calls_progress_and_errors() {
    let (_t, c) = setup().await;
    assert!(c.ping().await);
    assert_eq!(c.list_containers(true).await.unwrap().len(), 0);
    assert!(matches!(c.list_containers(false).await, Err(Error::NotFound(_))));
    let seen = Arc::new(std::sync::Mutex::new(vec![]));
    let s2 = seen.clone();
    let id = c
        .create_container(
            ContainerSpec { image: "alpine".into(), ..Default::default() },
            Arc::new(move |p| s2.lock().unwrap().push(p)),
        )
        .await
        .unwrap();
    assert_eq!(id, "id-alpine");
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(c.wait_container("x").await.unwrap(), 3);
    assert!(matches!(c.info().await, Err(Error::Unsupported(_))));
}

#[tokio::test]
async fn streams() {
    let (_t, c) = setup().await;
    let lines: Vec<String> = c.logs("x", Default::default()).await.unwrap().map(|l| l.unwrap().line).collect().await;
    assert_eq!(lines, vec!["l0", "l1", "l2"]);
    assert!(matches!(c.logs("missing", Default::default()).await, Err(Error::NotFound(_))));
}

#[tokio::test]
async fn attach_upgrade() {
    let (_t, c) = setup().await;
    let mut s = c.attach("x").await.unwrap();
    write_frame(&mut s, &Frame::Stdin(b"hello".to_vec())).await.unwrap();
    assert_eq!(read_frame(&mut s).await.unwrap(), Some(Frame::Stdout(b"hello".to_vec())));
    write_frame(&mut s, &Frame::CloseStdin).await.unwrap();
    assert_eq!(read_frame(&mut s).await.unwrap(), Some(Frame::Exit(7)));
}

#[test]
fn listen_parse() {
    use hullr_api::Listen;
    assert_eq!("vsock:1024".parse::<Listen>().unwrap(), Listen::Vsock(1024));
    assert_eq!("unix:/x.sock".parse::<Listen>().unwrap(), Listen::Unix("/x.sock".into()));
    assert!("vsock:x".parse::<Listen>().is_err());
}
