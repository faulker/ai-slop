//! tmux presence detection, `tmux -V` version parsing, capability-tier
//! mapping, and the per-platform "install tmux" hint. This module has no
//! knowledge of the tmux config file (`tmux/conf.rs`) or the server probe
//! (`tmux/mod.rs`); it only answers "is tmux here, which version, what can
//! we do with it". See DESIGN.md section 10.

use crate::error::TmuxError;
use std::process::Command;

/// A parsed `tmux -V` version. tmux versions are not semver (research
/// gotcha G): a release is `major.minor`, optionally followed by a single
/// lowercase patch letter (`3.7b`), and separately may carry a `next-`
/// development prefix or an `-rcN` release-candidate suffix.
///
/// Equality and ordering are on `(major, minor, letter)` only, with
/// `letter: None` sorting before any `Some(_)`. `next-X.Y` and `X.Y-rcN`
/// both collapse to the same key as bare `X.Y`, because for tier-gating
/// purposes they carry X.Y's feature set. `raw` keeps the exact text that
/// was parsed, for display.
#[derive(Clone, Debug)]
pub struct TmuxVersion {
    pub major: u32,
    pub minor: u32,
    pub letter: Option<char>,
    pub raw: String,
}

impl TmuxVersion {
    fn key(&self) -> (u32, u32, Option<char>) {
        (self.major, self.minor, self.letter)
    }
}

impl PartialEq for TmuxVersion {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for TmuxVersion {}

impl PartialOrd for TmuxVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TmuxVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

impl std::fmt::Display for TmuxVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Consume a leading run of ASCII digits from `s`, returning the parsed
/// value and the remainder. `None` if there is no leading digit at all
/// (including an empty string), which is how callers detect a missing
/// major/minor component.
fn take_digits(s: &str) -> Option<(u32, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let value: u32 = s[..end].parse().ok()?;
    Some((value, &s[end..]))
}

/// Parse a `tmux -V` version string using the hand-rolled grammar from
/// research item 8 (gotcha G), equivalent to the regex
/// `^(?:tmux\s+)?(next-)?(\d+)\.(\d+)([a-z]?)(-rc\d*)?$`.
///
/// Must parse: `tmux 3.7b`, `3.7`, `3.0a`, `2.9`, `next-3.8`, `3.4-rc`,
/// `3.4-rc1`. Must reject: `""`, `tmux`, `3`, `x.y`. Never feed this a
/// package-manager version string (e.g. Arch's `3.7_b`); only `tmux -V`
/// output has this shape.
pub fn parse_version(input: &str) -> Result<TmuxVersion, TmuxError> {
    let trimmed = input.trim();
    let fail = || TmuxError::Parse(format!("not a valid tmux version string: {input:?}"));

    if trimmed.is_empty() {
        return Err(fail());
    }

    // Optional "tmux " prefix: literal "tmux" followed by whitespace. If
    // "tmux" appears without trailing whitespace (or is the whole string),
    // it is left alone and will fail to parse as a version below.
    let mut rest = trimmed;
    if let Some(after) = rest.strip_prefix("tmux") {
        if after.starts_with(|c: char| c.is_whitespace()) {
            rest = after.trim_start();
        }
    }
    // Everything from here on is the part we display back to the user.
    let version_part = rest;

    // Optional "next-" development prefix.
    rest = rest.strip_prefix("next-").unwrap_or(rest);

    let (major, after) = take_digits(rest).ok_or_else(fail)?;
    rest = after;

    rest = rest.strip_prefix('.').ok_or_else(fail)?;

    let (minor, after) = take_digits(rest).ok_or_else(fail)?;
    rest = after;

    // Optional single lowercase patch letter, e.g. the "b" in "3.7b".
    let mut letter = None;
    let mut chars = rest.chars();
    if let Some(c) = chars.next() {
        if c.is_ascii_lowercase() {
            letter = Some(c);
            rest = chars.as_str();
        }
    }

    // Optional "-rc" or "-rcN" release-candidate suffix. Its digits (if
    // any) are not retained; a release candidate compares equal to the
    // release it precedes.
    if let Some(after_rc) = rest.strip_prefix("-rc") {
        rest = take_digits(after_rc).map(|(_, r)| r).unwrap_or(after_rc);
    }

    if !rest.is_empty() {
        return Err(fail());
    }

    Ok(TmuxVersion {
        major,
        minor,
        letter,
        raw: version_part.to_string(),
    })
}

/// Run `tmux -V` and return the raw version string from stdout. An
/// `io::ErrorKind::NotFound` on spawn means tmux is not installed; we never
/// shell out to `which` to check first (research item 8).
pub fn tmux_version_string() -> Result<String, TmuxError> {
    match Command::new("tmux").arg("-V").output() {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        Ok(out) => Err(crate::error::classify("tmux -V", &["-V".to_string()], &out)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(TmuxError::NotInstalled),
        Err(e) => Err(TmuxError::Io(e)),
    }
}

/// tmux capability tier, driven entirely by version (DESIGN.md section 10
/// step 4). Determines whether we hard-fail, degrade a couple of behaviours,
/// or run the full design.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Below 2.9: no `resize-window` / `window-size` option, so the whole
    /// tile-sizing model is impossible. Preflight hard-fails on this tier.
    Unsupported,
    /// 2.9 up to and including 3.2a: everything works except banner
    /// suppression (`remain-on-exit-format`, added in 3.3; gotcha A), which
    /// is compensated for by capturing dead panes with `-S -1`.
    Degraded,
    /// 3.3 and above: the full design, no compromises.
    Full,
}

impl std::fmt::Display for Tier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Tier::Unsupported => "unsupported",
            Tier::Degraded => "degraded",
            Tier::Full => "full",
        };
        f.write_str(s)
    }
}

