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
pub const CONF_VERSION: u32 = 5;

/// File name of the companion wrapper written next to `tmux.conf`. tmux's
/// `default-shell` points here so every job pane loads the user's profile.
pub const USER_SHELL_WRAPPER_NAME: &str = "user-shell";

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
///
/// The two `Escape` bindings back the status bar's "ESC exits scroll mode"
/// hint. tmux only cancels copy mode on `Escape` under `mode-keys emacs`; a
/// user config with `mode-keys vi` (or a vi-ish `$EDITOR`, which tmux infers
/// from) binds it to `clear-selection` instead, which leaves the pane stuck in
/// copy mode after a wheel scroll. Binding both tables makes the hint true
/// either way.
const BINDINGS: &str = concat!(
    "bind-key -n 'C-\\' detach-client\n",
    "bind-key -n M-d detach-client\n",
    "bind-key -n C-n switch-client -n\n",
    "bind-key -n C-p switch-client -p\n",
    "bind-key -T copy-mode Escape send-keys -X cancel\n",
    "bind-key -T copy-mode-vi Escape send-keys -X cancel\n",
);

/// The parts of the caller's environment that change the generated text.
/// Deliberately plain fields rather than `preflight::Tier`: this module has
/// no dependency on the preflight milestone, so the caller (whichever
/// module owns tier detection) maps its own enum onto `tmux_3_3_plus`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfOptions {
    /// `--no-user-conf`: when false, omit both `source-file` lines instead
    /// of sourcing `~/.tmux.conf` and `~/.config/tmux/tmux.conf`.
    pub source_user_conf: bool,
    /// `remain-on-exit-format` does not exist before tmux 3.3 (gotcha A,
    /// tmux-research.md). Set this from the detected version tier; only
    /// `Full` tier (>= 3.3) should pass `true`.
    pub tmux_3_3_plus: bool,
    /// Absolute path written as `set -g default-shell`. Production
    /// `ensure` points this at the companion `user-shell` wrapper so job
    /// panes load the user's profile; unit tests may pass `/bin/sh` for a
    /// plain POSIX shell with no profile load.
    pub default_shell: PathBuf,
}

impl Default for ConfOptions {
    /// The common case for unit tests: source the user's config, assume a
    /// modern tmux, and use `/bin/sh` (no profile wrapper). Production
    /// callers go through `ensure`, which overwrites `default_shell` with
    /// the wrapper path. Callers on a `Degraded` tier must explicitly set
    /// `tmux_3_3_plus = false`.
    fn default() -> Self {
        ConfOptions {
            source_user_conf: true,
            tmux_3_3_plus: true,
            default_shell: PathBuf::from("/bin/sh"),
        }
    }
}

/// Resolve the shell the user's terminal would use: `$SHELL` when it is an
/// absolute path to an existing file, otherwise `/bin/sh`.
pub fn resolve_user_shell() -> PathBuf {
    resolve_user_shell_from(std::env::var_os("SHELL").map(PathBuf::from))
}

/// Pure form of [`resolve_user_shell`] for tests that inject `$SHELL`.
pub fn resolve_user_shell_from(shell: Option<PathBuf>) -> PathBuf {
    if let Some(path) = shell
        && path.is_absolute()
        && path.is_file()
    {
        return path;
    }
    PathBuf::from("/bin/sh")
}

/// Path of the profile-loading wrapper that sits next to the generated
/// tmux.conf (`…/multi-run/user-shell`).
pub fn wrapper_path(conf_path: &Path) -> PathBuf {
    conf_path
        .parent()
        .map(|p| p.join(USER_SHELL_WRAPPER_NAME))
        .unwrap_or_else(|| PathBuf::from(USER_SHELL_WRAPPER_NAME))
}

/// Flags passed to `user_shell` so a non-interactive `shell -c 'cmd'` still
/// loads the same profile files a normal terminal session would.
///
/// tmux always invokes `default-shell -c '<command>'`. Plain `-c` skips
/// interactive rc files (e.g. zsh skips `.zshrc`), so the wrapper re-execs
/// with login/interactive flags. The real shell path is baked in: tmux sets
/// `$SHELL` to `default-shell`, so a wrapper that trusted `$SHELL` would
/// recurse into itself.
fn profile_flags_for(user_shell: &Path) -> &'static str {
    match user_shell.file_name().and_then(|n| n.to_str()) {
        Some("zsh") | Some("bash") | Some("ksh") | Some("ksh93") | Some("mksh") => "-il",
        // fish has no combined `-il` that matches bash/zsh; `-l` loads the
        // login config, which is where env setup usually lives.
        Some("fish") => "-l",
        _ => "",
    }
}

