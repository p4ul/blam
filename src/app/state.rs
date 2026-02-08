//! Application state management

use crate::game::validation::{validate_word, ValidationResult};

/// Default round duration in seconds
pub const DEFAULT_ROUND_DURATION: u32 = 60;

/// A claimed word with its point value
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedWord {
    pub word: String,
    pub points: u32,
}

/// A missed word submission with the reason it failed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissedWord {
    pub word: String,
    pub reason: MissReason,
}

/// Categorized reasons for missed words
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissReason {
    TooShort,
    InvalidLetters,
    NotInDictionary,
    AlreadyClaimed { by: String },
}

impl MissReason {
    pub fn label(&self) -> &'static str {
        match self {
            MissReason::TooShort => "Too Short",
            MissReason::InvalidLetters => "Invalid Letters",
            MissReason::NotInDictionary => "Not In Dictionary",
            MissReason::AlreadyClaimed { .. } => "Already Claimed",
        }
    }
}

/// End-of-round summary statistics
#[derive(Debug, Clone, Default)]
pub struct RoundSummary {
    /// Total score for the round
    pub total_score: u32,
    /// Words successfully claimed
    pub claimed_words: Vec<ClaimedWord>,
    /// Words that failed validation, grouped by reason
    pub too_short: Vec<String>,
    pub invalid_letters: Vec<String>,
    pub not_in_dictionary: Vec<String>,
    pub already_claimed: Vec<String>,
}

/// A claim in the feed (visible to all players)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFeedEntry {
    pub player_name: String,
    pub word: String,
    pub points: u32,
}

/// Player score in multiplayer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerScore {
    pub name: String,
    pub score: u32,
}

impl RoundSummary {
    /// Total number of successful claims
    pub fn claim_count(&self) -> usize {
        self.claimed_words.len()
    }

    /// Total number of misses across all categories
    pub fn miss_count(&self) -> usize {
        self.too_short.len() + self.invalid_letters.len() + self.not_in_dictionary.len()
    }
}

/// Main application state
pub struct App {
    /// Whether the application should quit
    pub should_quit: bool,
    /// Current letter rack
    pub letters: Vec<char>,
    /// Current user input
    pub input: String,
    /// Feedback message from last submission
    pub feedback: String,
    /// Current score
    pub score: u32,
    /// Time remaining in seconds
    pub time_remaining: u32,
    /// Whether the round has ended (timer hit 0)
    pub round_ended: bool,
    /// Words claimed this round (by the local player)
    claimed_words: Vec<ClaimedWord>,
    /// Missed submissions this round
    missed_words: Vec<MissedWord>,
    /// Multiplayer scoreboard (all players)
    pub scoreboard: Vec<PlayerScore>,
    /// Recent claims feed (all players)
    pub claim_feed: Vec<ClaimFeedEntry>,
    /// Maximum entries in claim feed
    claim_feed_max: usize,
    /// Local player name (for multiplayer)
    pub player_name: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            should_quit: false,
            letters: Vec::new(),
            input: String::new(),
            feedback: String::new(),
            score: 0,
            time_remaining: DEFAULT_ROUND_DURATION,
            round_ended: false,
            claimed_words: Vec::new(),
            missed_words: Vec::new(),
            scoreboard: Vec::new(),
            claim_feed: Vec::new(),
            claim_feed_max: 10,
            player_name: None,
        }
    }
}

impl App {
    /// Create a new application instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Signal the application to quit
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Handle character input (locked when round is over)
    pub fn on_char(&mut self, c: char) {
        if self.round_ended {
            return;
        }
        self.input.push(c);
        self.feedback.clear();
    }

    /// Handle backspace (locked when round is over)
    pub fn on_backspace(&mut self) {
        if self.round_ended {
            return;
        }
        self.input.pop();
        self.feedback.clear();
    }

