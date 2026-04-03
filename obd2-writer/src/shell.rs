use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::error::Result;
use crate::obd::{dtc, pid};
use crate::protocol::uds;
use crate::toyota::bcm;
use crate::toyota::enhanced_pids;
use crate::toyota::write_safety;
use crate::transport::elm327::Elm327;

/// Run the interactive shell.
pub async fn run(mut elm: Elm327) -> Result<()> {
    let mut rl = DefaultEditor::new().map_err(|e| crate::error::Error::Config(e.to_string()))?;
    let mut connected = false;

    // Register Ctrl+C handler once at startup for monitor interrupt
    let monitor_running = Arc::new(AtomicBool::new(false));
    let monitor_flag = monitor_running.clone();
    if let Err(e) = ctrlc::set_handler(move || {
        monitor_flag.store(false, Ordering::Relaxed);
    }) {
        eprintln!("Warning: failed to register Ctrl+C handler: {}. Monitor interrupt may not work.", e);
    }

    println!("OBD2 Interactive Shell");
    println!("Type 'help' for available commands, 'quit' to exit.");
    println!("NOTE: Non-default sessions will timeout after ~5s without activity.");
    println!();

    loop {
        let prompt = if connected { "obd2> " } else { "obd2 (disconnected)> " };

        match rl.readline(prompt) {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(&line);

                let parts: Vec<&str> = line.split_whitespace().collect();
                let cmd = parts[0].to_lowercase();

                match cmd.as_str() {
                    "help" => print_help(),

                    "quit" | "exit" | "q" => {
                        println!("Goodbye.");
                        break;
                    }

                    "connect" => {
                        match elm.initialize().await {
                            Ok(version) => {
                                println!("Connected: {}", version);
                                connected = true;
                            }
                            Err(e) => eprintln!("Connection failed: {}", e),
                        }
                    }

                    "read" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        if parts.len() < 2 {
                            eprintln!("Usage: read <pid_name|pid_hex>");
                            continue;
                        }
                        if let Err(e) = pid::read_pid(&mut elm, parts[1]).await {
                            if is_connection_error(&e) { connected = false; }
                            eprintln!("Error: {}", e);
                        }
                    }

                    "read-enhanced" | "re" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        if parts.len() < 2 {
                            eprintln!("Usage: read-enhanced <did_hex> [ecu]");
                            continue;
                        }
                        let ecu = parts.get(2).unwrap_or(&"7E0");
                        if let Err(e) = enhanced_pids::read_enhanced_did(&mut elm, parts[1], ecu).await {
                            if is_connection_error(&e) { connected = false; }
                            eprintln!("Error: {}", e);
                        }
                    }

                    "monitor" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        if parts.len() < 2 {
                            eprintln!("Usage: monitor <pid_name> [interval_ms]");
                            continue;
                        }
                        let interval_ms: u64 = parts.get(2)
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(500);

                        println!("Monitoring {} every {}ms. Press Ctrl+C to stop.", parts[1], interval_ms);
                        monitor_running.store(true, Ordering::Relaxed);

                        while monitor_running.load(Ordering::Relaxed) {
                            match pid::read_pid_value(&mut elm, parts[1]).await {
                                Ok((name, value, unit)) => {
                                    print!("\r{}: {:.1} {}    ", name, value, unit);
                                }
                                Err(e) => {
                                    if is_connection_error(&e) { connected = false; }
                                    eprintln!("\rError: {}", e);
                                    break;
                                }
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
                        }
                        println!();
                    }

                    "dtc" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        let action = parts.get(1).unwrap_or(&"list");
                        match *action {
                            "list" => {
                                if let Err(e) = dtc::read_dtcs(&mut elm).await {
                                    if is_connection_error(&e) { connected = false; }
                                    eprintln!("Error: {}", e);
                                }
                            }
                            "clear" => {
                                println!("WARNING: This will clear ALL diagnostic trouble codes.");
                                println!("Type 'yes' to confirm:");
                                let confirm = rl.readline("confirm> ");
                                match confirm {
                                    Ok(c) if c.trim().eq_ignore_ascii_case("yes") => {}
                                    _ => {
                                        println!("Clear cancelled.");
                                        continue;
                                    }
                                }
                                if let Err(e) = dtc::clear_dtcs(&mut elm).await {
                                    if is_connection_error(&e) { connected = false; }
                                    eprintln!("Error: {}", e);
                                }
                            }
                            _ => eprintln!("Usage: dtc [list|clear]"),
                        }
                    }

                    "session" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        if parts.len() < 2 {
                            eprintln!("Usage: session <default|extended|programming>");
                            continue;
                        }
                        let session = match parts[1] {
                            "default" => uds::SESSION_DEFAULT,
                            "extended" => uds::SESSION_EXTENDED,
                            "programming" => uds::SESSION_PROGRAMMING,
                            other => {
                                eprintln!("Unknown session: {}. Use: default, extended, programming", other);
                                continue;
                            }
                        };

                        let req = uds::diagnostic_session_control(session);
                        match uds::send_uds(&mut elm, &req).await {
                            Ok(resp) => {
                                println!("Session response: {}", uds::hex_string(&resp));
                                if session != uds::SESSION_DEFAULT {
                                    println!("WARNING: Session will timeout after ~5s of inactivity.");
                                    println!("Use 'keepalive' to send TesterPresent manually.");
                                }
                            }
                            Err(e) => {
                                if is_connection_error(&e) { connected = false; }
                                eprintln!("Error: {}", e);
                            }
                        }
                    }

                    "keepalive" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        let req = uds::tester_present();
                        match uds::send_uds(&mut elm, &req).await {
                            Ok(_) => println!("TesterPresent OK — session extended."),
                            Err(e) => {
                                if is_connection_error(&e) { connected = false; }
                                eprintln!("TesterPresent failed: {}", e);
                            }
                        }
                    }

                    "security" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        let level: u8 = match parts.get(1) {
                            Some(s) => match s.parse() {
                                Ok(v) => v,
                                Err(_) => {
                                    eprintln!("Invalid security level: '{}'. Must be a number (e.g., 1, 3, 17).", s);
                                    continue;
                                }
                            },
                            None => {
                                println!("No level specified, using default level 1.");
                                1
                            }
                        };

                        if let Err(e) = bcm::security_access_async_key(&mut elm, level).await {
                            if is_connection_error(&e) { connected = false; }
                            eprintln!("Error: {}", e);
                        }
                    }

                    "write" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        // Check for --dry-run flag
                        let dry_run = parts.iter().any(|p| *p == "--dry-run");
                        let args: Vec<&str> = parts.iter()
                            .filter(|p| **p != "--dry-run")
                            .copied()
                            .collect();

                        if args.len() < 3 {
                            eprintln!("Usage: write <did_hex> <value_hex> [ecu] [--dry-run]");
                            continue;
                        }
                        let ecu = args.get(3).unwrap_or(&"7E0");

                        if !dry_run {
                            println!("WARNING: About to write DID 0x{} = {} to ECU {}", args[1], args[2], ecu);
                            println!("Type 'yes' to confirm (or use --dry-run to preview):");
                            let confirm = rl.readline("confirm> ");
                            match confirm {
                                Ok(c) if c.trim().eq_ignore_ascii_case("yes") => {}
                                _ => {
                                    println!("Write cancelled.");
                                    continue;
                                }
                            }
                        }
                        if let Err(e) = write_safety::verified_write_did(&mut elm, args[1], args[2], ecu, dry_run).await {
                            if is_connection_error(&e) { connected = false; }
                            eprintln!("Error: {}", e);
                        }
                    }

                    "restore" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        if parts.len() < 2 {
                            eprintln!("Usage: restore <did_hex> [ecu]");
                            continue;
                        }
                        let ecu = parts.get(2).unwrap_or(&"7E0");
                        println!("WARNING: About to restore backed-up value for DID 0x{} on ECU {}", parts[1], ecu);
                        println!("Type 'yes' to confirm:");
                        let confirm = rl.readline("confirm> ");
                        match confirm {
                            Ok(c) if c.trim().eq_ignore_ascii_case("yes") => {}
                            _ => {
                                println!("Restore cancelled.");
                                continue;
                            }
                        }
                        if let Err(e) = write_safety::restore_did(&mut elm, parts[1], ecu).await {
                            if is_connection_error(&e) { connected = false; }
                            eprintln!("Error: {}", e);
                        }
                    }

                    "backups" => {
                        if let Err(e) = write_safety::print_backups() {
                            eprintln!("Error: {}", e);
                        }
                    }

                    "target" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        if parts.len() < 2 {
                            eprintln!("Usage: target <ecu_hex> (e.g., 7E0, 750, 7C0)");
                            continue;
                        }
                        match elm.set_header(parts[1]).await {
                            Ok(()) => println!("Target set to: {}", parts[1]),
                            Err(e) => {
                                if is_connection_error(&e) { connected = false; }
                                eprintln!("Error: {}", e);
                            }
                        }
                    }

                    "raw" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        if parts.len() < 2 {
                            eprintln!("Usage: raw <hex bytes>");
                            continue;
                        }
                        let raw_cmd = parts[1..].join(" ");
                        match elm.send_passthrough(&raw_cmd).await {
                            Ok(resp) => println!("{}", resp),
                            Err(e) => {
                                if is_connection_error(&e) { connected = false; }
                                eprintln!("Error: {}", e);
                            }
                        }
                    }

                    "at" => {
                        if !connected {
                            eprintln!("Not connected. Use 'connect' first.");
                            continue;
                        }
                        if parts.len() < 2 {
                            eprintln!("Usage: at <command>");
                            continue;
                        }
                        let at_cmd = parts[1..].join(" ");
                        let full_cmd = if at_cmd.to_uppercase().starts_with("AT") {
                            at_cmd
                        } else {
                            format!("AT{}", at_cmd)
                        };
                        match elm.at_command(&full_cmd).await {
                            Ok(resp) => println!("{}", resp),
                            Err(e) => {
                                if is_connection_error(&e) { connected = false; }
                                eprintln!("Error: {}", e);
                            }
                        }
                    }

                    "pids" => {
                        println!("Available PIDs:");
                        for p in pid::PIDS {
                            println!("  {:16} (0x{:02X}) — {}", p.name, p.pid, p.unit);
                        }
                    }

                    _ => {
                        eprintln!("Unknown command: '{}'. Type 'help' for available commands.", cmd);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("Ctrl+C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye.");
                break;
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                break;
            }
        }
    }

    Ok(())
}

