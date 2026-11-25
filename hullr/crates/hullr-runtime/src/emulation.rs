//! Foreign-architecture support through qemu-user and binfmt_misc.
//!
//! A container for another CPU only works if the kernel has a binfmt_misc
//! handler for that ELF machine with the `F` (fix-binary) flag: the kernel
//! then opens the interpreter at registration time, so it works inside any
//! mount namespace without the container having qemu in its rootfs.

use hullr_core::{Error, Platform, Result};
use std::path::{Path, PathBuf};

pub const BINFMT_DIR: &str = "/proc/sys/fs/binfmt_misc";

/// A parsed `/proc/sys/fs/binfmt_misc/<name>` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinfmtEntry {
    pub name: String,
    pub enabled: bool,
    pub interpreter: String,
    pub flags: String,
}

pub fn parse_entry(name: &str, text: &str) -> BinfmtEntry {
    let mut e = BinfmtEntry { name: name.to_string(), enabled: false, interpreter: String::new(), flags: String::new() };
    for line in text.lines() {
        let line = line.trim();
        if line == "enabled" {
            e.enabled = true;
        } else if let Some(i) = line.strip_prefix("interpreter ") {
            e.interpreter = i.trim().to_string();
        } else if let Some(f) = line.strip_prefix("flags:") {
            e.flags = f.trim().to_string();
        }
    }
    e
}

impl BinfmtEntry {
    /// Does this entry run binaries for the qemu arch `qarch` (e.g. `aarch64`)?
    pub fn is_for(&self, qarch: &str) -> bool {
        let interp = Path::new(&self.interpreter)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let want = format!("qemu-{qarch}");
        self.name == want
            || interp == want
            || interp == format!("{want}-static")
            // Debian's wrapper names: /usr/libexec/qemu-binfmt/aarch64-binfmt-P
            || interp == format!("{qarch}-binfmt-P")
    }
    pub fn usable(&self) -> bool {
        self.enabled && self.flags.contains('F')
    }
}

/// ELF magic and mask per qemu arch, as in qemu's `qemu-binfmt-conf.sh`.
pub const MAGIC: &[(&str, &str, &str)] = &[
    (
        "aarch64",
        r"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\xb7\x00",
        r"\xff\xff\xff\xff\xff\xff\xff\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff",
    ),
    (
        "arm",
        r"\x7fELF\x01\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x28\x00",
        r"\xff\xff\xff\xff\xff\xff\xff\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff",
    ),
    (
        "x86_64",
        r"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x3e\x00",
        r"\xff\xff\xff\xff\xff\xfe\xfe\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff",
    ),
    (
        "i386",
        r"\x7fELF\x01\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x03\x00",
        r"\xff\xff\xff\xff\xff\xfe\xfe\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff",
    ),
    (
        "ppc64le",
        r"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x15\x00",
        r"\xff\xff\xff\xff\xff\xff\xff\xfc\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\x00",
    ),
    (
        "s390x",
        r"\x7fELF\x02\x02\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\x16",
        r"\xff\xff\xff\xff\xff\xff\xff\xfc\xff\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff",
    ),
    (
        "riscv64",
        r"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x02\x00\xf3\x00",
        r"\xff\xff\xff\xff\xff\xff\xff\x00\xff\xff\xff\xff\xff\xff\xff\xff\xfe\xff\xff\xff",
    ),
];

/// The `register` line for one arch.
pub fn register_line(qarch: &str, interpreter: &Path) -> Option<String> {
    let (_, magic, mask) = MAGIC.iter().find(|(a, _, _)| *a == qarch)?;
    Some(format!(":qemu-{qarch}:M::{magic}:{mask}:{}:F", interpreter.display()))
}

/// OCI platform for a qemu arch name.
pub fn platform_for(qarch: &str) -> Platform {
    match qarch {
        "aarch64" => Platform::new("linux", "arm64", None),
        "arm" => Platform::new("linux", "arm", Some("v7")),
        "x86_64" => Platform::new("linux", "amd64", None),
        "i386" => Platform::new("linux", "386", None),
        other => Platform::new("linux", other, None),
    }
}

pub fn read_entries() -> Vec<BinfmtEntry> {
    let Ok(rd) = std::fs::read_dir(BINFMT_DIR) else { return vec![] };
    rd.flatten()
        .filter_map(|d| {
            let name = d.file_name().to_string_lossy().into_owned();
            if name == "register" || name == "status" {
                return None;
            }
            Some(parse_entry(&name, &std::fs::read_to_string(d.path()).ok()?))
        })
        .collect()
}

