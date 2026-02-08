#![allow(dead_code)]
//! Word validation for BLAM! game
//!
//! Validates submitted words against:
//! - Minimum length (3 characters)
//! - Letter availability in rack (with multiplicity)
//! - Dictionary presence

use super::dictionary;

/// Minimum word length for valid submissions
pub const MIN_WORD_LENGTH: usize = 3;

/// Result of word validation with specific error messages
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationResult {
    /// Word is valid
    Valid,
    /// Word is too short (less than 3 characters)
    TooShort { length: usize },
    /// Word uses letters not available in the rack
    InvalidLetters { missing: Vec<char> },
    /// Word not found in dictionary
    NotInDictionary,
}

impl ValidationResult {
    /// Returns true if the word is valid
    pub fn is_valid(&self) -> bool {
        matches!(self, ValidationResult::Valid)
    }

    /// Returns a user-friendly error message
    pub fn message(&self) -> String {
        match self {
            ValidationResult::Valid => "Valid word!".to_string(),
            ValidationResult::TooShort { length } => {
                format!("Too short ({} chars, need {}+)", length, MIN_WORD_LENGTH)
            }
            ValidationResult::InvalidLetters { missing } => {
                let letters: String = missing.iter().collect();
                format!("Missing letters: {}", letters)
            }
            ValidationResult::NotInDictionary => "Not in dictionary".to_string(),
        }
    }
}

/// Validate a word against the rack and dictionary
///
/// Checks in order:
/// 1. Length >= 3
/// 2. All letters available in rack (with multiplicity)
/// 3. Word exists in dictionary
pub fn validate_word(word: &str, rack: &[char]) -> ValidationResult {
    let word_upper = word.to_uppercase();

    // Check minimum length
    if word_upper.len() < MIN_WORD_LENGTH {
        return ValidationResult::TooShort {
            length: word_upper.len(),
        };
    }

    // Check letters are available in rack (with multiplicity)
    if let Some(missing) = check_letters_available(&word_upper, rack) {
        return ValidationResult::InvalidLetters { missing };
    }

    // Check word is in dictionary
    if !dictionary::is_valid_word(&word_upper) {
        return ValidationResult::NotInDictionary;
    }

    ValidationResult::Valid
}

/// Check if all letters in word are available in rack (respecting multiplicity)
/// Returns None if valid, Some(missing_letters) if invalid
fn check_letters_available(word: &str, rack: &[char]) -> Option<Vec<char>> {
    let mut available: Vec<char> = rack.to_vec();
    let mut missing: Vec<char> = Vec::new();

    for c in word.chars() {
        if let Some(pos) = available.iter().position(|&r| r == c) {
            available.remove(pos);
        } else {
            missing.push(c);
        }
    }

    if missing.is_empty() {
        None
    } else {
        // Deduplicate missing letters while preserving order
        let mut seen = std::collections::HashSet::new();
        missing.retain(|c| seen.insert(*c));
        Some(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_word() {
        let rack = ['C', 'A', 'T', 'D', 'O', 'G', 'E', 'R', 'S', 'T', 'A', 'N'];
        assert_eq!(validate_word("cat", &rack), ValidationResult::Valid);
        assert_eq!(validate_word("CAT", &rack), ValidationResult::Valid);
        assert_eq!(validate_word("dog", &rack), ValidationResult::Valid);
    }

    #[test]
    fn test_too_short() {
        let rack = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L'];
        assert_eq!(
            validate_word("ab", &rack),
            ValidationResult::TooShort { length: 2 }
        );
        assert_eq!(
            validate_word("a", &rack),
            ValidationResult::TooShort { length: 1 }
        );
        assert_eq!(
            validate_word("", &rack),
            ValidationResult::TooShort { length: 0 }
        );
    }

    #[test]
    fn test_missing_letters() {
        let rack = ['C', 'A', 'T', 'E', 'R', 'S', 'N', 'O', 'I', 'L', 'D', 'P'];
        // "xyz" uses letters not in rack
        let result = validate_word("xyz", &rack);
        match result {
            ValidationResult::InvalidLetters { missing } => {
                assert!(missing.contains(&'X'));
                assert!(missing.contains(&'Y'));
                assert!(missing.contains(&'Z'));
            }
            _ => panic!("Expected InvalidLetters, got {:?}", result),
        }
    }

    #[test]
    fn test_multiplicity_respected() {
        // Rack has only one 'L'
        let rack = ['H', 'E', 'L', 'O', 'W', 'R', 'D', 'A', 'T', 'S', 'I', 'N'];
        // "hello" needs two L's
        let result = validate_word("hello", &rack);
        match result {
            ValidationResult::InvalidLetters { missing } => {
                assert_eq!(missing, vec!['L']);
            }
            _ => panic!("Expected InvalidLetters for missing L, got {:?}", result),
        }
    }

    #[test]
    fn test_not_in_dictionary() {
        let rack = ['X', 'Y', 'Z', 'Z', 'Y', 'P', 'L', 'U', 'G', 'H', 'A', 'B'];
        assert_eq!(
            validate_word("xyzzy", &rack),
            ValidationResult::NotInDictionary
        );
    }

    #[test]
    fn test_validation_order() {
        // Test that validation fails on first check
        let rack = ['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L'];

        // Too short takes precedence over missing letters
        let result = validate_word("zz", &rack);
        assert!(matches!(result, ValidationResult::TooShort { .. }));
    }

    #[test]
    fn test_message_format() {
        assert_eq!(ValidationResult::Valid.message(), "Valid word!");
        assert_eq!(
            ValidationResult::TooShort { length: 2 }.message(),
            "Too short (2 chars, need 3+)"
        );
        assert_eq!(
            ValidationResult::InvalidLetters {
                missing: vec!['X', 'Y']
            }
            .message(),
            "Missing letters: XY"
        );
        assert_eq!(
            ValidationResult::NotInDictionary.message(),
            "Not in dictionary"
        );
    }
}
