#![allow(dead_code)]
//! UI rendering using ratatui
//!
//! Supports multiple screens:
//! - Menu: Main menu with options
//! - Browser: Lobby browser showing available games
//! - HostLobby: Hosting a lobby, waiting for players
//! - JoinedLobby: Joined a lobby, waiting for start
//! - Playing: In-game screen
//! - Error: Error message display

use crate::app::{App, AppCoordinator, MenuOption, Screen};
use crate::lobby::Player;
use crate::network::PeerInfo;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

/// Render the appropriate screen based on app state
pub fn render(frame: &mut Frame, coordinator: &AppCoordinator) {
    match &coordinator.screen {
        Screen::Menu { selected, handle, handle_input, editing_handle } => {
            render_menu(frame, *selected, handle, handle_input, *editing_handle);
        }
        Screen::Browser { lobbies, selected, .. } => {
            render_browser(frame, lobbies, *selected);
        }
        Screen::HostLobby { lobby, countdown } => {
            render_host_lobby(
                frame,
                &lobby.lobby_name,
                lobby.players(),
                lobby.port(),
                lobby.can_start(),
                *countdown,
                lobby.current_letters(),
            );
        }
        Screen::JoinedLobby { lobby, countdown } => {
            render_joined_lobby(
                frame,
                &lobby.lobby_name,
                &lobby.host_name,
                lobby.players(),
                countdown.as_ref(),
            );
        }
        Screen::Playing { app, .. } => {
            render_game(frame, app);
        }
        Screen::Error { message } => {
            render_error(frame, message);
        }
    }
}

/// Render the main menu
fn render_menu(frame: &mut Frame, selected: usize, handle: &str, handle_input: &str, editing_handle: bool) {
    let area = frame.area();

    // Main layout
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8),  // Logo
            Constraint::Length(3),  // Handle input
            Constraint::Length(1),  // Spacer
            Constraint::Min(6),     // Menu options
            Constraint::Length(2),  // Footer
        ])
        .margin(2)
        .split(area);

    // Logo
    let logo = r#"
 ____  _        _    __  __ _
| __ )| |      / \  |  \/  | |
|  _ \| |     / _ \ | |\/| | |
| |_) | |___ / ___ \| |  | |_|
|____/|_____/_/   \_\_|  |_(_)
"#;
    let logo_widget = Paragraph::new(logo)
        .style(Style::default().fg(Color::Yellow).bold())
        .alignment(Alignment::Center);
    frame.render_widget(logo_widget, layout[0]);

    // Handle input
    let handle_display = if editing_handle {
        format!("Handle: [{}]_", handle_input)
    } else {
        format!("Handle: {} (Tab to edit)", handle)
    };
    let handle_style = if editing_handle {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let handle_widget = Paragraph::new(handle_display)
        .style(handle_style)
        .alignment(Alignment::Center);
    frame.render_widget(handle_widget, layout[1]);

    // Menu options
    let items: Vec<ListItem> = MenuOption::all()
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let style = if i == selected {
                Style::default().fg(Color::Yellow).bold()
            } else {
                Style::default().fg(Color::White)
            };
            let prefix = if i == selected { "> " } else { "  " };
            ListItem::new(format!("{}{}", prefix, opt.label())).style(style)
        })
        .collect();

    let menu = List::new(items)
        .block(Block::default())
        .highlight_style(Style::default().fg(Color::Yellow));
    frame.render_widget(menu, layout[3]);

    // Footer
    let footer = Paragraph::new("↑↓ Navigate  Enter Select  Esc Quit")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(footer, layout[4]);
}

