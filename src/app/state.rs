//! Application state management

use crate::game::validation::{validate_word, ValidationResult};

/// Default round duration in seconds
pub const DEFAULT_ROUND_DURATION: u32 = 60;

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
        let result = validate_word(&word, &self.letters);

        match result {
            ValidationResult::Valid => {
                let points = word.len() as u32;
                self.score += points;
                self.feedback = format!("OK +{} ({})", points, word.to_uppercase());
            }
            _ => {
                self.feedback = result.message();
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
}
