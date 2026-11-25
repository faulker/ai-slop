//! Owner-only file writes for anything holding keys or WiFi passwords.

use std::path::Path;

use anyhow::{Context, Result};

/// Writes a secret-bearing file readable only by the owner (0600 on Unix),
/// tightening permissions on an existing file too (the overwrite case).
pub fn write_secret(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        opts.mode(0o600);
        let mut f = opts.open(path).with_context(|| format!("writing {}", path.display()))?;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("restricting permissions on {}", path.display()))?;
        f.write_all(bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let mut f = opts.open(path).with_context(|| format!("writing {}", path.display()))?;
        f.write_all(bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn secrets_are_owner_only_even_when_overwriting() {
        let path = std::env::temp_dir().join(format!("lt-perm-{}.toml", std::process::id()));
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        write_secret(&path, b"new").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let contents = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(mode, 0o600);
        assert_eq!(contents, b"new");
    }
}
