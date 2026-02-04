//! Application state management

/// Main application state
pub struct App {
    /// Whether the application should quit
    pub should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self { should_quit: false }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }
}
