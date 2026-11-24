//! Command line definition. Flags follow Docker's so muscle memory works.

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "hullr",
    version,
    about = "Lean, friendly containers for macOS and Linux",
    long_about = "hullr runs OCI containers (Docker Hub, GHCR, any registry), builds Dockerfiles and runs compose files.\n\
                  It works with or without its daemon: commands talk to the daemon when one is running, otherwise they do the work themselves.",
    after_help = "Try: hullr run --rm -it alpine sh   |   hullr compose up -d   |   hullr tui"
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args, Debug, Clone)]
pub struct Global {
    /// Daemon socket to use instead of the default.
    #[arg(long, global = true, env = "HULLR_SOCKET")]
    pub socket: Option<PathBuf>,
    /// Never use the daemon; do the work in this process (Linux only).
    #[arg(long, global = true)]
    pub no_daemon: bool,
    /// Disable colored output.
    #[arg(long, global = true, env = "NO_COLOR")]
    pub no_color: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Create and start a container
    Run(RunArgs),
    /// Create a container without starting it
    Create(RunArgs),
    /// Start stopped containers
    Start {
        /// Attach to output (and stdin with -i)
        #[arg(short, long)]
        attach: bool,
        #[arg(short, long)]
        interactive: bool,
        #[arg(required = true)]
        containers: Vec<String>,
    },
    /// Stop running containers
    Stop {
        /// Seconds to wait before killing
        #[arg(short, long, default_value_t = 10)]
        time: u32,
        #[arg(required = true)]
        containers: Vec<String>,
    },
    /// Restart containers
    Restart {
        #[arg(short, long, default_value_t = 10)]
        time: u32,
        #[arg(required = true)]
        containers: Vec<String>,
    },
    /// Send a signal to running containers
    Kill {
        #[arg(short, long, default_value = "KILL")]
        signal: String,
        #[arg(required = true)]
        containers: Vec<String>,
    },
    /// Remove containers
    Rm {
        /// Kill running containers first
        #[arg(short, long)]
        force: bool,
        #[arg(required = true)]
        containers: Vec<String>,
    },
    /// List containers
    #[command(alias = "ls")]
    Ps {
        /// Include stopped containers
        #[arg(short, long)]
        all: bool,
        /// Only print ids
        #[arg(short, long)]
        quiet: bool,
    },
    /// Show container output
    Logs {
        #[arg(short, long)]
        follow: bool,
        /// Number of lines from the end
        #[arg(short = 'n', long)]
        tail: Option<usize>,
        #[arg(short, long)]
        timestamps: bool,
        container: String,
    },
    /// Run a command in a running container
    Exec {
        #[arg(short, long)]
        interactive: bool,
        #[arg(short, long)]
        tty: bool,
        #[arg(short, long = "env")]
        env: Vec<String>,
        #[arg(short, long)]
        workdir: Option<String>,
        #[arg(short, long)]
        user: Option<String>,
        container: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Attach to a running container's main process
    Attach { container: String },
    /// Show low-level details of a container or image (JSON)
    Inspect { targets: Vec<String> },
    /// Live resource usage
    Stats {
        #[arg(long)]
        no_stream: bool,
        containers: Vec<String>,
    },
    /// Wait for containers to exit and print their exit codes
    Wait {
        #[arg(required = true)]
        containers: Vec<String>,
    },
    /// Download an image
    Pull {
        #[arg(long)]
        platform: Option<String>,
        image: String,
    },
    /// Upload an image to its registry
    Push { image: String },
    /// List images
    Images {
        #[arg(short, long)]
        quiet: bool,
    },
    /// Remove images
    Rmi {
        #[arg(short, long)]
        force: bool,
        #[arg(required = true)]
        images: Vec<String>,
    },
    /// Give an image another name
    Tag { source: String, target: String },
    /// Build an image from a Dockerfile
    Build(BuildArgs),
    /// Save credentials for a registry
    Login {
        #[arg(short, long)]
        username: Option<String>,
        #[arg(short, long)]
        password: Option<String>,
        #[arg(long)]
        password_stdin: bool,
        /// Defaults to Docker Hub
        registry: Option<String>,
    },
    /// Manage volumes
    #[command(subcommand)]
    Volume(VolumeCmd),
    /// Multi-container apps from compose files
    Compose(ComposeArgs),
    /// System information and cleanup
    #[command(subcommand)]
    System(SystemCmd),
    /// Same as `system info`
    Info,
    /// Run or manage the background daemon
    #[command(subcommand)]
    Daemon(DaemonCmd),
    /// Manage the Linux VM (macOS)
    #[command(subcommand)]
    Vm(VmCmd),
    /// Full-screen terminal dashboard
    Tui,
    /// Desktop app
    Gui,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RunArgs {
    /// Run in the background and print the id
    #[arg(short, long)]
    pub detach: bool,
    /// Keep stdin open
    #[arg(short, long)]
    pub interactive: bool,
    /// Allocate a terminal
    #[arg(short, long)]
    pub tty: bool,
    /// Remove the container when it exits
    #[arg(long)]
    pub rm: bool,
    #[arg(long)]
    pub name: Option<String>,
    /// KEY=VALUE (or KEY to pass through)
    #[arg(short, long = "env")]
    pub env: Vec<String>,
    #[arg(long)]
    pub env_file: Vec<PathBuf>,
    /// HOST:CONTAINER[/udp]
    #[arg(short, long = "publish")]
    pub publish: Vec<String>,
    /// SRC:DST[:ro]
    #[arg(short, long = "volume")]
    pub volume: Vec<String>,
    /// Mount a tmpfs at this path
    #[arg(long)]
    pub tmpfs: Vec<String>,
    #[arg(short, long)]
    pub workdir: Option<String>,
    #[arg(short, long)]
    pub user: Option<String>,
    #[arg(long)]
    pub entrypoint: Option<String>,
    /// e.g. linux/arm64 (runs under emulation if foreign)
    #[arg(long)]
    pub platform: Option<String>,
    /// private (default), host, none, container:NAME
    #[arg(long, alias = "net")]
    pub network: Option<String>,
    /// no, always, unless-stopped, on-failure[:N]
    #[arg(long, default_value = "no")]
    pub restart: String,
    #[arg(long)]
    pub hostname: Option<String>,
    /// HOST:IP
    #[arg(long)]
    pub add_host: Vec<String>,
    /// KEY=VALUE
    #[arg(short, long = "label")]
    pub label: Vec<String>,
    /// missing, always, never
    #[arg(long, default_value = "missing")]
    pub pull: String,
    /// Memory limit, e.g. 512m
    #[arg(short, long)]
    pub memory: Option<String>,
    #[arg(long)]
    pub cpus: Option<f64>,
    pub image: String,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct BuildArgs {
    /// name:tag (repeatable)
    #[arg(short, long = "tag")]
    pub tag: Vec<String>,
    /// Dockerfile path (default CONTEXT/Dockerfile)
    #[arg(short, long)]
    pub file: Option<PathBuf>,
    /// KEY=VALUE
    #[arg(long)]
    pub build_arg: Vec<String>,
    #[arg(long)]
    pub target: Option<String>,
    #[arg(long)]
    pub platform: Option<String>,
    #[arg(long)]
    pub no_cache: bool,
    /// Always pull base images
    #[arg(long)]
    pub pull: bool,
    #[arg(long)]
    pub label: Vec<String>,
    #[arg(default_value = ".")]
    pub context: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum VolumeCmd {
    #[command(alias = "list")]
    Ls,
    Create { name: String },
    Rm {
        #[arg(short, long)]
        force: bool,
        #[arg(required = true)]
        names: Vec<String>,
    },
    Inspect { name: String },
}

#[derive(Args, Debug)]
pub struct ComposeArgs {
    /// Compose file(s)
    #[arg(short, long = "file")]
    pub file: Vec<PathBuf>,
    #[arg(short, long = "project-name")]
    pub project_name: Option<String>,
    #[arg(long)]
    pub project_directory: Option<PathBuf>,
    #[command(subcommand)]
    pub command: ComposeCmd,
}

#[derive(Subcommand, Debug)]
pub enum ComposeCmd {
    /// Create and start services
    Up {
        #[arg(short, long)]
        detach: bool,
        /// Build images before starting
        #[arg(long)]
        build: bool,
        services: Vec<String>,
    },
    /// Stop and remove services
    Down {
        /// Also remove named volumes
        #[arg(short, long)]
        volumes: bool,
    },
    /// List service containers
    Ps,
    /// Show service output
    Logs {
        #[arg(short, long)]
        follow: bool,
        #[arg(short = 'n', long)]
        tail: Option<usize>,
        services: Vec<String>,
    },
    /// Build service images
    Build { services: Vec<String> },
    /// Pull service images
    Pull { services: Vec<String> },
    /// Stop services without removing them
    Stop { services: Vec<String> },
    /// Start stopped services
    Start { services: Vec<String> },
    /// Restart services
    Restart { services: Vec<String> },
    /// List projects
    Ls,
}

#[derive(Subcommand, Debug)]
pub enum SystemCmd {
    Info,
    /// Remove stopped containers and dangling images
    Prune {
        /// Remove all unused images, not just dangling ones
        #[arg(short, long)]
        all: bool,
        #[arg(long)]
        volumes: bool,
        /// Do not ask for confirmation
        #[arg(short, long)]
        force: bool,
    },
    /// Running images for other CPU architectures
    #[command(subcommand)]
    Emulation(EmulationCmd),
}

#[derive(Subcommand, Debug)]
pub enum EmulationCmd {
    /// Register qemu-user binfmt handlers (Linux, needs root)
    Enable,
    /// Show which foreign platforms can run
    Status,
}

#[derive(Subcommand, Debug)]
pub enum DaemonCmd {
    /// Run the daemon in the foreground
    Run {
        /// unix:/path or vsock:PORT (repeatable)
        #[arg(long)]
        listen: Vec<String>,
    },
    /// Start the daemon in the background
    Start,
    /// Stop the background daemon (containers keep running)
    Stop,
    /// Is the daemon running?
    Status,
    /// Install as a service (systemd on Linux, launchd on macOS)
    Install {
        /// System-wide service instead of per-user (Linux, needs root)
        #[arg(long)]
        system: bool,
    },
    /// Remove the service
    Uninstall {
        #[arg(long)]
        system: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum VmCmd {
    Start,
    Stop,
    Status,
    /// Show or change VM settings (applies on next start)
    Config {
        #[arg(long)]
        cpus: Option<u32>,
        /// MiB
        #[arg(long)]
        memory: Option<u64>,
        /// GiB
        #[arg(long)]
        disk: Option<u64>,
        #[arg(long)]
        rosetta: Option<bool>,
    },
}
