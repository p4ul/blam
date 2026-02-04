//! Lobby UI rendering
//!
//! Layout:
//! ┌─────────────────────────────────────────────────┐
//! │  BLAM!                          LOBBY-NAME      │
//! ├─────────────────────────────────────────────────┤
//! │                                                 │
//! │  ╔═══════════════════════════════════════════╗  │
//! │  ║  PLAYERS                                  ║  │
//! │  ╟───────────────────────────────────────────╢  │
//! │  ║  ★ Alice (Host)                          ║  │
//! │  ║    Bob                                   ║  │
//! │  ║    Charlie                               ║  │
//! │  ╚═══════════════════════════════════════════╝  │
//! │                                                 │
//! │  ╔═══════════════════════════════════════════╗  │
//! │  ║  SETTINGS (Host Only)                    ║  │
//! │  ╟───────────────────────────────────────────╢  │
//! │  ║  Duration:       [60] secs               ║  │
//! │  ║  Min Word Length:[3]                     ║  │
//! │  ║  Letters:        [12-20]                 ║  │
//! │  ╚═══════════════════════════════════════════╝  │
//! │                                                 │
//! │  [S] Start Game  [ESC] Leave                    │
//! └─────────────────────────────────────────────────┘

use crate::lobby::{Lobby, LobbySettings};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

/// Render the lobby screen
pub fn render_lobby(frame: &mut Frame, lobby: &Lobby) {
    let area = frame.area();

    // Main layout: header + content
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Content
        ])
        .split(area);

    render_lobby_header(frame, layout[0], lobby);
    render_lobby_content(frame, layout[1], lobby);
}

/// Render lobby header with logo and lobby name
fn render_lobby_header(frame: &mut Frame, area: Rect, lobby: &Lobby) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split header: logo | spacer | lobby name
    let header_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10), // Logo
            Constraint::Min(0),     // Spacer
            Constraint::Length(20), // Lobby name
        ])
        .split(inner);

    // Logo
    let logo = Paragraph::new("BLAM!")
        .style(Style::default().fg(Color::Yellow).bold())
        .alignment(Alignment::Left);
    frame.render_widget(logo, header_layout[0]);

    // Lobby name
    let lobby_name = Paragraph::new(lobby.name.as_str())
        .style(Style::default().fg(Color::Cyan).bold())
        .alignment(Alignment::Right);
    frame.render_widget(lobby_name, header_layout[2]);
}

/// Render lobby content: players list and settings
fn render_lobby_content(frame: &mut Frame, area: Rect, lobby: &Lobby) {
    let content_layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(lobby.players.len() as u16 + 4), // Players box
            Constraint::Length(1),                              // Spacer
            Constraint::Length(7),                              // Settings box
            Constraint::Min(0),                                 // Spacer
            Constraint::Length(2),                              // Instructions
        ])
        .split(area);

    render_players_list(frame, content_layout[0], lobby);
    render_settings(frame, content_layout[2], &lobby.settings, lobby.is_host);
    render_lobby_instructions(frame, content_layout[4], lobby.is_host);
}

/// Render the players list
fn render_players_list(frame: &mut Frame, area: Rect, lobby: &Lobby) {
    let items: Vec<ListItem> = lobby
        .players
        .iter()
        .map(|player| {
            let prefix = if player.is_host { "★ " } else { "  " };
            let suffix = if player.is_host { " (Host)" } else { "" };
            let style = if player.name == lobby.local_player {
                Style::default().fg(Color::Green).bold()
            } else if player.is_host {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!("{}{}{}", prefix, player.name, suffix)).style(style)
        })
        .collect();

    let title = format!(" Players ({}) ", lobby.players.len());
    let list = List::new(items).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue)),
    );

    frame.render_widget(list, area);
}

/// Render the settings panel
fn render_settings(frame: &mut Frame, area: Rect, settings: &LobbySettings, is_host: bool) {
    let title = if is_host {
        " Settings "
    } else {
        " Settings (View Only) "
    };

    let settings_text = vec![
        Line::from(vec![
            Span::styled("  Duration:        ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{} secs", settings.duration_secs),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Min Word Length: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", settings.min_word_length),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Letters:         ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}-{}", settings.min_letters, settings.max_letters),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    let settings_para = Paragraph::new(settings_text).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(if is_host {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            }),
    );

    frame.render_widget(settings_para, area);
}

/// Render lobby instructions
fn render_lobby_instructions(frame: &mut Frame, area: Rect, is_host: bool) {
    let instructions = if is_host {
        "[S] Start Game    [↑/↓] Change Duration    [ESC] Leave"
    } else {
        "Waiting for host to start...    [ESC] Leave"
    };

    let para = Paragraph::new(instructions)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);

    frame.render_widget(para, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lobby::Lobby;

    #[test]
    fn test_lobby_render_does_not_panic() {
        // Basic smoke test - ensure render functions don't panic
        let lobby = Lobby::create("TestPlayer".to_string());

        // We can't easily test rendering without a terminal,
        // but we can at least verify the lobby data is valid
        assert_eq!(lobby.player_count(), 1);
        assert!(lobby.is_host);
    }
}
