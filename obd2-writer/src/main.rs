mod bmw;
mod browse;
mod cli;
mod error;
mod obd;
mod protocol;
mod recon;
mod shell;
mod toyota;
mod transport;
mod tui;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{BmwAction, Cli, Command, DtcAction, Vehicle};
use crate::error::Result;
use crate::obd::dtc;
use crate::obd::pid;
use crate::protocol::uds;
use crate::toyota::{did_scan, ecu_scan, write_safety};
use crate::transport::bluetooth;
use crate::transport::elm327::Elm327;
use crate::transport::serial::{self, SerialConnection};

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

        Command::BackupAll => {
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            write_safety::backup_all_settings(&mut elm).await?;
        }

        Command::Ecus { sweep } => {
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            if cli.vehicle == Vehicle::BmwK51 {
                let found = bmw::modules::sweep(&mut elm, 0x00, 0xFF).await?;
                print_bmw_modules(&found);
            } else if let Some(range) = sweep {
                let (start, end) = did_scan::parse_range(range)?;
                if end > 0x7FF {
                    eprintln!("Sweep range must stay within 11-bit IDs (000-7FF).");
                    return Ok(());
                }
                let found = ecu_scan::sweep_ecus(&mut elm, start, end).await?;
                if found.is_empty() {
                    println!("No modules responded.");
                } else {
                    println!("Responding modules:");
                    for tx in &found {
                        let hex = format!("{:03X}", tx);
                        println!("  0x{} ({})", hex, ecu_scan::label_for(&hex).unwrap_or("unknown"));
                    }
                }
            } else {
                let found = ecu_scan::scan_ecus(&mut elm).await?;
                if found.is_empty() {
                    println!("No ECUs responded.");
                } else {
                    println!("Found {} ECU(s):", found.len());
                    for ecu in &found {
                        println!("  {} (0x{})", ecu.name, ecu.tx_address);
                    }
                }
            }
        }

        Command::Snapshot { ecu, scan, from, label, output } => {
            let snapshot = match (scan, from) {
                (Some(range), None) => {
                    let Some(ecu) = ecu else {
                        eprintln!("--ecu is required with --scan.");
                        return Ok(());
                    };
                    let ranges = vec![did_scan::parse_range(range)?];
                    let mut elm = connect(&cli).await?;
                    elm.initialize().await?;
                    recon::snapshot::scan_to_snapshot(&mut elm, ecu, &ranges, label).await?
                }
                (None, Some(path)) => {
                    let baseline = recon::snapshot::Snapshot::load(path)?;
                    println!("Re-reading {} DID(s) on ECU 0x{}", baseline.entries.len(), baseline.ecu);
                    let mut elm = connect(&cli).await?;
                    elm.initialize().await?;
                    recon::snapshot::resnapshot(&mut elm, &baseline, label).await?
                }
                _ => {
                    eprintln!("Specify exactly one of --scan RANGE or --from FILE.");
                    return Ok(());
                }
            };
            snapshot.save(output)?;
            println!("Saved {} DID(s) to {}", snapshot.entries.len(), output);
        }

        Command::Monitor { seconds, label, output } => {
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            let capture = recon::monitor::capture(&mut elm, *seconds, label).await?;
            capture.save(output)?;
            println!("Saved capture to {}", output);
        }

        Command::Recon { ecus, ranges, monitor_seconds, out_dir } => {
            let opts = recon::session::ReconOptions {
                ecus: match ecus {
                    Some(list) => recon::session::parse_ecu_list(list)?,
                    None => Vec::new(),
                },
                ranges: recon::session::parse_range_list(ranges)?,
                monitor_seconds: *monitor_seconds,
                out_dir: out_dir.clone(),
            };
            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            recon::session::run(&mut elm, &opts).await?;
        }

        Command::Diff { before, after } => {
            recon::diff::diff_files(before, after)?;
        }

        Command::Browse => {
            if let Some(pid_name) = browse::browse_pids() {
                let mut elm = connect(&cli).await?;
                elm.initialize().await?;
                pid::read_pid(&mut elm, &pid_name).await?;
            }
        }

        Command::BrowseEnhanced => {
            if let Some((did_hex, ecu)) = browse::browse_dids() {
                let mut elm = connect(&cli).await?;
                elm.initialize().await?;
                toyota::enhanced_pids::read_enhanced_did(&mut elm, &did_hex, &ecu).await?;
            }
        }

        Command::Scan { ecu, range, test_writable, output } => {
            let ranges = if let Some(range_str) = range {
                vec![did_scan::parse_range(range_str)?]
            } else {
                did_scan::TOYOTA_BCM_RANGES
                    .iter()
                    .map(|(_, s, e)| (*s, *e))
                    .collect()
            };

            let mut elm = connect(&cli).await?;
            elm.initialize().await?;
            did_scan::scan_and_save(
                &mut elm,
                ecu,
                &ranges,
                *test_writable,
                output.as_deref(),
            )
            .await?;
        }

        Command::Bmw { action } => match action {
            BmwAction::Sweep { range } => {
                let (start, end) = did_scan::parse_range(range)?;
                if end > 0xFF {
                    eprintln!("Extended addresses are one byte (00-FF).");
                    return Ok(());
                }
                let mut elm = connect(&cli).await?;
                elm.initialize().await?;
                let found = bmw::modules::sweep(&mut elm, start as u8, end as u8).await?;
                print_bmw_modules(&found);
            }
            BmwAction::Identify { module } => {
                let address = u8::from_str_radix(module, 16).map_err(|_| {
                    error::Error::Config(format!("invalid module address '{}'", module))
                })?;
                let mut elm = connect(&cli).await?;
                elm.initialize().await?;
                let ids = bmw::modules::identify(&mut elm, address).await?;
                if ids.is_empty() {
                    println!("Module 0x{:02X} answered no identification DIDs.", address);
                }
                for (label, data) in ids {
                    println!("  {:<17} {}", label, bmw::modules::display_id(&data));
                }
            }
            BmwAction::Explain { module, read, file } => {
                let path = file
                    .as_ref()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(bmw::settings::default_path);
                let settings = bmw::settings::SettingsFile::load(&path)?;
                let current = if *read {
                    let mut elm = connect(&cli).await?;
                    elm.initialize().await?;
                    bmw::explain::read_mapped(&mut elm, &settings).await?
                } else {
                    Default::default()
                };
                bmw::explain::print(&settings, module.as_deref(), &current);
            }
        },

        Command::Shell => {
            let port = resolve_port(&cli)?;
            ensure_bluetooth(&port);
            let serial = SerialConnection::open(&port, cli.baud_rate, cli.timeout)?;
            let elm = Elm327::new(serial);
            shell::run(elm).await?;
        }

        Command::Tui => {
            let port = resolve_port(&cli)?;
            ensure_bluetooth(&port);
            let serial = SerialConnection::open(&port, cli.baud_rate, cli.timeout)?;
            let elm = Elm327::new(serial);
            tui::run(elm).await?;
        }
    }

    Ok(())
}

