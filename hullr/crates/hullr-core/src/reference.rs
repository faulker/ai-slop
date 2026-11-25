use serde::{Deserialize, Serialize};
use std::fmt;

pub const DOCKER_HUB: &str = "docker.io";
/// Docker Hub's actual registry API host.
pub const DOCKER_HUB_API: &str = "registry-1.docker.io";

/// A normalized image reference: `registry/repository[:tag][@digest]`.
///
/// `nginx` becomes `docker.io/library/nginx:latest`,
/// `ghcr.io/foo/bar` becomes `ghcr.io/foo/bar:latest`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ImageRef {
    pub registry: String,
    pub repository: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
}

impl ImageRef {
    pub fn parse(s: &str) -> crate::Result<Self> {
        let s = s.trim();
        if s.is_empty() || s.contains(char::is_whitespace) {
            return Err(crate::Error::Invalid(format!("bad image reference '{s}'")));
        }
        let (rest, digest) = match s.split_once('@') {
            Some((r, d)) => (r, Some(d.to_string())),
            None => (s, None),
        };
        // A tag is the part after the last ':' that comes after the last '/'.
        let last_slash = rest.rfind('/').map(|i| i + 1).unwrap_or(0);
        let (name, tag) = match rest[last_slash..].rfind(':') {
            Some(i) => (&rest[..last_slash + i], Some(rest[last_slash + i + 1..].to_string())),
            None => (rest, None),
        };
        // The first component is a registry if it looks like a host.
        let (registry, repository) = match name.split_once('/') {
            Some((first, remainder))
                if first.contains('.') || first.contains(':') || first == "localhost" =>
            {
                (first.to_string(), remainder.to_string())
            }
            _ => (DOCKER_HUB.to_string(), name.to_string()),
        };
        let registry = if registry == "index.docker.io" || registry == DOCKER_HUB_API {
            DOCKER_HUB.to_string()
        } else {
            registry
        };
        let repository = if registry == DOCKER_HUB && !repository.contains('/') {
            format!("library/{repository}")
        } else {
            repository
        };
        if repository.is_empty() || repository.chars().any(|c| c.is_ascii_uppercase()) {
            return Err(crate::Error::Invalid(format!(
                "bad repository name in '{s}' (must be lowercase)"
            )));
        }
        let tag = match (tag, &digest) {
            (None, None) => Some("latest".to_string()),
            (t, _) => t,
        };
        Ok(Self { registry, repository, tag, digest })
    }

    /// Host to talk to for the registry API.
    pub fn api_host(&self) -> &str {
        if self.registry == DOCKER_HUB {
            DOCKER_HUB_API
        } else {
            &self.registry
        }
    }

    /// Tag or digest used in `/v2/<repo>/manifests/<reference>`.
    pub fn manifest_reference(&self) -> &str {
        self.digest
            .as_deref()
            .or(self.tag.as_deref())
            .unwrap_or("latest")
    }

    /// Short human form: drops `docker.io/library/` like Docker does.
    pub fn short(&self) -> String {
        let mut s = if self.registry == DOCKER_HUB {
            self.repository
                .strip_prefix("library/")
                .unwrap_or(&self.repository)
                .to_string()
        } else {
            format!("{}/{}", self.registry, self.repository)
        };
        if let Some(t) = &self.tag {
            s.push(':');
            s.push_str(t);
        }
        if let Some(d) = &self.digest {
            s.push('@');
            s.push_str(d);
        }
        s
    }
}

impl fmt::Display for ImageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.registry, self.repository)?;
        if let Some(t) = &self.tag {
            write!(f, ":{t}")?;
        }
        if let Some(d) = &self.digest {
            write!(f, "@{d}")?;
        }
        Ok(())
    }
}

impl std::str::FromStr for ImageRef {
    type Err = crate::Error;
    fn from_str(s: &str) -> crate::Result<Self> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_hub_official() {
        let r = ImageRef::parse("nginx").unwrap();
        assert_eq!(r.to_string(), "docker.io/library/nginx:latest");
        assert_eq!(r.api_host(), "registry-1.docker.io");
        assert_eq!(r.short(), "nginx:latest");
    }

    #[test]
    fn docker_hub_user_and_tag() {
        let r = ImageRef::parse("bitnami/redis:7.2").unwrap();
        assert_eq!(r.repository, "bitnami/redis");
        assert_eq!(r.tag.as_deref(), Some("7.2"));
    }

    #[test]
    fn ghcr_and_port_registry() {
        let r = ImageRef::parse("ghcr.io/owner/app").unwrap();
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.tag.as_deref(), Some("latest"));
        let r = ImageRef::parse("localhost:5000/foo:dev").unwrap();
        assert_eq!(r.registry, "localhost:5000");
        assert_eq!(r.repository, "foo");
        assert_eq!(r.tag.as_deref(), Some("dev"));
    }

    #[test]
    fn digest() {
        let r = ImageRef::parse("alpine@sha256:abc").unwrap();
        assert_eq!(r.tag, None);
        assert_eq!(r.manifest_reference(), "sha256:abc");
    }

    #[test]
    fn rejects_bad() {
        assert!(ImageRef::parse("").is_err());
        assert!(ImageRef::parse("Nginx").is_err());
    }
}