    /// Handle word submission (Enter key, locked when round is over)
    pub fn on_submit(&mut self) {
        if self.round_ended {
            return;
        }
        if self.input.is_empty() {
            return;
        }

        let word = self.input.clone();
        let word_upper = word.to_uppercase();
        let result = validate_word(&word, &self.letters);

        match result {
            ValidationResult::Valid => {
                let points = word.len() as u32;
                self.score += points;
                self.feedback = format!("OK +{} ({})", points, word_upper);
                self.claimed_words.push(ClaimedWord {
                    word: word_upper,
                    points,
                });
            }
            ValidationResult::TooShort { .. } => {
                self.feedback = result.message();
                self.missed_words.push(MissedWord {
                    word: word_upper,
                    reason: MissReason::TooShort,
                });
            }
            ValidationResult::InvalidLetters { .. } => {
                self.feedback = "CLANK".to_string();
                self.missed_words.push(MissedWord {
                    word: word_upper,
                    reason: MissReason::InvalidLetters,
                });
            }
            ValidationResult::NotInDictionary => {
                self.feedback = "NOPE".to_string();
                self.missed_words.push(MissedWord {
                    word: word_upper,
                    reason: MissReason::NotInDictionary,
                });
            }
        }

        self.input.clear();
    }

    /// Set the letter rack
    pub fn set_letters(&mut self, letters: Vec<char>) {
        self.letters = letters;
    }

    /// Update the timer and trigger end-of-round when it hits zero
    pub fn tick(&mut self) {
        if self.time_remaining > 0 {
            self.time_remaining -= 1;
            if self.time_remaining == 0 {
                self.end_round();
            }
        }
    }

    /// Check if the round is over
    pub fn is_round_over(&self) -> bool {
        self.round_ended
    }

    /// End the current round (locks input, triggers results)
    fn end_round(&mut self) {
        self.round_ended = true;
        self.feedback = "TIME'S UP!".to_string();
    }

    /// Force end the round (called when host signals RoundEnd)
    pub fn force_end_round(&mut self) {
        self.end_round();
    }

    /// Start a new round with given letters and duration
    pub fn start_round(&mut self, letters: Vec<char>, duration: u32) {
        self.letters = letters;
        self.time_remaining = duration;
        self.score = 0;
        self.input.clear();
        self.feedback.clear();
        self.round_ended = false;
        self.claimed_words.clear();
        self.missed_words.clear();
        self.claim_feed.clear();
        // Reset scoreboard scores but keep players
        for player in &mut self.scoreboard {
            player.score = 0;
        }
    }

    /// Set the local player name (for multiplayer)
    pub fn set_player_name(&mut self, name: String) {
        self.player_name = Some(name);
    }

    /// Initialize scoreboard with players
    pub fn set_scoreboard(&mut self, players: Vec<String>) {
        self.scoreboard = players
            .into_iter()
            .map(|name| PlayerScore { name, score: 0 })
            .collect();
    }

    /// Update scoreboard from score update message
    pub fn update_scoreboard(&mut self, scores: Vec<(String, u32)>) {
        for (name, score) in scores {
            if let Some(player) = self.scoreboard.iter_mut().find(|p| p.name == name) {
                player.score = score;
            } else {
                self.scoreboard.push(PlayerScore { name, score });
            }
        }
        // Sort by score descending
        self.scoreboard.sort_by(|a, b| b.score.cmp(&a.score));
    }

    /// Handle a claim accepted from the host (multiplayer)
    pub fn on_claim_accepted(&mut self, word: String, player_name: String, points: u32) {
        // Add to claim feed
        self.claim_feed.push(ClaimFeedEntry {
            player_name: player_name.clone(),
            word: word.clone(),
            points,
        });
        // Trim feed if too long
        while self.claim_feed.len() > self.claim_feed_max {
            self.claim_feed.remove(0);
        }

        // If it's our claim, update our state
        if self.player_name.as_ref() == Some(&player_name) {
            self.score += points;
            self.feedback = format!("OK +{} ({})", points, word);
            self.claimed_words.push(ClaimedWord {
                word,
                points,
            });
        }

        // Update scoreboard
        if let Some(player) = self.scoreboard.iter_mut().find(|p| p.name == player_name) {
            player.score += points;
        }
        // Re-sort scoreboard
        self.scoreboard.sort_by(|a, b| b.score.cmp(&a.score));
    }

