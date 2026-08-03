//! clap derive CLI surface (DESIGN.md section 2), plus `GroupId`/`JobId`/
//! `Target` parsing and display for the three id shapes a user can type:
//! `g1`, `g1.2`, and the raw tmux session name `mr-g1-2`. Legacy four-
//! character ids (`k3f9`) are still accepted so older live groups remain
//! attachable. No tmux invocation and no dispatch logic live here; this
//! module only turns argv into typed values.

use clap::Parser;
use std::path::PathBuf;
use std::str::FromStr;

/// Alphabet used by legacy four-character group ids: digits 2-9 and
/// lowercase letters, excluding `0`/`1`/`i`/`l`/`o`/`u`. New groups use
/// sequential `gN` ids instead; this alphabet remains so `--attach` /
/// `--kill` can still target a group created before that change.
pub const LEGACY_ID_ALPHABET: &str = "23456789abcdefghjkmnpqrstvwxyz";

/// Fixed length of a legacy `GroupId`.
const LEGACY_GROUP_ID_LEN: usize = 4;

/// Error returned when a string does not match the group/job/target id
/// grammar. Deliberately separate from `TmuxError` (`error.rs`): this is a
/// malformed CLI argument, not anything that came out of a tmux subprocess.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid id: {0:?}")]
pub struct ParseIdError(pub String);

/// A run group: everything created by one `multi-run` invocation. Displays
/// as its sequential id, e.g. `g1`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct GroupId(String);

impl GroupId {
    /// Build a `GroupId` directly, without format validation. Used for
    /// constructing a freshly generated id (validated at the generation
    /// site) and for building fixtures in other modules' tests. Validate
    /// untrusted/user-typed input with `str::parse` instead.
    pub fn new(id: impl Into<String>) -> Self {
        GroupId(id.into())
    }

    /// Borrow the id text, e.g. for building a tmux session name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse the numeric suffix of a sequential `gN` id, if this string is
    /// one. Used by group-id generation to pick the smallest unused N.
    pub fn sequential_number(s: &str) -> Option<u32> {
        let rest = s.strip_prefix('g')?;
        if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        // Reject leading zeros (`g01`) and `g0`.
        if rest.starts_with('0') {
            return None;
        }
        rest.parse().ok()
    }
}

impl std::fmt::Display for GroupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for GroupId {
    type Err = ParseIdError;

    /// Accepts sequential `gN` (`g1`, `g2`, …) or a legacy four-character
    /// id drawn from `LEGACY_ID_ALPHABET`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if GroupId::sequential_number(s).is_some() {
            return Ok(GroupId(s.to_string()));
        }
        if s.chars().count() == LEGACY_GROUP_ID_LEN
            && s.chars().all(|c| LEGACY_ID_ALPHABET.contains(c))
        {
            return Ok(GroupId(s.to_string()));
        }
        Err(ParseIdError(s.to_string()))
    }
}

/// One job inside a group. `index` is 1-based, monotonic within a group,
/// and never reused. Displays as `g1.2`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct JobId {
    pub group: GroupId,
    pub index: u16,
}

impl JobId {
    /// tmux session name, e.g. `mr-g1-2`. Purely for human readability in
    /// `tmux ls`; all logic keys off `@mr_group`/`@mr_index`.
    pub fn tmux_name(&self) -> String {
        format!("mr-{}-{}", self.group.as_str(), self.index)
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.group, self.index)
    }
}

/// Parse a 1-based job index: digits only, and zero is not a valid index
/// (DESIGN.md section 4: `index: u16, // 1-based`).
fn parse_index(s: &str) -> Option<u16> {
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u16 = s.parse().ok()?;
    if n == 0 { None } else { Some(n) }
}

impl FromStr for JobId {
    type Err = ParseIdError;

    /// Accepts `g1.2` (group dot index) and `mr-g1-2` (the raw tmux
    /// session name, for copy-paste from `tmux ls`). Legacy forms like
    /// `k3f9.2` / `mr-k3f9-2` are also accepted. Does not accept a bare
    /// group id; use `Target` or `GroupId::from_str` for that.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || ParseIdError(s.to_string());

