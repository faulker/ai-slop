use serde::{Deserialize, Serialize};
use std::fmt;

/// An OCI platform, e.g. `linux/amd64` or `linux/arm/v7`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Platform {
    pub os: String,
    pub architecture: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl Platform {
    pub fn new(os: &str, arch: &str, variant: Option<&str>) -> Self {
        Self {
            os: os.into(),
            architecture: normalize_arch(arch).into(),
            variant: variant.map(Into::into),
        }
    }

    /// The platform containers run on natively. Always `linux/*`, because on
    /// macOS containers run in a Linux VM of the host's architecture.
    pub fn host() -> Self {
        let arch = normalize_arch(std::env::consts::ARCH);
        let variant = if arch == "arm64" { Some("v8") } else { None };
        Self::new("linux", arch, variant)
    }

    /// Parse `os/arch[/variant]` or a bare `arch`.
    pub fn parse(s: &str) -> crate::Result<Self> {
        let parts: Vec<&str> = s.trim().split('/').collect();
        match parts.as_slice() {
            [arch] if !arch.is_empty() => Ok(Self::new("linux", arch, None)),
            [os, arch] => Ok(Self::new(os, arch, None)),
            [os, arch, variant] => Ok(Self::new(os, arch, Some(variant))),
            _ => Err(crate::Error::Invalid(format!("bad platform '{s}'"))),
        }
    }

    /// True if this platform runs without emulation on the host.
    pub fn is_native(&self) -> bool {
        let h = Self::host();
        self.os == h.os && self.architecture == h.architecture
    }

    /// Does `self` (a requested platform) accept `other` (an image's platform)?
    /// A missing variant on the request matches any variant.
    pub fn matches(&self, other: &Platform) -> bool {
        self.os == other.os
            && self.architecture == normalize_arch(&other.architecture)
            && match (&self.variant, &other.variant) {
                (None, _) => true,
                (Some(a), Some(b)) => a == b,
                // arm64 images usually omit the implicit v8
                (Some(v), None) => self.architecture == "arm64" && v == "v8",
            }
    }

    /// The qemu-user binary suffix for this architecture (`qemu-<suffix>`).
    pub fn qemu_arch(&self) -> &str {
        match self.architecture.as_str() {
            "amd64" => "x86_64",
            "arm64" => "aarch64",
            "arm" => "arm",
            "386" => "i386",
            "ppc64le" => "ppc64le",
            "s390x" => "s390x",
            "riscv64" => "riscv64",
            other => other,
        }
    }
}

impl Default for Platform {
    fn default() -> Self {
        Self::host()
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.os, self.architecture)?;
        if let Some(v) = &self.variant {
            write!(f, "/{v}")?;
        }
        Ok(())
    }
}

/// Map Rust/uname arch names to OCI (GOARCH) names.
pub fn normalize_arch(a: &str) -> &str {
    match a {
        "x86_64" | "x86-64" | "amd64" => "amd64",
        "aarch64" | "arm64" => "arm64",
        "armv7l" | "armhf" | "arm" => "arm",
        "i386" | "i686" | "x86" | "386" => "386",
        "powerpc64le" | "ppc64le" => "ppc64le",
        "riscv64" | "riscv64gc" => "riscv64",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        assert_eq!(Platform::parse("linux/amd64").unwrap().to_string(), "linux/amd64");
        assert_eq!(Platform::parse("linux/arm/v7").unwrap().variant.as_deref(), Some("v7"));
        assert_eq!(Platform::parse("aarch64").unwrap().architecture, "arm64");
        assert!(Platform::parse("a/b/c/d").is_err());
    }

    #[test]
    fn matching() {
        let want = Platform::parse("linux/arm64/v8").unwrap();
        assert!(want.matches(&Platform::new("linux", "arm64", None)));
        let any = Platform::parse("linux/arm").unwrap();
        assert!(any.matches(&Platform::new("linux", "arm", Some("v7"))));
        assert!(!Platform::parse("linux/amd64").unwrap().matches(&Platform::new("linux", "arm64", None)));
    }
}
