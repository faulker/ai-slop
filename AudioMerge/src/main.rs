mod args;
mod scanner;
mod dry_run;

use clap::Parser;
use std::path::Path;
use args::Args;

fn main() {
    let args = Args::parse();
    
    // Check if source exists
    if !Path::new(&args.source).exists() {
        eprintln!("Error: Source directory '{}' does not exist.", args.source);
        std::process::exit(1);
    }

    let groups = scanner::scan_audio_files(Path::new(&args.source));

    if groups.is_empty() {
        println!("No MP3 files found in '{}'.", args.source);
        return;
    }

    if args.dry_run {
        let output = dry_run::format_dry_run(&groups, &args.output);
        println!("{}", output);
    } else {
        println!("Found {} folders with audio files.", groups.len());
        println!("Run with --dry-run to see details.");
        // TODO: Implement actual merging
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_main() {
        assert_eq!(2 + 2, 4);
    }
}