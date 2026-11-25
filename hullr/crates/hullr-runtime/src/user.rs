//! User and group resolution from a container's `/etc/passwd` and
//! `/etc/group`, plus `/etc/subuid` / `/etc/subgid` range lookup for
//! rootless user namespaces. Pure parsing, so it is unit tested on any OS.

use hullr_core::{Error, Result};

/// The identity a container process runs as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub uid: u32,
    pub gid: u32,
    /// Supplementary groups (includes `gid`).
    pub groups: Vec<u32>,
    /// Home directory from passwd, used for `$HOME` when unset.
    pub home: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswdEntry {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntry {
    pub name: String,
    pub gid: u32,
    pub members: Vec<String>,
}

pub fn parse_passwd(text: &str) -> Vec<PasswdEntry> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 4 {
                return None;
            }
            Some(PasswdEntry {
                name: f[0].to_string(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                home: f.get(5).unwrap_or(&"").to_string(),
            })
        })
        .collect()
}

pub fn parse_group(text: &str) -> Vec<GroupEntry> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let f: Vec<&str> = l.split(':').collect();
            if f.len() < 3 {
                return None;
            }
            Some(GroupEntry {
                name: f[0].to_string(),
                gid: f[2].parse().ok()?,
                members: f
                    .get(3)
                    .map(|m| m.split(',').filter(|s| !s.is_empty()).map(String::from).collect())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Resolve a Docker `--user` value (`name`, `uid`, `uid:gid`, `name:group`,
/// ...) against passwd/group file contents. `None` or empty means root.
/// A numeric uid not in passwd is allowed (gid defaults to the uid's passwd
/// gid, or 0), like Docker.
pub fn resolve(user: Option<&str>, passwd: &str, group: &str) -> Result<Identity> {
    let pw = parse_passwd(passwd);
    let gr = parse_group(group);
    let spec = user.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("0");
    let (u, g) = match spec.split_once(':') {
        Some((u, g)) => (u, Some(g)),
        None => (spec, None),
    };
    let entry = match u.parse::<u32>() {
        Ok(uid) => pw.iter().find(|e| e.uid == uid).cloned().or(Some(PasswdEntry {
            name: String::new(),
            uid,
            gid: 0,
            home: String::new(),
        })),
        Err(_) => pw.iter().find(|e| e.name == u).cloned(),
    }
    .ok_or_else(|| Error::Invalid(format!("unable to find user {u}: no matching entries in passwd file")))?;

    let gid = match g {
        None | Some("") => entry.gid,
        Some(g) => match g.parse::<u32>() {
            Ok(n) => n,
            Err(_) => gr
                .iter()
                .find(|e| e.name == g)
                .map(|e| e.gid)
                .ok_or_else(|| Error::Invalid(format!("unable to find group {g}: no matching entries in group file")))?,
        },
    };
    let mut groups = vec![gid];
    // Supplementary groups only when the group was not forced explicitly.
    if g.is_none() && !entry.name.is_empty() {
        for e in &gr {
            if e.members.iter().any(|m| m == &entry.name) && !groups.contains(&e.gid) {
                groups.push(e.gid);
            }
        }
    }
    Ok(Identity {
        uid: entry.uid,
        gid,
        groups,
        home: Some(entry.home).filter(|h| !h.is_empty()),
    })
}

/// One `/etc/subuid` style range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubIdRange {
    pub start: u32,
    pub count: u32,
}

/// Find the first subordinate id range for the user (by name or numeric id).
pub fn find_subid(text: &str, name: &str, id: u32) -> Option<SubIdRange> {
    let id_s = id.to_string();
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.trim().split(':').collect();
        if f.len() != 3 || (f[0] != name && f[0] != id_s) {
            return None;
        }
        let r = SubIdRange { start: f[1].parse().ok()?, count: f[2].parse().ok()? };
        (r.count > 0).then_some(r)
    })
}

