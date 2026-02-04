//! UI rendering using ratatui
//!
//! Layout:
//! ┌─────────────────────────────────────┐
//! │  BLAM!    [A B C D E F G H]   0:45  │ <- Header: logo, letters, timer
//! ├─────────────────────────────────────┤
//! │                                     │
//! │  > ________                         │ <- Input line (always focused)
//! │                                     │
//! │  OK +5                              │ <- Feedback line
//! │                                     │
//! │  Score: 42                          │ <- Score display
//! │                                     │
//! └─────────────────────────────────────┘

use crate::app::App;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

/// Render the game UI
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Main layout: header (3 lines) + content
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header with letters, timer
            Constraint::Min(0),    // Main content area
        ])
        .split(area);

    render_header(frame, layout[0], app);
    render_main(frame, layout[1], app);
}

/// Render the header: logo, letter rack, timer
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split header into: logo | letters | timer
    let header_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10), // Logo
            Constraint::Min(20),    // Letters (centered, flexible)
            Constraint::Length(10), // Timer
        ])
        .split(inner);

    // Logo
    let logo = Paragraph::new("BLAM!")
        .style(Style::default().fg(Color::Yellow).bold())
        .alignment(Alignment::Left);
    frame.render_widget(logo, header_layout[0]);

    // Letter rack - prominent and centered
    let letters_display = format_letter_rack(&app.letters);
    let letters = Paragraph::new(letters_display)
        .style(Style::default().fg(Color::Cyan).bold())
        .alignment(Alignment::Center);
    frame.render_widget(letters, header_layout[1]);

    // Timer
    let timer_display = format_timer(app.time_remaining);
    let timer_color = if app.time_remaining <= 10 {
        Color::Red
    } else if app.time_remaining <= 30 {
        Color::Yellow
    } else {
        Color::Green
    };
    let timer = Paragraph::new(timer_display)
        .style(Style::default().fg(timer_color).bold())
        .alignment(Alignment::Right);
    frame.render_widget(timer, header_layout[2]);
}

/// Render the main content area: input, feedback, score
fn render_main(frame: &mut Frame, area: Rect, app: &App) {
    // Vertical layout for main content
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1), // Input line
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Feedback line
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Score
            Constraint::Min(0),    // Remaining space
        ])
        .split(area);

    // Input line with cursor indicator
    let input_display = format!("> {}_", app.input);
    let input = Paragraph::new(input_display)
        .style(Style::default().fg(Color::White));
    frame.render_widget(input, main_layout[0]);

    // Feedback line
    let (feedback_text, feedback_color) = format_feedback(&app.feedback);
    let feedback = Paragraph::new(feedback_text)
        .style(Style::default().fg(feedback_color));
    frame.render_widget(feedback, main_layout[2]);

    // Score
    let score_display = format!("Score: {}", app.score);
    let score = Paragraph::new(score_display)
        .style(Style::default().fg(Color::Magenta).bold());
    frame.render_widget(score, main_layout[4]);
}

/// Format the letter rack for display
fn format_letter_rack(letters: &[char]) -> String {
    if letters.is_empty() {
        return String::from("[ Press ENTER to start ]");
    }

    let letters_str: String = letters
        .iter()
        .map(|c| c.to_ascii_uppercase().to_string())
        .collect::<Vec<_>>()
        .join(" ");

    format!("[ {} ]", letters_str)
}

/// Format the timer display
fn format_timer(seconds: u32) -> String {
    let mins = seconds / 60;
    let secs = seconds % 60;
    format!("{}:{:02}", mins, secs)
}

/// Format feedback with appropriate color
fn format_feedback(feedback: &str) -> (String, Color) {
    if feedback.is_empty() {
        return (String::new(), Color::White);
    }

    let color = if feedback.starts_with("OK") {
        Color::Green
    } else if feedback.starts_with("NOPE") || feedback.starts_with("CLANK") {
        Color::Red
    } else if feedback.starts_with("TOO LATE") {
        Color::Yellow
    } else {
        Color::White
    };

    (feedback.to_string(), color)
}
