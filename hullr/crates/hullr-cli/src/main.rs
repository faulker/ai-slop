//! `hullr`: run containers on macOS and Linux without the bloat.

mod cli;
mod cmd;
mod connect;
mod output;

use clap::Parser;

fn main() {
    // Internal helpers re-exec this binary; they must run before anything
    // else (no threads, no tokio).
    if let Some(code) = hullr_runtime::hidden_entry() {
        std::process::exit(code);
    }
    if let Some(code) = hidden_vm_entry() {
        std::process::exit(code);
    }

    let cli = cli::Cli::parse();
    output::init_color(cli.global.no_color);
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            output::error(&format!("cannot start async runtime: {e}"));
            std::process::exit(1);
        }
    };

    // The GUI must own the main thread (macOS requirement).
    if matches!(cli.command, cli::Command::Gui) {
        let api = match rt.block_on(connect::connect(&cli.global)) {
            Ok(a) => a,
            Err(e) => {
                output::error(&e.to_string());
                std::process::exit(1);
            }
        };
        let code = match hullr_gui::run(api, rt.handle().clone()) {
            Ok(()) => 0,
            Err(e) => {
                output::error(&e.to_string());
                1
            }
        };
        std::process::exit(code);
    }

    let code = rt.block_on(async {
        match cmd::run(cli).await {
            Ok(code) => code,
            Err(e) => {
                output::error(&format!("{e:#}"));
                1
            }
        }
    });
    // Do not wait for lingering stdin readers.
    rt.shutdown_background();
    std::process::exit(code);
}

/// `__vm` (macOS VM host process) and `__guest-agent` (inside the VM).
fn hidden_vm_entry() -> Option<i32> {
    let arg = std::env::args().nth(1)?;
    match arg.as_str() {
        "__vm" => Some(hullr_vm::vm_main(&hullr_core::paths::Paths::from_env())),
        "__guest-agent" => {
            #[cfg(target_os = "linux")]
            {
                let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().ok()?;
                Some(match rt.block_on(hullr_vm::guest::run_guest_agent()) {
                    Ok(()) => 0,
                    Err(e) => {
                        eprintln!("guest agent: {e}");
                        1
                    }
                })
            }
            #[cfg(not(target_os = "linux"))]
            Some(1)
        }
        _ => None,
    }
}
