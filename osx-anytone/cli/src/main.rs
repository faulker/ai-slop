//! `anytone-cli`: back up and restore AnyTone AT-D878UVII Plus codeplugs over
//! the USB programming cable. Phase 0/1 commands: `ports`, `info`, `backup`,
//! `restore`, plus the offline `dump`/`edit` and the `dump-range` read-only
//! diagnostic. The firmware UPDATE path is intentionally absent.

use std::fs;
use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use anytone_core::document::Document;
use anytone_core::driver::{driver_for, write_verified_image};
use anytone_core::model::Model;
use anytone_core::{
    autodetect_radio, list_ports, Radio, SerialTransport, Transport,
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

    /// Radio model id: "anytone-d878uv2" or "qyt-kt5000". Required when the
    /// port's USB identity does not name a model, which is the normal case for
    /// the QYT: it talks through a generic USB-serial cable whose VID/PID is
    /// shared with unrelated hardware, so it can never be attributed on sight.
    #[arg(long, global = true)]
    model: Option<String>,

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
    /// Read an arbitrary address range from the radio and save or hexdump it.
    ///
    /// Read-only diagnostic: it never writes. Use it to capture regions the
    /// codeplug region map does not cover, notably the radio-settings block at
    /// 0x02500000 length 0x1500, which backups do not include.
    DumpRange {
        /// Start address, decimal or 0x-prefixed hex (e.g. 0x02500000).
        #[arg(long, value_parser = parse_u32_auto)]
        address: u32,
        /// Byte count, decimal or 0x-prefixed hex. Must be a multiple of 16.
        #[arg(long, value_parser = parse_u32_auto)]
        length: u32,
        /// Destination file for the raw bytes. Omit to hexdump to stdout.
        #[arg(short, long)]
        output: Option<String>,
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
        Command::Info => cmd_info(&resolve_port(cli)?, resolve_model(cli)?),
        Command::Backup { file } => cmd_backup(&resolve_port(cli)?, resolve_model(cli)?, file),
        Command::Restore { file, force } => cmd_restore(&resolve_port(cli)?, file, *force),
        Command::Dump { file } => cmd_dump(file),
        Command::DumpRange {
            address,
            length,
            output,
        } => cmd_dump_range(&resolve_port(cli)?, *address, *length, output.as_deref()),
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
    // Model-aware: the envelope dispatches on the model detected from `file`,
    // and a spec with no model key is still accepted as an AnyTone spec.
    let result = anytone_core::document::apply_edits(&data, &edits_json)
        .map_err(anytone_core::Error::InvalidArgument)?;
    fs::write(out_path, &result)?;
    println!("applied edits: wrote {} bytes to {out_path}", result.len());
    Ok(())
}

/// `dump`: load a codeplug `.bin` from disk (offline, no radio), parse it, and
/// print its channels and zones as pretty JSON to stdout.
fn cmd_dump(file: &str) -> anytone_core::Result<()> {
    let data = fs::read(file)?;
    // Model-aware: the radio a .bin belongs to is detected from its contents, so
    // this works for either supported radio without a --model hint.
    let document = Document::parse(&data)?;
    let json = serde_json::to_string_pretty(&document.to_json())
        .map_err(|e| anytone_core::Error::Parse(format!("failed to encode JSON: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Parse a CLI address or length written as decimal or `0x`-prefixed hex,
/// ignoring `_` separators. Addresses in the memory map are quoted in hex, so
/// accepting both spellings avoids error-prone hand conversion.
fn parse_u32_auto(s: &str) -> std::result::Result<u32, String> {
    let t = s.trim().replace('_', "");
    let (body, radix) = match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(rest) => (rest.to_string(), 16),
        None => (t, 10),
    };
    u32::from_str_radix(&body, radix).map_err(|e| format!("invalid number {s:?}: {e}"))
}

/// Format `data` as a hexdump: 16 bytes per line, each line prefixed with its
/// absolute radio address and followed by an ASCII gutter.
fn hexdump(base: u32, data: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in data.chunks(16).enumerate() {
        let addr = base + (i * 16) as u32;
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if (0x20..0x7f).contains(&b) {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!("{addr:08x}  {:<47}  |{ascii}|\n", hex.join(" ")));
    }
    out
}

/// `dump-range`: read `length` bytes at `address` straight off the radio and
/// either save them or hexdump them. Read-only; nothing is written. Program
/// mode is always exited, even if the read fails partway.
fn cmd_dump_range(
    port: &str,
    address: u32,
    length: u32,
    output: Option<&str>,
) -> anytone_core::Result<()> {
    let mut radio = open_radio(port)?;
    radio.enter()?;
    let model = radio.identify()?;
    eprintln!("radio: {model}");
    let read = radio.read_region(address, length as usize);
    let exit = radio.exit();
    let data = read?;
    exit?;

    match output {
        Some(path) => {
            fs::write(path, &data)?;
            println!("wrote {} bytes from 0x{address:08x} to {path}", data.len());
        }
        None => print!("{}", hexdump(address, &data)),
    }
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

/// Resolve which radio we are talking to.
///
/// An explicit `--model` always wins. Otherwise fall back to the model named by
/// the port's USB identity. A generic USB-serial bridge names no model, so the
/// QYT normally requires `--model qyt-kt5000`; guessing there would mean
/// speaking the wrong protocol at whatever is on the other end of the cable.
fn resolve_model(cli: &Cli) -> anytone_core::Result<Model> {
    if let Some(id) = &cli.model {
        return Model::from_id(id).ok_or_else(|| {
            let known: Vec<&str> = Model::ALL.iter().map(|m| m.id()).collect();
            anytone_core::Error::InvalidArgument(format!(
                "unknown model {id:?}; expected one of: {}",
                known.join(", ")
            ))
        });
    }
    let port = resolve_port(cli)?;
    let named = list_ports()?
        .into_iter()
        .find(|p| p.name == port)
        .and_then(|p| p.model);
    named.ok_or_else(|| {
        anytone_core::Error::InvalidArgument(format!(
            "cannot tell which radio is on {port}; pass --model (one of: {})",
            Model::ALL
                .iter()
                .map(|m| m.id())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
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
fn cmd_info(port: &str, model: Model) -> anytone_core::Result<()> {
    let driver = driver_for(model);
    let mut t = SerialTransport::open(port, BAUD, TIMEOUT)?;
    let identity = {
        let mut dyn_t: &mut dyn Transport = &mut t;
        driver.identify(&mut dyn_t)?
    };
    println!("model:         {} ({})", model.display_name(), model.id());
    println!("identity:      {identity}");
    if !driver.accepts_identity(&identity) {
        eprintln!(
            "warning: identity does not look like a {}; check --model",
            model.display_name()
        );
    }
    Ok(())
}

/// `backup`: read the full codeplug and write it to `file`.
fn cmd_backup(port: &str, model: Model, file: &str) -> anytone_core::Result<()> {
    let driver = driver_for(model);
    let mut t = SerialTransport::open(port, BAUD, TIMEOUT)?;
    let mut progress = make_progress("reading");
    eprintln!("radio: {}", model.display_name());
    // One driver call per open port. Each method is a complete session, and the
    // QYT rejects a second handshake on an open link, so there is deliberately
    // no separate identify() here.
    let data = {
        let mut dyn_t: &mut dyn Transport = &mut t;
        driver.read_image(&mut dyn_t, &mut progress)?
    };
    eprintln!();
    fs::write(file, &data)?;
    println!("wrote {} bytes to {file}", data.len());
    Ok(())
}

/// `restore`: write a codeplug file back, requiring `--force`, verifying the
/// model, and relying on per-block read-back verification inside the core.
fn cmd_restore(port: &str, file: &str, force: bool) -> anytone_core::Result<()> {
    let data = fs::read(file)?;

    // Which radio this image belongs to is a property of the file, not a flag.
    // Deriving it here is what lets the core refuse to write a codeplug to the
    // wrong radio.
    let model = Model::detect(&data).ok_or_else(|| {
        anytone_core::Error::InvalidArgument(format!(
            "{file} is {} bytes, which matches no supported radio image",
            data.len()
        ))
    })?;

    if !force {
        return Err(anytone_core::Error::InvalidArgument(format!(
            "restore writes to the radio and can overwrite its config; \
             re-run with --force to proceed (back up first!). This file is a \
             {} image.",
            model.display_name()
        )));
    }

    eprintln!("!!! WRITING CODEPLUG TO RADIO ({}) !!!", model.display_name());
    eprintln!("!!! Do not disconnect the cable or power off the radio. !!!");

    let mut t = SerialTransport::open(port, BAUD, TIMEOUT)?;
    let mut progress = make_progress("writing");
    // One driver call per open port: write_verified_image handshakes, checks the
    // radio's identity, and transfers, all inside a single session.
    let result = {
        let mut dyn_t: &mut dyn Transport = &mut t;
        write_verified_image(model, &mut dyn_t, &data, port, &mut progress)
    };
    eprintln!();
    result.map_err(anytone_core::Error::InvalidArgument)?;
    println!("restore complete: {} bytes to {file}", data.len());
    if model == Model::QytKt5000 {
        println!("the radio reboots now; reconnect before reading back to verify");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_u32_auto_accepts_hex_and_decimal() {
        assert_eq!(parse_u32_auto("0x02500000").unwrap(), 0x0250_0000);
        assert_eq!(parse_u32_auto("0X02500000").unwrap(), 0x0250_0000);
        assert_eq!(parse_u32_auto("0x0250_0000").unwrap(), 0x0250_0000);
        assert_eq!(parse_u32_auto("  0x1500 ").unwrap(), 0x1500);
        // No prefix means decimal, so this is not 0x1500.
        assert_eq!(parse_u32_auto("1500").unwrap(), 1500);
        assert_eq!(parse_u32_auto("0").unwrap(), 0);
    }

    #[test]
    fn parse_u32_auto_rejects_garbage_and_overflow() {
        assert!(parse_u32_auto("nope").is_err());
        assert!(parse_u32_auto("0xzz").is_err());
        assert!(parse_u32_auto("").is_err());
        // Decimal digits that are not valid hex must not sneak through.
        assert!(parse_u32_auto("0x1g").is_err());
        // Wider than u32.
        assert!(parse_u32_auto("0x100000000").is_err());
    }

    #[test]
    fn hexdump_labels_lines_with_absolute_addresses() {
        let data: Vec<u8> = (0u8..32).collect();
        let out = hexdump(0x0250_0000, &data);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "32 bytes is two 16-byte lines");
        assert!(lines[0].starts_with("02500000  00 01 02"));
        // The second line's address advances by one block, not one byte.
        assert!(lines[1].starts_with("02500010  10 11 12"));
    }

    #[test]
    fn hexdump_renders_ascii_gutter_and_pads_short_lines() {
        let out = hexdump(0, b"HI\x00\xff");
        assert!(out.contains("|HI..|"), "non-printables become dots: {out}");
        // The hex column stays 47 wide so the gutter lines up with full rows.
        assert!(out.contains("48 49 00 ff"), "got {out}");
        assert!(
            out.contains("                                   |"),
            "short row should pad: {out}"
        );
    }

    #[test]
    fn hexdump_of_erased_flash_is_recognisable() {
        // The diagnostic this exists for: an erased settings block reads 0xff.
        let out = hexdump(0x0250_0000, &[0xffu8; 16]);
        assert_eq!(
            out.trim_end(),
            "02500000  ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff ff  |................|"
        );
    }

    #[test]
    fn hexdump_of_empty_input_is_empty() {
        assert_eq!(hexdump(0x1000, &[]), "");
    }
}
