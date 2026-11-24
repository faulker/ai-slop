//! Command handlers.

mod compose;
mod containers;
mod daemon;
mod images;
mod system;
mod vm;

use crate::cli::{Cli, Command};
use crate::connect::connect;
use anyhow::Result;

/// Runs a command; returns the process exit code.
pub async fn run(cli: Cli) -> Result<i32> {
    let g = cli.global.clone();
    // Commands that must not auto-connect (they manage the daemon/VM itself).
    match cli.command {
        Command::Daemon(c) => return daemon::run(c, &g).await,
        Command::Vm(c) => return vm::run(c).await,
        Command::System(crate::cli::SystemCmd::Emulation(crate::cli::EmulationCmd::Enable)) => {
            return system::emulation_enable(&g).await
        }
        _ => {}
    }
    let api = connect(&g).await?;
    let api = api.as_ref();
    match cli.command {
        Command::Run(a) => containers::run(api, a, false).await,
        Command::Create(a) => containers::run(api, a, true).await,
        Command::Start { attach, interactive, containers: c } => containers::start(api, &c, attach, interactive).await,
        Command::Stop { time, containers: c } => containers::each(&c, |id| api.stop_container(id, time)).await,
        Command::Restart { time, containers: c } => containers::each(&c, |id| api.restart_container(id, time)).await,
        Command::Kill { signal, containers: c } => {
            let s = signal.clone();
            containers::each(&c, move |id| {
                let s = s.clone();
                async move { api.kill_container(id, &s).await }
            })
            .await
        }
        Command::Rm { force, containers: c } => containers::each(&c, |id| api.remove_container(id, force)).await,
        Command::Ps { all, quiet } => containers::ps(api, all, quiet).await,
        Command::Logs { follow, tail, timestamps, container } => {
            containers::logs(api, &container, follow, tail, timestamps).await
        }
        Command::Exec { interactive, tty, env, workdir, user, container, command } => {
            containers::exec(api, &container, command, interactive, tty, env, workdir, user).await
        }
        Command::Attach { container } => containers::attach(api, &container).await,
        Command::Inspect { targets } => containers::inspect(api, &targets).await,
        Command::Stats { no_stream, containers: c } => containers::stats(api, &c, no_stream).await,
        Command::Wait { containers: c } => containers::wait(api, &c).await,
        Command::Pull { platform, image } => images::pull(api, &image, platform).await,
        Command::Push { image } => images::push(api, &image).await,
        Command::Images { quiet } => images::list(api, quiet).await,
        Command::Rmi { force, images: i } => images::rmi(api, &i, force).await,
        Command::Tag { source, target } => {
            api.tag_image(&source, &target).await?;
            Ok(0)
        }
        Command::Build(b) => images::build(api, b).await,
        Command::Login { username, password, password_stdin, registry } => {
            images::login(api, registry, username, password, password_stdin).await
        }
        Command::Volume(v) => system::volume(api, v).await,
        Command::Compose(c) => compose::run(api, c).await,
        Command::System(s) => system::run(api, s).await,
        Command::Info => system::info(api).await,
        Command::Tui => {
            // TUI needs an owned Arc; reconnect cheaply.
            let api = connect(&g).await?;
            hullr_tui::run(api).await?;
            Ok(0)
        }
        Command::Gui | Command::Daemon(_) | Command::Vm(_) => unreachable!("handled earlier"),
    }
}