        if let Some(rest) = s.strip_prefix("mr-") {
            let mut parts = rest.splitn(2, '-');
            let group_str = parts.next().ok_or_else(bad)?;
            let index_str = parts.next().ok_or_else(bad)?;
            let group: GroupId = group_str.parse().map_err(|_| bad())?;
            let index = parse_index(index_str).ok_or_else(bad)?;
            return Ok(JobId { group, index });
        }

        let mut parts = s.splitn(2, '.');
        let group_str = parts.next().ok_or_else(bad)?;
        let index_str = parts.next().ok_or_else(bad)?;
        let group: GroupId = group_str.parse().map_err(|_| bad())?;
        let index = parse_index(index_str).ok_or_else(bad)?;
        Ok(JobId { group, index })
    }
}

/// What an id-like CLI argument (`--attach`, `--kill`) resolves to. `--kill`
/// additionally accepts the literal `all`; `--attach` never receives it (the
/// caller is expected to reject that combination, since resolving `all` to
/// a TUI target does not make sense).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Target {
    Group(GroupId),
    Job(JobId),
    All,
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Group(g) => write!(f, "{g}"),
            Target::Job(j) => write!(f, "{j}"),
            Target::All => f.write_str("all"),
        }
    }
}

impl FromStr for Target {
    type Err = ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "all" {
            return Ok(Target::All);
        }
        if let Ok(job) = s.parse::<JobId>() {
            return Ok(Target::Job(job));
        }
        s.parse::<GroupId>().map(Target::Group)
    }
}

/// Initial view mode, set with `--view`. Mirrors `model::ViewMode` but lives
/// here as a `clap::ValueEnum` so this module has no dependency on
/// `model.rs`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum ViewArg {
    Grid,
    List,
}

/// multi-run: run N shell commands in parallel, each in its own tmux
/// session, and watch them all in one TUI. See DESIGN.md section 2.
#[derive(Parser, Debug)]
#[command(
    name = "multi-run",
    version,
    about = "Run shell commands in parallel tmux sessions with a live TUI"
)]
pub struct Cli {
    /// Shell commands to run, one per argument. Each runs via /bin/sh -c
    /// in the current directory. If omitted, the compose screen opens.
    #[arg(allow_hyphen_values = true)]
    pub commands: Vec<String>,

    /// Add a command. Repeatable. Use this when the command starts with a
    /// hyphen and you do not want to use `--`.
    #[arg(short = 'c', long = "command", value_name = "CMD")]
    pub command: Vec<String>,

    /// List active multi-run groups and exit.
    #[arg(short, long)]
    pub list: bool,

    /// Open the TUI on an existing group. `g1` opens the group; `g1.2`
    /// opens it and immediately attaches to job 2.
    #[arg(short, long, value_name = "ID")]
    pub attach: Option<String>,

    /// Kill a group (`g1`), a single job (`g1.2`), or everything
    /// (`all`). Exits without opening the TUI.
    #[arg(long, value_name = "ID")]
    pub kill: Option<String>,

    /// Initial view: `grid` or `list`. Default adapts to fit.
    #[arg(long, value_enum, value_name = "MODE")]
    pub view: Option<ViewArg>,

    /// Human label for this group, shown in --list.
    #[arg(long, value_name = "TEXT")]
    pub label: Option<String>,

    /// tmux socket name. Default `multi-run`. For tests and for running
    /// isolated groups side by side.
    #[arg(long, value_name = "NAME", default_value = "multi-run")]
    pub socket: String,

    /// Override the generated tmux config path.
    #[arg(long, value_name = "PATH")]
    pub tmux_conf: Option<PathBuf>,

    /// Do not source ~/.tmux.conf into our private server.
    #[arg(long)]
    pub no_user_conf: bool,

    /// Kill our tmux server before starting, forcing the current config to
    /// take effect. Destroys all existing groups.
    #[arg(long)]
    pub restart_server: bool,
}

