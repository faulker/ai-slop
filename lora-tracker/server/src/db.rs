//! SQLite storage for decrypted reports.
//!
//! `(device_id, seq)` is the primary key. That doubles as replay protection: a
//! replayed packet is byte-identical to one already stored, so it changes nothing.
//! Strictly increasing seq (`ReplayGuard`) isn't used here because reports can
//! arrive out of order when several paths deliver from the tracker's queue.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tracker_proto::packet::Report;

/// One stored report, shaped for the JSON API.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredReport {
    pub seq: u32,
    /// Device clock, unix seconds; 0 when the tracker had no time yet.
    pub ts: u32,
    /// Server clock when the report arrived, unix seconds.
    pub received_at: i64,
    pub lat: f64,
    pub lon: f64,
    pub alt_m: i16,
    pub speed_kmh: f64,
    pub hdop: f64,
    pub batt_mv: u16,
    pub no_fix: bool,
    pub stationary: bool,
    /// City-level estimate from the network's IP, not GPS.
    pub approx: bool,
}

/// Whether an insert added a row or matched one already stored.
#[derive(Debug, PartialEq, Eq)]
pub enum Insert {
    New,
    Duplicate,
}

/// Opens (creating if needed) the database and applies the schema.
pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    init(&conn)?;
    Ok(conn)
}

/// Creates the schema on a fresh connection; safe to call on an existing database.
pub fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE IF NOT EXISTS reports (
             device_id   INTEGER NOT NULL,
             seq         INTEGER NOT NULL,
             ts          INTEGER NOT NULL,
             received_at INTEGER NOT NULL,
             lat_e7      INTEGER NOT NULL,
             lon_e7      INTEGER NOT NULL,
             alt_m       INTEGER NOT NULL,
             speed_cms   INTEGER NOT NULL,
             hdop_x10    INTEGER NOT NULL,
             batt_mv     INTEGER NOT NULL,
             flags       INTEGER NOT NULL,
             PRIMARY KEY (device_id, seq)
         ) WITHOUT ROWID;",
    )
    .context("creating schema")?;
    Ok(())
}

/// Stores a report that has already been authenticated by `packet::open`.
pub fn insert(conn: &Connection, r: &Report, received_at: i64) -> Result<Insert> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO reports
             (device_id, seq, ts, received_at, lat_e7, lon_e7, alt_m, speed_cms, hdop_x10, batt_mv, flags)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            r.device_id, r.seq, r.ts, received_at, r.lat_e7, r.lon_e7, r.alt_m,
            r.speed_cms, r.hdop_x10, r.batt_mv, r.flags
        ],
    )?;
    Ok(if n == 1 { Insert::New } else { Insert::Duplicate })
}

const COLUMNS: &str = "seq, ts, received_at, lat_e7, lon_e7, alt_m, speed_cms, hdop_x10, batt_mv, flags";

/// Converts a row selected with [`COLUMNS`] into the API shape.
fn row_to_report(row: &rusqlite::Row) -> rusqlite::Result<StoredReport> {
    let flags: u8 = row.get(9)?;
    Ok(StoredReport {
        seq: row.get(0)?,
        ts: row.get(1)?,
        received_at: row.get(2)?,
        lat: row.get::<_, i32>(3)? as f64 / 1e7,
        lon: row.get::<_, i32>(4)? as f64 / 1e7,
        alt_m: row.get(5)?,
        speed_kmh: row.get::<_, u16>(6)? as f64 * 0.036,
        hdop: row.get::<_, u8>(7)? as f64 / 10.0,
        batt_mv: row.get(8)?,
        no_fix: flags & tracker_proto::packet::FLAG_NO_FIX != 0,
        stationary: flags & tracker_proto::packet::FLAG_STATIONARY != 0,
        approx: flags & tracker_proto::packet::FLAG_APPROX != 0,
    })
}

/// The newest report (highest seq) for a device, if any.
pub fn latest(conn: &Connection, device_id: u32) -> Result<Option<StoredReport>> {
    let sql = format!("SELECT {COLUMNS} FROM reports WHERE device_id = ?1 ORDER BY seq DESC LIMIT 1");
    Ok(conn.query_row(&sql, [device_id], row_to_report).optional()?)
}

