mod user_dict;

use std::sync::Arc;

use harper_core::linting::{LintGroup, Linter};
use harper_core::parsers::PlainEnglish;
use harper_core::spell::{FstDictionary, MergedDictionary, MutableDictionary};
use harper_core::{DictWordMetadata, Dialect, DialectFlags, Document};

use user_dict::UserDict;

#[swift_bridge::bridge]
mod ffi {
    extern "Rust" {
        type SpellEngine;

        #[swift_bridge(init)]
        fn new() -> SpellEngine;

        fn lint_text(&mut self, text: &str) -> LintResults;
        fn add_user_word(&mut self, word: &str);
        fn remove_user_word(&mut self, word: &str);
        fn is_degraded(&self) -> bool;
    }

    extern "Rust" {
        type LintResults;

        fn count(&self) -> usize;
        fn error_type(&self, index: usize) -> String;
        fn message(&self, index: usize) -> String;
        fn start_offset(&self, index: usize) -> usize;
        fn end_offset(&self, index: usize) -> usize;
        fn suggestion_count(&self, index: usize) -> usize;
        fn suggestion(&self, lint_index: usize, suggestion_index: usize) -> String;
    }
}

// MARK: - LintResults (opaque wrapper to avoid Vec<Struct> FFI limitation)

struct LintResultItem {
    error_type: String,
    message: String,
    start_offset: usize,
    end_offset: usize,
    suggestions: Vec<String>,
}

pub struct LintResults {
    items: Vec<LintResultItem>,
}

impl LintResults {
    fn count(&self) -> usize {
        self.items.len()
    }
    fn error_type(&self, index: usize) -> String {
        self.items.get(index).map(|i| i.error_type.clone()).unwrap_or_default()
    }
    fn message(&self, index: usize) -> String {
        self.items.get(index).map(|i| i.message.clone()).unwrap_or_default()
    }
    fn start_offset(&self, index: usize) -> usize {
        self.items.get(index).map(|i| i.start_offset).unwrap_or(0)
    }
    fn end_offset(&self, index: usize) -> usize {
        self.items.get(index).map(|i| i.end_offset).unwrap_or(0)
    }
    fn suggestion_count(&self, index: usize) -> usize {
        self.items.get(index).map(|i| i.suggestions.len()).unwrap_or(0)
    }
    fn suggestion(&self, lint_index: usize, suggestion_index: usize) -> String {
        self.items
            .get(lint_index)
            .and_then(|i| i.suggestions.get(suggestion_index))
            .cloned()
            .unwrap_or_default()
    }
}

// MARK: - SpellEngine

pub struct SpellEngine {
    linter: Option<LintGroup>,
    dictionary: Option<MergedDictionary>,
    parser: PlainEnglish,
    user_dict: Option<UserDict>,
    dialect: Dialect,
    degraded: bool,
}

