//! Container networking helpers: `/etc/hosts` and `/etc/resolv.conf`
//! generation, bringing up `lo`, and user-mode networking through `pasta`
//! (preferred) or `slirp4netns`, both run from the host side against the
//! pid of a process inside the new network namespace.

use hullr_core::{NetworkMode, PortMapping, Protocol};
use std::path::{Path, PathBuf};

/// `/etc/hosts` for a container.
pub fn hosts_file(hostname: &str, mode: &NetworkMode, extra: &[(String, String)], host_hosts: &str) -> String {
    let mut s = String::new();
    if matches!(mode, NetworkMode::Host) {
        s.push_str(host_hosts);
        if !s.ends_with('\n') && !s.is_empty() {
            s.push('\n');
        }
    } else {
        s.push_str("127.0.0.1\tlocalhost localhost.localdomain\n");
        s.push_str("::1\tlocalhost ip6-localhost ip6-loopback\n");
        s.push_str("fe00::0\tip6-localnet\nff00::0\tip6-mcastprefix\nff02::1\tip6-allnodes\nff02::2\tip6-allrouters\n");
    }
    if !hostname.is_empty() {
        s.push_str(&format!("127.0.1.1\t{hostname}\n"));
    }
    for (name, ip) in extra {
        // Docker's special value for the host gateway.
        let ip = if ip == "host-gateway" { "10.0.2.2" } else { ip.as_str() };
        s.push_str(&format!("{ip}\t{name}\n"));
    }
    s
}