/// Map a parsed version to its tier. An unparseable version string is not
/// this function's concern: callers that fail `parse_version` should assume
/// `Tier::Full` and warn, per DESIGN.md section 10 step 4, so a future tmux
/// with an unrecognised version shape does not brick the tool.
pub fn tier_for(version: &TmuxVersion) -> Tier {
    const MIN_SUPPORTED: (u32, u32, Option<char>) = (2, 9, None);
    const MIN_FULL: (u32, u32, Option<char>) = (3, 3, None);
    let key = (version.major, version.minor, version.letter);
    if key < MIN_SUPPORTED {
        Tier::Unsupported
    } else if key < MIN_FULL {
        Tier::Degraded
    } else {
        Tier::Full
    }
}

/// Map an `/etc/os-release` `ID` (falling back to `ID_LIKE`) to an install
/// command. Pure and file-IO-free so it is unit-testable; the wrapper that
/// actually reads the file is `install_command`. An unrecognised id falls
/// back to a message naming both fields so the user can search themselves.
pub fn install_hint_for_id(id: &str, id_like: &str) -> String {
    let id = id.trim().trim_matches('"').to_lowercase();
    let id_like = id_like.trim().trim_matches('"').to_lowercase();
    let candidates = std::iter::once(id.as_str()).chain(id_like.split_whitespace());

    for candidate in candidates {
        match candidate {
            "debian" | "ubuntu" => return "sudo apt install tmux".to_string(),
            "fedora" | "rhel" => return "sudo dnf install tmux".to_string(),
            "arch" => return "sudo pacman -S tmux".to_string(),
            "alpine" => return "sudo apk add tmux".to_string(),
            _ => {}
        }
    }

    format!(
        "install tmux with your distribution's package manager (unrecognised id={id:?} id_like={id_like:?})"
    )
}