/// Can the host CPU run this platform without a binfmt handler?
fn natively_runnable(p: &Platform) -> bool {
    if p.is_native() {
        return true;
    }
    let host = Platform::host();
    // 32-bit x86 runs on x86_64 kernels (IA32 emulation is near universal).
    host.architecture == "amd64" && p.architecture == "386"
}

/// Error out unless `p` can run here (natively or via a usable handler).
pub fn check(p: &Platform) -> Result<()> {
    if p.os != "linux" {
        return Err(Error::Unsupported(format!("{p}: only linux containers are supported")));
    }
    if natively_runnable(p) {
        return Ok(());
    }
    let q = p.qemu_arch();
    if read_entries().iter().any(|e| e.is_for(q) && e.usable()) {
        return Ok(());
    }
    Err(Error::Unsupported(format!(
        "{p} needs emulation: run `hullr system emulation enable` (installs binfmt handlers for qemu-user-static)"
    )))
}

/// Foreign platforms with a usable handler right now.
pub fn emulated() -> Vec<Platform> {
    let entries = read_entries();
    MAGIC
        .iter()
        .map(|(a, _, _)| *a)
        .filter(|a| entries.iter().any(|e| e.is_for(a) && e.usable()))
        .map(platform_for)
        .filter(|p| !p.is_native())
        .collect()
}

fn find_qemu(qarch: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.push("/usr/bin".into());
    dirs.push("/usr/local/bin".into());
    for d in dirs {
        let c = d.join(format!("qemu-{qarch}-static"));
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

/// Register binfmt handlers with the F flag for every qemu-*-static found.
/// Existing handlers without F for the same arch are replaced.
#[cfg(target_os = "linux")]
pub fn enable() -> Result<Vec<String>> {
    if nix::unistd::geteuid().as_raw() != 0 {
        return Err(Error::Unsupported("registering binfmt handlers needs root (try sudo)".into()));
    }
    let reg = Path::new(BINFMT_DIR).join("register");
    if !reg.exists() {
        nix::mount::mount(
            Some("binfmt_misc"),
            BINFMT_DIR,
            Some("binfmt_misc"),
            nix::mount::MsFlags::empty(),
            None::<&str>,
        )
        .map_err(|e| Error::Runtime(format!("mount binfmt_misc: {e}")))?;
    }
    let host_q = Platform::host().qemu_arch().to_string();
    let mut enabled = vec![];
    let entries = read_entries();
    for (qarch, _, _) in MAGIC {
        if *qarch == host_q || (host_q == "x86_64" && *qarch == "i386") {
            continue;
        }
        if entries.iter().any(|e| e.is_for(qarch) && e.usable()) {
            enabled.push(qarch.to_string());
            continue;
        }
        let Some(interp) = find_qemu(qarch) else { continue };
        for e in entries.iter().filter(|e| e.is_for(qarch)) {
            // Replace a handler that lacks F: write -1 removes it.
            let _ = std::fs::write(Path::new(BINFMT_DIR).join(&e.name), "-1");
        }
        let line = register_line(qarch, &interp).expect("arch in table");
        std::fs::write(&reg, line).map_err(|e| Error::Runtime(format!("register qemu-{qarch}: {e}")))?;
        enabled.push(qarch.to_string());
    }
    if enabled.is_empty() {
        return Err(Error::NotFound(
            "no qemu-*-static binaries found; install qemu-user-static first".into(),
        ));
    }
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse() {
        let t = "enabled\ninterpreter /usr/libexec/qemu-binfmt/aarch64-binfmt-P\nflags: POCF\noffset 0\nmagic 7f454c46\nmask ffffff\n";
        let e = parse_entry("qemu-aarch64", t);
        assert!(e.enabled && e.usable());
        assert!(e.is_for("aarch64"));
        assert!(!e.is_for("arm"));
        let e = parse_entry("arm", "disabled\ninterpreter /usr/bin/qemu-arm-static\nflags: \n");
        assert!(e.is_for("arm") && !e.usable());
    }

    #[test]
    fn register() {
        let l = register_line("aarch64", Path::new("/usr/bin/qemu-aarch64-static")).unwrap();
        assert!(l.starts_with(r":qemu-aarch64:M::\x7fELF\x02"));
        assert!(l.ends_with(":/usr/bin/qemu-aarch64-static:F"));
        assert_eq!(l.matches(':').count(), 7);
        assert!(register_line("vax", Path::new("/x")).is_none());
    }

    #[test]
    fn native_ok() {
        check(&Platform::host()).unwrap();
        assert!(check(&Platform::new("windows", "amd64", None)).is_err());
    }
}