/// `/etc/resolv.conf` for a container, from the host's. Loopback resolvers
/// (systemd-resolved's 127.0.0.53, dnsmasq on 127.0.0.1) are unreachable
/// from a private netns, so they are replaced by `upstream` (systemd's real
/// upstream list) or public resolvers.
pub fn resolv_conf(host: &str, upstream: Option<&str>, host_net: bool) -> String {
    let servers = |t: &str| -> Vec<String> {
        t.lines()
            .filter_map(|l| l.trim().strip_prefix("nameserver"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let is_loop = |s: &String| s.starts_with("127.") || s == "::1";
    let mut out = String::new();
    for l in host.lines() {
        let t = l.trim();
        if t.starts_with("search") || t.starts_with("options") {
            out.push_str(t);
            out.push('\n');
        }
    }
    let mut ns = servers(host);
    if !host_net && ns.iter().any(is_loop) {
        ns.retain(|s| !is_loop(s));
        if ns.is_empty() {
            ns = upstream.map(servers).unwrap_or_default();
            ns.retain(|s| !is_loop(s));
        }
    }
    if ns.is_empty() {
        ns = vec!["8.8.8.8".into(), "1.1.1.1".into()];
    }
    for s in ns {
        out.push_str(&format!("nameserver {s}\n"));
    }
    out
}

pub fn host_resolv_conf(host_net: bool) -> String {
    let host = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let upstream = std::fs::read_to_string("/run/systemd/resolve/resolv.conf").ok();
    resolv_conf(&host, upstream.as_deref(), host_net)
}

pub fn find_on_path(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into());
    std::env::split_paths(&path)
        .chain(["/usr/bin".into(), "/usr/sbin".into(), "/usr/local/bin".into()])
        .map(|d| d.join(bin))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// Which user-mode network tool is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserNet {
    Pasta(PathBuf),
    Slirp(PathBuf),
}

pub fn detect_user_net() -> Option<UserNet> {
    if std::env::var_os("HULLR_NO_PASTA").is_none() {
        if let Some(p) = find_on_path("pasta") {
            return Some(UserNet::Pasta(p));
        }
    }
    find_on_path("slirp4netns").map(UserNet::Slirp)
}

/// pasta `-t`/`-u` arguments for the given mappings (explicit `none` when
/// empty, since pasta's default is to forward every bound port).
pub fn pasta_port_args(ports: &[PortMapping]) -> Vec<String> {
    let mut args = vec![];
    for (flag, proto) in [("-t", Protocol::Tcp), ("-u", Protocol::Udp)] {
        let specs: Vec<String> = ports
            .iter()
            .filter(|p| p.protocol == proto)
            .map(|p| match p.host_ip.as_deref() {
                Some(ip) if !ip.is_empty() && ip != "0.0.0.0" => {
                    format!("{ip}/{}:{}", p.host_port, p.container_port)
                }
                _ => format!("{}:{}", p.host_port, p.container_port),
            })
            .collect();
        args.push(flag.to_string());
        args.push(if specs.is_empty() { "none".into() } else { specs.join(",") });
    }
    // No forwarding from the namespace back to host ports.
    args.extend(["-T".into(), "none".into(), "-U".into(), "none".into()]);
    args
}

/// A running user-mode network helper; killed on drop.
#[cfg(target_os = "linux")]
pub struct NetHelper {
    child: std::process::Child,
}

#[cfg(target_os = "linux")]
impl Drop for NetHelper {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Attach user-mode networking to the netns of `pid`. `api_dir` is a
/// directory for slirp4netns's API socket. Returns once ports are set up.
#[cfg(target_os = "linux")]
pub fn attach(tool: &UserNet, pid: u32, ports: &[PortMapping], api_dir: &Path) -> Result<NetHelper, String> {
    use std::process::{Command, Stdio};
    match tool {
        UserNet::Pasta(bin) => {
            let mut cmd = Command::new(bin);
            cmd.args(["--config-net", "--quiet", "--foreground", "--no-map-gw"])
                .args(pasta_port_args(ports))
                .arg(pid.to_string())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            let mut child = cmd.spawn().map_err(|e| format!("spawn pasta: {e}"))?;
            // pasta configures the namespace before it starts forwarding;
            // wait until the tap device has an address (or pasta dies).
            for _ in 0..200 {
                if let Ok(Some(st)) = child.try_wait() {
                    let mut err = String::new();
                    if let Some(mut e) = child.stderr.take() {
                        use std::io::Read;
                        let _ = e.read_to_string(&mut err);
                    }
                    return Err(format!("pasta exited ({st}): {}", err.trim()));
                }
                if netns_configured(pid) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            // Keep stderr drained so pasta never blocks on a full pipe.
            if let Some(mut e) = child.stderr.take() {
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut e, &mut std::io::sink());
                });
            }
            Ok(NetHelper { child })
        }
        UserNet::Slirp(bin) => {
            let sock = api_dir.join("slirp.sock");
            let _ = std::fs::remove_file(&sock);
            let mut child = Command::new(bin)
                .args(["--configure", "--mtu=65520", "--disable-host-loopback", "--api-socket"])
                .arg(&sock)
                .arg(pid.to_string())
                .arg("tap0")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("spawn slirp4netns: {e}"))?;
            let mut ok = false;
            for _ in 0..300 {
                if let Ok(Some(st)) = child.try_wait() {
                    return Err(format!("slirp4netns exited ({st})"));
                }
                if sock.exists() && netns_configured(pid) {
                    ok = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            if !ok {
                let _ = child.kill();
                return Err("slirp4netns did not become ready".into());
            }
            let helper = NetHelper { child };
            for p in ports {
                slirp_hostfwd(&sock, p)?;
            }
            Ok(helper)
        }
    }
}

#[cfg(target_os = "linux")]
fn slirp_hostfwd(sock: &Path, p: &PortMapping) -> Result<(), String> {
    use std::io::{Read, Write};
    let req = serde_json::json!({
        "execute": "add_hostfwd",
        "arguments": {
            "proto": if p.protocol == Protocol::Udp { "udp" } else { "tcp" },
            "host_addr": p.host_ip.clone().unwrap_or_else(|| "0.0.0.0".into()),
            "host_port": p.host_port,
            "guest_port": p.container_port,
        }
    });
    let mut s = std::os::unix::net::UnixStream::connect(sock).map_err(|e| format!("slirp api: {e}"))?;
    s.write_all(req.to_string().as_bytes()).map_err(|e| e.to_string())?;
    let _ = s.shutdown(std::net::Shutdown::Write);
    let mut resp = String::new();
    let _ = s.read_to_string(&mut resp);
    if resp.contains("error") {
        return Err(format!("slirp4netns port {p}: {resp}"));
    }
    Ok(())
}

/// True once the netns of `pid` has a non-loopback interface with a route.
#[cfg(target_os = "linux")]
fn netns_configured(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/net/route"))
        .map(|r| r.lines().skip(1).any(|l| !l.trim().is_empty()))
        .unwrap_or(false)
}

/// Set IFF_UP on `lo` in the current network namespace.
#[cfg(target_os = "linux")]
pub fn loopback_up() -> std::io::Result<()> {
    // SAFETY: plain socket/ioctl calls on a zeroed, correctly sized ifreq;
    // the fd is closed before returning.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut ifr: libc::ifreq = std::mem::zeroed();
        for (i, b) in b"lo\0".iter().enumerate() {
            ifr.ifr_name[i] = *b as libc::c_char;
        }
        let mut res = libc::ioctl(fd, libc::SIOCGIFFLAGS as _, &mut ifr);
        if res == 0 {
            ifr.ifr_ifru.ifru_flags |= (libc::IFF_UP | libc::IFF_RUNNING) as libc::c_short;
            res = libc::ioctl(fd, libc::SIOCSIFFLAGS as _, &ifr);
        }
        let err = std::io::Error::last_os_error();
        libc::close(fd);
        if res != 0 {
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        let h = hosts_file("web", &NetworkMode::Private, &[("db".into(), "127.0.0.1".into())], "");
        assert!(h.starts_with("127.0.0.1\tlocalhost"));
        assert!(h.contains("127.0.1.1\tweb\n"));
        assert!(h.ends_with("127.0.0.1\tdb\n"));
        let h = hosts_file("web", &NetworkMode::Host, &[], "10.0.0.1 myhost");
        assert!(h.starts_with("10.0.0.1 myhost\n127.0.1.1\tweb"));
    }

    #[test]
    fn resolv() {
        let stub = "nameserver 127.0.0.53\noptions edns0 trust-ad\nsearch lan\n";
        let up = "nameserver 192.168.1.1\nnameserver 127.0.0.1\n";
        let r = resolv_conf(stub, Some(up), false);
        assert_eq!(r, "options edns0 trust-ad\nsearch lan\nnameserver 192.168.1.1\n");
        assert_eq!(resolv_conf(stub, None, false), "options edns0 trust-ad\nsearch lan\nnameserver 8.8.8.8\nnameserver 1.1.1.1\n");
        // Host networking can use the stub directly.
        assert!(resolv_conf(stub, None, true).contains("127.0.0.53"));
        assert_eq!(resolv_conf("nameserver 9.9.9.9\n", None, false), "nameserver 9.9.9.9\n");
    }

    #[test]
    fn pasta_ports() {
        let ports = vec![
            PortMapping::parse("8080:80").unwrap(),
            PortMapping::parse("127.0.0.1:5353:53/udp").unwrap(),
            PortMapping::parse("9000:9000").unwrap(),
        ];
        assert_eq!(
            pasta_port_args(&ports),
            ["-t", "8080:80,9000:9000", "-u", "127.0.0.1/5353:53", "-T", "none", "-U", "none"]
        );
        assert_eq!(pasta_port_args(&[])[..4], ["-t", "none", "-u", "none"]);
    }
}
