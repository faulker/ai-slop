use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Source directory to scan for audio files
    #[arg(short, long, default_value = ".")]
    pub source: PathBuf,

    /// Output directory for merged files
    #[arg(short, long, default_value = "output")]
    pub output: PathBuf,

    /// Perform a dry run without writing any files
    #[arg(short, long)]
    pub dry_run: bool,

    /// Error handling strategy
    #[arg(long, value_enum, default_value_t = OnError::Skip)]
    pub on_error: OnError,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq)]
pub enum OnError {
    Halt,
    Prompt,
    Skip,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_args_parsing_defaults() {
        let args = Args::parse_from(&["app"]);
        assert_eq!(args.source, PathBuf::from("."));
        assert_eq!(args.output, PathBuf::from("output"));
        assert_eq!(args.dry_run, false);
        assert_eq!(args.on_error, OnError::Skip);
    }

    #[test]
    fn test_args_parsing_custom() {
        let args = Args::parse_from(&[
            "app",
            "--source", "src_dir",
            "--output", "out_dir",
            "--dry-run",
            "--on-error", "halt",
        ]);
        assert_eq!(args.source, PathBuf::from("src_dir"));
        assert_eq!(args.output, PathBuf::from("out_dir"));
        assert_eq!(args.dry_run, true);
        assert_eq!(args.on_error, OnError::Halt);
    }
}