/// Quote a path for use inside a single-quoted shell string.
fn sh_single_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// Build the `user-shell` wrapper script that tmux will run as
/// `default-shell`. Bakes `user_shell` in by absolute path so tmux's habit
/// of exporting `SHELL=default-shell` cannot cause recursion.
pub fn generate_wrapper(user_shell: &Path) -> String {
    let quoted = sh_single_quote(user_shell);
    let flags = profile_flags_for(user_shell);
    let mut out = String::new();
    out.push_str("#!/bin/sh\n");
    out.push_str(&format!(
        "# multi-run user-shell wrapper (conf-version {CONF_VERSION}). Do not edit.\n"
    ));
    out.push_str(
        "# tmux invokes this as: user-shell -c '<command>'. Re-exec the user's\n",
    );
    out.push_str(
        "# real shell with profile-loading flags so .zshrc / .bashrc / etc. apply.\n",
    );
    if flags.is_empty() {
        out.push_str(&format!("exec {quoted} \"$@\"\n"));
    } else {
        out.push_str(&format!("exec {quoted} {flags} \"$@\"\n"));
    }
    out
}

/// Build the generated config text for the given options. Matches
/// DESIGN.md section 5's template, with the header regeneration marker,
/// the user-conf `source-file` lines gated by `source_user_conf`, and
/// `remain-on-exit-format` gated by `tmux_3_3_plus` (gotcha A). The
/// remaining `set -g` lines are unconditional:
/// - `exit-empty off` (gotcha J): a private server with zero sessions must
///   not exit before the first `new-session` lands.
/// - `default-shell <path>`: the profile-loading wrapper in production, so
///   each job pane sees the same env as the user's terminal.
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
    // Without this the wheel does nothing in an attached session: the TUI's
    // own mouse capture is gone by then (attach.rs tears it down before
    // handing over the terminal), so it is tmux, not us, that has to turn a
    // wheel notch into a scroll. `mouse on` gives the default root-table
    // WheelUpPane binding something to fire, which drops the pane into copy
    // mode and walks back through `history-limit` lines. Set after the
    // `source-file` lines so a user config with `set -g mouse off` cannot
    // leave the attached view unscrollable.
    out.push_str("set -g mouse on\n");
    out.push_str(&format!(
        "set -g default-shell {}\n",
        opts.default_shell.display()
    ));
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

/// Make sure the config file at `path` and its companion `user-shell`
/// wrapper hold what production needs: the wrapper execs `user_shell` with
/// profile-loading flags, and the conf's `default-shell` points at that
/// wrapper. Creates parent directories as needed. Returns `true` if either
/// file was written, `false` if both already matched.
///
/// `opts.default_shell` from the caller is ignored; `ensure` always
/// overwrites it with the wrapper path so the two files stay paired. The
/// version header alone is not enough to decide conf freshness: parts of
/// the text come from the caller's flags, so `--no-user-conf` on a run
/// after a normal one produces the same header over different content.
/// Comparing the text catches that, and still leaves the common case
/// (nothing changed) as a read with no write.
pub fn ensure(path: &Path, opts: ConfOptions) -> io::Result<bool> {
    ensure_with_user_shell(path, opts, &resolve_user_shell())
}

