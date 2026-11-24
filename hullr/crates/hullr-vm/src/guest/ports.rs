//! Discovering published ports inside the guest by reading the kernel's TCP
//! socket tables (`/proc/net/tcp` and `/proc/net/tcp6`) and diffing
//! snapshots into control messages.
//!
//! Only sockets in LISTEN state bound to a wildcard address (`0.0.0.0`, `::`)
//! or loopback (`127.0.0.1`, `::1`) are announced. Wildcard maps to host
//! `0.0.0.0`, loopback to host `127.0.0.1`; a port with both is announced as
//! wildcard. Sockets bound to other addresses (e.g. a container bridge IP)
//! are not reachable from the host and are skipped.

use crate::protocol::ControlMsg;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Kernel `TCP_LISTEN` state as printed in `/proc/net/tcp`.
const TCP_LISTEN: &str = "0A";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenSocket {
    pub ip: IpAddr,
    pub port: u16,
}

/// Published ports: port -> host bind address.
pub type Published = BTreeMap<u16, String>;

/// Parse `/proc/net/tcp` or `/proc/net/tcp6` content, returning listening
/// sockets. Unparseable lines (including the header) are skipped.
///
/// Addresses are printed as the raw in-memory words in host byte order, so
/// on little-endian guests (arm64, x86_64) `0100007F` is 127.0.0.1.
pub fn parse_proc_net_tcp(text: &str) -> Vec<ListenSocket> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<ListenSocket> {
    let mut fields = line.split_whitespace();
    let _slot = fields.next()?;
    let local = fields.next()?;
    let _remote = fields.next()?;
    let state = fields.next()?;
    if state != TCP_LISTEN {
        return None;
    }
    let (addr, port) = local.split_once(':')?;
    let port = u16::from_str_radix(port, 16).ok()?;
    let ip = match addr.len() {
        8 => IpAddr::V4(Ipv4Addr::from(word(addr)?)),
        32 => {
            let mut bytes = [0u8; 16];
            for i in 0..4 {
                bytes[i * 4..i * 4 + 4].copy_from_slice(&word(&addr[i * 8..i * 8 + 8])?);
            }
            IpAddr::V6(Ipv6Addr::from(bytes))
        }
        _ => return None,
    };
    Some(ListenSocket { ip, port })
}

/// One 32-bit word as printed by the kernel, back to its memory bytes.
fn word(hex: &str) -> Option<[u8; 4]> {
    Some(u32::from_str_radix(hex, 16).ok()?.to_ne_bytes())
}

/// Collapse listening sockets into the host-visible published set.
pub fn published(sockets: &[ListenSocket]) -> Published {
    let mut out = Published::new();
    for s in sockets {
        if s.port == 0 {
            continue;
        }
        let ip = match s.ip {
            IpAddr::V4(v4) => v4.to_ipv6_mapped(),
            IpAddr::V6(v6) => v6,
        };
        let wildcard = ip.is_unspecified() || ip.to_ipv4_mapped().is_some_and(|v| v.is_unspecified());
        let loopback = ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v| v.is_loopback());
        if wildcard {
            out.insert(s.port, "0.0.0.0".into());
        } else if loopback {
            out.entry(s.port).or_insert_with(|| "127.0.0.1".into());
        }
    }
    out
}

/// Messages that turn `old` into `new`: removals first, then additions. A
/// changed address is a remove plus an add.
pub fn diff(old: &Published, new: &Published) -> Vec<ControlMsg> {
    let mut out = Vec::new();
    for (port, ip) in old {
        if new.get(port) != Some(ip) {
            out.push(ControlMsg::remove(ip, *port));
        }
    }
    for (port, ip) in new {
        if old.get(port) != Some(ip) {
            out.push(ControlMsg::add(ip, *port));
        }
    }
    out
}

/// Snapshot of the guest's published ports right now.
pub fn read_published() -> Published {
    let mut sockets = Vec::new();
    for path in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(text) = std::fs::read_to_string(path) {
            sockets.extend(parse_proc_net_tcp(&text));
        }
    }
    published(&sockets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Op;

    const TCP: &str = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1001 1 0000000000000000 100 0 0 10 0
   1: 0100007F:0CEA 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1002 1 0000000000000000 100 0 0 10 0
   2: 0F02000A:0016 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1003 1 0000000000000000 100 0 0 10 0
   3: 0100007F:1F90 0100007F:D2C4 01 00000000:00000000 00:00000000 00000000     0        0 1004 1 0000000000000000 20 4 30 10 -1
";

    const TCP6: &str = "\
  sl  local_address                         remote_address                        st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 00000000000000000000000000000000:01BB 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 2001 1 0000000000000000 100 0 0 10 0
   1: 00000000000000000000000001000000:1388 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 2002 1 0000000000000000 100 0 0 10 0
   2: 0000000000000000FFFF00000100007F:0CEA 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 2003 1 0000000000000000 100 0 0 10 0
";

    #[test]
    #[cfg(target_endian = "little")]
    fn parses_v4_and_v6_listeners() {
        let v4 = parse_proc_net_tcp(TCP);
        assert_eq!(
            v4,
            vec![
                ListenSocket { ip: "0.0.0.0".parse().unwrap(), port: 8080 },
                ListenSocket { ip: "127.0.0.1".parse().unwrap(), port: 3306 },
                ListenSocket { ip: "10.0.2.15".parse().unwrap(), port: 22 },
            ]
        );
        let v6 = parse_proc_net_tcp(TCP6);
        assert_eq!(
            v6,
            vec![
                ListenSocket { ip: "::".parse().unwrap(), port: 443 },
                ListenSocket { ip: "::1".parse().unwrap(), port: 5000 },
                ListenSocket { ip: "::ffff:127.0.0.1".parse().unwrap(), port: 3306 },
            ]
        );
    }

    #[test]
    #[cfg(target_endian = "little")]
    fn collapses_to_published() {
        let mut all = parse_proc_net_tcp(TCP);
        all.extend(parse_proc_net_tcp(TCP6));
        // Loopback first, wildcard later on the same port: wildcard wins.
        all.push(ListenSocket { ip: "0.0.0.0".parse().unwrap(), port: 5000 });
        let p = published(&all);
        let want: Published = [
            (443, "0.0.0.0".to_string()),
            (3306, "127.0.0.1".to_string()),
            (5000, "0.0.0.0".to_string()),
            (8080, "0.0.0.0".to_string()),
        ]
        .into_iter()
        .collect();
        assert_eq!(p, want);
    }

    #[test]
    fn diffing() {
        let old: Published = [(80, "0.0.0.0".into()), (81, "127.0.0.1".into()), (82, "0.0.0.0".into())].into();
        let new: Published = [(80, "0.0.0.0".into()), (81, "0.0.0.0".into()), (90, "0.0.0.0".into())].into();
        let d = diff(&old, &new);
        assert_eq!(
            d,
            vec![
                ControlMsg::remove("127.0.0.1", 81),
                ControlMsg::remove("0.0.0.0", 82),
                ControlMsg::add("0.0.0.0", 81),
                ControlMsg::add("0.0.0.0", 90),
            ]
        );
        assert!(diff(&new, &new).is_empty());
        assert!(diff(&Published::new(), &new).iter().all(|m| m.op == Op::Add));
    }

    #[test]
    fn garbage_is_ignored() {
        assert!(parse_proc_net_tcp("nonsense\n 0: zz:yy 0 0A\n 1: 0100007F 00 0A\n").is_empty());
    }
}
