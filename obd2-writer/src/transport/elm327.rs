use tracing::{debug, warn};

use crate::error::{Error, Result};
use crate::transport::serial::SerialConnection;

/// How many times to send ATZ before giving up on a silent adapter.
const RESET_ATTEMPTS: usize = 3;

/// Base CAN ID that BMW-style modules answer on: a module with extended
/// address `ea` replies from `0x600 + ea`. Unverified on the K51; see
/// docs/bmw-k51/findings.md.
pub const EXT_RESPONSE_BASE: u16 = 0x600;

/// Split a diagnostic target into its CAN header and optional extended address.
/// "7E0" is a plain 11-bit target; "6F1:60" is header 6F1 addressed to module 0x60
/// (ISO 15765-2 extended addressing, as used by BMW D-CAN).
pub fn parse_target(target: &str) -> Result<(String, Option<u8>)> {
    match target.split_once(':') {
        None => {
            validate_can_id(target)?;
            Ok((target.to_string(), None))
        }
        Some((header, ea)) => {
            validate_can_id(header)?;
            if header.len() != 3 {
                return Err(Error::Config(format!(
                    "extended addressing needs an 11-bit header: '{}'",
                    target
                )));
            }
            let ea = u8::from_str_radix(ea, 16)
                .map_err(|_| Error::Config(format!("invalid extended address in '{}'", target)))?;
            Ok((header.to_string(), Some(ea)))
        }
    }
}

/// Drop the tester-address byte that extended-addressing replies carry before
/// the ISO-TP PCI byte, so "612 F1 02 7E 00" becomes "612 02 7E 00".
/// A PCI byte is never 0xF_, so a matching byte here is never real payload.
fn strip_tester_byte(line: &str, tester: u8) -> String {
    let mut tokens = line.split_whitespace();
    let Some(id) = tokens.next() else {
        return line.to_string();
    };
    let rest: Vec<&str> = tokens.collect();
    match rest.first() {
        Some(first) if u8::from_str_radix(first, 16).ok() == Some(tester) => {
            std::iter::once(id).chain(rest[1..].iter().copied()).collect::<Vec<_>>().join(" ")
        }
        _ => line.to_string(),
    }
}