/// Render the lobby browser
fn render_browser(frame: &mut Frame, lobbies: &[PeerInfo], selected: usize) {
    let area = frame.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Min(6),     // Lobby list
            Constraint::Length(2),  // Footer
        ])
        .margin(1)
        .split(area);

    // Header
    let header = Paragraph::new("Available Lobbies")
        .style(Style::default().fg(Color::Cyan).bold())
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, layout[0]);

    // Lobby list
    if lobbies.is_empty() {
        let searching = Paragraph::new("Searching for lobbies on LAN...\n\n(Make sure another player has started a lobby)")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(searching, layout[1]);
    } else {
        let items: Vec<ListItem> = lobbies
            .iter()
            .enumerate()
            .map(|(i, peer)| {
                let style = if i == selected {
                    Style::default().fg(Color::Yellow).bold()
                } else {
                    Style::default().fg(Color::White)
                };
                let prefix = if i == selected { "> " } else { "  " };
                let lobby_name = peer.lobby_name.as_deref().unwrap_or("Unknown");
                ListItem::new(format!("{}{} (Host: {})", prefix, lobby_name, peer.handle))
                    .style(style)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Lobbies"));
        frame.render_widget(list, layout[1]);
    }

    // Footer
    let footer = Paragraph::new("↑↓ Select  Enter Join  Esc Back")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(footer, layout[2]);
}

/// Render the host lobby screen
fn render_host_lobby(
    frame: &mut Frame,
    lobby_name: &str,
    players: &[Player],
    port: u16,
    can_start: bool,
    countdown: Option<u32>,
    letters: &[char],
) {
    let area = frame.area();

    // If in countdown, render the countdown screen
    if let Some(count) = countdown {
        render_countdown(frame, area, count, letters);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(3),  // Lobby info
            Constraint::Min(6),     // Player list
            Constraint::Length(3),  // Start button
            Constraint::Length(2),  // Footer
        ])
        .margin(1)
        .split(area);

    // Header
    let header = Paragraph::new(format!("Lobby: {}", lobby_name))
        .style(Style::default().fg(Color::Yellow).bold())
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, layout[0]);

    // Lobby info
    let info = Paragraph::new(format!("Port: {} | Players: {}/12", port, players.len()))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(info, layout[1]);

    // Player list
    let items: Vec<ListItem> = players
        .iter()
        .map(|p| {
            let suffix = if p.is_host { " (Host)" } else { "" };
            let style = if p.is_local {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!("  {} {}{}", "●", p.name, suffix)).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Players"));
    frame.render_widget(list, layout[2]);

    // Start button
    let start_text = if can_start {
        "[ Press ENTER to START ]".to_string()
    } else {
        "Waiting for players (need at least 2)".to_string()
    };

    let start_style = if can_start {
        Style::default().fg(Color::Green).bold()
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let start = Paragraph::new(start_text)
        .style(start_style)
        .alignment(Alignment::Center);
    frame.render_widget(start, layout[3]);

    // Footer
    let footer = Paragraph::new("Enter Start  Esc Cancel")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(footer, layout[4]);
}

/// Render the countdown screen (3-2-1-BLAM!)
fn render_countdown(frame: &mut Frame, area: Rect, count: u32, letters: &[char]) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(30),  // Top spacer
            Constraint::Length(5),       // Big countdown number
            Constraint::Length(3),       // Letters preview
            Constraint::Percentage(30),  // Bottom spacer
        ])
        .margin(2)
        .split(area);

    // Big countdown number
    let countdown_text = match count {
        3 => "   3",
        2 => "   2",
        1 => "   1",
        0 => "BLAM!",
        _ => &format!("   {}", count),
    };

    let countdown_color = match count {
        3 => Color::Green,
        2 => Color::Yellow,
        1 => Color::Red,
        0 => Color::Magenta,
        _ => Color::White,
    };

    let countdown = Paragraph::new(countdown_text)
        .style(Style::default().fg(countdown_color).bold())
        .alignment(Alignment::Center);
    frame.render_widget(countdown, layout[1]);

    // Letters preview
    let letters_display = format_letter_rack(letters);
    let letters_widget = Paragraph::new(letters_display)
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center);
    frame.render_widget(letters_widget, layout[2]);
}

/// Render the joined lobby screen
fn render_joined_lobby(
    frame: &mut Frame,
    lobby_name: &str,
    host_name: &str,
    players: &[Player],
    countdown: Option<&(u32, Vec<char>, u32)>,
) {
    let area = frame.area();

    // If in countdown, render the countdown screen
    if let Some((count, letters, _duration)) = countdown {
        render_countdown(frame, area, *count, letters);
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(3),  // Lobby info
            Constraint::Min(6),     // Player list
            Constraint::Length(3),  // Status
            Constraint::Length(2),  // Footer
        ])
        .margin(1)
        .split(area);

    // Header
    let header = Paragraph::new(format!("Lobby: {}", lobby_name))
        .style(Style::default().fg(Color::Yellow).bold())
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(header, layout[0]);

    // Lobby info
    let info = Paragraph::new(format!("Host: {} | Players: {}/12", host_name, players.len()))
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(info, layout[1]);

    // Player list
    let items: Vec<ListItem> = players
        .iter()
        .map(|p| {
            let suffix = if p.is_host {
                " (Host)"
            } else if p.is_local {
                " (You)"
            } else {
                ""
            };
            let style = if p.is_local {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!("  {} {}{}", "●", p.name, suffix)).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Players"));
    frame.render_widget(list, layout[2]);

    // Status
    let status = Paragraph::new("Waiting for host to start...")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(status, layout[3]);

    // Footer
    let footer = Paragraph::new("Esc Leave")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(footer, layout[4]);
}

/// Render the in-game screen
fn render_game(frame: &mut Frame, app: &App) {
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

    if app.is_round_over() {
        render_end_of_round(frame, layout[1], app);
    } else {
        render_main(frame, layout[1], app);
    }
}

/// Render error screen
fn render_error(frame: &mut Frame, message: &str) {
    let area = frame.area();

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Percentage(40),
        ])
        .margin(2)
        .split(area);

    let error = Paragraph::new(format!("Error: {}", message))
        .style(Style::default().fg(Color::Red))
        .alignment(Alignment::Center);
    frame.render_widget(error, layout[1]);

    let hint = Paragraph::new("Press Esc to go back")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(hint, layout[2]);
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

