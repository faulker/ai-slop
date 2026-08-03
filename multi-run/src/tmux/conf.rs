//! Generates and locates multi-run's private tmux config: the one file this
//! tool is ever allowed to write (DESIGN.md section 1, section 5). Owns:
//! - the template text, tier-conditional per DESIGN.md section 10
//! - path resolution across `--tmux-conf`, `$XDG_CONFIG_HOME`, `$HOME`
//! - regeneration only when the text we would write differs from the file,
//!   so an unchanged setup never rewrites on startup
//!
//! Pure string building plus the minimal filesystem IO needed to read back
//! and write one file. No `std::process`, no tmux invocation; that is
//! `tmux/mod.rs`'s job. `-f` is only honoured when the tmux server first
//! starts (DESIGN.md section 5), so rewriting this file does not by itself
//! reach an already-running server; the caller re-applies it with
//! `Tmux::source_conf` when `ensure` reports a write.

use std::io;
use std::path::{Path, PathBuf};

/// Bumped whenever the generated template changes in a way worth naming.
/// It is written into the file as `# multi-run-conf-version: N`, so bumping
/// it also guarantees `ensure` sees a difference and rewrites, even if
/// nothing else about the text changed. Reading it back is for humans and
/// bug reports; `ensure` compares the whole text.
pub const CONF_VERSION: u32 = 3;

/// The attached-session status line: job identity on the left, a copy-mode
/// hint in the middle, the key legend on the right. Written as one
/// `status-format[0]` rather than `status-left`/`status-right` so the three
/// zones cannot fight each other for width.
///
/// Escaping runs through two layers. In this Rust literal `\\\\` is the two
/// characters `\\`, which is what tmux needs inside a double-quoted string to
/// produce one literal backslash on screen, so the legend reads `Ctrl+\`.
///
/// Every `#[...]` here separates its attributes with spaces, never commas. A
/// comma inside `#{?condition,then,else}` ends the branch, so a comma-styled
/// `#[fg=#f9e2af,nobold]` in the middle section truncates the hint to
/// `#[fg=#f9e2af` and drops the text after it.
const STATUS_FORMAT: &str = concat!(
    "set -g status-format[0] \"",
    "#[align=left bg=#1e1e2e fg=#cdd6f4] ",
    "#[fg=#89b4fa bold]#{session_name}#[fg=#cdd6f4 nobold] #{=40:@mr_cmd}",
    "#[align=centre]",
    "#{?pane_in_mode,#[fg=#f38ba8 bold]ESC exits scroll mode,}",
    "#[align=right]",
    "#[fg=#f38ba8 bold]Ctrl+\\\\ #[fg=#a6adc8 nobold]detach  ",
    "#[fg=#f38ba8 bold]Ctrl+n #[fg=#a6adc8 nobold]next  ",
    "#[fg=#f38ba8 bold]Ctrl+p #[fg=#a6adc8 nobold]prev ",
    "\"\n",
);

/// No-prefix bindings applied to the private server only. `C-\`, `C-n` and
/// `C-p` mirror the ccsm session browser so muscle memory carries across;
/// `M-d` stays bound for anyone who learned the version-1 config.
const BINDINGS: &str = concat!(
    "bind-key -n 'C-\\' detach-client\n",
    "bind-key -n M-d detach-client\n",
    "bind-key -n C-n switch-client -n\n",
    "bind-key -n C-p switch-client -p\n",
);

/// The parts of the caller's environment that change the generated text.
/// Deliberately plain fields rather than `preflight::Tier`: this module has
/// no dependency on the preflight milestone, so the caller (whichever
/// module owns tier detection) maps its own enum onto `tmux_3_3_plus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfOptions {
    /// `--no-user-conf`: when false, omit both `source-file` lines instead
    /// of sourcing `~/.tmux.conf` and `~/.config/tmux/tmux.conf`.
    pub source_user_conf: bool,
    /// `remain-on-exit-format` does not exist before tmux 3.3 (gotcha A,
    /// tmux-research.md). Set this from the detected version tier; only
    /// `Full` tier (>= 3.3) should pass `true`.
    pub tmux_3_3_plus: bool,
}

impl Default for ConfOptions {
    /// The common case: source the user's config, and assume a modern tmux.
    /// Callers on a `Degraded` tier must explicitly set `tmux_3_3_plus =
    /// false`.
    fn default() -> Self {
        ConfOptions {
            source_user_conf: true,
            tmux_3_3_plus: true,
        }
    }
}

