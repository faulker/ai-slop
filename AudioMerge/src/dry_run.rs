use std::path::{Path, PathBuf};
use std::collections::HashMap;

pub fn format_dry_run(groups: &HashMap<PathBuf, Vec<PathBuf>>, source_root: &Path, output_dir: &Path) -> String {
    let mut output = String::new();
    output.push_str("Dry Run Results:\n");
    output.push_str("----------------\n");

    // Sort keys for consistent output
    let mut dirs: Vec<_> = groups.keys().collect();
    dirs.sort();

    for dir in dirs {
        let files = &groups[dir];
        let relative_path = dir.strip_prefix(source_root).unwrap_or(dir);
        
        let output_file = if relative_path.as_os_str().is_empty() {
            let dir_name = dir.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_else(|| std::borrow::Cow::from("merged"));
            output_dir.join(format!("{}.mp3", dir_name))
        } else {
            output_dir.join(relative_path).with_extension("mp3")
        };
        
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
        let source_root = PathBuf::from("src");
        groups.insert(source_root.join("book1"), vec![
            source_root.join("book1/chap1.mp3"),
            source_root.join("book1/chap2.mp3"),
        ]);
        
        let output = format_dry_run(&groups, &source_root, Path::new("out_dir"));
        
        assert!(output.contains("Source: src/book1"));
        assert!(output.contains("Output: out_dir/book1.mp3"));
        assert!(output.contains("  - chap1.mp3"));
        assert!(output.contains("  - chap2.mp3"));
    }
}