impl Cli {
    /// Commands from `-c`/`--command` (in the order given) followed by
    /// positional commands, per DESIGN.md section 2: "Commands from `-c`
    /// and from positionals are concatenated in the order `-c` values
    /// first, then positionals."
    pub fn all_commands(&self) -> Vec<String> {
        self.command
            .iter()
            .cloned()
            .chain(self.commands.iter().cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- GroupId ----

    #[test]
    fn group_id_parses_sequential() {
        let g: GroupId = "g1".parse().unwrap();
        assert_eq!(g.as_str(), "g1");
        assert_eq!(g.to_string(), "g1");
        assert_eq!("g12".parse::<GroupId>().unwrap().as_str(), "g12");
    }

    #[test]
    fn group_id_parses_legacy_four_char() {
        let g: GroupId = "k3f9".parse().unwrap();
        assert_eq!(g.as_str(), "k3f9");
    }

    #[test]
    fn group_id_new_skips_validation_for_fixtures() {
        // Other modules' tests need a way to build a GroupId without going
        // through FromStr. `new` is that escape hatch and deliberately
        // does not validate.
        let g = GroupId::new("not-a-valid-id");
        assert_eq!(g.as_str(), "not-a-valid-id");
    }

    #[test]
    fn group_id_rejects_malformed_sequential() {
        assert!("g".parse::<GroupId>().is_err());
        assert!("g0".parse::<GroupId>().is_err());
        assert!("g01".parse::<GroupId>().is_err());
        assert!("G1".parse::<GroupId>().is_err());
        assert!("".parse::<GroupId>().is_err());
    }

    #[test]
    fn group_id_rejects_uppercase_legacy() {
        assert!("K3F9".parse::<GroupId>().is_err());
    }

    #[test]
    fn group_id_rejects_legacy_with_excluded_letters() {
        // 0, 1, i, l, o, u are excluded from LEGACY_ID_ALPHABET.
        assert!("k0f9".parse::<GroupId>().is_err());
        assert!("k1f9".parse::<GroupId>().is_err());
        assert!("kif9".parse::<GroupId>().is_err());
        assert!("klf9".parse::<GroupId>().is_err());
        assert!("kof9".parse::<GroupId>().is_err());
        assert!("kuf9".parse::<GroupId>().is_err());
    }

    #[test]
    fn legacy_alphabet_excludes_ambiguous_characters() {
        for c in ['0', '1', 'i', 'l', 'o', 'u'] {
            assert!(
                !LEGACY_ID_ALPHABET.contains(c),
                "legacy alphabet should exclude {c:?}"
            );
        }
    }

    #[test]
    fn sequential_number_extracts_n() {
        assert_eq!(GroupId::sequential_number("g1"), Some(1));
        assert_eq!(GroupId::sequential_number("g42"), Some(42));
        assert_eq!(GroupId::sequential_number("g0"), None);
        assert_eq!(GroupId::sequential_number("g01"), None);
        assert_eq!(GroupId::sequential_number("k3f9"), None);
    }

    // ---- JobId ----

    #[test]
    fn job_id_parses_dotted_form() {
        let j: JobId = "g1.2".parse().unwrap();
        assert_eq!(j.group.as_str(), "g1");
        assert_eq!(j.index, 2);
        assert_eq!(j.to_string(), "g1.2");
    }

    #[test]
    fn job_id_parses_legacy_dotted_form() {
        let j: JobId = "k3f9.2".parse().unwrap();
        assert_eq!(j.group.as_str(), "k3f9");
        assert_eq!(j.index, 2);
    }

    #[test]
    fn job_id_parses_raw_tmux_name_form() {
        let j: JobId = "mr-g1-2".parse().unwrap();
        assert_eq!(j.group.as_str(), "g1");
        assert_eq!(j.index, 2);
        assert_eq!(j.tmux_name(), "mr-g1-2");
    }

    #[test]
    fn job_id_round_trips_through_tmux_name() {
        let j: JobId = "g1.12".parse().unwrap();
        assert_eq!(j.index, 12);
        let round_tripped: JobId = j.tmux_name().parse().unwrap();
        assert_eq!(j, round_tripped);
    }

    #[test]
    fn job_id_rejects_trailing_dot_with_no_index() {
        assert!("g1.".parse::<JobId>().is_err());
    }

    #[test]
    fn job_id_rejects_zero_index() {
        assert!("g1.0".parse::<JobId>().is_err());
    }

    #[test]
    fn job_id_rejects_uppercase_group() {
        assert!("G1".parse::<JobId>().is_err());
        assert!("G1.1".parse::<JobId>().is_err());
    }

    #[test]
    fn job_id_rejects_wrong_group() {
        assert!("abcde.1".parse::<JobId>().is_err());
    }

    #[test]
    fn job_id_rejects_non_numeric_index() {
        assert!("g1.abc".parse::<JobId>().is_err());
    }

    #[test]
    fn job_id_rejects_bare_group_with_no_dot() {
        assert!("g1".parse::<JobId>().is_err());
    }

    #[test]
    fn job_id_rejects_malformed_mr_prefix() {
        assert!("mr-g1".parse::<JobId>().is_err()); // missing index
        assert!("mr-g1-".parse::<JobId>().is_err()); // empty index
        assert!("mr-g1-0".parse::<JobId>().is_err()); // zero index
        assert!("mr-G1-1".parse::<JobId>().is_err()); // uppercase group
    }

    // ---- Target ----

    #[test]
    fn target_parses_bare_group() {
        assert_eq!(
            "g1".parse::<Target>().unwrap(),
            Target::Group("g1".parse().unwrap())
        );
    }

    #[test]
    fn target_parses_dotted_job() {
        assert_eq!(
            "g1.2".parse::<Target>().unwrap(),
            Target::Job("g1.2".parse().unwrap())
        );
    }

    #[test]
    fn target_parses_raw_tmux_name() {
        assert_eq!(
            "mr-g1-2".parse::<Target>().unwrap(),
            Target::Job("mr-g1-2".parse().unwrap())
        );
    }

    #[test]
    fn target_parses_legacy_group() {
        assert_eq!(
            "k3f9".parse::<Target>().unwrap(),
            Target::Group("k3f9".parse().unwrap())
        );
    }

    #[test]
    fn target_parses_all() {
        assert_eq!("all".parse::<Target>().unwrap(), Target::All);
    }

    #[test]
    fn target_rejects_malformed_ids() {
        for bad in ["g1.", "g1.0", "G1", "abcde", "g1.abc", "g0"] {
            assert!(
                bad.parse::<Target>().is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn target_displays_each_variant() {
        assert_eq!(Target::All.to_string(), "all");
        assert_eq!(Target::Group("g1".parse().unwrap()).to_string(), "g1");
        assert_eq!(Target::Job("g1.2".parse().unwrap()).to_string(), "g1.2");
    }

    // ---- clap parsing: every example invocation from DESIGN.md section 2 ----

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["multi-run"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).unwrap_or_else(|e| panic!("failed to parse {args:?}: {e}"))
    }

    #[test]
    fn parses_bare_positionals() {
        let cli = parse(&["cargo watch -x test", "npm run dev", "cargo fmt --check"]);
        assert_eq!(
            cli.commands,
            vec!["cargo watch -x test", "npm run dev", "cargo fmt --check"]
        );
        assert_eq!(cli.all_commands(), cli.commands);
    }

    #[test]
    fn bare_invocation_opens_compose_screen() {
        let cli = parse(&[]);
        assert!(cli.commands.is_empty());
        assert!(cli.command.is_empty());
        assert!(!cli.list);
    }

    #[test]
    fn parses_list_flag() {
        assert!(parse(&["--list"]).list);
        assert!(parse(&["-l"]).list);
    }

    #[test]
    fn parses_attach_group_only() {
        let cli = parse(&["--attach", "g1"]);
        assert_eq!(cli.attach.as_deref(), Some("g1"));
        // Confirms the CLI-layer string round trips through Target.
        assert_eq!(
            cli.attach.unwrap().parse::<Target>().unwrap(),
            Target::Group("g1".parse().unwrap())
        );
    }

    #[test]
    fn parses_attach_group_and_job() {
        let cli = parse(&["--attach", "g1.2"]);
        assert_eq!(cli.attach.as_deref(), Some("g1.2"));
    }

    #[test]
    fn parses_attach_short_flag() {
        let cli = parse(&["-a", "g1"]);
        assert_eq!(cli.attach.as_deref(), Some("g1"));
    }

    #[test]
    fn parses_kill_group() {
        let cli = parse(&["--kill", "g1"]);
        assert_eq!(cli.kill.as_deref(), Some("g1"));
    }

    #[test]
    fn parses_kill_all() {
        let cli = parse(&["--kill", "all"]);
        assert_eq!(cli.kill.as_deref(), Some("all"));
        assert_eq!(cli.kill.unwrap().parse::<Target>().unwrap(), Target::All);
    }

    #[test]
    fn parses_view_with_positionals() {
        let cli = parse(&["--view", "list", "make -j8", "tail -f /var/log/system.log"]);
        assert_eq!(cli.view, Some(ViewArg::List));
        assert_eq!(
            cli.commands,
            vec!["make -j8", "tail -f /var/log/system.log"]
        );
    }

    #[test]
    fn parses_view_grid() {
        assert_eq!(parse(&["--view", "grid"]).view, Some(ViewArg::Grid));
    }

    #[test]
    fn commands_with_their_own_flags_stay_as_one_argv_element() {
        let cli = parse(&["cargo test -- --nocapture", "npm run dev -- --port 3000"]);
        assert_eq!(
            cli.commands,
            vec!["cargo test -- --nocapture", "npm run dev -- --port 3000"]
        );
    }

    #[test]
    fn double_dash_separator_allows_leading_hyphen_positionals() {
        let cli = parse(&["--", "-x weird-first-char-command", "cargo build --release"]);
        assert_eq!(
            cli.commands,
            vec!["-x weird-first-char-command", "cargo build --release"]
        );
    }

    #[test]
    fn repeated_command_flag_accumulates_in_order() {
        let cli = parse(&["-c", "cargo watch -x test", "-c", "npm run dev"]);
        assert_eq!(cli.command, vec!["cargo watch -x test", "npm run dev"]);
        assert!(cli.commands.is_empty());
    }

    #[test]
    fn all_commands_puts_flag_values_before_positionals() {
        // The variadic positional (allow_hyphen_values = true) greedily
        // consumes every token after its first value, `-c` included, so
        // `-c` values must all come before the positionals on the command
        // line. That is also the only order the section 2 examples use.
        let cli = parse(&["-c", "flag one", "-c", "flag two", "positional one"]);
        assert_eq!(cli.command, vec!["flag one", "flag two"]);
        assert_eq!(cli.commands, vec!["positional one"]);
        assert_eq!(
            cli.all_commands(),
            vec!["flag one", "flag two", "positional one"]
        );
    }

    #[test]
    fn parses_label_socket_conf_and_toggle_flags() {
        let cli = parse(&[
            "--label",
            "backend checks",
            "--socket",
            "test-sock",
            "--tmux-conf",
            "/tmp/mr.conf",
            "--no-user-conf",
            "--restart-server",
        ]);
        assert_eq!(cli.label.as_deref(), Some("backend checks"));
        assert_eq!(cli.socket, "test-sock");
        assert_eq!(cli.tmux_conf, Some(PathBuf::from("/tmp/mr.conf")));
        assert!(cli.no_user_conf);
        assert!(cli.restart_server);
    }

    #[test]
    fn socket_defaults_to_multi_run() {
        assert_eq!(parse(&[]).socket, "multi-run");
    }

    #[test]
    fn invalid_view_value_is_a_usage_error() {
        // `--view` is a known flag with a validated `ValueEnum`, so an
        // unrecognised value is unambiguously a usage error. A bare unknown
        // token like `--not-a-flag` is deliberately *not* tested here: the
        // `commands` positional has `allow_hyphen_values = true` precisely
        // so a hyphen-leading command can be typed without `--` (section 2
        // forms 2 and 3 exist for users who want to avoid that ambiguity).
        let result = Cli::try_parse_from(["multi-run", "--view", "nonsense"]);
        assert!(result.is_err());
    }

    #[test]
    fn help_and_version_flags_are_registered() {
        // clap intercepts these and returns a DisplayHelp/DisplayVersion
        // error variant rather than a parsed Cli; just confirm they exist
        // and short-circuit parsing rather than erroring as unknown flags.
        let help = Cli::try_parse_from(["multi-run", "--help"]).unwrap_err();
        assert_eq!(help.kind(), clap::error::ErrorKind::DisplayHelp);
        let version = Cli::try_parse_from(["multi-run", "-V"]).unwrap_err();
        assert_eq!(version.kind(), clap::error::ErrorKind::DisplayVersion);
    }
}