    /// Handle a claim rejected from the host (multiplayer)
    pub fn on_claim_rejected(&mut self, word: String, reason: MissReason) {
        let word_upper = word.to_uppercase();
        self.feedback = match &reason {
            MissReason::TooShort => "Too short (need 3+ letters)".to_string(),
            MissReason::InvalidLetters => "CLANK".to_string(),
            MissReason::NotInDictionary => "NOPE".to_string(),
            MissReason::AlreadyClaimed { by } => format!("TOO LATE (already claimed by {})", by),
        };
        self.missed_words.push(MissedWord {
            word: word_upper,
            reason,
        });
    }

    /// Get current input for sending to host (multiplayer)
    pub fn get_pending_claim(&self) -> Option<String> {
        if self.input.is_empty() || self.round_ended {
            None
        } else {
            Some(self.input.clone())
        }
    }

    /// Clear input after sending claim attempt (multiplayer)
    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    /// Get the list of claimed words this round
    pub fn claimed_words(&self) -> &[ClaimedWord] {
        &self.claimed_words
    }

    /// Get the list of missed words this round
    pub fn missed_words(&self) -> &[MissedWord] {
        &self.missed_words
    }

    /// Generate end-of-round summary with categorized misses
    pub fn round_summary(&self) -> RoundSummary {
        let mut summary = RoundSummary {
            total_score: self.score,
            claimed_words: self.claimed_words.clone(),
            ..Default::default()
        };

        for miss in &self.missed_words {
            match &miss.reason {
                MissReason::TooShort => summary.too_short.push(miss.word.clone()),
                MissReason::InvalidLetters => summary.invalid_letters.push(miss.word.clone()),
                MissReason::NotInDictionary => summary.not_in_dictionary.push(miss.word.clone()),
                MissReason::AlreadyClaimed { .. } => summary.already_claimed.push(miss.word.clone()),
            }
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_round_duration() {
        let app = App::new();
        assert_eq!(app.time_remaining, DEFAULT_ROUND_DURATION);
        assert_eq!(DEFAULT_ROUND_DURATION, 60);
    }

    #[test]
    fn test_timer_countdown() {
        let mut app = App::new();
        app.start_round(vec!['A', 'B', 'C'], 5);
        assert_eq!(app.time_remaining, 5);
        assert!(!app.is_round_over());

        app.tick();
        assert_eq!(app.time_remaining, 4);
        assert!(!app.is_round_over());

        app.tick();
        app.tick();
        app.tick();
        assert_eq!(app.time_remaining, 1);
        assert!(!app.is_round_over());

        app.tick();
        assert_eq!(app.time_remaining, 0);
        assert!(app.is_round_over());
    }

    #[test]
    fn test_timer_triggers_end_of_round() {
        let mut app = App::new();
        app.start_round(vec!['A', 'B', 'C'], 1);

        assert!(!app.round_ended);
        app.tick();
        assert!(app.round_ended);
        assert_eq!(app.feedback, "TIME'S UP!");
    }

    #[test]
    fn test_input_locked_when_round_over() {
        let mut app = App::new();
        app.start_round(vec!['A', 'B', 'C'], 1);

        // Type during round
        app.on_char('A');
        assert_eq!(app.input, "A");

        // End round
        app.tick();
        assert!(app.is_round_over());

        // Attempt to type after round - should be ignored
        app.on_char('B');
        assert_eq!(app.input, "A"); // Still just 'A'

        // Backspace should also be ignored
        app.on_backspace();
        assert_eq!(app.input, "A"); // Still 'A'
    }

    #[test]
    fn test_submit_locked_when_round_over() {
        let mut app = App::new();
        app.start_round(vec!['A', 'B', 'C'], 1);

        // Type a word and end round before submitting
        app.on_char('C');
        app.on_char('A');
        app.on_char('B');
        app.tick(); // End round

        // Attempt to submit after round
        let score_before = app.score;
        app.on_submit();
        assert_eq!(app.score, score_before); // Score unchanged
        assert_eq!(app.input, "CAB"); // Input still there (not cleared by submit)
    }

    #[test]
    fn test_timer_does_not_go_negative() {
        let mut app = App::new();
        app.start_round(vec!['A', 'B', 'C'], 1);

        app.tick(); // 0
        app.tick(); // Should stay at 0
        app.tick(); // Should stay at 0

        assert_eq!(app.time_remaining, 0);
    }

    #[test]
    fn test_start_round_resets_state() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'B'], 5);

