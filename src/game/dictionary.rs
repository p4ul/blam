#![allow(dead_code)]
//! Dictionary module for word validation
//!
//! Embeds SCOWL American size-60 wordlist at build time.
//! Provides O(1) hash set lookup with case-insensitive matching.

use once_cell::sync::Lazy;
use std::collections::HashSet;

/// Embedded wordlist (SCOWL American size-60, ~90K words)
/// Words are lowercase, alphabetic only, one per line
static WORDS_DATA: &str = include_str!("../../data/words.txt");

/// Pre-built hash set for O(1) word lookup
static DICTIONARY: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    WORDS_DATA.lines().collect()
});

/// Check if a word is valid in the dictionary.
/// Case-insensitive: input is converted to lowercase before lookup.
pub fn is_valid_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    DICTIONARY.contains(lower.as_str())
}

/// Returns the total number of words in the dictionary
pub fn word_count() -> usize {
    DICTIONARY.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_words() {
        assert!(is_valid_word("hello"));
        assert!(is_valid_word("world"));
        assert!(is_valid_word("game"));
        assert!(is_valid_word("word"));
    }

    #[test]
    fn test_case_insensitive() {
        assert!(is_valid_word("Hello"));
        assert!(is_valid_word("HELLO"));
        assert!(is_valid_word("HeLLo"));
    }

    #[test]
    fn test_invalid_words() {
        assert!(!is_valid_word("xyzzyplugh"));
        assert!(!is_valid_word("asdfghjkl"));
        assert!(!is_valid_word(""));
    }

    #[test]
    fn test_word_count() {
        let count = word_count();
        assert!(count > 80000, "Expected 80K+ words, got {}", count);
        assert!(count < 100000, "Expected <100K words, got {}", count);
    }

    #[test]
    fn test_three_letter_words() {
        // Common 3-letter words should be in dictionary
        assert!(is_valid_word("the"));
        assert!(is_valid_word("and"));
        assert!(is_valid_word("cat"));
        assert!(is_valid_word("dog"));
    }
}
