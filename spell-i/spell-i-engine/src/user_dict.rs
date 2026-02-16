use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// Plain-text user dictionary stored at ~/Library/Application Support/Spell-i/dictionary.txt
/// One word per line, case-preserved.
pub struct UserDict {
    path: PathBuf,
    words: HashSet<String>,
}

impl UserDict {
    /// Load or create the user dictionary file.
    pub fn load() -> Self {
        let path = Self::dict_path();
        let words = if path.exists() {
            fs::read_to_string(&path)
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .collect()
        } else {
            HashSet::new()
        };
        UserDict { path, words }
    }

    #[cfg(test)]
    pub fn load_from(path: PathBuf) -> Self {
        let words = if path.exists() {
            fs::read_to_string(&path)
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .collect()
        } else {
            HashSet::new()
        };
        UserDict { path, words }
    }

    pub fn words(&self) -> Vec<String> {
        self.words.iter().cloned().collect()
    }

    /// Add a word (no-op if already present). Persists atomically.
    pub fn add(&mut self, word: &str) {
        let w = word.trim().to_string();
        if w.is_empty() {
            return;
        }
        
        // Case-insensitive check
        if self.words.iter().any(|existing| existing.eq_ignore_ascii_case(&w)) {
            return;
        }

        self.words.insert(w);
        let _ = self.persist();
    }

    /// Remove a word (case-insensitive match). Persists atomically.
    pub fn remove(&mut self, word: &str) {
        let before = self.words.len();
        let w_trim = word.trim();
        self.words.retain(|w| !w.eq_ignore_ascii_case(w_trim));
        
        if self.words.len() != before {
            let _ = self.persist();
        }
    }

    /// Atomic write: write to temp file, flush, sync, then rename.
    fn persist(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let tmp = self.path.with_extension("tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            for w in &self.words {
                writeln!(f, "{}", w)?;
            }
            f.flush()?;
            f.sync_all()?;
        }
        
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn dict_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home)
            .join("Library/Application Support/Spell-i/dictionary.txt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn tmp_path() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "spell-i-test-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).unwrap();
        dir.join("dictionary.txt")
    }

    #[test]
    fn test_add_and_persist() {
        let path = tmp_path();
        let _ = fs::remove_file(&path);

        let mut dict = UserDict::load_from(path.clone());
        assert!(dict.words().is_empty());

        dict.add("fluffernutter");
        let words = dict.words();
        assert_eq!(words.len(), 1);
        assert!(words.contains(&"fluffernutter".to_string()));

        // Reload from disk
        let reloaded = UserDict::load_from(path.clone());
        let reloaded_words = reloaded.words();
        assert_eq!(reloaded_words.len(), 1);
        assert!(reloaded_words.contains(&"fluffernutter".to_string()));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_remove_and_persist() {
        let path = tmp_path();
        let _ = fs::remove_file(&path);

        let mut dict = UserDict::load_from(path.clone());
        dict.add("Rustacean");
        dict.add("Swiftie");
        dict.remove("rustacean"); // case-insensitive

        let words = dict.words();
        assert_eq!(words.len(), 1);
        assert!(words.contains(&"Swiftie".to_string()));

        let reloaded = UserDict::load_from(path.clone());
        let reloaded_words = reloaded.words();
        assert_eq!(reloaded_words.len(), 1);
        assert!(reloaded_words.contains(&"Swiftie".to_string()));

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_no_duplicate_add() {
        let path = tmp_path();
        let _ = fs::remove_file(&path);

        let mut dict = UserDict::load_from(path.clone());
        dict.add("hello");
        dict.add("Hello"); // case-insensitive duplicate
        dict.add("hello");

        assert_eq!(dict.words().len(), 1);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_empty_word_ignored() {
        let path = tmp_path();
        let _ = fs::remove_file(&path);

        let mut dict = UserDict::load_from(path.clone());
        dict.add("");
        dict.add("  ");

        assert!(dict.words().is_empty());

        let _ = fs::remove_file(&path);
    }
}