/// Build the generated config text for the given options. Matches
/// DESIGN.md section 5's template exactly, with the header regeneration
/// marker, the user-conf `source-file` lines gated by
/// `source_user_conf`, and `remain-on-exit-format` gated by
/// `tmux_3_3_plus` (gotcha A). The remaining `set -g` lines are
/// unconditional:
/// - `exit-empty off` (gotcha J): a private server with zero sessions must
///   not exit before the first `new-session` lands.
/// - `default-shell /bin/sh` (gotcha B): predictable command syntax
///   regardless of the user's login shell.
pub fn generate(opts: ConfOptions) -> String {
    let mut out = String::new();
    out.push_str(
        "# multi-run generated config. Do not edit; regenerated when the version below changes.\n",
    );
    out.push_str(&format!("# multi-run-conf-version: {CONF_VERSION}\n"));
    out.push('\n');

    if opts.source_user_conf {
        out.push_str("source-file -q ~/.tmux.conf\n");
        out.push_str("source-file -q ~/.config/tmux/tmux.conf\n");
        out.push('\n');
    }

    out.push_str("set -g remain-on-exit on\n");
    if opts.tmux_3_3_plus {
        out.push_str("set -g remain-on-exit-format \"\"\n");
    }
    out.push_str("set -g exit-empty off\n");
    out.push_str("set -g detach-on-destroy on\n");
    out.push_str("set -g history-limit 50000\n");
    out.push_str("set -g default-shell /bin/sh\n");
    out.push_str("set -g escape-time 10\n");
    // Every job's own pane is window 0, pane 0, and `tmux/mod.rs` targets it
    // by that address so an extra window or split opened while attached
    // cannot be mistaken for the job. Forcing both bases here is what makes
    // that address true even when the sourced user config sets them to 1.
    out.push_str("set -g base-index 0\n");
    out.push_str("set -g pane-base-index 0\n");
    out.push('\n');

    // Only an attached client ever renders this, so it costs the polled
    // tiles nothing: the status line belongs to the client, not the window.
    out.push_str("set -g status on\n");
    out.push_str("set -g status-position bottom\n");
    out.push_str("set -g status-interval 5\n");
    out.push_str("set -g status-style \"bg=#1e1e2e,fg=#cdd6f4\"\n");
    out.push_str(STATUS_FORMAT);
    out.push('\n');

    out.push_str(BINDINGS);

    out
}

/// Resolve the config file path, given already-read environment values so
/// this stays pure and testable without touching real process env vars.
/// Priority, per DESIGN.md section 5:
/// 1. `override_path` (`--tmux-conf <PATH>`)
/// 2. `$XDG_CONFIG_HOME/multi-run/tmux.conf`
/// 3. `$HOME/.config/multi-run/tmux.conf`
///
/// Returns `None` only when there is no override and neither environment
/// variable is set (or is set to an empty string, which most shells treat
/// as unset for this purpose).
pub fn resolve_path_from(
    override_path: Option<&Path>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(p) = override_path {
        return Some(p.to_path_buf());
    }
    if let Some(xdg) = xdg_config_home.filter(|s| !s.is_empty()) {
        return Some(Path::new(xdg).join("multi-run").join("tmux.conf"));
    }
    if let Some(home) = home.filter(|s| !s.is_empty()) {
        return Some(
            Path::new(home)
                .join(".config")
                .join("multi-run")
                .join("tmux.conf"),
        );
    }
    None
}