/// Print BMW sweep results as ready-to-use targets for `snapshot --ecu`.
fn print_bmw_modules(found: &[u8]) {
    if found.is_empty() {
        println!("No modules answered on 6F1 extended addressing.");
        println!("Try `ecus --sweep` for plain 11-bit addressing, or `monitor` to see what the bus carries.");
        return;
    }
    println!("Responding modules (use these with snapshot --ecu / bmw identify):");
    for address in found {
        println!("  {}", bmw::modules::target(*address));
    }
}

/// Bring the OBDLink's Bluetooth link up before touching its serial port.
/// Best effort: a missing blueutil or a non-Bluetooth port is not fatal, but
/// a failed connection attempt is reported so the user knows why the serial
/// port is about to time out.
fn ensure_bluetooth(port: &str) {
    if !port.to_lowercase().contains("obdlink") {
        return;
    }
    if let Err(e) = bluetooth::ensure_obdlink_connected() {
        eprintln!("Bluetooth: {}", e);
    }
}

fn resolve_port(cli: &Cli) -> Result<String> {
    match &cli.port {
        Some(port) => Ok(port.clone()),
        None => serial::select_port(),
    }
}

async fn connect(cli: &Cli) -> Result<Elm327> {
    let port = resolve_port(cli)?;
    ensure_bluetooth(&port);
    println!("Opening {}...", port);
    let serial = SerialConnection::open(&port, cli.baud_rate, cli.timeout)?;
    Ok(Elm327::new(serial))
}