/// Like [`ensure`], but takes an explicit user shell so tests can inject a
/// fake `$SHELL` without mutating process environment.
pub fn ensure_with_user_shell(
    path: &Path,
    opts: ConfOptions,
    user_shell: &Path,
) -> io::Result<bool> {
    let wrapper = wrapper_path(path);
    let wrapper_text = generate_wrapper(user_shell);
    let mut wrote = false;

    if !std::fs::read_to_string(&wrapper).is_ok_and(|existing| existing == wrapper_text) {
        if let Some(parent) = wrapper.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&wrapper, &wrapper_text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&wrapper)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&wrapper, perms)?;
        }
        wrote = true;
    }

    let conf_opts = ConfOptions {
        default_shell: wrapper,
        ..opts
    };
    let wanted = generate(conf_opts);
    if std::fs::read_to_string(path).is_ok_and(|existing| existing == wanted) {
        return Ok(wrote);
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
            default_shell: PathBuf::from("/bin/sh"),
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
# multi-run-conf-version: 5

source-file -q ~/.tmux.conf
source-file -q ~/.config/tmux/tmux.conf

set -g remain-on-exit on
set -g remain-on-exit-format \"\"
set -g exit-empty off
set -g detach-on-destroy on
set -g history-limit 50000
set -g mouse on
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
    fn mouse_is_on_so_the_wheel_scrolls_an_attached_session() {
        // The TUI's own mouse capture is off while attached, so scrollback in
        // the single-session view is entirely tmux's job.
        for source_user_conf in [true, false] {
            for tmux_3_3_plus in [true, false] {
                let text = generate(ConfOptions {
                    source_user_conf,
                    tmux_3_3_plus,
                    default_shell: PathBuf::from("/bin/sh"),
                });
                assert!(text.contains("set -g mouse on\n"));
                assert!(!text.contains("set -g mouse off"));
                // Scrolling back is pointless without history to scroll into.
                assert!(text.contains("set -g history-limit 50000\n"));
            }
        }
    }

    #[test]
    fn mouse_is_set_after_the_user_conf_is_sourced() {
        // A user config with `set -g mouse off` would otherwise win and leave
        // the attached view unscrollable.
        let text = generate(ConfOptions::default());
        let mouse = text.find("set -g mouse on").unwrap();
        let source = text.find("source-file").unwrap();
        assert!(mouse > source);
    }

    #[test]
    fn escape_cancels_copy_mode_in_both_key_tables() {
        // The status bar promises "ESC exits scroll mode"; under
        // `mode-keys vi` the stock binding is `clear-selection`, which does
        // not exit, so both tables are bound explicitly.
        let text = generate(ConfOptions::default());
        assert!(text.contains("bind-key -T copy-mode Escape send-keys -X cancel\n"));
        assert!(text.contains("bind-key -T copy-mode-vi Escape send-keys -X cancel\n"));
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
        // default-shell and gotcha J: not tier-conditional, must always be present.
        for source_user_conf in [true, false] {
            for tmux_3_3_plus in [true, false] {
                let text = generate(ConfOptions {
                    source_user_conf,
                    tmux_3_3_plus,
                    default_shell: PathBuf::from("/bin/sh"),
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

    #[test]
    fn generate_writes_whatever_default_shell_path_it_is_given() {
        let text = generate(ConfOptions {
            default_shell: PathBuf::from("/tmp/multi-run/user-shell"),
            ..ConfOptions::default()
        });
        assert!(text.contains("set -g default-shell /tmp/multi-run/user-shell\n"));
        assert!(!text.contains("set -g default-shell /bin/sh\n"));
    }

    // ---- user-shell wrapper --------------------------------------------

    #[test]
    fn zsh_wrapper_reexecs_with_login_and_interactive_flags() {
        let text = generate_wrapper(Path::new("/bin/zsh"));
        assert!(text.starts_with("#!/bin/sh\n"));
        assert!(text.contains("exec '/bin/zsh' -il \"$@\"\n"));
        // Must not consult $SHELL: tmux sets SHELL to default-shell itself.
        assert!(!text.contains("$SHELL"));
    }

    #[test]
    fn bash_wrapper_uses_login_interactive_flags() {
        let text = generate_wrapper(Path::new("/bin/bash"));
        assert!(text.contains("exec '/bin/bash' -il \"$@\"\n"));
    }

    #[test]
    fn fish_wrapper_uses_login_flag() {
        let text = generate_wrapper(Path::new("/usr/local/bin/fish"));
        assert!(text.contains("exec '/usr/local/bin/fish' -l \"$@\"\n"));
        assert!(!text.contains(" -il "));
    }

    #[test]
    fn plain_sh_wrapper_passes_args_through() {
        let text = generate_wrapper(Path::new("/bin/sh"));
        assert!(text.contains("exec '/bin/sh' \"$@\"\n"));
        assert!(!text.contains(" -il "));
        assert!(!text.contains(" -l "));
    }

    #[test]
    fn wrapper_single_quotes_paths_with_apostrophes() {
        let text = generate_wrapper(Path::new("/tmp/o'reilly/zsh"));
        assert!(text.contains("exec '/tmp/o'\\''reilly/zsh' -il \"$@\"\n"));
    }

    #[test]
    fn wrapper_path_sits_next_to_conf() {
        assert_eq!(
            wrapper_path(Path::new("/xdg/multi-run/tmux.conf")),
            PathBuf::from("/xdg/multi-run/user-shell")
        );
    }

    #[test]
    fn resolve_user_shell_accepts_absolute_existing_path() {
        // /bin/sh exists on every unix we support; relative and missing
        // paths must fall back rather than be trusted.
        assert_eq!(
            resolve_user_shell_from(Some(PathBuf::from("/bin/sh"))),
            PathBuf::from("/bin/sh")
        );
        assert_eq!(
            resolve_user_shell_from(Some(PathBuf::from("zsh"))),
            PathBuf::from("/bin/sh")
        );
        assert_eq!(
            resolve_user_shell_from(Some(PathBuf::from("/no/such/shell"))),
            PathBuf::from("/bin/sh")
        );
        assert_eq!(resolve_user_shell_from(None), PathBuf::from("/bin/sh"));
    }

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

    /// Expected conf text after `ensure_with_user_shell` pairs conf + wrapper.
    fn expected_ensured_conf(path: &Path, opts: ConfOptions) -> String {
        generate(ConfOptions {
            default_shell: wrapper_path(path),
            ..opts
        })
    }

    #[test]
    fn ensure_creates_missing_conf_wrapper_and_parent_dir() {
        let dir = scratch_dir("create");
        let path = dir.join("nested").join("tmux.conf");
        assert!(!path.exists());

        let wrote = ensure_with_user_shell(&path, ConfOptions::default(), Path::new("/bin/zsh")).unwrap();
        assert!(wrote);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents, expected_ensured_conf(&path, ConfOptions::default()));
        assert!(contents.contains(&format!(
            "set -g default-shell {}\n",
            wrapper_path(&path).display()
        )));

        let wrapper = wrapper_path(&path);
        let wrapper_text = std::fs::read_to_string(&wrapper).unwrap();
        assert_eq!(wrapper_text, generate_wrapper(Path::new("/bin/zsh")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&wrapper).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "wrapper must be executable");
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_leaves_matching_conf_and_wrapper_untouched() {
        let dir = scratch_dir("current");
        let path = dir.join("tmux.conf");
        let shell = Path::new("/bin/zsh");
        assert!(ensure_with_user_shell(&path, ConfOptions::default(), shell).unwrap());

        // The steady state: same options, same text, no write. This is what
        // keeps startup from touching the file on every run.
        let wrote = ensure_with_user_shell(&path, ConfOptions::default(), shell).unwrap();
        assert!(!wrote);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_restores_a_file_that_drifted_from_the_template() {
        let dir = scratch_dir("drift");
        let path = dir.join("tmux.conf");
        let shell = Path::new("/bin/zsh");
        ensure_with_user_shell(&path, ConfOptions::default(), shell).unwrap();

        // Hand edits and half-written files go back to the real template:
        // the header alone cannot tell us the body is still right.
        let mut contents = std::fs::read_to_string(&path).unwrap();
        contents.push_str("# sentinel line appended by the test\n");
        std::fs::write(&path, &contents).unwrap();

        assert!(ensure_with_user_shell(&path, ConfOptions::default(), shell).unwrap());
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(!after.contains("sentinel line appended by the test"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_rewrites_wrapper_when_user_shell_changes() {
        let dir = scratch_dir("shellchange");
        let path = dir.join("tmux.conf");
        ensure_with_user_shell(&path, ConfOptions::default(), Path::new("/bin/zsh")).unwrap();

        let wrote =
            ensure_with_user_shell(&path, ConfOptions::default(), Path::new("/bin/bash")).unwrap();
        assert!(wrote);
        let wrapper = std::fs::read_to_string(wrapper_path(&path)).unwrap();
        assert!(wrapper.contains("exec '/bin/bash' -il"));
        assert!(!wrapper.contains("/bin/zsh"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ensure_rewrites_when_only_the_options_changed() {
        // The version header is identical in both, so a version-only check
        // would leave a `--no-user-conf` run still sourcing the user's config.
        let dir = scratch_dir("optchange");
        let path = dir.join("tmux.conf");
        let shell = Path::new("/bin/zsh");
        ensure_with_user_shell(&path, ConfOptions::default(), shell).unwrap();

        let wrote = ensure_with_user_shell(
            &path,
            ConfOptions {
                source_user_conf: false,
                ..ConfOptions::default()
            },
            shell,
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

        let wrote =
            ensure_with_user_shell(&path, ConfOptions::default(), Path::new("/bin/zsh")).unwrap();
        assert!(wrote);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(!contents.contains("stale content"));
        assert_eq!(contents, expected_ensured_conf(&path, ConfOptions::default()));

        std::fs::remove_dir_all(&dir).ok();
    }
}
