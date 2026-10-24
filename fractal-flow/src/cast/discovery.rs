//! mDNS discovery of Chromecast devices via the `_googlecast._tcp.local.` service.

use std::collections::HashMap;
use std::net::IpAddr;

use anyhow::Context;
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent};

use super::CastDeviceInfo;

const SERVICE_TYPE: &str = "_googlecast._tcp.local.";

/// Browses the local network for Chromecast devices and keeps the current list.
pub struct Discovery {
    daemon: ServiceDaemon,
    receiver: Receiver<ServiceEvent>,
    /// Found devices keyed by the mDNS full instance name.
    found: HashMap<String, CastDeviceInfo>,
}

impl Discovery {
    /// Starts the mDNS daemon and begins browsing for Chromecast devices.
    pub fn start() -> anyhow::Result<Discovery> {
        let daemon = ServiceDaemon::new().context("could not start the mDNS daemon")?;
        let receiver = daemon
            .browse(SERVICE_TYPE)
            .context("could not browse for Chromecast devices")?;
        Ok(Discovery {
            daemon,
            receiver,
            found: HashMap::new(),
        })
    }

    /// Drains pending mDNS events without blocking and returns the device list sorted by name.
    pub fn devices(&mut self) -> Vec<CastDeviceInfo> {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let addrs: Vec<IpAddr> = info
                        .get_addresses()
                        .iter()
                        .map(|a| a.to_ip_addr())
                        .collect();
                    let friendly = info.get_property_val_str("fn");
                    match device_from_parts(friendly, info.get_fullname(), &addrs, info.get_port())
                    {
                        Some(device) => {
                            self.found.insert(info.get_fullname().to_string(), device);
                        }
                        None => log::debug!(
                            "ignoring cast service {} with no addresses",
                            info.get_fullname()
                        ),
                    }
                }
                ServiceEvent::ServiceRemoved(_, fullname) => {
                    self.found.remove(&fullname);
                }
                _ => {}
            }
        }
        let mut list: Vec<CastDeviceInfo> = self.found.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.host.cmp(&b.host)));
        list
    }

    /// Clears the device list and restarts browsing.
    pub fn refresh(&mut self) -> anyhow::Result<()> {
        self.found.clear();
        if let Err(e) = self.daemon.stop_browse(SERVICE_TYPE) {
            log::warn!("stop_browse failed: {e}");
        }
        self.receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .context("could not restart browsing for Chromecast devices")?;
        Ok(())
    }
}

impl Drop for Discovery {
    /// Shuts the mDNS daemon down.
    fn drop(&mut self) {
        if let Err(e) = self.daemon.shutdown() {
            log::warn!("mDNS shutdown failed: {e}");
        }
    }
}

/// Builds device info from the TXT "fn" friendly name (falling back to the instance name),
/// preferring an IPv4 address. Returns None when there are no addresses.
pub fn device_from_parts(
    friendly: Option<&str>,
    fullname: &str,
    addrs: &[IpAddr],
    port: u16,
) -> Option<CastDeviceInfo> {
    let host = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()?;
    let name = match friendly.map(str::trim).filter(|s| !s.is_empty()) {
        Some(f) => f.to_string(),
        None => fullname
            .strip_suffix(SERVICE_TYPE)
            .map(|s| s.trim_end_matches('.'))
            .unwrap_or(fullname)
            .to_string(),
    };
    Some(CastDeviceInfo { name, host, port })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    /// Friendly name from the TXT record wins over the instance name.
    #[test]
    fn uses_friendly_name() {
        let addrs = [IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5))];
        let d = device_from_parts(
            Some("Living Room TV"),
            "Chromecast-abc._googlecast._tcp.local.",
            &addrs,
            8009,
        )
        .unwrap();
        assert_eq!(d.name, "Living Room TV");
        assert_eq!(d.port, 8009);
    }

    /// Without a friendly name the instance name prefix is used.
    #[test]
    fn falls_back_to_instance_name() {
        let addrs = [IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))];
        let d = device_from_parts(None, "Chromecast-abc._googlecast._tcp.local.", &addrs, 8009)
            .unwrap();
        assert_eq!(d.name, "Chromecast-abc");
        let d = device_from_parts(
            Some("  "),
            "Chromecast-abc._googlecast._tcp.local.",
            &addrs,
            8009,
        )
        .unwrap();
        assert_eq!(d.name, "Chromecast-abc");
    }

    /// IPv4 is chosen even when an IPv6 address is listed first.
    #[test]
    fn prefers_ipv4() {
        let v6 = IpAddr::V6(Ipv6Addr::LOCALHOST);
        let v4 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 9));
        let d =
            device_from_parts(Some("TV"), "x._googlecast._tcp.local.", &[v6, v4], 8009).unwrap();
        assert_eq!(d.host, v4);
        let d = device_from_parts(Some("TV"), "x._googlecast._tcp.local.", &[v6], 8009).unwrap();
        assert_eq!(d.host, v6);
    }

    /// No addresses means no usable device.
    #[test]
    fn none_without_addresses() {
        assert!(device_from_parts(Some("TV"), "x._googlecast._tcp.local.", &[], 8009).is_none());
    }
}
