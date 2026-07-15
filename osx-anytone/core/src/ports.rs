//! Serial port enumeration and radio detection on macOS.
//!
//! The radio enumerates as a standard CDC-ACM device (`/dev/cu.usbmodem*`).
//! We list all ports and flag any whose USB VID/PID matches a known AnyTone
//! variant, but detection is confirmed later via the identify command, so a
//! non-matching VID/PID is not treated as a hard failure.

use crate::error::Result;
use std::collections::HashSet;

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

/// macOS exposes every serial device twice: once as a callout device
/// (`/dev/cu.*`) and once as a dial-in device (`/dev/tty.*`). Both names refer
/// to the same physical port, so listing both shows every radio twice and makes
/// [`autodetect_radio`] report an ambiguity that doesn't exist.
///
/// Drop the `tty.` half of any such pair. Only the callout device is usable
/// here anyway: the dial-in device blocks on open waiting for carrier detect,
/// which a USB CDC-ACM bridge never asserts. A `tty.` port with no `cu.` twin
/// is left alone, since it is then the only handle on that device.
fn dedupe_callout_ports(ports: &mut Vec<PortInfo>) {
    let callouts: HashSet<&str> = ports
        .iter()
        .filter_map(|p| p.name.strip_prefix("/dev/cu."))
        .collect();
    let shadowed: HashSet<String> = ports
        .iter()
        .filter(|p| {
            p.name
                .strip_prefix("/dev/tty.")
                .is_some_and(|suffix| callouts.contains(suffix))
        })
        .map(|p| p.name.clone())
        .collect();
    ports.retain(|p| !shadowed.contains(&p.name));
}

/// Enumerate the serial ports, flagging likely radios by VID/PID and collapsing
/// each macOS `cu.`/`tty.` pair down to its callout half.
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
    dedupe_callout_ports(&mut out);
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

    /// A radio port with the given device path. VID/PID are a known AnyTone
    /// bridge so `likely_radio` reflects what the real enumeration would say.
    fn radio_at(name: &str) -> PortInfo {
        PortInfo {
            name: name.to_string(),
            vid: Some(0x28e9),
            pid: Some(0x018a),
            product: Some("AnyTone".to_string()),
            likely_radio: true,
        }
    }

    fn names(ports: &[PortInfo]) -> Vec<&str> {
        ports.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn dedupe_drops_the_tty_half_of_a_callout_pair() {
        // The exact shape macOS reports for one plugged-in radio, and the reason
        // the device list showed every radio twice.
        let mut ports = vec![
            radio_at("/dev/cu.usbmodem14201"),
            radio_at("/dev/tty.usbmodem14201"),
        ];
        dedupe_callout_ports(&mut ports);
        assert_eq!(names(&ports), ["/dev/cu.usbmodem14201"]);
    }

    #[test]
    fn dedupe_keeps_a_tty_port_with_no_callout_twin() {
        // Hiding it would leave the device with no handle at all.
        let mut ports = vec![radio_at("/dev/tty.usbmodem14201")];
        dedupe_callout_ports(&mut ports);
        assert_eq!(names(&ports), ["/dev/tty.usbmodem14201"]);
    }

    #[test]
    fn dedupe_pairs_by_suffix_not_across_devices() {
        // Two radios plugged in at once must stay two radios.
        let mut ports = vec![
            radio_at("/dev/cu.usbmodem14201"),
            radio_at("/dev/tty.usbmodem14201"),
            radio_at("/dev/cu.usbmodem14301"),
            radio_at("/dev/tty.usbmodem14301"),
        ];
        dedupe_callout_ports(&mut ports);
        assert_eq!(
            names(&ports),
            ["/dev/cu.usbmodem14201", "/dev/cu.usbmodem14301"]
        );
    }

    #[test]
    fn dedupe_leaves_unrelated_ports_alone() {
        let mut ports = vec![
            radio_at("/dev/cu.usbmodem14201"),
            radio_at("/dev/tty.usbmodem14201"),
            radio_at("/dev/cu.Bluetooth-Incoming-Port"),
        ];
        dedupe_callout_ports(&mut ports);
        assert_eq!(
            names(&ports),
            ["/dev/cu.usbmodem14201", "/dev/cu.Bluetooth-Incoming-Port"]
        );
    }

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
