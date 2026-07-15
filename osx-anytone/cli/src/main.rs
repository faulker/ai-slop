//! `anytone-cli`: back up and restore AnyTone AT-D878UVII Plus codeplugs over
//! the USB programming cable. Phase 0/1 commands: `ports`, `info`, `backup`,
//! `restore`. The firmware UPDATE path is intentionally absent.

use std::fs;
use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use anytone_core::{
    autodetect_radio, codeplug_size, is_supported_model, list_ports, Codeplug, Radio,
    SerialTransport,
};
use clap::{Parser, Subcommand};

/// Serial baud rate. Largely ignored by the CDC-ACM driver but set anyway.
const BAUD: u32 = 115_200;

/// Per-read timeout for the serial link.
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Parser)]
#[command(
    name = "anytone-cli",
    version,
    about = "Program AnyTone AT-D878UVII Plus codeplugs over USB serial"
)]
struct Cli {
    /// Serial port path (e.g. /dev/cu.usbmodem1234). If omitted, the single
    /// matching radio port is auto-selected.
    #[arg(long, global = true)]
    port: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List serial ports, marking likely radios.
    Ports,
    /// Enter program mode, identify the radio, print model/version, exit.
    Info,
    /// Read the full codeplug from the radio into a binary file.
    Backup {
        /// Destination file for the codeplug image.
        file: String,
    },
    /// Write a codeplug file back to the radio with read-back verification.
    Restore {
        /// Codeplug image to write.
        file: String,
        /// Required to actually write. Without it, restore refuses.
        #[arg(long)]
        force: bool,
    },
    /// Parse a codeplug .bin file on disk (offline, no radio) and print its
    /// channels, zones, contacts, group lists, and radio IDs as JSON.
    Dump {
        /// Codeplug image to parse.
        file: String,
    },
    /// Apply a JSON edit batch to a codeplug .bin offline (no radio). The edits
    /// file has the same schema as the GUI / `anytone_apply_edits`.
    Edit {
        /// Codeplug image to edit.
        file: String,
        /// JSON file describing the edits.
        edits: String,
        /// Output file; defaults to editing `file` in place.
        #[arg(short, long)]
        output: Option<String>,
    },
}

/// Entry point: dispatch the subcommand and map any error to a non-zero exit.
fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Dispatch to the selected subcommand.
fn run(cli: &Cli) -> anytone_core::Result<()> {
    match &cli.command {
        Command::Ports => cmd_ports(),
        Command::Info => cmd_info(&resolve_port(cli)?),
        Command::Backup { file } => cmd_backup(&resolve_port(cli)?, file),
        Command::Restore { file, force } => cmd_restore(&resolve_port(cli)?, file, *force),
        Command::Dump { file } => cmd_dump(file),
        Command::Edit {
            file,
            edits,
            output,
        } => cmd_edit(file, edits, output.as_deref()),
    }
}

/// `edit`: load a codeplug `.bin` and a JSON edit batch, apply the edits through
/// the shared core path (verified by re-parsing), and write the result.
fn cmd_edit(file: &str, edits: &str, output: Option<&str>) -> anytone_core::Result<()> {
    let data = fs::read(file)?;
    let edits_json = fs::read_to_string(edits)?;
    let out_path = output.unwrap_or(file);
    let result = anytone_core::apply_edits(&data, &edits_json)
        .map_err(anytone_core::Error::InvalidArgument)?;
    fs::write(out_path, &result)?;
    println!("applied edits: wrote {} bytes to {out_path}", result.len());
    Ok(())
}

