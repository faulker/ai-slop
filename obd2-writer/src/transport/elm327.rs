use tracing::{debug, warn};

use crate::error::{Error, Result};
use crate::transport::serial::SerialConnection;

/// Validate that a string is a valid CAN hex identifier (3-8 hex chars).
fn validate_can_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 8 {
        return Err(Error::Config(format!("invalid CAN ID length: '{}'", id)));
    }
    if !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::Config(format!("invalid CAN ID (non-hex chars): '{}'", id)));
    }
    Ok(())
}

/// ELM327-compatible OBD adapter interface.
/// Handles AT command initialization, OBD command framing, and response parsing.
pub struct Elm327 {
    serial: SerialConnection,
}

/// Parsed response from the ELM327
#[derive(Debug, Clone)]
pub struct ElmResponse {
    /// Raw response lines (each line is one CAN frame's data as hex)
    pub lines: Vec<String>,
    /// Full raw response text
    #[allow(dead_code)]
    pub raw: String,
}

impl Elm327 {
    pub fn new(serial: SerialConnection) -> Self {
        Self { serial }
    }

    /// Send a raw string command and read the response.
    async fn send_raw(&mut self, cmd: &str) -> Result<String> {
        debug!(cmd, "TX");
        let data = format!("{}\r", cmd);
        self.serial.write_bytes(data.as_bytes()).await?;
        let response = self.serial.read_until_prompt().await?;
        debug!(response = response.as_str(), "RX");
        Ok(response)
    }

    /// Send an AT command and return the response.
    pub async fn at_command(&mut self, cmd: &str) -> Result<String> {
        let response = self.send_raw(cmd).await?;
        check_elm_error(&response)?;
        Ok(response)
    }

    /// Initialize the ELM327 adapter for CAN communication with a Toyota.
    pub async fn initialize(&mut self) -> Result<String> {
        // Drain any leftover data
        self.serial.drain().await?;

        // Reset
        let version = self.send_raw("ATZ").await?;
        debug!(version = version.as_str(), "ELM327 reset");

        // Echo off
        self.at_command("ATE0").await?;
        // Linefeeds off
        self.at_command("ATL0").await?;
        // Spaces on (easier to parse)
        self.at_command("ATS1").await?;
        // Headers on (need to identify source ECU)
        self.at_command("ATH1").await?;
        // Set protocol: ISO 15765-4 CAN 11-bit 500kbps
        self.at_command("ATSP6").await?;
        // CAN auto-formatting on
        self.at_command("ATCAF1").await?;
        // Adaptive timing auto
        self.at_command("ATAT1").await?;
        // Set timeout to max (~1 second per retry)
        self.at_command("ATST FF").await?;

        // Verify protocol
        let protocol = self.at_command("ATDP").await?;
        debug!(protocol = protocol.as_str(), "detected protocol");

        // Extract version from reset response
        let version_clean = version
            .lines()
            .find(|l| l.contains("ELM") || l.contains("STN"))
            .unwrap_or(&version)
            .trim()
            .to_string();

        Ok(version_clean)
    }

    /// Send an OBD/UDS hex command and parse the response.
    /// `hex_cmd` should be space-separated hex bytes, e.g. "01 0C" or "22 01 00".
    pub async fn send_obd(&mut self, hex_cmd: &str) -> Result<ElmResponse> {
        let response = self.send_raw(hex_cmd).await?;
        check_elm_error(&response)?;
        parse_response(&response)
    }

    /// Read a pending response without sending a new command.
    /// Used for NRC 0x78 (responsePending) handling — the ECU will send the
    /// real response asynchronously, so we just need to read from the port.
    pub async fn read_pending_response(&mut self) -> Result<ElmResponse> {
        let response = self.serial.read_until_prompt().await?;
        debug!(response = response.as_str(), "RX (pending)");
        check_elm_error(&response)?;
        parse_response(&response)
    }

    /// Set the CAN transmit header (target ECU address).
    /// e.g., "7E0" for ECM, "750" for BCM.
    /// Validates that the header contains only hex characters (3-8 chars).
    pub async fn set_header(&mut self, header: &str) -> Result<()> {
        validate_can_id(header)?;
        self.at_command(&format!("ATSH {}", header)).await?;
        Ok(())
    }

