use std::path::{Path, PathBuf};
use std::collections::HashMap;
use walkdir::WalkDir;

pub fn scan_audio_files(root: &Path) -> HashMap<PathBuf, Vec<PathBuf>> {
    let mut groups: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();

    for result in WalkDir::new(root) {
        let entry = match result {
            Ok(e) => e,
            Err(err) => {
                eprintln!("Error accessing file: {}", err);
                // TODO: Integrate 'on_error' strategy logic here (Halt/Skip/Prompt)
                continue;
            }
        };

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
            let a_str = a.file_name().unwrap_or_default().to_string_lossy();
            let b_str = b.file_name().unwrap_or_default().to_string_lossy();
            compare_natural(&a_str, &b_str)
        });
    }

    groups
}

fn compare_natural(s1: &str, s2: &str) -> std::cmp::Ordering {
    let chunks1 = get_chunks(s1);
    let chunks2 = get_chunks(s2);

    let len = std::cmp::min(chunks1.len(), chunks2.len());

    for i in 0..len {
        match (&chunks1[i], &chunks2[i]) {
            (Chunk::Numeric(n1, len1), Chunk::Numeric(n2, len2)) => {
                match n1.cmp(n2) {
                    std::cmp::Ordering::Equal => {
                        // If values are equal, verify leading zeros via length or string
                        // e.g. "01" (1) vs "1" (1).
                        // If we want "01" < "1", then longer string means more zeros? No.
                        // "01" length 2. "1" length 1.
                        // If we treat "01" as coming BEFORE "1", then len1 > len2 => Less?
                        // Actually, purely numeric equality is usually enough, but for stability:
                        match len2.cmp(len1) { // Longer string (more leading zeros) usually comes first? or last?
                             // "01" vs "1". 1 == 1.
                             // Alphabetical: "01" < "1".
                             // So longer length (due to zeros) -> starts with 0.
                             // Let's just compare the raw substrings if values are equal?
                             // But we didn't save raw substrings in Chunk.
                             // Let's assume equal value is Equal for now, or use length as tie breaker.
                             // If len1 > len2 ("01" vs "1"), "01" has more zeros.
                             // "01" < "1" lexicographically.
                             // So larger length -> Less?
                             // "001" (3) vs "01" (2). "001" < "01".
                             // So yes, larger length -> Less.
                             std::cmp::Ordering::Equal => continue,
                             ord => return ord,
                        }
                    },
                    ord => return ord,
                }
            },
            (Chunk::Text(t1), Chunk::Text(t2)) => {
                // Case-insensitive comparison for filenames
                match t1.to_lowercase().cmp(&t2.to_lowercase()) {
                    std::cmp::Ordering::Equal => {
                        // Tie-break with case-sensitive
                         match t1.cmp(t2) {
                             std::cmp::Ordering::Equal => continue,
                             ord => return ord,
                         }
                    },
                    ord => return ord,
                }
            },
            (Chunk::Numeric(_, _), Chunk::Text(_)) => {
                // Numbers usually come before letters in file systems
                return std::cmp::Ordering::Less; 
            },
            (Chunk::Text(_), Chunk::Numeric(_, _)) => {
                return std::cmp::Ordering::Greater;
            }
        }
    }

    chunks1.len().cmp(&chunks2.len())
}

#[derive(Debug)]
enum Chunk {
    Text(String),
    Numeric(u64, usize), // Value, Original Length (for leading zero tie-break)
}

fn get_chunks(s: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut current_digits = String::new();
    let mut current_text = String::new();

    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            if !current_text.is_empty() {
                chunks.push(Chunk::Text(current_text.clone()));
                current_text.clear();
            }
            current_digits.push(c);
        } else {
            if !current_digits.is_empty() {
                if let Ok(num) = current_digits.parse::<u64>() {
                    chunks.push(Chunk::Numeric(num, current_digits.len()));
                } else {
                    // Fallback for overflow (very long numbers treated as text)
                    chunks.push(Chunk::Text(current_digits.clone()));
                }
                current_digits.clear();
            }
            current_text.push(c);
        }
    }

    // Flush
    if !current_digits.is_empty() {
        if let Ok(num) = current_digits.parse::<u64>() {
            chunks.push(Chunk::Numeric(num, current_digits.len()));
        } else {
            chunks.push(Chunk::Text(current_digits));
        }
    }
    if !current_text.is_empty() {
        chunks.push(Chunk::Text(current_text));
    }

    chunks
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
        
        // Scenario 1: Simple numbers
        File::create(root.join("1.mp3")).unwrap();
        File::create(root.join("10.mp3")).unwrap();
        File::create(root.join("2.mp3")).unwrap();

        // Scenario 2: Mixed text and numbers with leading zeros
        File::create(root.join("file 01.mp3")).unwrap();
        File::create(root.join("file 02.mp3")).unwrap();
        File::create(root.join("file 3.mp3")).unwrap();
        File::create(root.join("file 04.mp3")).unwrap();

        let groups = scan_audio_files(root);
        let files = groups.get(root).unwrap();

        assert_eq!(files.len(), 7);
        // '1' < 'f', so simple numbers come first
        assert!(files[0].ends_with("1.mp3"));
        assert!(files[1].ends_with("2.mp3"));
        assert!(files[2].ends_with("10.mp3"));
        
        assert!(files[3].ends_with("file 01.mp3"));
        assert!(files[4].ends_with("file 02.mp3"));
        assert!(files[5].ends_with("file 3.mp3"));
        assert!(files[6].ends_with("file 04.mp3"));
    }
}
