use std::path::{Path, PathBuf};
use std::collections::HashMap;
use walkdir::WalkDir;

pub fn scan_audio_files(root: &Path) -> HashMap<PathBuf, Vec<PathBuf>> {
    let mut groups: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            // For MVP we assume mp3, but let's check extension just in case
            if let Some(ext) = entry.path().extension() {
                if ext.to_string_lossy().to_lowercase() == "mp3" {
                    if let Some(parent) = entry.path().parent() {
                        groups.entry(parent.to_path_buf())
                            .or_default()
                            .push(entry.path().to_path_buf());
                    }
                }
            }
        }
    }

    // Sort files naturally within each group
    for files in groups.values_mut() {
        files.sort_by(|a, b| {
            natord::compare(
                &a.file_name().unwrap_or_default().to_string_lossy(),
                &b.file_name().unwrap_or_default().to_string_lossy()
            )
        });
    }

    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::TempDir;

    #[test]
    fn test_scan_audio_files_recursive() {
        // Create a temporary directory structure
        // root/
        //   audio1.mp3
        //   sub/
        //     audio2.mp3
        //     audio3.mp3
        //   other.txt
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        
        let sub_dir = root.join("sub");
        fs::create_dir(&sub_dir).unwrap();

        File::create(root.join("audio1.mp3")).unwrap();
        File::create(sub_dir.join("audio2.mp3")).unwrap();
        File::create(sub_dir.join("audio3.mp3")).unwrap();
        File::create(root.join("other.txt")).unwrap(); // Should be ignored

        let groups = scan_audio_files(root);

        assert_eq!(groups.len(), 2);
        
        // Check root group
        let root_files = groups.get(root).unwrap();
        assert_eq!(root_files.len(), 1);
        assert!(root_files.iter().any(|p| p.ends_with("audio1.mp3")));

        // Check sub group
        let sub_files = groups.get(&sub_dir).unwrap();
        assert_eq!(sub_files.len(), 2);
        assert!(sub_files.iter().any(|p| p.ends_with("audio2.mp3")));
        assert!(sub_files.iter().any(|p| p.ends_with("audio3.mp3")));
    }

    #[test]
    fn test_scan_audio_files_sorting() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        
        File::create(root.join("1.mp3")).unwrap();
        File::create(root.join("10.mp3")).unwrap();
        File::create(root.join("2.mp3")).unwrap();

        let groups = scan_audio_files(root);
        let files = groups.get(root).unwrap();

        assert_eq!(files.len(), 3);
        assert!(files[0].ends_with("1.mp3"));
        assert!(files[1].ends_with("2.mp3"));
        assert!(files[2].ends_with("10.mp3"));
    }
}
