use crate::error::{Error, Result};
use crate::protocol::isotp;
use crate::transport::elm327::Elm327;

pub struct PidDefinition {
    pub mode: u8,
    pub pid: u8,
    pub name: &'static str,
    pub unit: &'static str,
    /// Minimum number of data bytes required by this PID's formula.
    pub data_bytes: usize,
    pub formula: fn(&[u8]) -> f64,
}

/// Standard OBD2 Mode 01 PIDs
pub static PIDS: &[PidDefinition] = &[
    PidDefinition {
        mode: 0x01,
        pid: 0x04,
        name: "load",
        unit: "%",
        data_bytes: 1,
        formula: |d| d[0] as f64 * 100.0 / 255.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x05,
        name: "coolant_temp",
        unit: "°C",
        data_bytes: 1,
        formula: |d| d[0] as f64 - 40.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x0B,
        name: "intake_pressure",
        unit: "kPa",
        data_bytes: 1,
        formula: |d| d[0] as f64,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x0C,
        name: "rpm",
        unit: "RPM",
        data_bytes: 2,
        formula: |d| ((d[0] as f64) * 256.0 + d[1] as f64) / 4.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x0D,
        name: "speed",
        unit: "km/h",
        data_bytes: 1,
        formula: |d| d[0] as f64,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x0E,
        name: "timing_advance",
        unit: "°",
        data_bytes: 1,
        formula: |d| d[0] as f64 / 2.0 - 64.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x0F,
        name: "intake_temp",
        unit: "°C",
        data_bytes: 1,
        formula: |d| d[0] as f64 - 40.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x10,
        name: "maf",
        unit: "g/s",
        data_bytes: 2,
        formula: |d| ((d[0] as f64) * 256.0 + d[1] as f64) / 100.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x11,
        name: "throttle",
        unit: "%",
        data_bytes: 1,
        formula: |d| d[0] as f64 * 100.0 / 255.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x1F,
        name: "runtime",
        unit: "s",
        data_bytes: 2,
        formula: |d| (d[0] as f64) * 256.0 + d[1] as f64,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x2F,
        name: "fuel_level",
        unit: "%",
        data_bytes: 1,
        formula: |d| d[0] as f64 * 100.0 / 255.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x33,
        name: "baro_pressure",
        unit: "kPa",
        data_bytes: 1,
        formula: |d| d[0] as f64,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x42,
        name: "battery_voltage",
        unit: "V",
        data_bytes: 2,
        formula: |d| ((d[0] as f64) * 256.0 + d[1] as f64) / 1000.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x46,
        name: "ambient_temp",
        unit: "°C",
        data_bytes: 1,
        formula: |d| d[0] as f64 - 40.0,
    },
    PidDefinition {
        mode: 0x01,
        pid: 0x5C,
        name: "oil_temp",
        unit: "°C",
        data_bytes: 1,
        formula: |d| d[0] as f64 - 40.0,
    },
];

/// Find a PID by name or hex code.
pub fn find_pid(name_or_hex: &str) -> Option<&'static PidDefinition> {
    let lower = name_or_hex.to_lowercase();
    if let Some(pid) = PIDS.iter().find(|p| p.name == lower) {
        return Some(pid);
    }

    if let Ok(code) = u8::from_str_radix(name_or_hex.trim_start_matches("0x"), 16) {
        return PIDS.iter().find(|p| p.pid == code);
    }

    None
}

/// Fetch a PID value from the vehicle. Returns (name, value, unit).
pub async fn fetch_pid(elm: &mut Elm327, name_or_hex: &str) -> Result<(String, f64, String)> {
    let pid_def = find_pid(name_or_hex).ok_or_else(|| {
        let available: Vec<&str> = PIDS.iter().map(|p| p.name).collect();
        Error::Config(format!(
            "unknown PID: '{}'. Available: {}",
            name_or_hex,
            available.join(", ")
        ))
    })?;

    let hex_cmd = format!("{:02X} {:02X}", pid_def.mode, pid_def.pid);
    let response = elm.send_obd(&hex_cmd).await?;
    let payload = isotp::extract_single_frame_data(&response.lines)?;

    // Payload: [response_mode, pid, data...]
    let min_len = 2 + pid_def.data_bytes;
    if payload.len() < min_len {
        return Err(Error::Protocol(format!(
            "PID 0x{:02X} response too short: got {} bytes, need at least {} (2 header + {} data)",
            pid_def.pid,
            payload.len(),
            min_len,
            pid_def.data_bytes,
        )));
    }

    let data = &payload[2..];
    let value = (pid_def.formula)(data);

    Ok((pid_def.name.to_string(), value, pid_def.unit.to_string()))
}

/// Read a PID and print the result.
pub async fn read_pid(elm: &mut Elm327, name_or_hex: &str) -> Result<()> {
    let (name, value, unit) = fetch_pid(elm, name_or_hex).await?;
    println!("{}: {:.1} {}", name, value, unit);
    Ok(())
}

/// Read a PID and return the computed value (for monitor/shell use).
pub async fn read_pid_value(elm: &mut Elm327, name_or_hex: &str) -> Result<(String, f64, String)> {
    fetch_pid(elm, name_or_hex).await
}