/// The newest report with a GPS fix, i.e. the last known location.
pub fn latest_fix(conn: &Connection, device_id: u32) -> Result<Option<StoredReport>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM reports WHERE device_id = ?1 AND (flags & ?2) = 0 ORDER BY seq DESC LIMIT 1"
    );
    let no_fix = tracker_proto::packet::FLAG_NO_FIX;
    Ok(conn.query_row(&sql, params![device_id, no_fix], row_to_report).optional()?)
}

/// Every device id that has at least one stored report, ascending.
pub fn reported_ids(conn: &Connection) -> Result<Vec<u32>> {
    let mut stmt = conn.prepare("SELECT DISTINCT device_id FROM reports ORDER BY device_id")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Up to `limit` reports received at or after `since`, oldest first, ending at the newest.
pub fn track(conn: &Connection, device_id: u32, since: i64, limit: u32) -> Result<Vec<StoredReport>> {
    let sql = format!(
        "SELECT * FROM (SELECT {COLUMNS} FROM reports WHERE device_id = ?1 AND received_at >= ?2
                        ORDER BY seq DESC LIMIT ?3)
         ORDER BY seq ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![device_id, since, limit], row_to_report)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A report at a fixed spot with the given seq.
    fn report(seq: u32) -> Report {
        Report {
            device_id: 3,
            seq,
            ts: 1_700_000_000,
            lat_e7: 401_234_567,
            lon_e7: -1_050_000_000,
            alt_m: 1600,
            speed_cms: 1000,
            hdop_x10: 12,
            batt_mv: 4100,
            flags: tracker_proto::packet::FLAG_STATIONARY,
        }
    }

    /// A fresh in-memory database with the schema applied.
    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        init(&c).unwrap();
        c
    }

    #[test]
    fn duplicate_seq_is_ignored() {
        let c = mem();
        assert_eq!(insert(&c, &report(5), 10).unwrap(), Insert::New);
        assert_eq!(insert(&c, &report(5), 11).unwrap(), Insert::Duplicate);
        assert_eq!(track(&c, 3, 0, 100).unwrap().len(), 1);
    }

    #[test]
    fn out_of_order_reports_are_kept_and_sorted() {
        let c = mem();
        for seq in [3, 1, 2] {
            insert(&c, &report(seq), 10).unwrap();
        }
        let seqs: Vec<u32> = track(&c, 3, 0, 100).unwrap().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, [1, 2, 3]);
        assert_eq!(latest(&c, 3).unwrap().unwrap().seq, 3);
    }

    #[test]
    fn track_limit_keeps_newest_and_since_filters() {
        let c = mem();
        for seq in 1..=5 {
            insert(&c, &report(seq), seq as i64 * 100).unwrap();
        }
        let seqs: Vec<u32> = track(&c, 3, 0, 2).unwrap().iter().map(|r| r.seq).collect();
        assert_eq!(seqs, [4, 5]);
        assert_eq!(track(&c, 3, 300, 100).unwrap().len(), 3);
    }

    #[test]
    fn latest_fix_skips_no_fix_reports_and_ids_cover_all_devices() {
        let c = mem();
        insert(&c, &report(1), 10).unwrap();
        insert(&c, &Report { flags: tracker_proto::packet::FLAG_NO_FIX, ..report(2) }, 20).unwrap();
        insert(&c, &Report { device_id: 8, ..report(1) }, 30).unwrap();
        assert_eq!(latest(&c, 3).unwrap().unwrap().seq, 2);
        assert_eq!(latest_fix(&c, 3).unwrap().unwrap().seq, 1);
        assert_eq!(reported_ids(&c).unwrap(), [3, 8]);
    }

    #[test]
    fn ip_estimates_are_flagged_approx_and_still_count_as_a_location() {
        let c = mem();
        insert(&c, &Report { flags: tracker_proto::packet::FLAG_APPROX, ..report(1) }, 10).unwrap();
        let r = latest_fix(&c, 3).unwrap().unwrap();
        assert!(r.approx && !r.no_fix);
    }

    #[test]
    fn converts_fixed_point_to_api_units() {
        let c = mem();
        insert(&c, &report(1), 10).unwrap();
        let r = latest(&c, 3).unwrap().unwrap();
        assert!((r.lat - 40.1234567).abs() < 1e-9);
        assert!((r.lon + 105.0).abs() < 1e-9);
        assert!((r.speed_kmh - 36.0).abs() < 1e-9);
        assert!((r.hdop - 1.2).abs() < 1e-9);
        assert!(r.stationary && !r.no_fix && !r.approx);
        assert!(latest(&c, 99).unwrap().is_none());
    }
}