    /// Set the CAN receive address filter.
    /// e.g., "7E8" for ECM responses.
    /// Validates that the filter contains only hex characters.
    #[allow(dead_code)]
    pub async fn set_receive_filter(&mut self, filter: &str) -> Result<()> {
        validate_can_id(filter)?;
        self.at_command(&format!("ATCRA {}", filter)).await?;
        Ok(())
    }

    /// Clear the CAN receive address filter (accept all).
    #[allow(dead_code)]
    pub async fn clear_receive_filter(&mut self) -> Result<()> {
        self.at_command("ATAR").await?;
        Ok(())
    }

    /// Send a raw AT or OBD command (passthrough for shell mode).
    pub async fn send_passthrough(&mut self, cmd: &str) -> Result<String> {
        self.send_raw(cmd).await
    }
}

/// Check for ELM327 error responses.
fn check_elm_error(response: &str) -> Result<()> {
    let upper = response.to_uppercase();
    if upper.contains("NO DATA") {
        return Err(Error::Elm("NO DATA - no response from vehicle".into()));
    }
    if upper.contains("UNABLE TO CONNECT") {
        return Err(Error::Elm("UNABLE TO CONNECT - check vehicle ignition and OBD port".into()));
    }
    if upper.contains("CAN ERROR") {
        return Err(Error::Elm("CAN ERROR - communication error on CAN bus".into()));
    }
    if upper.contains("BUFFER FULL") {
        return Err(Error::Elm("BUFFER FULL - response too large".into()));
    }
    if upper.contains("BUS INIT") && upper.contains("ERROR") {
        return Err(Error::Elm("BUS INIT ERROR - failed to initialize CAN bus".into()));
    }
    if upper.trim() == "?" {
        return Err(Error::Elm("unknown command".into()));
    }
    if upper.contains("STOPPED") {
        warn!("received STOPPED response");
    }
    Ok(())
}

/// Parse the ELM327 response into structured data.
/// Filters out echo lines, blank lines, and extracts hex data lines.
fn parse_response(response: &str) -> Result<ElmResponse> {
    let lines: Vec<String> = response
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .filter(|l| {
            // Filter out non-data lines (AT command echoes, etc.)
            // Data lines contain hex characters and spaces
            l.chars().all(|c| c.is_ascii_hexdigit() || c == ' ')
        })
        .collect();

    Ok(ElmResponse {
        lines,
        raw: response.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_elm_error_no_data() {
        assert!(check_elm_error("NO DATA").is_err());
    }

    #[test]
    fn test_check_elm_error_ok() {
        assert!(check_elm_error("7E8 04 41 0C 1A F8").is_ok());
    }

    #[test]
    fn test_parse_response_single_frame() {
        let resp = parse_response("7E8 04 41 0C 1A F8").unwrap();
        assert_eq!(resp.lines.len(), 1);
        assert_eq!(resp.lines[0], "7E8 04 41 0C 1A F8");
    }

    #[test]
    fn test_parse_response_multi_frame() {
        let input = "7E8 10 14 62 01 00 AA BB\n7E8 21 CC DD EE FF 00 11\n7E8 22 22 33 44 55 66 77";
        let resp = parse_response(input).unwrap();
        assert_eq!(resp.lines.len(), 3);
    }

    #[test]
    fn test_parse_response_filters_non_hex() {
        let input = "SEARCHING...\n7E8 04 41 0C 1A F8";
        let resp = parse_response(input).unwrap();
        assert_eq!(resp.lines.len(), 1);
    }

    #[test]
    fn test_validate_can_id_valid() {
        assert!(validate_can_id("7E0").is_ok());
        assert!(validate_can_id("750").is_ok());
        assert!(validate_can_id("7C0").is_ok());
        assert!(validate_can_id("18DA00FF").is_ok()); // 29-bit
    }

    #[test]
    fn test_validate_can_id_rejects_injection() {
        assert!(validate_can_id("7E0\rATZ").is_err());
        assert!(validate_can_id("7E0 ATZ").is_err());
        assert!(validate_can_id("").is_err());
        assert!(validate_can_id("123456789").is_err()); // too long
        assert!(validate_can_id("7G0").is_err()); // non-hex
    }
}