/// Render the main content area: input, feedback, score, with optional side panels
fn render_main(frame: &mut Frame, area: Rect, app: &App) {
    // Check if we have multiplayer content to show
    let has_scoreboard = !app.scoreboard.is_empty();
    let has_claim_feed = !app.claim_feed.is_empty();
    let has_side_panels = has_scoreboard || has_claim_feed;

    if has_side_panels {
        // Three-column layout: main | scoreboard | claim feed
        let horizontal_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(30),       // Main area
                Constraint::Length(20),    // Scoreboard
                Constraint::Length(25),    // Claim feed
            ])
            .split(area);

        render_input_area(frame, horizontal_layout[0], app);
        render_scoreboard(frame, horizontal_layout[1], app);
        render_claim_feed(frame, horizontal_layout[2], app);
    } else {
        // Solo mode - just the input area
        render_input_area(frame, area, app);
    }
}

/// Render the input/feedback area (center panel)
fn render_input_area(frame: &mut Frame, area: Rect, app: &App) {
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

/// Render the live scoreboard (right panel)
fn render_scoreboard(frame: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .scoreboard
        .iter()
        .enumerate()
        .map(|(i, player)| {
            let prefix = match i {
                0 => "🥇",
                1 => "🥈",
                2 => "🥉",
                _ => "  ",
            };
            let is_local = app.player_name.as_ref() == Some(&player.name);
            let style = if is_local {
                Style::default().fg(Color::Cyan).bold()
            } else if i == 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!("{} {} - {}", prefix, player.name, player.score)).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title("Scoreboard"),
        );
    frame.render_widget(list, area);
}

/// Render the claim feed (rightmost panel)
fn render_claim_feed(frame: &mut Frame, area: Rect, app: &App) {
    // Show most recent claims first (reverse order)
    let items: Vec<ListItem> = app
        .claim_feed
        .iter()
        .rev()
        .take(8)
        .map(|entry| {
            let is_local = app.player_name.as_ref() == Some(&entry.player_name);
            let style = if is_local {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Green)
            };
            ListItem::new(format!(
                "{}: {} +{}",
                entry.player_name, entry.word, entry.points
            ))
            .style(style)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title("Claims"),
        );
    frame.render_widget(list, area);
}

/// Render the end-of-round summary
fn render_end_of_round(frame: &mut Frame, area: Rect, app: &App) {
    let has_scoreboard = !app.scoreboard.is_empty();

    if has_scoreboard {
        // Multiplayer end-of-round: show scoreboard alongside summary
        let horizontal_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(30),    // Summary area
                Constraint::Length(22), // Final scoreboard
                Constraint::Length(25), // Claim feed
            ])
            .split(area);

        render_end_summary(frame, horizontal_layout[0], app);
        render_scoreboard(frame, horizontal_layout[1], app);
        render_claim_feed(frame, horizontal_layout[2], app);
    } else {
        // Solo end-of-round
        render_end_summary(frame, area, app);
    }
}

/// Render the end-of-round summary text
fn render_end_summary(frame: &mut Frame, area: Rect, app: &App) {
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(2), // Title
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Final score
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Words claimed
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Instructions
            Constraint::Min(0),    // Remaining space
        ])
        .split(area);

    // TIME'S UP title
    let title = Paragraph::new("TIME'S UP!")
        .style(Style::default().fg(Color::Red).bold())
        .alignment(Alignment::Center);
    frame.render_widget(title, main_layout[0]);

    // Final score
    let score_text = format!("Final Score: {}", app.score);
    let score = Paragraph::new(score_text)
        .style(Style::default().fg(Color::Yellow).bold())
        .alignment(Alignment::Center);
    frame.render_widget(score, main_layout[2]);

    // Words claimed count
    let words_text = format!("Words Claimed: {}", app.claimed_words().len());
    let words = Paragraph::new(words_text)
        .style(Style::default().fg(Color::Cyan))
        .alignment(Alignment::Center);
    frame.render_widget(words, main_layout[4]);

    // Instructions
    let instructions = Paragraph::new("Press ESC to return to menu")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(instructions, main_layout[6]);
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
    } else if feedback.starts_with("NOPE")
        || feedback.starts_with("CLANK")
        || feedback.starts_with("Not in dictionary")
        || feedback.starts_with("Missing")
        || feedback.starts_with("Too short")
    {
        Color::Red
    } else if feedback.starts_with("TOO LATE")
        || feedback.starts_with("Already claimed")
        || feedback.starts_with("Round has ended")
    {
        Color::Yellow
    } else {
        Color::White
    };

    (feedback.to_string(), color)
}

// Legacy function for backwards compatibility
pub fn render_app(frame: &mut Frame, app: &App) {
    render_game(frame, app);
}
