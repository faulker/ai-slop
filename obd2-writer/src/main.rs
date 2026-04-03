mod cli;
mod error;
mod obd;
mod protocol;
mod shell;
mod toyota;
mod transport;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Command, DtcAction};
use crate::error::Result;
use crate::obd::dtc;
use crate::obd::pid;
use crate::protocol::uds;
use crate::toyota::write_safety;
use crate::transport::elm327::Elm327;
use crate::transport::serial::SerialConnection;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match &cli.command {
        Command::Connect => {
            let mut elm = connect(&cli).await?;
            let version = elm.initialize().await?;
            println!("Connected: {}", version);
            let protocol = elm.at_command("ATDP").await?;
            println!("Protocol: {}", protocol);
        }

        Command::Read { pid: pid_name } => {
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            pid::read_pid(&mut elm, pid_name).await?;
        }

        Command::ReadEnhanced { did, ecu } => {
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            toyota::enhanced_pids::read_enhanced_did(&mut elm, did, ecu).await?;
        }

        Command::Dtc { action } => {
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            match action {
                DtcAction::List => dtc::read_dtcs(&mut elm).await?,
                DtcAction::Clear { confirm } => {
                    if !confirm {
                        eprintln!("WARNING: This will clear ALL stored diagnostic trouble codes.");
                        eprintln!("Re-run with --confirm to proceed.");
                        return Ok(());
                    }
                    dtc::clear_dtcs(&mut elm).await?
                }
            }
        }

        Command::Session { session_type } => {
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            let session = match session_type.as_str() {
                "default" => 0x01,
                "extended" => 0x03,
                "programming" => 0x02,
                other => {
                    eprintln!("Unknown session type: {}. Use: default, extended, programming", other);
                    return Ok(());
                }
            };
            let cmd = uds::diagnostic_session_control(session);
            let resp = uds::send_uds(&mut elm, &cmd).await?;
            println!("Session response: {}", uds::hex_string(&resp));
            if session != 0x01 {
                println!("WARNING: Session will timeout after ~5s of inactivity.");
            }
        }

        Command::Write { did, data, ecu, confirm, dry_run } => {
            if !dry_run && !confirm {
                eprintln!("WARNING: Writing to an ECU can cause permanent damage.");
                eprintln!("You are about to write DID 0x{} = {} to ECU {}", did, data, ecu);
                eprintln!("Re-run with --confirm to proceed, or --dry-run to preview.");
                return Ok(());
            }
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            write_safety::verified_write_did(&mut elm, did, data, ecu, *dry_run).await?;
        }

        Command::Restore { did, ecu, confirm } => {
            if !confirm {
                eprintln!("WARNING: Restoring will write the backed-up value to the ECU.");
                eprintln!("Re-run with --confirm to proceed.");
                return Ok(());
            }
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            write_safety::restore_did(&mut elm, did, ecu).await?;
        }

        Command::Backups => {
            write_safety::print_backups()?;
        }

        Command::Shell => {
            let serial = SerialConnection::open(&cli.port, cli.baud_rate, cli.timeout)?;
            let elm = Elm327::new(serial);
            shell::run(elm).await?;
        }
    }

    Ok(())
}

async fn connect(cli: &Cli) -> Result<Elm327> {
    println!("Opening {}...", cli.port);
    let serial = SerialConnection::open(&cli.port, cli.baud_rate, cli.timeout)?;
    Ok(Elm327::new(serial))
}
