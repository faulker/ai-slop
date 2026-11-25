//! The macOS VM host process (`hullr __vm`) and the client side that spawns
//! it (`ensure_running`).
//!
//! Threads: the main thread runs the main dispatch queue (`dispatch_main`),
//! which owns the `VZVirtualMachine`. A small tokio runtime runs the unix
//! socket bridge, the port forwarder, the readiness probe and signal
//! handling; it reaches the VM only through [`vsock::VsockConnector`], which
//! hops to the main queue.

mod vm;
mod vsock;

use crate::boot::{ensure_disk, kernel_cmdline, plan_shares, resolve_boot_files};
use crate::config::load_config;
use crate::forward::{run_control, Forwarder};
use crate::protocol::{ControlMsg, API_PORT};
use crate::state::{clear_state, status, write_state, VmFiles, VmLock, VmStatus};
use crate::transport::Connector;
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use hullr_core::paths::Paths;
use hullr_core::{Error, Result};
use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_foundation::NSError;
use objc2_virtualization::{VZVirtualMachine, VZVirtualMachineState};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use vsock::VsockConnector;

/// How long `ensure_running` waits for the guest API.
const READY_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a clean guest shutdown may take before the VM is force-stopped.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(20);

static STARTED: AtomicBool = AtomicBool::new(false);
static CLEANUP: OnceLock<(VmFiles, PathBuf)> = OnceLock::new();

/// Remove state files and the socket, then exit the process.
fn finish(code: i32) -> ! {
    if let Some((files, socket)) = CLEANUP.get() {
        clear_state(files);
        let _ = std::fs::remove_file(socket);
    }
    std::process::exit(code)
}

pub fn vm_main(paths: &Paths) -> i32 {
    let files = VmFiles::new(paths);
    // `status()` in other processes takes the lock for an instant; retry.
    let mut lock = None;
    for _ in 0..20 {
        match VmLock::try_acquire(&files) {
            Ok(Some(l)) => {
                lock = Some(l);
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                eprintln!("hullr vm: cannot lock {}: {e}", files.lock().display());
                return 1;
            }
        }
    }
    let Some(lock) = lock else {
        eprintln!("hullr vm: another VM host process is already running");
        return 0;
    };
    let _ = CLEANUP.set((files.clone(), paths.socket()));
    if let Err(e) = start(paths, &files) {
        eprintln!("hullr vm: {e}");
        clear_state(&files);
        return 1;
    }
    // The lock must be held until the process exits.
    std::mem::forget(lock);
    dispatch2::dispatch_main()
}

fn start(paths: &Paths, files: &VmFiles) -> std::result::Result<(), String> {
    MainThreadMarker::new().ok_or("vm_main must run on the main thread")?;
    let cfg = load_config(paths);
    cfg.validate().map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (kernel, initrd) = resolve_boot_files(files, &cfg, &exe).map_err(|e| e.to_string())?;
    ensure_disk(&files.disk(), cfg.disk_gib).map_err(|e| e.to_string())?;
    // Keep only the current boot in console.log.
    std::fs::write(files.console_log(), b"").map_err(|e| e.to_string())?;

    let shares = plan_shares(&cfg.shares);
    let rosetta = cfg.rosetta && vm::rosetta_available();
    let cmdline = kernel_cmdline(&shares, rosetta);
    eprintln!("hullr vm: kernel {} initrd {} cmdline {cmdline:?}", kernel.display(), initrd.display());
    write_state(files, "starting").map_err(|e| e.to_string())?;

    let config = vm::build_configuration(&vm::VmSpec {
        kernel,
        initrd,
        cmdline,
        cpus: cfg.cpus,
        memory_mib: cfg.memory_mib,
        disk: files.disk(),
        console_log: files.console_log(),
        shares,
        rosetta,
    })?;

    // The runtime lives for the rest of the process.
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?,
    ));
    let socket = paths.socket();
    let listener = rt.block_on(crate::bridge::bind_unix(&socket)).map_err(|e| format!("{}: {e}", socket.display()))?;

    // SAFETY: created on the main thread; `initWithConfiguration:` binds the
    // VM to the main queue, which `dispatch_main` serves from here on.
    let machine: Retained<VZVirtualMachine> =
        unsafe { VZVirtualMachine::initWithConfiguration(VZVirtualMachine::alloc(), &config) };
    let start_handler = RcBlock::new(|err: *mut NSError| {
        // SAFETY: the framework passes a valid NSError or null.
        match unsafe { err.as_ref() } {
            None => {
                STARTED.store(true, Ordering::SeqCst);
                eprintln!("hullr vm: started");
            }
            Some(e) => {
                eprintln!("hullr vm: start failed: {}", e.localizedDescription());
                finish(1);
            }
        }
    });
    // SAFETY: on the main queue's thread before dispatch_main; the call only
    // enqueues work, and the completion runs on the main queue.
    unsafe { machine.startWithCompletionHandler(&start_handler) };
    vsock::set_vm(machine);
    schedule_monitor();

    let connector = Arc::new(VsockConnector);
    let (out_tx, out_rx) = tokio::sync::mpsc::channel::<ControlMsg>(4);
    {
        let connector = connector.clone();
        rt.spawn(async move {
            if let Err(e) = crate::bridge::serve(listener, connector, API_PORT).await {
                eprintln!("hullr vm: api socket failed: {e}");
            }
        });
    }
    {
        let connector = connector.clone();
        let files = files.clone();
        rt.spawn(async move {
            loop {
                if connector.connect(API_PORT).await.is_ok() {
                    let _ = write_state(&files, "running");
                    eprintln!("hullr vm: guest api is up");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }
    {
        let fwd = Arc::new(Forwarder::new(connector.clone()));
        rt.spawn(run_control(connector, fwd, out_rx));
    }
    rt.spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {},
            _ = int.recv() => {},
        }
        eprintln!("hullr vm: stopping");
        let _ = out_tx.send(ControlMsg::shutdown()).await;
        tokio::time::sleep(SHUTDOWN_GRACE).await;
        eprintln!("hullr vm: guest did not power off in time, forcing stop");
        DispatchQueue::main().exec_async(force_stop);
    });
    Ok(())
}

