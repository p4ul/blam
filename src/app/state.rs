//! Application state management

use crate::game::validation::{validate_word, ValidationResult};
use crate::lobby::Lobby;

/// Default round duration in seconds
pub const DEFAULT_ROUND_DURATION: u32 = 60;

/// Current application mode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppMode {
    /// In the lobby, waiting for game to start
    Lobby,
    /// Active game round
    Game,
}

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
}

impl MissReason {
    pub fn label(&self) -> &'static str {
        match self {
            MissReason::TooShort => "Too Short",
            MissReason::InvalidLetters => "Invalid Letters",
            MissReason::NotInDictionary => "Not In Dictionary",
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
    /// Current application mode
    pub mode: AppMode,
    /// Current lobby (if in lobby mode)
    pub lobby: Option<Lobby>,
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
    /// Words claimed this round
    claimed_words: Vec<ClaimedWord>,
    /// Missed submissions this round
    missed_words: Vec<MissedWord>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            should_quit: false,
            mode: AppMode::Game,
            lobby: None,
            letters: Vec::new(),
            input: String::new(),
            feedback: String::new(),
            score: 0,
            time_remaining: DEFAULT_ROUND_DURATION,
            round_ended: false,
            claimed_words: Vec::new(),
            missed_words: Vec::new(),
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
                self.feedback = result.message();
                self.missed_words.push(MissedWord {
                    word: word_upper,
                    reason: MissReason::InvalidLetters,
                });
            }
            ValidationResult::NotInDictionary => {
                self.feedback = result.message();
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
            match miss.reason {
                MissReason::TooShort => summary.too_short.push(miss.word.clone()),
                MissReason::InvalidLetters => summary.invalid_letters.push(miss.word.clone()),
                MissReason::NotInDictionary => summary.not_in_dictionary.push(miss.word.clone()),
            }
        }

