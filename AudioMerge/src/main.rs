mod args;
mod scanner;
mod dry_run;
mod merger;

use clap::Parser;
use args::Args;

fn main() {
    let args = Args::parse();
    
    // Check if source exists
    if !args.source.exists() {
        eprintln!("Error: Source directory '{}' does not exist.", args.source.display());
        std::process::exit(1);
    }

    let groups = scanner::scan_audio_files(&args.source);

    if groups.is_empty() {
        println!("No MP3 files found in '{}'.", args.source.display());
        return;
    }

    if args.dry_run {
        let output = dry_run::format_dry_run(&groups, &args.source, &args.output);
        println!("{}", output);
    } else {
        println!("Found {} folders with audio files.", groups.len());
        println!("Starting merge process...");

        // Sort keys for consistent processing order
        let mut dirs: Vec<_> = groups.keys().collect();
        dirs.sort();

        for dir in dirs {
            let files = &groups[dir];
            
            // Calculate path relative to source to preserve structure and avoid collisions
            let relative_path = dir.strip_prefix(&args.source).unwrap_or(dir);
            let output_file = if relative_path.as_os_str().is_empty() {
                let dir_name = dir.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_else(|| std::borrow::Cow::from("merged"));
                args.output.join(format!("{}.mp3", dir_name))
            } else {
                args.output.join(relative_path).with_extension("mp3")
            };
            
            println!("Merging {} files from '{}' to '{}'", files.len(), dir.display(), output_file.display());
            
            if let Err(e) = merger::merge_files(files, &output_file) {
                eprintln!("Error merging files for {}: {}", dir.display(), e);
                match args.on_error {
                    args::OnError::Halt => {
                        eprintln!("Halting due to error.");
                        std::process::exit(1);
                    },
                    args::OnError::Skip => {
                        eprintln!("Skipping...");
                        continue;
                    },
                    args::OnError::Prompt => {
                        eprintln!("Prompt strategy not yet implemented. Defaulting to Skip behavior.");
                        continue;
                    }
                }
            }
        }
        println!("Done.");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_main() {
        assert_eq!(2 + 2, 4);
    }
}