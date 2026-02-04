//! Application state management

use crate::game::validation::{validate_word, ValidationResult};

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
}

impl Default for App {
    fn default() -> Self {
        Self {
            should_quit: false,
            letters: Vec::new(),
            input: String::new(),
            feedback: String::new(),
            score: 0,
            time_remaining: 60,
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

    /// Handle character input
    pub fn on_char(&mut self, c: char) {
        self.input.push(c);
        self.feedback.clear();
    }

    /// Handle backspace
    pub fn on_backspace(&mut self) {
        self.input.pop();
        self.feedback.clear();
    }

    /// Handle word submission (Enter key)
    pub fn on_submit(&mut self) {
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

    /// Update the timer
    pub fn tick(&mut self) {
        if self.time_remaining > 0 {
            self.time_remaining -= 1;
        }
    }

    /// Check if the round is over
    pub fn is_round_over(&self) -> bool {
        self.time_remaining == 0
    }

    /// Start a new round with given letters and duration
    pub fn start_round(&mut self, letters: Vec<char>, duration: u32) {
        self.letters = letters;
        self.time_remaining = duration;
        self.score = 0;
        self.input.clear();
        self.feedback.clear();
    }
}