/// Read `ID` and `ID_LIKE` out of `/etc/os-release`. A missing file or
/// missing fields resolve to empty strings, which `install_hint_for_id`
/// treats as unknown.
fn read_os_release() -> (String, String) {
    let content = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut id = String::new();
    let mut id_like = String::new();
    for line in content.lines() {
        if let Some(v) = line.strip_prefix("ID=") {
            id = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("ID_LIKE=") {
            id_like = v.trim_matches('"').to_string();
        }
    }
    (id, id_like)
}

/// The install command for the current platform: Homebrew on macOS,
/// `/etc/os-release`-driven on everything else (DESIGN.md section 10 step 2).
pub fn install_command() -> String {
    if cfg!(target_os = "macos") {
        return "brew install tmux".to_string();
    }
    let (id, id_like) = read_os_release();
    install_hint_for_id(&id, &id_like)
}

/// The full "tmux is not installed" message printed at preflight step 2,
/// including the copy-pasteable install command. We print this; we never
/// run it ourselves (DESIGN.md section 10: most platforms need `sudo`, and
/// prompting for a root password from inside a TUI is not something we do).
pub fn not_installed_message() -> String {
    format!(
        "multi-run needs tmux, and it is not installed.\n\n  Install it with:\n\n      {}\n\n  Then run multi-run again.\n",
        install_command()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u32, minor: u32, letter: Option<char>) -> TmuxVersion {
        TmuxVersion {
            major,
            minor,
            letter,
            raw: String::new(),
        }
    }

    #[test]
    fn parses_plain_release() {
        let p = parse_version("3.7").unwrap();
        assert_eq!((p.major, p.minor, p.letter), (3, 7, None));
        assert_eq!(p.raw, "3.7");
    }

    #[test]
    fn parses_with_tmux_prefix() {
        let p = parse_version("tmux 3.7b").unwrap();
        assert_eq!((p.major, p.minor, p.letter), (3, 7, Some('b')));
        assert_eq!(p.raw, "3.7b");
    }

    #[test]
    fn parses_patch_letter() {
        let p = parse_version("3.0a").unwrap();
        assert_eq!((p.major, p.minor, p.letter), (3, 0, Some('a')));
    }

    #[test]
    fn parses_two_component_release() {
        let p = parse_version("2.9").unwrap();
        assert_eq!((p.major, p.minor, p.letter), (2, 9, None));
    }

    #[test]
    fn parses_next_prefix() {
        let p = parse_version("next-3.8").unwrap();
        assert_eq!((p.major, p.minor, p.letter), (3, 8, None));
        assert_eq!(p.raw, "next-3.8");
    }

    #[test]
    fn parses_bare_rc_suffix() {
        let p = parse_version("3.4-rc").unwrap();
        assert_eq!((p.major, p.minor, p.letter), (3, 4, None));
    }

    #[test]
    fn parses_numbered_rc_suffix() {
        let p = parse_version("3.4-rc1").unwrap();
        assert_eq!((p.major, p.minor, p.letter), (3, 4, None));
    }

    #[test]
    fn parses_two_component_release_with_trailing_whitespace() {
        // tmux -V output includes a trailing newline.
        let p = parse_version("tmux 3.7b\n").unwrap();
        assert_eq!((p.major, p.minor, p.letter), (3, 7, Some('b')));
    }

    #[test]
    fn rejects_empty_string() {
        assert!(parse_version("").is_err());
    }

    #[test]
    fn rejects_bare_tmux() {
        assert!(parse_version("tmux").is_err());
    }

    #[test]
    fn rejects_major_only() {
        assert!(parse_version("3").is_err());
    }

    #[test]
    fn rejects_non_numeric_components() {
        assert!(parse_version("x.y").is_err());
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse_version("3.7bc").is_err());
        assert!(parse_version("3.7b garbage").is_err());
    }

    #[test]
    fn rejects_package_manager_underscore_form() {
        // Arch renders the package version as "3.7_b"; only `tmux -V`
        // output ("3.7b") is ever fed to this parser.
        assert!(parse_version("3.7_b").is_err());
    }

    #[test]
    fn ordering_across_patch_letters_and_minors() {
        let a = parse_version("3.7").unwrap();
        let b = parse_version("3.7a").unwrap();
        let c = parse_version("3.7b").unwrap();
        let d = parse_version("3.8").unwrap();
        assert!(a < b);
        assert!(b < c);
        assert!(c < d);
    }

    #[test]
    fn ordering_across_majors() {
        let a = parse_version("2.9").unwrap();
        let b = parse_version("3.0").unwrap();
        assert!(a < b);
    }

    #[test]
    fn next_and_rc_compare_equal_to_the_release_they_precede() {
        let released = parse_version("3.8").unwrap();
        let dev = parse_version("next-3.8").unwrap();
        let rc = parse_version("3.4-rc1").unwrap();
        let release = parse_version("3.4").unwrap();
        assert_eq!(released, dev);
        assert_eq!(rc, release);
    }

    #[test]
    fn tier_boundaries() {
        assert_eq!(tier_for(&v(2, 8, None)), Tier::Unsupported);
        assert_eq!(tier_for(&v(2, 9, None)), Tier::Degraded);
        assert_eq!(tier_for(&v(3, 2, Some('a'))), Tier::Degraded);
        assert_eq!(tier_for(&v(3, 3, None)), Tier::Full);
        assert_eq!(tier_for(&v(3, 7, Some('b'))), Tier::Full);
    }

    #[test]
    fn tier_display_is_lowercase() {
        assert_eq!(Tier::Unsupported.to_string(), "unsupported");
        assert_eq!(Tier::Degraded.to_string(), "degraded");
        assert_eq!(Tier::Full.to_string(), "full");
    }

    #[test]
    fn install_hint_known_ids() {
        assert_eq!(install_hint_for_id("debian", ""), "sudo apt install tmux");
        assert_eq!(install_hint_for_id("ubuntu", ""), "sudo apt install tmux");
        assert_eq!(install_hint_for_id("fedora", ""), "sudo dnf install tmux");
        assert_eq!(install_hint_for_id("rhel", ""), "sudo dnf install tmux");
        assert_eq!(install_hint_for_id("arch", ""), "sudo pacman -S tmux");
        assert_eq!(install_hint_for_id("alpine", ""), "sudo apk add tmux");
    }

    #[test]
    fn install_hint_falls_back_to_id_like() {
        // e.g. Linux Mint: ID=linuxmint, ID_LIKE="ubuntu debian"
        assert_eq!(
            install_hint_for_id("linuxmint", "ubuntu debian"),
            "sudo apt install tmux"
        );
    }

    #[test]
    fn install_hint_unknown_id_names_both_fields() {
        let hint = install_hint_for_id("gentoo", "");
        assert!(hint.contains("gentoo"));
        assert!(!hint.contains("apt"));
        assert!(!hint.contains("dnf"));
    }

    #[test]
    fn install_hint_is_case_insensitive() {
        assert_eq!(install_hint_for_id("Debian", ""), "sudo apt install tmux");
    }

    #[test]
    fn not_installed_message_contains_a_command() {
        let msg = not_installed_message();
        assert!(msg.contains("multi-run needs tmux"));
        assert!(msg.contains("Install it with"));
    }
}
