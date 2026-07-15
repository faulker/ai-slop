//! Serial port enumeration and radio detection on macOS.
//!
//! The radio enumerates as a standard CDC-ACM device (`/dev/cu.usbmodem*`).
//! We list all ports and flag any whose USB VID/PID matches a known AnyTone
//! variant, but detection is confirmed later via the identify command, so a
//! non-matching VID/PID is not treated as a hard failure.

use crate::error::Result;

/// Known (VID, PID) pairs for the AT-D878UVII Plus USB bridges.
/// GD32 variant and STM32 variant, respectively.
pub const KNOWN_USB_IDS: &[(u16, u16)] = &[(0x28e9, 0x018a), (0x2e3c, 0x5740)];

/// A discovered serial port and whether it looks like an AnyTone radio.
#[derive(Debug, Clone)]
pub struct PortInfo {
    /// Device path, e.g. `/dev/cu.usbmodem1234`.
    pub name: String,
    /// USB vendor ID, if this is a USB serial port.
    pub vid: Option<u16>,
    /// USB product ID, if this is a USB serial port.
    pub pid: Option<u16>,
    /// USB product string, if the OS reported one.
    pub product: Option<String>,
    /// True when the VID/PID matches a known AnyTone bridge.
    pub likely_radio: bool,
}

/// Return true if this VID/PID pair is a known AnyTone USB bridge.
pub fn is_known_radio(vid: u16, pid: u16) -> bool {
    KNOWN_USB_IDS.contains(&(vid, pid))
}

/// Enumerate all serial ports, flagging likely radios by VID/PID. On macOS we
/// prefer the callout (`cu.`) devices; the `serialport` crate already returns
/// those alongside the `tty.` variants.
pub fn list_ports() -> Result<Vec<PortInfo>> {
    let ports = serialport::available_ports()?;
    let mut out = Vec::with_capacity(ports.len());
    for p in ports {
        let (vid, pid, product) = match &p.port_type {
            serialport::SerialPortType::UsbPort(info) => {
                (Some(info.vid), Some(info.pid), info.product.clone())
            }
            _ => (None, None, None),
        };
        let likely_radio = match (vid, pid) {
            (Some(v), Some(d)) => is_known_radio(v, d),
            _ => false,
        };
        out.push(PortInfo {
            name: p.port_name,
            vid,
            pid,
            product,
            likely_radio,
        });
    }
    Ok(out)
}

/// Pick the single likely-radio port, if exactly one is present.
///
/// - `Ok(Some(path))` when exactly one likely radio was found.
/// - `Ok(None)` when none were found.
/// - `Err(..)` (ambiguous) when more than one likely radio was found; the
///   caller should ask the user to disambiguate with an explicit port.
pub fn autodetect_radio() -> Result<Option<String>> {
    let mut radios: Vec<PortInfo> =
        list_ports()?.into_iter().filter(|p| p.likely_radio).collect();
    match radios.len() {
        0 => Ok(None),
        1 => Ok(Some(radios.remove(0).name)),
        n => Err(crate::error::Error::InvalidArgument(format!(
            "{n} candidate radio ports found; pass --port to choose one"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ids_match_and_others_do_not() {
        assert!(is_known_radio(0x28e9, 0x018a));
        assert!(is_known_radio(0x2e3c, 0x5740));
        assert!(!is_known_radio(0x1234, 0x5678));
        assert!(!is_known_radio(0x28e9, 0x0000));
    }

    #[test]
    fn list_ports_does_not_error() {
        // We can't assert contents without hardware, but enumeration itself
        // must succeed (or return an empty list) on any host.
        let _ = list_ports().expect("port enumeration should not error");
    }
}
