#![allow(dead_code)]
//! Claim arbitration for multiplayer games
//!
//! The host runs the arbitrator to validate claims and ensure only
//! the first claimant gets points. This provides the authoritative
//! "first claimant wins" logic for the game.

use super::validation::{validate_word, ValidationResult};
use std::collections::HashMap;

/// Result of attempting to claim a word
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimResult {
    /// Claim accepted - word is valid and unclaimed
    Accepted {
        /// Points awarded for the word
        points: u32,
        /// Monotonic sequence number for CRDT ordering
        claim_sequence: u64,
    },
    /// Claim rejected - word was already claimed
    AlreadyClaimed { by: String },
    /// Claim rejected - word is too short
    TooShort,
    /// Claim rejected - word uses invalid letters
    InvalidLetters { missing: Vec<char> },
    /// Claim rejected - word not in dictionary
    NotInDictionary,
    /// Claim rejected - round has ended
    RoundEnded,
}

/// Tracks claimed words and player scores during a round
pub struct RoundArbitrator {
    /// The letter rack for this round
    letters: Vec<char>,
    /// Words claimed this round, mapping word -> claimant
    claimed_words: HashMap<String, String>,
    /// Player scores
    scores: HashMap<String, u32>,
    /// Whether the round is still active
    round_active: bool,
    /// Monotonic counter for claim ordering (for CRDT log)
    claim_sequence: u64,
}

impl RoundArbitrator {
    /// Create a new arbitrator for a round
    pub fn new(letters: Vec<char>, players: &[String]) -> Self {
        let mut scores = HashMap::new();
        for player in players {
            scores.insert(player.clone(), 0);
        }

        Self {
            letters,
            claimed_words: HashMap::new(),
            scores,
            round_active: true,
            claim_sequence: 0,
        }
    }

    /// Attempt to claim a word for a player
    pub fn try_claim(&mut self, word: &str, player_name: &str) -> ClaimResult {
        // Check if round is still active
        if !self.round_active {
            return ClaimResult::RoundEnded;
        }

        let word_upper = word.to_uppercase();

        // Check if already claimed
        if let Some(claimed_by) = self.claimed_words.get(&word_upper) {
            return ClaimResult::AlreadyClaimed {
                by: claimed_by.clone(),
            };
        }

        // Validate the word
        let result = validate_word(&word_upper, &self.letters);
        match result {
            ValidationResult::Valid => {
                // Word is valid and unclaimed - accept the claim
                let points = word_upper.len() as u32;

                // Record the claim
                self.claimed_words
                    .insert(word_upper, player_name.to_string());

                // Update player's score
                *self.scores.entry(player_name.to_string()).or_insert(0) += points;

                // Increment and return sequence number for CRDT ordering
                self.claim_sequence += 1;
                ClaimResult::Accepted {
                    points,
                    claim_sequence: self.claim_sequence,
                }
            }
            ValidationResult::TooShort { .. } => ClaimResult::TooShort,
            ValidationResult::InvalidLetters { missing } => {
                ClaimResult::InvalidLetters { missing }
            }
            ValidationResult::NotInDictionary => ClaimResult::NotInDictionary,
        }
    }

    /// End the round (no more claims accepted)
    pub fn end_round(&mut self) {
        self.round_active = false;
    }

    /// Check if round is still active
    pub fn is_active(&self) -> bool {
        self.round_active
    }

    /// Get current scores as a sorted list (highest first)
    pub fn scores(&self) -> Vec<(String, u32)> {
        let mut scores: Vec<_> = self.scores.iter().map(|(k, v)| (k.clone(), *v)).collect();
        scores.sort_by(|a, b| b.1.cmp(&a.1));
        scores
    }

    /// Get all claimed words
    pub fn claimed_words(&self) -> &HashMap<String, String> {
        &self.claimed_words
    }

    /// Get a player's score
    pub fn player_score(&self, player_name: &str) -> u32 {
        *self.scores.get(player_name).unwrap_or(&0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_letters() -> Vec<char> {
        vec!['C', 'A', 'T', 'D', 'O', 'G', 'E', 'R', 'S', 'T', 'A', 'N']
    }

    fn test_players() -> Vec<String> {
        vec!["Alice".to_string(), "Bob".to_string()]
    }

    #[test]
    fn test_first_claim_wins() {
        let mut arb = RoundArbitrator::new(test_letters(), &test_players());

        // Alice claims CAT first
        let result = arb.try_claim("cat", "Alice");
        assert!(matches!(result, ClaimResult::Accepted { points: 3, claim_sequence: 1 }));

        // Bob tries to claim CAT - rejected
        let result = arb.try_claim("cat", "Bob");
        assert!(matches!(
            result,
            ClaimResult::AlreadyClaimed { by } if by == "Alice"
        ));
    }

    #[test]
    fn test_different_words_can_be_claimed() {
        let mut arb = RoundArbitrator::new(test_letters(), &test_players());

        // Alice claims CAT
        let result = arb.try_claim("cat", "Alice");
        assert!(matches!(result, ClaimResult::Accepted { points: 3, claim_sequence: 1 }));

        // Bob claims DOG
        let result = arb.try_claim("dog", "Bob");
        assert!(matches!(result, ClaimResult::Accepted { points: 3, claim_sequence: 2 }));
    }

    #[test]
    fn test_scores_tracked() {
        let mut arb = RoundArbitrator::new(test_letters(), &test_players());

        arb.try_claim("cat", "Alice"); // 3 points
        arb.try_claim("dog", "Bob"); // 3 points
        arb.try_claim("dogs", "Alice"); // 4 points

        assert_eq!(arb.player_score("Alice"), 7);
        assert_eq!(arb.player_score("Bob"), 3);
    }

    #[test]
    fn test_invalid_word_rejected() {
        let mut arb = RoundArbitrator::new(test_letters(), &test_players());

        // Word too short
        let result = arb.try_claim("at", "Alice");
        assert!(matches!(result, ClaimResult::TooShort));

        // Word with missing letters
        let result = arb.try_claim("xyz", "Alice");
        assert!(matches!(result, ClaimResult::InvalidLetters { .. }));

        // Word not in dictionary
        let result = arb.try_claim("tac", "Alice");
        assert!(matches!(result, ClaimResult::NotInDictionary));
    }

    #[test]
    fn test_round_ended() {
        let mut arb = RoundArbitrator::new(test_letters(), &test_players());

        arb.end_round();

        let result = arb.try_claim("cat", "Alice");
        assert!(matches!(result, ClaimResult::RoundEnded));
    }

    #[test]
    fn test_case_insensitive() {
        let mut arb = RoundArbitrator::new(test_letters(), &test_players());

        arb.try_claim("CAT", "Alice");

        // Same word in lowercase should be rejected
        let result = arb.try_claim("cat", "Bob");
        assert!(matches!(
            result,
            ClaimResult::AlreadyClaimed { by } if by == "Alice"
        ));
    }

    #[test]
    fn test_scores_sorted() {
        let mut arb = RoundArbitrator::new(test_letters(), &test_players());

        arb.try_claim("cat", "Bob"); // 3 points
        arb.try_claim("dogs", "Alice"); // 4 points

        let scores = arb.scores();
        assert_eq!(scores[0].0, "Alice");
        assert_eq!(scores[0].1, 4);
        assert_eq!(scores[1].0, "Bob");
        assert_eq!(scores[1].1, 3);
    }
}
