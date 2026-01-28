use std::path::{Path, PathBuf};
use std::collections::HashMap;

pub fn format_dry_run(groups: &HashMap<PathBuf, Vec<PathBuf>>, output_dir: &Path) -> String {
    let mut output = String::new();
    output.push_str("Dry Run Results:\n");
    output.push_str("----------------\n");

    // Sort keys for consistent output
    let mut dirs: Vec<_> = groups.keys().collect();
    dirs.sort();

    for dir in dirs {
        let files = &groups[dir];
        let dir_name = dir.file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_else(|| std::borrow::Cow::from("merged"));
            
        let output_file = output_dir.join(format!("{}.mp3", dir_name));
        
        output.push_str(&format!("Source: {}\n", dir.display()));
        output.push_str(&format!("Output: {}\n", output_file.display()));
        output.push_str("Files to merge:\n");
        for file in files {
            output.push_str(&format!("  - {}\n", file.file_name().unwrap_or_default().to_string_lossy()));
        }
        output.push_str("\n");
    }
    
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_dry_run() {
        let mut groups = HashMap::new();
        groups.insert(PathBuf::from("src/book1"), vec![
            PathBuf::from("src/book1/chap1.mp3"),
            PathBuf::from("src/book1/chap2.mp3"),
        ]);
        
        let output = format_dry_run(&groups, Path::new("out_dir"));
        
        assert!(output.contains("Source: src/book1"));
        assert!(output.contains("Output: out_dir/book1.mp3")); // Separator might differ on Windows but we are on unix-like for now
        assert!(output.contains("  - chap1.mp3"));
        assert!(output.contains("  - chap2.mp3"));
    }
}