/// `dump`: load a codeplug `.bin` from disk (offline, no radio), parse it, and
/// print its channels and zones as pretty JSON to stdout.
fn cmd_dump(file: &str) -> anytone_core::Result<()> {
    let data = fs::read(file)?;
    let codeplug = Codeplug::parse(&data)?;
    let json = serde_json::to_string_pretty(&codeplug.to_json())
        .map_err(|e| anytone_core::Error::Parse(format!("failed to encode JSON: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Resolve the serial port: use `--port` if given, else auto-detect the single
/// likely radio, erroring clearly when none or several are present.
fn resolve_port(cli: &Cli) -> anytone_core::Result<String> {
    if let Some(p) = &cli.port {
        return Ok(p.clone());
    }
    match autodetect_radio()? {
        Some(p) => {
            eprintln!("auto-selected radio port: {p}");
            Ok(p)
        }
        None => Err(anytone_core::Error::InvalidArgument(
            "no radio port detected; plug in the cable or pass --port".into(),
        )),
    }
}

/// Open a serial transport and wrap it in a `Radio`.
fn open_radio(port: &str) -> anytone_core::Result<Radio<SerialTransport>> {
    let t = SerialTransport::open(port, BAUD, TIMEOUT)?;
    Ok(Radio::new(t))
}

/// `ports`: enumerate serial ports and mark likely radios.
fn cmd_ports() -> anytone_core::Result<()> {
    let ports = list_ports()?;
    if ports.is_empty() {
        println!("no serial ports found");
        return Ok(());
    }
    for p in ports {
        let mark = if p.likely_radio { "* radio" } else { "" };
        let ids = match (p.vid, p.pid) {
            (Some(v), Some(d)) => format!(" [{v:04x}:{d:04x}]"),
            _ => String::new(),
        };
        let product = p.product.map(|s| format!(" {s}")).unwrap_or_default();
        println!("{}{}{}  {}", p.name, ids, product, mark);
    }
    Ok(())
}

/// `info`: enter program mode, identify, print, exit.
fn cmd_info(port: &str) -> anytone_core::Result<()> {
    let mut radio = open_radio(port)?;
    radio.enter()?;
    let model = radio.identify()?;
    radio.exit()?;
    println!("model/version: {model}");
    if !is_supported_model(&model) {
        eprintln!("warning: identify string does not look like a D878UV-family radio");
    }
    Ok(())
}

/// `backup`: read the full codeplug and write it to `file`.
fn cmd_backup(port: &str, file: &str) -> anytone_core::Result<()> {
    let mut radio = open_radio(port)?;
    radio.enter()?;
    let model = radio.identify()?;
    eprintln!("radio: {model}");
    let mut progress = make_progress("reading");
    let data = radio.read_codeplug(&mut progress)?;
    radio.exit()?;
    eprintln!();
    fs::write(file, &data)?;
    println!("wrote {} bytes to {file}", data.len());
    Ok(())
}

/// `restore`: write a codeplug file back, requiring `--force`, verifying the
/// model, and relying on per-block read-back verification inside the core.
fn cmd_restore(port: &str, file: &str, force: bool) -> anytone_core::Result<()> {
    let data = fs::read(file)?;
    if data.len() != codeplug_size() {
        return Err(anytone_core::Error::InvalidArgument(format!(
            "codeplug file is {} bytes, expected {}",
            data.len(),
            codeplug_size()
        )));
    }

    if !force {
        return Err(anytone_core::Error::InvalidArgument(
            "restore writes to the radio and can overwrite its config; \
             re-run with --force to proceed (back up first!)"
                .into(),
        ));
    }

    let mut radio = open_radio(port)?;
    radio.enter()?;
    let model = radio.identify()?;
    if !is_supported_model(&model) {
        radio.exit()?;
        return Err(anytone_core::Error::InvalidArgument(format!(
            "refusing to write: identify string {model:?} is not a supported D878UV radio"
        )));
    }

    eprintln!("!!! WRITING CODEPLUG TO RADIO ({model}) !!!");
    eprintln!("!!! Do not disconnect the cable or power off the radio. !!!");

    let mut progress = make_progress("writing");
    let result = radio.write_codeplug(&data, &mut progress);
    // Always attempt a clean exit from program mode, even on failure.
    let exit = radio.exit();
    eprintln!();
    result?;
    exit?;
    println!("restore complete and verified: {} bytes to {file}", data.len());
    Ok(())
}

/// Build a progress callback that prints `label i/total` on one line.
fn make_progress(label: &'static str) -> impl FnMut(usize, usize) {
    move |done, total| {
        // Overwrite the same line; flush so it appears promptly.
        eprint!("\r{label} {done}/{total} blocks");
        let _ = std::io::stderr().flush();
    }
}
