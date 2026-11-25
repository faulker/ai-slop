//! One-connection recon session: sweep, baseline, wait for the user to change
//! something in the car, re-read, and diff, without ever closing the serial
//! port. Bluetooth ELM adapters often refuse a quick re-open, so keeping a
//! single connection for the whole procedure is far more reliable than one
//! process per step.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

use crate::error::Result;
use crate::recon::diff::{diff_captures, diff_snapshots};
use crate::recon::monitor::{self, MonitorCapture};
use crate::recon::snapshot::{self, Snapshot};
use crate::toyota::ecu_scan;
use crate::transport::elm327::Elm327;

pub struct ReconOptions {
    /// ECU request addresses to snapshot; empty means sweep 700-7FF and use every responder.
    pub ecus: Vec<u16>,
    /// DID ranges to scan on each ECU.
    pub ranges: Vec<(u16, u16)>,
    /// Seconds per passive monitor window; 0 skips the monitor step.
    pub monitor_seconds: u64,
    /// Directory for the JSON files.
    pub out_dir: String,
}

/// Run the full before/after procedure over one open connection.
pub async fn run(elm: &mut Elm327, opts: &ReconOptions) -> Result<()> {
    std::fs::create_dir_all(&opts.out_dir)
        .map_err(|e| crate::error::Error::Config(format!("cannot create {}: {}", opts.out_dir, e)))?;
    let out = |name: &str| Path::new(&opts.out_dir).join(name).to_string_lossy().to_string();

    // 1. Which modules are present.
    let ecus = if opts.ecus.is_empty() {
        println!("== Step 1: sweeping 0x700-0x7FF for responding modules");
        ecu_scan::sweep_ecus(elm, 0x700, 0x7FF).await?
    } else {
        opts.ecus.clone()
    };
    if ecus.is_empty() {
        println!("No modules responded; nothing to snapshot.");
        return Ok(());
    }
    println!("Modules: {}", ecus.iter().map(|e| format!("0x{:03X}", e)).collect::<Vec<_>>().join(", "));

    // 2. Baseline DID snapshots.
    println!("\n== Step 2: baseline DID snapshots");
    let mut baselines: Vec<Snapshot> = Vec::new();
    for ecu in &ecus {
        let hex = format!("{:03X}", ecu);
        let started = Instant::now();
        println!("-- ECU 0x{} ({})", hex, ecu_scan::label_for(&hex).unwrap_or("unknown"));
        let snap = snapshot::scan_to_snapshot(elm, &hex, &opts.ranges, "before").await?;
        let path = out(&format!("before-{}.json", hex));
        snap.save(&path)?;
        println!("   {} DID(s) in {:.0}s -> {}", snap.entries.len(), started.elapsed().as_secs_f64(), path);
        baselines.push(snap);
    }

    // 3. Baseline broadcast capture.
    let mon_before = if opts.monitor_seconds > 0 {
        println!("\n== Step 3: baseline CAN broadcast capture");
        let cap = monitor::capture(elm, opts.monitor_seconds, "before").await?;
        cap.save(&out("mon-before.json"))?;
        Some(cap)
    } else {
        None
    };

    // 4. Wait for the user.
    println!("\n== Step 4: change the state in the car now (press the buttons).");
    print!("   When the indicators have changed, press Enter here to continue... ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok();

    // 5. Re-read the same DIDs and diff.
    println!("\n== Step 5: re-reading DIDs");
    for base in &baselines {
        let after = snapshot::resnapshot(elm, base, "after").await?;
        after.save(&out(&format!("after-{}.json", base.ecu)))?;
        let d = diff_snapshots(base, &after);
        println!("-- ECU 0x{}: {} changed, {} vanished, {} new",
            base.ecu, d.changed.len(), d.only_before.len(), d.only_after.len());
        for c in &d.changed {
            println!("   0x{} offsets {:?}\n      before: {}\n      after:  {}", c.did, c.offsets, c.before, c.after);
        }
    }

    // 6. Second capture and diff.
    if let Some(before) = mon_before {
        println!("\n== Step 6: second CAN broadcast capture");
        let after: MonitorCapture = monitor::capture(elm, opts.monitor_seconds, "after").await?;
        after.save(&out("mon-after.json"))?;
        let d = diff_captures(&before, &after);
        println!("-- {} stable byte(s) changed:", d.changed.len());
        for c in &d.changed {
            println!("   ID {} byte {}: 0x{:02X} -> 0x{:02X}", c.id, c.byte, c.before, c.after);
        }
        if !d.only_after.is_empty() {
            println!("   IDs only after: {}", d.only_after.join(", "));
        }
    }

    println!("\nDone. Files are in {}. Re-run any comparison with `diff`, e.g.:", opts.out_dir);
    if let Some(b) = baselines.first() {
        println!("  obd2-writer diff {} {}", out(&format!("before-{}.json", b.ecu)), out(&format!("after-{}.json", b.ecu)));
    }
    Ok(())
}

/// Parse "7E0,787" into addresses.
pub fn parse_ecu_list(list: &str) -> Result<Vec<u16>> {
    list.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| {
            u16::from_str_radix(s.trim_start_matches("0x"), 16)
                .map_err(|_| crate::error::Error::Config(format!("invalid ECU address '{}'", s)))
        })
        .collect()
}

/// Parse "0000-1FFF,F000-FFFF" into ranges.
pub fn parse_range_list(list: &str) -> Result<Vec<(u16, u16)>> {
    list.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(crate::toyota::did_scan::parse_range)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ecu_list() {
        assert_eq!(parse_ecu_list("7E0, 0x787,").unwrap(), vec![0x7E0, 0x787]);
        assert!(parse_ecu_list("7E0,zz").is_err());
        assert!(parse_ecu_list("").unwrap().is_empty());
    }

    #[test]
    fn test_parse_range_list() {
        assert_eq!(parse_range_list("0000-1FFF,F000-FFFF").unwrap(), vec![(0, 0x1FFF), (0xF000, 0xFFFF)]);
        assert!(parse_range_list("1FFF-0000").is_err());
    }
}