/// Every 500ms on the main queue: exit once the started VM has stopped.
fn schedule_monitor() {
    let when = DispatchTime::try_from(Duration::from_millis(500)).unwrap_or(DispatchTime::NOW);
    let _ = DispatchQueue::main().after(when, || {
        // SAFETY: `state` is read on the VM's queue.
        let state = vsock::with_vm(|vm| unsafe { vm.state() });
        if STARTED.load(Ordering::SeqCst) {
            match state {
                Some(VZVirtualMachineState::Stopped) | None => {
                    eprintln!("hullr vm: guest stopped");
                    finish(0);
                }
                Some(VZVirtualMachineState::Error) => {
                    eprintln!("hullr vm: vm entered the error state; see console.log");
                    finish(1);
                }
                _ => {}
            }
        }
        schedule_monitor();
    });
}

/// Hard stop (main queue).
fn force_stop() {
    // SAFETY: on the VM's queue.
    let stopped = vsock::with_vm(|vm| unsafe {
        if vm.canStop() {
            let done = RcBlock::new(|_err: *mut NSError| finish(0));
            vm.stopWithCompletionHandler(&done);
            true
        } else {
            false
        }
    });
    if stopped != Some(true) {
        finish(0);
    }
}

fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn spawn_host(paths: &Paths, exe: &Path, files: &VmFiles) -> Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    std::fs::create_dir_all(&files.dir)?;
    let log = std::fs::File::create(files.host_log())?;
    let child = std::process::Command::new(exe)
        .arg(crate::VM_SUBCOMMAND)
        .env("HULLR_HOME", &paths.root)
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0)
        .spawn()
        .map_err(|e| Error::Runtime(format!("cannot start vm host process {}: {e}", exe.display())))?;
    Ok(child)
}

pub async fn ensure_running(paths: &Paths, exe: &Path) -> Result<PathBuf> {
    let socket = paths.socket();
    let files = VmFiles::new(paths);
    let mut child = None;
    if status(paths) == VmStatus::Stopped {
        // Fail fast with a useful message before spawning anything.
        resolve_boot_files(&files, &load_config(paths), exe)?;
        child = Some(spawn_host(paths, exe, &files)?);
    }
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        if let VmStatus::Running { .. } = status(paths) {
            if tokio::net::UnixStream::connect(&socket).await.is_ok() {
                return Ok(socket);
            }
        }
        if let Some(c) = child.as_mut() {
            if let Ok(Some(code)) = c.try_wait() {
                // A concurrent `ensure_running` may have won the race.
                if status(paths) == VmStatus::Stopped {
                    return Err(Error::Runtime(format!(
                        "vm host process exited ({code}); {}:\n{}",
                        files.host_log().display(),
                        tail(&files.host_log(), 20)
                    )));
                }
                child = None;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Runtime(format!(
                "the VM did not become ready within {}s; check the guest console at {} and {}",
                READY_TIMEOUT.as_secs(),
                files.console_log().display(),
                files.host_log().display()
            )));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