pub const DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// The process environment: `base` then `extra` (later wins), plus the
/// defaults Docker adds: PATH, HOSTNAME, HOME and TERM (tty only).
pub fn build_env(base: &[String], extra: &[String], hostname: &str, tty: bool, home: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for kv in base.iter().chain(extra) {
        let key = kv.split_once('=').map(|(k, _)| k).unwrap_or(kv);
        out.retain(|e| e.split_once('=').map(|(k, _)| k).unwrap_or(e) != key);
        if kv.contains('=') {
            out.push(kv.clone());
        }
    }
    let has = |out: &Vec<String>, k: &str| out.iter().any(|e| e.starts_with(&format!("{k}=")));
    if !has(&out, "PATH") {
        out.insert(0, format!("PATH={DEFAULT_PATH}"));
    }
    if !has(&out, "HOSTNAME") && !hostname.is_empty() {
        out.push(format!("HOSTNAME={hostname}"));
    }
    if tty && !has(&out, "TERM") {
        out.push("TERM=xterm".into());
    }
    if !has(&out, "HOME") {
        out.push(format!("HOME={}", home.unwrap_or("/")));
    }
    out
}

/// The value of `key` in a `KEY=VALUE` list.
pub fn env_get<'a>(env: &'a [String], key: &str) -> Option<&'a str> {
    env.iter().find_map(|e| e.strip_prefix(key)?.strip_prefix('='))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env() {
        let e = build_env(&["A=1".into(), "B=2".into()], &["A=3".into()], "box", true, Some("/root"));
        assert_eq!(e, ["PATH=".to_string() + DEFAULT_PATH, "B=2".into(), "A=3".into(), "HOSTNAME=box".into(), "TERM=xterm".into(), "HOME=/root".into()]);
        let e = build_env(&["PATH=/x".into(), "HOME=/h".into()], &[], "", false, None);
        assert_eq!(e, ["PATH=/x", "HOME=/h"]);
        assert_eq!(env_get(&e, "PATH"), Some("/x"));
        assert_eq!(env_get(&e, "PAT"), None);
    }

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/sh\n\
        daemon:x:2:2:daemon:/sbin:/sbin/nologin\n\
        nginx:x:101:102:nginx:/var/lib/nginx:/sbin/nologin\n\
        # comment\n\
        app:x:1000:1000::/home/app:/bin/sh\n";
    const GROUP: &str = "root:x:0:root\nbin:x:1:root,bin,daemon\nwheel:x:10:root,app\n\
        nginx:x:102:nginx\napp:x:1000:\nstaff:x:50:app\n";

    #[test]
    fn default_is_root() {
        let id = resolve(None, PASSWD, GROUP).unwrap();
        assert_eq!((id.uid, id.gid), (0, 0));
        assert_eq!(id.groups, vec![0, 1, 10]);
        assert_eq!(id.home.as_deref(), Some("/root"));
    }

    #[test]
    fn by_name_and_number() {
        let id = resolve(Some("nginx"), PASSWD, GROUP).unwrap();
        assert_eq!((id.uid, id.gid), (101, 102));
        let id = resolve(Some("app"), PASSWD, GROUP).unwrap();
        assert_eq!(id.groups, vec![1000, 10, 50]);
        let id = resolve(Some("1000"), PASSWD, GROUP).unwrap();
        assert_eq!(id.home.as_deref(), Some("/home/app"));
        // Unknown numeric uid is allowed, gid 0.
        let id = resolve(Some("4242"), PASSWD, GROUP).unwrap();
        assert_eq!((id.uid, id.gid, id.home), (4242, 0, None));
    }

    #[test]
    fn with_group() {
        let id = resolve(Some("app:wheel"), PASSWD, GROUP).unwrap();
        assert_eq!((id.uid, id.gid), (1000, 10));
        assert_eq!(id.groups, vec![10]);
        let id = resolve(Some("1000:7"), PASSWD, GROUP).unwrap();
        assert_eq!((id.uid, id.gid), (1000, 7));
        assert!(resolve(Some("nobody"), PASSWD, GROUP).is_err());
        assert!(resolve(Some("app:nogroup"), PASSWD, GROUP).is_err());
    }

    #[test]
    fn subid() {
        let t = "alice:100000:65536\n1001:200000:65536\nbob:0:0\n";
        assert_eq!(find_subid(t, "alice", 1000), Some(SubIdRange { start: 100000, count: 65536 }));
        assert_eq!(find_subid(t, "carol", 1001).unwrap().start, 200000);
        assert_eq!(find_subid(t, "bob", 5), None);
    }
}