/// Services refused while extended addressing is on: writes, routines, resets,
/// DTC clearing, security access, programming session and downloads. The bike
/// stays read-only until a write path gated by `bmw::settings::check_writable` exists.
fn is_blocked_ext_request(cmd: &str) -> bool {
    let upper = cmd.trim().to_uppercase();
    if upper.starts_with("AT") || upper.starts_with("ST") {
        return false;
    }
    // The adapter accepts hex with or without spaces, so read it as pairs.
    let hex: Vec<char> = upper.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes: Vec<u8> = hex
        .chunks(2)
        .take(2)
        .map_while(|pair| u8::from_str_radix(&pair.iter().collect::<String>(), 16).ok())
        .collect();
    match bytes.as_slice() {
        [0x10, 0x02, ..] => true,
        [sid, ..] => matches!(sid, 0x11 | 0x14 | 0x27 | 0x2E | 0x2F | 0x31 | 0x34..=0x37 | 0x3B | 0x3D),
        [] => false,
    }
}

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
    /// Tester address (low byte of the header) while extended addressing is on.
    ext_tester: Option<u8>,
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
        Self { serial, ext_tester: None }
    }

    /// Send a raw string command and read the response.
    async fn send_raw(&mut self, cmd: &str) -> Result<String> {
        if self.ext_tester.is_some() && is_blocked_ext_request(cmd) {
            return Err(Error::Config(format!(
                "refusing '{}': BMW modules are read-only in this version",
                cmd.trim()
            )));
        }
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

        // Reset. A sleeping adapter can swallow the first command or two after
        // the Bluetooth link comes up, so retry with a short pause.
        let mut version = String::new();
        for attempt in 1..=RESET_ATTEMPTS {
            match self.send_raw("ATZ").await {
                Ok(v) => {
                    version = v;
                    break;
                }
                Err(Error::Timeout) if attempt < RESET_ATTEMPTS => {
                    warn!(attempt, "no response to ATZ, retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    self.serial.drain().await?;
                }
                Err(e) => return Err(e),
            }
        }
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
        self.parse_with_addressing(&response)
    }

    /// Parse a response, removing extended-address bytes when they are in use.
    fn parse_with_addressing(&self, response: &str) -> Result<ElmResponse> {
        let mut parsed = parse_response(response)?;
        if let Some(tester) = self.ext_tester {
            parsed.lines = parsed.lines.iter().map(|l| strip_tester_byte(l, tester)).collect();
        }
        Ok(parsed)
    }

    /// Read a pending response without sending a new command.
    /// Used for NRC 0x78 (responsePending) handling — the ECU will send the
    /// real response asynchronously, so we just need to read from the port.
    pub async fn read_pending_response(&mut self) -> Result<ElmResponse> {
        let response = self.serial.read_until_prompt().await?;
        debug!(response = response.as_str(), "RX (pending)");
        check_elm_error(&response)?;
        self.parse_with_addressing(&response)
    }

    /// Set the diagnostic target. Accepts a plain CAN header ("7E0", "750") or a
    /// BMW-style extended-address target ("6F1:60"); see `parse_target`.
    /// Switching back to a plain header turns extended addressing off again.
    pub async fn set_header(&mut self, header: &str) -> Result<()> {
        let (can_id, ext) = parse_target(header)?;
        self.at_command(&format!("ATSH {}", can_id)).await?;
        match ext {
            Some(ea) => self.enable_extended_address(&can_id, ea).await?,
            None if self.ext_tester.is_some() => self.disable_extended_address().await?,
            None => {}
        }
        Ok(())
    }

    /// Address every request to module `ea`, listen on its reply ID, and send
    /// ISO-TP flow control from the tester header with the module byte first.
    async fn enable_extended_address(&mut self, header: &str, ea: u8) -> Result<()> {
        let tester = u16::from_str_radix(header, 16).unwrap_or(0) as u8;
        self.at_command(&format!("ATCEA {:02X}", ea)).await?;
        self.at_command(&format!("ATCRA {:03X}", EXT_RESPONSE_BASE + ea as u16)).await?;
        self.at_command(&format!("ATFCSH {}", header)).await?;
        self.at_command(&format!("ATFCSD {:02X} 30 00 00", ea)).await?;
        self.at_command("ATFCSM1").await?;
        self.ext_tester = Some(tester);
        Ok(())
    }

    /// Return to normal 11-bit addressing and automatic flow control.
    async fn disable_extended_address(&mut self) -> Result<()> {
        self.at_command("ATCEA").await?;
        self.at_command("ATFCSM0").await?;
        self.at_command("ATAR").await?;
        self.ext_tester = None;
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

    /// Passively monitor CAN traffic for a fixed window, accepting only IDs where
    /// `(id & mask) == filter`. Returns the raw frame lines and whether the adapter
    /// reported BUFFER FULL (meaning the window was truncated).
    ///
    /// Sends ATMA, reads for `window`, then sends a carriage return to stop
    /// monitoring and drains to the prompt. CAN auto-formatting is turned off
    /// during the capture so each line is `ID B0 B1 ... B7`, and restored after.
    pub async fn monitor_frames(
        &mut self,
        filter: &str,
        mask: &str,
        window: std::time::Duration,
    ) -> Result<(Vec<String>, bool)> {
        validate_can_id(filter)?;
        validate_can_id(mask)?;

        self.at_command("ATCAF0").await?;
        self.at_command(&format!("ATCF {}", filter)).await?;
        self.at_command(&format!("ATCM {}", mask)).await?;

        // ATMA never returns a prompt on its own, so bypass send_raw.
        self.serial.write_bytes(b"ATMA\r").await?;
        let captured = self.serial.read_for(window).await?;

        // Any character stops monitoring; the adapter then prints STOPPED and a prompt.
        self.serial.write_bytes(b"\r").await?;
        let tail = self.serial.read_until_prompt().await.unwrap_or_default();

        // Restore normal request/response filtering and formatting.
        self.at_command("ATAR").await?;
        self.at_command("ATCAF1").await?;

        let combined = format!("{}\n{}", captured, tail);
        let truncated = combined.to_uppercase().contains("BUFFER FULL");
        let lines = combined
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        debug!(truncated, "monitor window complete");

        Ok((lines, truncated))
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
    fn test_parse_target_plain_and_extended() {
        assert_eq!(parse_target("7E0").unwrap(), ("7E0".to_string(), None));
        assert_eq!(parse_target("6F1:60").unwrap(), ("6F1".to_string(), Some(0x60)));
        assert_eq!(parse_target("6f1:0a").unwrap(), ("6f1".to_string(), Some(0x0A)));
    }

    #[test]
    fn test_parse_target_rejects_bad_input() {
        assert!(parse_target("6F1:").is_err());
        assert!(parse_target("6F1:100").is_err());
        assert!(parse_target("6F1:6Z").is_err());
        assert!(parse_target("18DA00F1:60").is_err()); // ext addressing is 11-bit only
        assert!(parse_target("6F1:60\rATZ").is_err());
    }

    #[test]
    fn test_blocked_ext_requests() {
        assert!(is_blocked_ext_request("2E 30 00 01"));
        assert!(is_blocked_ext_request("3B A7 00"));
        assert!(is_blocked_ext_request("31 01 FF 00"));
        assert!(is_blocked_ext_request("14 FF FF FF"));
        assert!(is_blocked_ext_request("27 01"));
        assert!(is_blocked_ext_request("10 02"));
        assert!(is_blocked_ext_request("2e3000")); // unspaced raw input
    }

    #[test]
    fn test_allowed_ext_requests() {
        assert!(!is_blocked_ext_request("22 F1 90"));
        assert!(!is_blocked_ext_request("3E 00"));
        assert!(!is_blocked_ext_request("10 03"));
        assert!(!is_blocked_ext_request("10 01"));
        assert!(!is_blocked_ext_request("19 02 FF"));
        assert!(!is_blocked_ext_request("ATSH 6F1"));
        assert!(!is_blocked_ext_request("STDI"));
    }

    #[test]
    fn test_strip_tester_byte() {
        assert_eq!(strip_tester_byte("612 F1 02 7E 00", 0xF1), "612 02 7E 00");
        assert_eq!(strip_tester_byte("612 F1 10 14 62 F1 90 57", 0xF1), "612 10 14 62 F1 90 57");
        // Adapter already removed it: leave the frame alone.
        assert_eq!(strip_tester_byte("612 02 7E 00", 0xF1), "612 02 7E 00");
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