fn is_connection_error(err: &crate::error::Error) -> bool {
    matches!(
        err,
        crate::error::Error::Timeout
            | crate::error::Error::Io(_)
            | crate::error::Error::Serial(_)
    ) || matches!(err, crate::error::Error::Elm(msg) if msg.contains("UNABLE TO CONNECT") || msg.contains("CAN ERROR") || msg.contains("BUS INIT"))
}

fn print_help() {
    println!("Commands:");
    println!("  connect                    Connect and initialize OBDLink MX+");
    println!("  read <pid>                 Read a standard OBD2 PID (name or hex)");
    println!("  read-enhanced <did> [ecu]  Read Toyota-specific DID (Mode 22)");
    println!("  monitor <pid> [ms]         Continuously read a PID (default 500ms)");
    println!("  dtc [list|clear]           Read or clear DTCs");
    println!("  session <type>             Set diagnostic session (default/extended/programming)");
    println!("  keepalive                  Send TesterPresent to extend session");
    println!("  security [level]           Perform security access handshake");
    println!("  write <did> <data> [ecu]   Write to a DID (verified, requires confirmation)");
    println!("  write <did> <data> [ecu] --dry-run  Preview write without changing anything");
    println!("  restore <did> [ecu]        Restore a backed-up DID value");
    println!("  backups                    List all backed-up DID values");
    println!("  target <ecu>               Set target ECU (e.g., 7E0, 750)");
    println!("  raw <hex>                  Send raw hex command");
    println!("  at <cmd>                   Send AT command");
    println!("  pids                       List available PIDs");
    println!("  help                       Show this help");
    println!("  quit                       Exit");
}