impl SpellEngine {
    fn new() -> Self {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let dialect = Dialect::American;
            let user_dict = UserDict::load();
            let dictionary = Self::build_dictionary(user_dict.words(), dialect);
            let linter = LintGroup::new_curated(Arc::new(dictionary.clone()), dialect);

            SpellEngine {
                linter: Some(linter),
                dictionary: Some(dictionary),
                parser: PlainEnglish,
                user_dict: Some(user_dict),
                dialect,
                degraded: false,
            }
        }));

        match result {
            Ok(engine) => engine,
            Err(e) => {
                eprintln!("[spell-i-engine] SpellEngine::new() panicked: {:?}", e);
                SpellEngine {
                    linter: None,
                    dictionary: None,
                    parser: PlainEnglish,
                    user_dict: None,
                    dialect: Dialect::American,
                    degraded: true,
                }
            }
        }
    }

    fn is_degraded(&self) -> bool {
        self.degraded
    }

    fn lint_text(&mut self, text: &str) -> LintResults {
        if text.is_empty() || self.degraded {
            return LintResults { items: Vec::new() };
        }

        if self.linter.is_none() || self.dictionary.is_none() {
            return LintResults { items: Vec::new() };
        }

        // Scope the mutable/immutable borrows so they're released after catch_unwind,
        // allowing us to set self.degraded on panic.
        let result = {
            let linter = self.linter.as_mut().unwrap();
            let dictionary = self.dictionary.as_ref().unwrap();
            let parser = &self.parser;
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let document = Document::new(text, parser, dictionary);
                linter.lint(&document)
            }))
        };

        let lints = match result {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[spell-i-engine] Linter panicked: {:?}", e);
                // Mark as degraded — the linter may be in an inconsistent state
                self.degraded = true;
                self.linter = None;
                return LintResults { items: Vec::new() };
            }
        };

        let items = lints
            .into_iter()
            .map(|lint| {
                let suggestions = lint
                    .suggestions
                    .iter()
                    .filter_map(|s| {
                        s.as_replace_with()
                            .map(|chars| chars.iter().collect::<String>())
                    })
                    .collect();

                LintResultItem {
                    error_type: format!("{:?}", lint.lint_kind),
                    message: lint.message,
                    start_offset: lint.span.start,
                    end_offset: lint.span.end,
                    suggestions,
                }
            })
            .collect();

        LintResults { items }
    }

    fn add_user_word(&mut self, word: &str) {
        if let Some(ref mut ud) = self.user_dict {
            ud.add(word);
            self.rebuild_linter();
        }
    }

    fn remove_user_word(&mut self, word: &str) {
        if let Some(ref mut ud) = self.user_dict {
            ud.remove(word);
            self.rebuild_linter();
        }
    }

    fn rebuild_linter(&mut self) {
        if let Some(ref ud) = self.user_dict {
            let dictionary = Self::build_dictionary(ud.words(), self.dialect);
            self.linter = Some(LintGroup::new_curated(Arc::new(dictionary.clone()), self.dialect));
            self.dictionary = Some(dictionary);
        }
    }

    fn build_dictionary(user_words: Vec<String>, dialect: Dialect) -> MergedDictionary {
        let mut merged = MergedDictionary::new();
        merged.add_dictionary(FstDictionary::curated());

        if !user_words.is_empty() {
            let mut user_mut = MutableDictionary::new();
            let dialect_flags = DialectFlags::from_dialect(dialect);
            for word in &user_words {
                user_mut.append_word_str(
                    word,
                    DictWordMetadata {
                        dialects: dialect_flags,
                        ..Default::default()
                    },
                );
            }
            merged.add_dictionary(Arc::from(user_mut));
        }

        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lint_misspelled_word() {
        let mut engine = SpellEngine::new();
        let results = engine.lint_text("I havv a speling eror.");
        assert!(results.count() > 0, "Should detect spelling errors");
        let has_suggestions = (0..results.count()).any(|i| results.suggestion_count(i) > 0);
        assert!(has_suggestions, "Should provide suggestions for misspellings");
    }

    #[test]
    fn test_correct_text() {
        let mut engine = SpellEngine::new();
        let results = engine.lint_text("The quick brown fox jumps over the lazy dog.");
        assert_eq!(results.count(), 0, "Correct text should produce no lints");
    }

    #[test]
    fn test_empty_text() {
        let mut engine = SpellEngine::new();
        let results = engine.lint_text("");
        assert_eq!(results.count(), 0, "Empty text should produce no lints");
    }

    #[test]
    fn test_unicode_text() {
        let mut engine = SpellEngine::new();
        let results = engine.lint_text("The café serves naïve customers résumés.");
        assert_eq!(results.count(), 0, "Correct Unicode text should produce no lints");
    }

    #[test]
    fn test_lint_result_fields() {
        let mut engine = SpellEngine::new();
        let results = engine.lint_text("I havv a problem.");
        if results.count() > 0 {
            assert!(!results.error_type(0).is_empty());
            assert!(!results.message(0).is_empty());
            assert!(results.start_offset(0) < results.end_offset(0));
        }
    }

    #[test]
    fn test_add_user_word_suppresses_lint() {
        let mut engine = SpellEngine::new();
        engine.remove_user_word("speling");

        // Use a reliably detectable misspelled word
        let word = "speling";
        let text = format!("I have a {} problem.", word);
        let before = engine.lint_text(&text);
        for i in 0..before.count() {
            println!("Found lint: {} at {}", before.message(i), before.start_offset(i));
        }
        let before_count = (0..before.count())
            .filter(|&i| {
                let s = before.start_offset(i);
                s >= 9 && s < 9 + word.len()
            })
            .count();
        assert!(before_count > 0, "{} should be flagged as misspelled", word);

        engine.add_user_word(word);

        let after = engine.lint_text(&text);
        let after_count = (0..after.count())
            .filter(|&i| {
                let s = after.start_offset(i);
                s >= 2 && s < 2 + word.len()
            })
            .count();

        assert_eq!(
            after_count, 0,
            "Adding word to dictionary should suppress its lint",
        );
    }

    #[test]
    fn test_lint_multiple_errors() {
        let mut engine = SpellEngine::new();
        let results = engine.lint_text("Thsi is a tset with multiple erors.");
        assert!(results.count() >= 3, "Should detect multiple errors");
    }

    #[test]
    fn test_punctuation_handling() {
        let mut engine = SpellEngine::new();
        let results = engine.lint_text("Hello, world! Thsi is a test.");
        assert!(results.count() > 0, "Should detect error after punctuation");
        assert_eq!(results.suggestion(0, 0), "This");
    }

    #[test]
    fn test_engine_new_does_not_panic() {
        let engine = SpellEngine::new();
        assert!(!engine.is_degraded(), "Normal construction should not be degraded");
    }

    #[test]
    fn test_degraded_engine_returns_empty_results() {
        let mut engine = SpellEngine {
            linter: None,
            dictionary: None,
            parser: PlainEnglish,
            user_dict: None,
            dialect: Dialect::American,
            degraded: true,
        };
        assert!(engine.is_degraded(), "Should be marked as degraded");

        let results = engine.lint_text("I havv a speling eror.");
        assert_eq!(results.count(), 0, "Degraded engine should return no lints");

        // add/remove should not panic on degraded engine
        engine.add_user_word("test");
        engine.remove_user_word("test");
    }

    #[test]
    fn test_remove_user_word_restores_lint() {
        let mut engine = SpellEngine::new();
        engine.add_user_word("xyzzyworp");

        let while_added = engine.lint_text("I found a xyzzyworp today.");
        let no_lint = !(0..while_added.count())
            .any(|i| while_added.start_offset(i) >= 10 && while_added.end_offset(i) <= 19);

        engine.remove_user_word("xyzzyworp");

        let after_remove = engine.lint_text("I found a xyzzyworp today.");
        let has_lint_now = (0..after_remove.count())
            .any(|i| after_remove.start_offset(i) >= 10 && after_remove.end_offset(i) <= 19);

        if no_lint {
            assert!(
                has_lint_now,
                "Removing word from dictionary should restore its lint"
            );
        }
    }
}