        // Accumulate some state - use valid 3-letter word "CAB"
        app.on_char('C');
        app.on_char('A');
        app.on_char('B');
        app.on_submit();
        app.tick();
        app.tick();
        app.tick();
        app.tick();
        app.tick(); // End round

        assert!(app.is_round_over());
        assert!(app.score > 0);

        // Start new round
        app.start_round(vec!['X', 'Y', 'Z'], 10);

        assert!(!app.is_round_over());
        assert_eq!(app.score, 0);
        assert!(app.input.is_empty());
        assert!(app.feedback.is_empty());
        assert_eq!(app.time_remaining, 10);
        assert_eq!(app.letters, vec!['X', 'Y', 'Z']);
    }

    #[test]
    fn test_claimed_words_tracked() {
        let mut app = App::new();
        // Use letters that can form "CAT" and "CAB"
        app.start_round(vec!['C', 'A', 'T', 'B', 'E', 'R', 'S', 'O', 'N', 'D', 'I', 'G'], 60);

        // Submit "CAT"
        app.on_char('C');
        app.on_char('A');
        app.on_char('T');
        app.on_submit();

        assert_eq!(app.claimed_words().len(), 1);
        assert_eq!(app.claimed_words()[0].word, "CAT");
        assert_eq!(app.claimed_words()[0].points, 3);

        // Submit "CAB"
        app.on_char('C');
        app.on_char('A');
        app.on_char('B');
        app.on_submit();

        assert_eq!(app.claimed_words().len(), 2);
        assert_eq!(app.claimed_words()[1].word, "CAB");
        assert_eq!(app.claimed_words()[1].points, 3);
    }

    #[test]
    fn test_missed_words_categorized() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'T', 'B', 'E', 'R', 'S', 'O', 'N', 'D', 'I', 'G'], 60);

        // Too short
        app.on_char('A');
        app.on_char('B');
        app.on_submit();

        // Invalid letters (Z not in rack)
        app.on_char('Z');
        app.on_char('A');
        app.on_char('P');
        app.on_submit();

        // Not in dictionary
        app.on_char('C');
        app.on_char('A');
        app.on_char('G');
        app.on_submit();

        let summary = app.round_summary();
        assert_eq!(summary.too_short.len(), 1);
        assert_eq!(summary.too_short[0], "AB");

        assert_eq!(summary.invalid_letters.len(), 1);
        assert_eq!(summary.invalid_letters[0], "ZAP");

        assert_eq!(summary.not_in_dictionary.len(), 1);
        assert_eq!(summary.not_in_dictionary[0], "CAG");
    }

    #[test]
    fn test_round_summary_totals() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'T', 'B', 'E', 'R', 'S', 'O', 'N', 'D', 'I', 'G'], 60);

        // Valid words
        app.on_char('C');
        app.on_char('A');
        app.on_char('T');
        app.on_submit();

        app.on_char('D');
        app.on_char('O');
        app.on_char('G');
        app.on_submit();

        // Some misses
        app.on_char('X');
        app.on_char('Y');
        app.on_char('Z');
        app.on_submit();

        let summary = app.round_summary();
        assert_eq!(summary.total_score, 6); // CAT(3) + DOG(3)
        assert_eq!(summary.claim_count(), 2);
        assert_eq!(summary.miss_count(), 1);
    }

    #[test]
    fn test_start_round_clears_tracking() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'T', 'B', 'E', 'R', 'S', 'O', 'N', 'D', 'I', 'G'], 60);

        // Accumulate some words
        app.on_char('C');
        app.on_char('A');
        app.on_char('T');
        app.on_submit();

        app.on_char('X');
        app.on_char('Y');
        app.on_char('Z');
        app.on_submit();

        assert!(!app.claimed_words().is_empty());
        assert!(!app.missed_words().is_empty());

        // Start new round
        app.start_round(vec!['A', 'B', 'C'], 30);

        assert!(app.claimed_words().is_empty());
        assert!(app.missed_words().is_empty());
    }

    #[test]
    fn test_points_per_letter() {
        let mut app = App::new();
        app.start_round(
            vec!['C', 'A', 'T', 'S', 'E', 'R', 'A', 'T', 'E', 'D', 'O', 'G'],
            60,
        );

        // 3-letter word: 3 points
        app.on_char('C');
        app.on_char('A');
        app.on_char('T');
        app.on_submit();
        assert_eq!(app.score, 3);

        // 4-letter word: 4 points (total: 7)
        app.on_char('C');
        app.on_char('A');
        app.on_char('T');
        app.on_char('S');
        app.on_submit();
        assert_eq!(app.score, 7);
    }

    #[test]
    fn test_miss_reason_labels() {
        assert_eq!(MissReason::TooShort.label(), "Too Short");
        assert_eq!(MissReason::InvalidLetters.label(), "Invalid Letters");
        assert_eq!(MissReason::NotInDictionary.label(), "Not In Dictionary");
    }

    #[test]
    fn test_claim_feedback_ok() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'T', 'D', 'O', 'G', 'E', 'R', 'S', 'T', 'A', 'N'], 60);
        app.on_char('C');
        app.on_char('A');
        app.on_char('T');
        app.on_submit();
        assert_eq!(app.feedback, "OK +3 (CAT)");
    }

    #[test]
    fn test_claim_feedback_nope() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'G', 'D', 'O', 'T', 'E', 'R', 'S', 'T', 'A', 'N'], 60);
        app.on_char('C');
        app.on_char('A');
        app.on_char('G');
        app.on_submit();
        assert_eq!(app.feedback, "NOPE");
    }

    #[test]
    fn test_claim_feedback_clank() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'T', 'D', 'O', 'G', 'E', 'R', 'S', 'T', 'A', 'N'], 60);
        app.on_char('Z');
        app.on_char('A');
        app.on_char('P');
        app.on_submit();
        assert_eq!(app.feedback, "CLANK");
    }

    #[test]
    fn test_claim_feedback_too_late() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'T'], 60);
        app.on_claim_rejected("CAT".to_string(), MissReason::AlreadyClaimed { by: "Bob".to_string() });
        assert_eq!(app.feedback, "TOO LATE (already claimed by Bob)");
    }

    #[test]
    fn test_multiplayer_claim_feedback_nope() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'T'], 60);
        app.on_claim_rejected("XYZ".to_string(), MissReason::NotInDictionary);
        assert_eq!(app.feedback, "NOPE");
    }

    #[test]
    fn test_multiplayer_claim_feedback_clank() {
        let mut app = App::new();
        app.start_round(vec!['C', 'A', 'T'], 60);
        app.on_claim_rejected("ZAP".to_string(), MissReason::InvalidLetters);
        assert_eq!(app.feedback, "CLANK");
    }

    #[test]
    fn test_scoreboard_initialization() {
        let mut app = App::new();
        app.set_scoreboard(vec!["Alice".into(), "Bob".into(), "Charlie".into()]);

        assert_eq!(app.scoreboard.len(), 3);
        assert_eq!(app.scoreboard[0].name, "Alice");
        assert_eq!(app.scoreboard[0].score, 0);
    }

    #[test]
    fn test_scoreboard_sorts_by_score() {
        let mut app = App::new();
        app.set_scoreboard(vec!["Alice".into(), "Bob".into()]);

        app.update_scoreboard(vec![("Bob".into(), 10), ("Alice".into(), 5)]);

        assert_eq!(app.scoreboard[0].name, "Bob");
        assert_eq!(app.scoreboard[0].score, 10);
        assert_eq!(app.scoreboard[1].name, "Alice");
        assert_eq!(app.scoreboard[1].score, 5);
    }

    #[test]
    fn test_claim_feed_updates_on_accepted() {
        let mut app = App::new();
        app.set_player_name("Alice".into());
        app.set_scoreboard(vec!["Alice".into(), "Bob".into()]);
        app.start_round(vec!['A', 'B', 'C'], 60);

        app.on_claim_accepted("CAB".into(), "Bob".into(), 3);

        assert_eq!(app.claim_feed.len(), 1);
        assert_eq!(app.claim_feed[0].player_name, "Bob");
        assert_eq!(app.claim_feed[0].word, "CAB");
        assert_eq!(app.claim_feed[0].points, 3);
        // Bob's score should be updated in scoreboard
        assert_eq!(app.scoreboard[0].name, "Bob");
        assert_eq!(app.scoreboard[0].score, 3);
    }

    #[test]
    fn test_claim_feed_max_entries() {
        let mut app = App::new();
        app.set_player_name("Alice".into());
        app.set_scoreboard(vec!["Alice".into(), "Bob".into()]);
        app.start_round(vec!['A', 'B', 'C'], 60);

        // Add more than max entries
        for i in 0..15 {
            app.on_claim_accepted(format!("WORD{}", i), "Bob".into(), 3);
        }

        assert_eq!(app.claim_feed.len(), 10); // Max is 10
    }

    #[test]
    fn test_own_claim_updates_score() {
        let mut app = App::new();
        app.set_player_name("Alice".into());
        app.set_scoreboard(vec!["Alice".into(), "Bob".into()]);
        app.start_round(vec!['A', 'B', 'C'], 60);

        app.on_claim_accepted("CAB".into(), "Alice".into(), 3);

        assert_eq!(app.score, 3);
        assert_eq!(app.claimed_words().len(), 1);
    }

    #[test]
    fn test_other_player_claim_does_not_update_own_score() {
        let mut app = App::new();
        app.set_player_name("Alice".into());
        app.set_scoreboard(vec!["Alice".into(), "Bob".into()]);
        app.start_round(vec!['A', 'B', 'C'], 60);

        app.on_claim_accepted("CAB".into(), "Bob".into(), 3);

        assert_eq!(app.score, 0); // Alice's score unchanged
        assert_eq!(app.claimed_words().len(), 0); // Not in Alice's claimed list
    }

    #[test]
    fn test_claim_rejected_updates_feedback() {
        let mut app = App::new();
        app.set_player_name("Alice".into());
        app.start_round(vec!['A', 'B', 'C'], 60);

        app.on_claim_rejected("CAB".into(), MissReason::AlreadyClaimed { by: "Bob".into() });

        assert!(app.feedback.contains("Bob"));
        assert_eq!(app.missed_words().len(), 1);
    }

    #[test]
    fn test_force_end_round() {
        let mut app = App::new();
        app.start_round(vec!['A', 'B', 'C'], 60);

        assert!(!app.is_round_over());
        app.force_end_round();
        assert!(app.is_round_over());
    }

    #[test]
    fn test_get_pending_claim() {
        let mut app = App::new();
        app.start_round(vec!['A', 'B', 'C'], 60);

        assert!(app.get_pending_claim().is_none());

        app.on_char('A');
        app.on_char('B');
        assert_eq!(app.get_pending_claim(), Some("AB".into()));

        // After round ends, no pending claims
        app.force_end_round();
        assert!(app.get_pending_claim().is_none());
    }

    #[test]
    fn test_start_round_resets_scoreboard_scores() {
        let mut app = App::new();
        app.set_scoreboard(vec!["Alice".into(), "Bob".into()]);
        app.start_round(vec!['A', 'B', 'C'], 60);

        app.on_claim_accepted("CAB".into(), "Alice".into(), 3);
        assert_eq!(app.scoreboard[0].score, 3);

        // Starting new round resets scores but keeps players
        app.start_round(vec!['X', 'Y', 'Z'], 60);
        assert_eq!(app.scoreboard.len(), 2);
        assert_eq!(app.scoreboard[0].score, 0);
        assert_eq!(app.scoreboard[1].score, 0);
    }

    #[test]
    fn test_scoreboard_adds_new_player() {
        let mut app = App::new();
        app.set_scoreboard(vec!["Alice".into()]);

        // ScoreUpdate with new player
        app.update_scoreboard(vec![("Alice".into(), 5), ("Bob".into(), 10)]);

        assert_eq!(app.scoreboard.len(), 2);
        assert_eq!(app.scoreboard[0].name, "Bob"); // Sorted by score
        assert_eq!(app.scoreboard[1].name, "Alice");
    }
}