/// Convenience wrapper over `resolve_path_from` that reads the real process
/// environment. Kept separate so unit tests exercise `resolve_path_from`
/// directly instead of mutating global env state.
pub fn resolve_path(override_path: Option<&Path>) -> Option<PathBuf> {
    resolve_path_from(
        override_path,
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Make sure the config file at `path` holds exactly what `generate(opts)`
/// produces, writing it if it is missing, stale by `CONF_VERSION`, or simply
/// different. Creates parent directories as needed. Returns `true` if the
/// file was written, `false` if it already matched.
///
/// The version header alone is not enough to decide this: parts of the text
/// come from the caller's flags rather than the template, so `--no-user-conf`
/// on a run after a normal one produces the same header over different
/// content. Comparing the text catches that, and still leaves the common case
/// (nothing changed) as a read with no write.
pub fn ensure(path: &Path, opts: ConfOptions) -> io::Result<bool> {
    let wanted = generate(opts);
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == wanted) {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, wanted)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- generated content -------------------------------------------

    #[test]
    fn generates_expected_content_full_tier_with_user_conf() {
        let text = generate(ConfOptions {
            source_user_conf: true,
            tmux_3_3_plus: true,
        });
        // The two long blocks are pinned by their own tests below; folding
        // them to placeholders keeps this one about line order and content.
        let skeleton = text
            .replace(STATUS_FORMAT, "<status-format>\n")
            .replace(BINDINGS, "<bindings>\n");
        assert_eq!(
            skeleton,
            "\
# multi-run generated config. Do not edit; regenerated when the version below changes.
# multi-run-conf-version: 3

source-file -q ~/.tmux.conf
source-file -q ~/.config/tmux/tmux.conf

set -g remain-on-exit on
set -g remain-on-exit-format \"\"
set -g exit-empty off
set -g detach-on-destroy on
set -g history-limit 50000
set -g default-shell /bin/sh
set -g escape-time 10
set -g base-index 0
set -g pane-base-index 0

set -g status on
set -g status-position bottom
set -g status-interval 5
set -g status-style \"bg=#1e1e2e,fg=#cdd6f4\"
<status-format>

<bindings>
"
        );
    }

    #[test]
    fn detach_binding_is_a_single_literal_backslash() {
        // The whole point of the key: tmux must see `C-\`, not `C-\\`.
        let text = generate(ConfOptions::default());
        assert!(text.contains("bind-key -n 'C-\\' detach-client\n"));
        // M-d stays bound so the version-1 muscle memory still works.
        assert!(text.contains("bind-key -n M-d detach-client\n"));
    }

    #[test]
    fn binds_session_switching() {
        let text = generate(ConfOptions::default());
        assert!(text.contains("bind-key -n C-n switch-client -n\n"));
        assert!(text.contains("bind-key -n C-p switch-client -p\n"));
        // Nothing on this server opens windows for the user; a job session
        // is one pane and stays that way unless they do it themselves.
        assert!(!text.contains("new-window"));
    }

    #[test]
    fn status_bar_is_on_and_names_every_bound_key() {
        let text = generate(ConfOptions::default());
        assert!(text.contains("set -g status on\n"));
        assert!(text.contains("set -g status-position bottom\n"));
        assert!(!text.contains("set -g status off"));
        // Inside the double-quoted `status-format` value, the two characters
        // `\\` are what render as one backslash, so the bar reads `Ctrl+\`.
        for legend in ["Ctrl+\\\\ ", "detach", "Ctrl+n ", "next", "Ctrl+p ", "prev"] {
            assert!(text.contains(legend), "status line is missing {legend:?}");
        }
        // The legend lists exactly what is bound, nothing aspirational.
        assert!(!text.contains("Ctrl+t"));
    }

    #[test]
    fn pins_the_job_pane_address_regardless_of_user_config() {
        // `tmux/mod.rs` addresses every job as `=<session>:0.0`; a sourced
        // user config setting either base to 1 would break that.
        let text = generate(ConfOptions::default());
        assert!(text.contains("set -g base-index 0\n"));
        assert!(text.contains("set -g pane-base-index 0\n"));
        let bases = text.find("set -g base-index").unwrap();
        let source = text.find("source-file").unwrap();
        assert!(
            bases > source,
            "our bases must be set after the user's conf"
        );
    }

    #[test]
    fn omits_remain_on_exit_format_below_3_3_gotcha_a() {
        let text = generate(ConfOptions {
            tmux_3_3_plus: false,
            ..ConfOptions::default()
        });
        assert!(!text.contains("remain-on-exit-format"));
        // Everything else in the degraded-tier config is unaffected.
        assert!(text.contains("set -g remain-on-exit on\n"));
        assert!(text.contains("set -g default-shell /bin/sh\n"));
    }

    #[test]
    fn omits_source_file_lines_with_no_user_conf() {
        let text = generate(ConfOptions {
            source_user_conf: false,
            ..ConfOptions::default()
        });
        assert!(!text.contains("source-file"));
        assert!(!text.contains("~/.tmux.conf"));
    }

    #[test]
    fn always_includes_default_shell_and_exit_empty_off() {
        // gotcha B and gotcha J: not tier-conditional, must always be present.
        for source_user_conf in [true, false] {
            for tmux_3_3_plus in [true, false] {
                let text = generate(ConfOptions {
                    source_user_conf,
                    tmux_3_3_plus,
                });
                assert!(text.contains("set -g default-shell /bin/sh\n"));
                assert!(text.contains("set -g exit-empty off\n"));
            }
        }
    }

    #[test]
    fn header_carries_the_compiled_conf_version() {
        let text = generate(ConfOptions::default());
        assert!(text.contains(&format!("# multi-run-conf-version: {CONF_VERSION}\n")));
    }

    // ---- version header / regeneration --------------------------------

    // ---- path resolution -----------------------------------------------

    #[test]
    fn override_path_wins_over_everything() {
        let over = Path::new("/custom/tmux.conf");
        let resolved = resolve_path_from(Some(over), Some("/xdg"), Some("/home/u"));
        assert_eq!(resolved, Some(PathBuf::from("/custom/tmux.conf")));
    }

    #[test]
    fn xdg_config_home_used_when_no_override() {
        let resolved = resolve_path_from(None, Some("/xdg"), Some("/home/u"));
        assert_eq!(resolved, Some(PathBuf::from("/xdg/multi-run/tmux.conf")));
    }

    #[test]
    fn falls_back_to_home_when_xdg_unset() {
        let resolved = resolve_path_from(None, None, Some("/home/u"));
        assert_eq!(
            resolved,
            Some(PathBuf::from("/home/u/.config/multi-run/tmux.conf"))
        );
    }

    #[test]
    fn empty_xdg_is_treated_as_unset() {
        let resolved = resolve_path_from(None, Some(""), Some("/home/u"));
        assert_eq!(
            resolved,
            Some(PathBuf::from("/home/u/.config/multi-run/tmux.conf"))
        );
    }

    #[test]
    fn none_when_nothing_is_available() {
        let resolved = resolve_path_from(None, None, None);
        assert_eq!(resolved, None);
    }

    #[test]
    fn none_when_env_vars_are_empty_strings() {
        let resolved = resolve_path_from(None, Some(""), Some(""));
        assert_eq!(resolved, None);
    }

    // ---- ensure() filesystem behaviour ---------------------------------

    /// A directory unique to this test process/run, under the OS temp dir,
    /// so parallel `cargo test` runs never collide with each other or with
    /// a real `$HOME`.
    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "multi-run-conf-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ensure_creates_missing_file_and_parent_dir() {
        let dir = scratch_dir("create");
        let path = dir.join("nested").join("tmux.conf");
        assert!(!path.exists());

        let wrote = ensure(&path, ConfOptions::default()).unwrap();
        assert!(wrote);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, generate(ConfOptions::default()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_leaves_a_matching_file_untouched() {
        let dir = scratch_dir("current");
        let path = dir.join("tmux.conf");
        assert!(ensure(&path, ConfOptions::default()).unwrap());

        // The steady state: same options, same text, no write. This is what
        // keeps startup from touching the file on every run.
        let wrote = ensure(&path, ConfOptions::default()).unwrap();
        assert!(!wrote);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_restores_a_file_that_drifted_from_the_template() {
        let dir = scratch_dir("drift");
        let path = dir.join("tmux.conf");
        ensure(&path, ConfOptions::default()).unwrap();

        // Hand edits and half-written files go back to the real template:
        // the header alone cannot tell us the body is still right.
        let mut contents = std::fs::read_to_string(&path).unwrap();
        contents.push_str("# sentinel line appended by the test\n");
        std::fs::write(&path, &contents).unwrap();

        assert!(ensure(&path, ConfOptions::default()).unwrap());
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("sentinel line appended by the test"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_rewrites_when_only_the_options_changed() {
        // The version header is identical in both, so a version-only check
        // would leave a `--no-user-conf` run still sourcing the user's config.
        let dir = scratch_dir("optchange");
        let path = dir.join("tmux.conf");
        ensure(&path, ConfOptions::default()).unwrap();

        let wrote = ensure(
            &path,
            ConfOptions {
                source_user_conf: false,
                ..ConfOptions::default()
            },
        )
        .unwrap();
        assert!(wrote);
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("source-file")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_rewrites_stale_file() {
        let dir = scratch_dir("stale");
        let path = dir.join("tmux.conf");
        std::fs::write(&path, "# multi-run-conf-version: 0\nstale content\n").unwrap();

        let wrote = ensure(&path, ConfOptions::default()).unwrap();
        assert!(wrote);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("stale content"));
        assert_eq!(contents, generate(ConfOptions::default()));

        std::fs::remove_dir_all(&dir).ok();
    }
}