        summary
    }

    // === Lobby Management ===

    /// Create a new lobby and enter lobby mode
    pub fn create_lobby(&mut self, player_name: String) {
        self.lobby = Some(Lobby::create(player_name));
        self.mode = AppMode::Lobby;
    }

    /// Join an existing lobby
    pub fn join_lobby(&mut self, lobby_name: String, player_name: String) {
        self.lobby = Some(Lobby::join(lobby_name, player_name));
        self.mode = AppMode::Lobby;
    }

    /// Leave the current lobby
    pub fn leave_lobby(&mut self) {
        self.lobby = None;
        self.mode = AppMode::Game;
    }

    /// Check if we're in lobby mode
    pub fn is_in_lobby(&self) -> bool {
        self.mode == AppMode::Lobby && self.lobby.is_some()
    }

    /// Check if we're the host of the current lobby
    pub fn is_lobby_host(&self) -> bool {
        self.lobby.as_ref().map(|l| l.is_host).unwrap_or(false)
    }

    /// Add a player to the lobby (host receives this from network)
    pub fn lobby_add_player(&mut self, name: String) {
        if let Some(lobby) = &mut self.lobby {
            lobby.add_player(name);
        }
    }

    /// Remove a player from the lobby
    pub fn lobby_remove_player(&mut self, name: &str) {
        if let Some(lobby) = &mut self.lobby {
            lobby.remove_player(name);
        }
    }

    /// Increase lobby duration setting (host only)
    pub fn lobby_increase_duration(&mut self) {
        if let Some(lobby) = &mut self.lobby {
            if lobby.is_host && lobby.settings.duration_secs < 180 {
                lobby.settings.duration_secs += 15;
            }
        }
    }

    /// Decrease lobby duration setting (host only)
    pub fn lobby_decrease_duration(&mut self) {
        if let Some(lobby) = &mut self.lobby {
            if lobby.is_host && lobby.settings.duration_secs > 30 {
                lobby.settings.duration_secs -= 15;
            }
        }
    }

    /// Start the game from lobby (host only)
    pub fn start_game_from_lobby(&mut self) {
        if let Some(lobby) = &self.lobby {
            if lobby.can_start() {
                // Transition from lobby to game mode
                let duration = lobby.settings.duration_secs;
                self.mode = AppMode::Game;
                // Note: letters will be set when round actually starts
                self.time_remaining = duration;
            }
        }
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

    // === Lobby Tests ===

    #[test]
    fn test_create_lobby() {
        let mut app = App::new();
        assert!(!app.is_in_lobby());

        app.create_lobby("Alice".to_string());
        assert!(app.is_in_lobby());
        assert!(app.is_lobby_host());
        assert_eq!(app.mode, AppMode::Lobby);
        assert!(app.lobby.is_some());

        let lobby = app.lobby.as_ref().unwrap();
        assert_eq!(lobby.player_count(), 1);
        assert_eq!(lobby.local_player, "Alice");
    }

    #[test]
    fn test_join_lobby() {
        let mut app = App::new();

        app.join_lobby("TEST-LOBBY".to_string(), "Bob".to_string());
        assert!(app.is_in_lobby());
        assert!(!app.is_lobby_host());

        let lobby = app.lobby.as_ref().unwrap();
        assert_eq!(lobby.name, "TEST-LOBBY");
        assert_eq!(lobby.local_player, "Bob");
    }

    #[test]
    fn test_leave_lobby() {
        let mut app = App::new();
        app.create_lobby("Alice".to_string());
        assert!(app.is_in_lobby());

        app.leave_lobby();
        assert!(!app.is_in_lobby());
        assert!(app.lobby.is_none());
        assert_eq!(app.mode, AppMode::Game);
    }

    #[test]
    fn test_lobby_add_remove_players() {
        let mut app = App::new();
        app.create_lobby("Alice".to_string());

        app.lobby_add_player("Bob".to_string());
        assert_eq!(app.lobby.as_ref().unwrap().player_count(), 2);

        app.lobby_add_player("Charlie".to_string());
        assert_eq!(app.lobby.as_ref().unwrap().player_count(), 3);

        app.lobby_remove_player("Bob");
        assert_eq!(app.lobby.as_ref().unwrap().player_count(), 2);
    }

    #[test]
    fn test_lobby_duration_settings() {
        let mut app = App::new();
        app.create_lobby("Alice".to_string());

        let initial_duration = app.lobby.as_ref().unwrap().settings.duration_secs;

        app.lobby_increase_duration();
        assert_eq!(
            app.lobby.as_ref().unwrap().settings.duration_secs,
            initial_duration + 15
        );

        app.lobby_decrease_duration();
        assert_eq!(
            app.lobby.as_ref().unwrap().settings.duration_secs,
            initial_duration
        );
    }

    #[test]
    fn test_lobby_duration_limits() {
        let mut app = App::new();
        app.create_lobby("Alice".to_string());

        // Decrease to minimum
        for _ in 0..20 {
            app.lobby_decrease_duration();
        }
        assert!(app.lobby.as_ref().unwrap().settings.duration_secs >= 30);

        // Increase to maximum
        for _ in 0..20 {
            app.lobby_increase_duration();
        }
        assert!(app.lobby.as_ref().unwrap().settings.duration_secs <= 180);
    }

    #[test]
    fn test_start_game_from_lobby() {
        let mut app = App::new();
        app.create_lobby("Alice".to_string());
        assert_eq!(app.mode, AppMode::Lobby);

        app.start_game_from_lobby();
        assert_eq!(app.mode, AppMode::Game);
    }

    #[test]
    fn test_non_host_cannot_change_settings() {
        let mut app = App::new();
        app.join_lobby("TEST".to_string(), "Bob".to_string());

        let initial_duration = app.lobby.as_ref().unwrap().settings.duration_secs;

        // Non-host attempts to change settings
        app.lobby_increase_duration();
        assert_eq!(
            app.lobby.as_ref().unwrap().settings.duration_secs,
            initial_duration
        );

        app.lobby_decrease_duration();
        assert_eq!(
            app.lobby.as_ref().unwrap().settings.duration_secs,
            initial_duration
        );
    }
}
