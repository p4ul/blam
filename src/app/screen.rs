#![allow(dead_code)]
//! Application screen state management
//!
//! Handles transitions between different application screens:
//! - Main menu
//! - Lobby browser
//! - Hosted lobby
//! - Joined lobby
//! - Playing (solo or multiplayer)
//! - End of round results

use crate::game::LetterRack;
use crate::lobby::{HostedLobby, JoinedLobby, LobbyBrowser, LobbyEvent};
use crate::network::{ClaimRejectReason, PeerInfo};

use super::state::{App, DEFAULT_ROUND_DURATION};

/// Menu option on the main screen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuOption {
    StartLobby,
    JoinLobby,
    SoloPractice,
    Quit,
}

impl MenuOption {
    /// Get all menu options in order
    pub fn all() -> &'static [MenuOption] {
        &[
            MenuOption::StartLobby,
            MenuOption::JoinLobby,
            MenuOption::SoloPractice,
            MenuOption::Quit,
        ]
    }

    /// Get the display label for this option
    pub fn label(&self) -> &'static str {
        match self {
            MenuOption::StartLobby => "Start Lobby",
            MenuOption::JoinLobby => "Join Lobby",
            MenuOption::SoloPractice => "Solo Practice",
            MenuOption::Quit => "Quit",
        }
    }
}

/// The current application screen
pub enum Screen {
    /// Main menu
    Menu {
        selected: usize,
        handle: String,
        handle_input: String,
        editing_handle: bool,
    },
    /// Browsing for lobbies to join
    Browser {
        browser: LobbyBrowser,
        lobbies: Vec<PeerInfo>,
        selected: usize,
        player_name: String,
    },
    /// Hosting a lobby
    HostLobby {
        lobby: HostedLobby,
        countdown: Option<u32>,
    },
    /// Joined a lobby
    JoinedLobby {
        lobby: JoinedLobby,
        /// Countdown state (seconds remaining, letters, duration)
        countdown: Option<(u32, Vec<char>, u32)>,
    },
    /// Playing a round (solo or multiplayer)
    Playing {
        app: App,
        is_host: bool,
        hosted_lobby: Option<HostedLobby>,
        joined_lobby: Option<JoinedLobby>,
    },
    /// Connection error
    Error {
        message: String,
    },
}

/// Main application coordinator
pub struct AppCoordinator {
    /// Current screen
    pub screen: Screen,
    /// Whether the application should quit
    pub should_quit: bool,
}

impl Default for AppCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl AppCoordinator {
    /// Create a new app coordinator starting at the menu
    pub fn new() -> Self {
        // Try to get a default handle from the environment
        let default_handle = std::env::var("USER")
            .unwrap_or_else(|_| "Player".to_string())
            .chars()
            .take(12)
            .collect::<String>();

        Self {
            screen: Screen::Menu {
                selected: 0,
                handle: default_handle.clone(),
                handle_input: default_handle,
                editing_handle: false,
            },
            should_quit: false,
        }
    }

    /// Quit the application
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Go back to the main menu
    pub fn go_to_menu(&mut self) {
        let handle = self.get_current_handle();
        self.screen = Screen::Menu {
            selected: 0,
            handle: handle.clone(),
            handle_input: handle,
            editing_handle: false,
        };
    }

    /// Get the current player handle
    fn get_current_handle(&self) -> String {
        match &self.screen {
            Screen::Menu { handle, .. } => handle.clone(),
            Screen::Browser { player_name, .. } => player_name.clone(),
            Screen::HostLobby { lobby, .. } => lobby.host_name.clone(),
            Screen::JoinedLobby { lobby, .. } => lobby.player_name.clone(),
            Screen::Playing { .. } => "Player".to_string(),
            Screen::Error { .. } => "Player".to_string(),
        }
    }

    /// Handle menu navigation (up)
    pub fn menu_up(&mut self) {
        if let Screen::Menu { selected, editing_handle, .. } = &mut self.screen {
            if !*editing_handle && *selected > 0 {
                *selected -= 1;
            }
        }
    }

    /// Handle menu navigation (down)
    pub fn menu_down(&mut self) {
        if let Screen::Menu { selected, editing_handle, .. } = &mut self.screen {
            if !*editing_handle && *selected < MenuOption::all().len() - 1 {
                *selected += 1;
            }
        }
    }

    /// Handle menu character input (for handle editing)
    pub fn menu_char(&mut self, c: char) {
        if let Screen::Menu { handle_input, editing_handle, .. } = &mut self.screen {
            if *editing_handle && handle_input.len() < 12 {
                handle_input.push(c);
            }
        }
    }

    /// Handle menu backspace (for handle editing)
    pub fn menu_backspace(&mut self) {
        if let Screen::Menu { handle_input, editing_handle, .. } = &mut self.screen {
            if *editing_handle {
                handle_input.pop();
            }
        }
    }

    /// Handle Tab key to toggle handle editing
    pub fn menu_tab(&mut self) {
        if let Screen::Menu { handle, handle_input, editing_handle, .. } = &mut self.screen {
            if *editing_handle {
                // Finish editing - save the input
                if !handle_input.is_empty() {
                    *handle = handle_input.clone();
                } else {
                    // Restore previous handle if empty
                    *handle_input = handle.clone();
                }
            }
            *editing_handle = !*editing_handle;
        }
    }

    /// Handle menu selection (Enter)
    pub fn menu_select(&mut self) {
        let (selected, handle, _editing_handle) = match &self.screen {
            Screen::Menu { selected, handle, editing_handle, handle_input, .. } => {
                if *editing_handle {
                    // Just finish editing
                    let mut h = handle.clone();
                    if !handle_input.is_empty() {
                        h = handle_input.clone();
                    }
                    self.screen = Screen::Menu {
                        selected: *selected,
                        handle: h.clone(),
                        handle_input: h,
                        editing_handle: false,
                    };
                    return;
                }
                (*selected, handle.clone(), *editing_handle)
            }
            _ => return,
        };

        let option = MenuOption::all()[selected];
        match option {
            MenuOption::StartLobby => {
                match HostedLobby::new(handle) {
                    Ok(lobby) => {
                        self.screen = Screen::HostLobby { lobby, countdown: None };
                    }
                    Err(e) => {
                        self.screen = Screen::Error { message: e };
                    }
                }
            }
            MenuOption::JoinLobby => {
                match LobbyBrowser::new() {
                    Ok(browser) => {
                        self.screen = Screen::Browser {
                            browser,
                            lobbies: Vec::new(),
                            selected: 0,
                            player_name: handle,
                        };
                    }
                    Err(e) => {
                        self.screen = Screen::Error { message: e };
                    }
                }
            }
            MenuOption::SoloPractice => {
                let mut app = App::new();
                let letters = LetterRack::generate().letters().to_vec();
                app.start_round(letters, DEFAULT_ROUND_DURATION);
                self.screen = Screen::Playing {
                    app,
                    is_host: true,
                    hosted_lobby: None,
                    joined_lobby: None,
                };
            }
            MenuOption::Quit => {
                self.should_quit = true;
            }
        }
    }

    /// Browser navigation (up)
    pub fn browser_up(&mut self) {
        if let Screen::Browser { selected, lobbies: _, .. } = &mut self.screen {
            if *selected > 0 {
                *selected -= 1;
            }
        }
    }

    /// Browser navigation (down)
    pub fn browser_down(&mut self) {
        if let Screen::Browser { selected, lobbies, .. } = &mut self.screen {
            if *selected < lobbies.len().saturating_sub(1) {
                *selected += 1;
            }
        }
    }

    /// Browser selection (Enter)
    pub fn browser_select(&mut self) {
        let (peer, player_name) = match &self.screen {
            Screen::Browser { lobbies, selected, player_name, .. } => {
                if lobbies.is_empty() {
                    return;
                }
                (lobbies[*selected].clone(), player_name.clone())
            }
            _ => return,
        };

        match JoinedLobby::join(&peer, player_name) {
            Ok(lobby) => {
                self.screen = Screen::JoinedLobby { lobby, countdown: None };
            }
            Err(e) => {
                self.screen = Screen::Error { message: e };
            }
        }
    }

    /// Host lobby: start the game
    pub fn host_start_round(&mut self) {
        if let Screen::HostLobby { lobby, .. } = &mut self.screen {
            if lobby.can_start() {
                let letters = LetterRack::generate().letters().to_vec();
                lobby.start_round(letters.clone(), DEFAULT_ROUND_DURATION);

                // Transition to playing
                let mut app = App::new();
                app.start_round(letters, DEFAULT_ROUND_DURATION);

                // We need to take ownership of the lobby
                // This is a bit tricky - we'll need to restructure
            }
        }
    }

    /// Poll for updates (call regularly)
    pub fn poll(&mut self) {
        match &mut self.screen {
            Screen::Browser { browser, lobbies, .. } => {
                *lobbies = browser.poll();
            }
            Screen::HostLobby { lobby, .. } => {
                let _events = lobby.poll();
            }
            Screen::JoinedLobby { lobby, countdown } => {
                let events = lobby.poll();
                let mut transition = None;
                for event in events {
                    match event {
                        LobbyEvent::Countdown {
                            letters,
                            duration,
                            countdown: count,
                        } => {
                            *countdown = Some((count, letters, duration));
                        }
                        LobbyEvent::RoundStart { letters, duration } => {
                            transition = Some((letters, duration));
                            break;
                        }
                        LobbyEvent::Disconnected => {
                            self.screen = Screen::Error {
                                message: "Connection lost to host".to_string(),
                            };
                            return;
                        }
                        _ => {}
                    }
                }
                if let Some((letters, duration)) = transition {
                    // Take ownership of the JoinedLobby by replacing the screen
                    let old_screen = std::mem::replace(
                        &mut self.screen,
                        Screen::Error {
                            message: String::new(),
                        },
                    );
                    if let Screen::JoinedLobby { lobby, .. } = old_screen {
                        let player_names: Vec<String> =
                            lobby.players().iter().map(|p| p.name.clone()).collect();
                        let player_name = lobby.player_name.clone();

                        let mut app = App::new();
                        app.set_player_name(player_name);
                        app.set_scoreboard(player_names);
                        app.start_round(letters, duration);

                        self.screen = Screen::Playing {
                            app,
                            is_host: false,
                            hosted_lobby: None,
                            joined_lobby: Some(lobby),
                        };
                    }
                }
            }
            Screen::Playing {
                app,
                hosted_lobby,
                joined_lobby,
                ..
            } => {
                // Process multiplayer events during gameplay
                Self::poll_multiplayer_events(app, hosted_lobby, joined_lobby);
            }
            _ => {}
        }
    }

    /// Process multiplayer events during gameplay
    fn poll_multiplayer_events(
        app: &mut App,
        hosted_lobby: &mut Option<HostedLobby>,
        joined_lobby: &mut Option<JoinedLobby>,
    ) {
        let events: Vec<LobbyEvent> = if let Some(lobby) = hosted_lobby {
            lobby.poll()
        } else if let Some(lobby) = joined_lobby {
            lobby.poll()
        } else {
            return;
        };

        for event in events {
            match event {
                LobbyEvent::ClaimAccepted {
                    word,
                    player_name,
                    points,
                } => {
                    app.on_claim_accepted(word, player_name, points);
                }
                LobbyEvent::ClaimRejected { word, reason } => {
                    app.on_claim_rejected(word, Self::map_reject_reason(reason));
                }
                LobbyEvent::ScoreUpdate { scores } => {
                    app.update_scoreboard(scores);
                }
                LobbyEvent::RoundEnd => {
                    app.force_end_round();
                }
                _ => {}
            }
        }
    }

    /// Convert network ClaimRejectReason to app MissReason (public for main.rs)
    pub fn map_reject_reason_pub(reason: ClaimRejectReason) -> super::state::MissReason {
        Self::map_reject_reason(reason)
    }

    /// Convert network ClaimRejectReason to app MissReason
    fn map_reject_reason(reason: ClaimRejectReason) -> super::state::MissReason {
        match reason {
            ClaimRejectReason::TooShort => super::state::MissReason::TooShort,
            ClaimRejectReason::InvalidLetters { .. } => super::state::MissReason::InvalidLetters,
            ClaimRejectReason::NotInDictionary => super::state::MissReason::NotInDictionary,
            ClaimRejectReason::AlreadyClaimed { by } => {
                super::state::MissReason::AlreadyClaimed { by }
            }
            ClaimRejectReason::RoundEnded => super::state::MissReason::TooShort, // round ended is effectively a rejection
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_option_all() {
        let options = MenuOption::all();
        assert_eq!(options.len(), 4);
        assert_eq!(options[0], MenuOption::StartLobby);
        assert_eq!(options[1], MenuOption::JoinLobby);
        assert_eq!(options[2], MenuOption::SoloPractice);
        assert_eq!(options[3], MenuOption::Quit);
    }

    #[test]
    fn test_menu_option_labels() {
        assert_eq!(MenuOption::StartLobby.label(), "Start Lobby");
        assert_eq!(MenuOption::JoinLobby.label(), "Join Lobby");
        assert_eq!(MenuOption::SoloPractice.label(), "Solo Practice");
        assert_eq!(MenuOption::Quit.label(), "Quit");
    }

    #[test]
    fn test_app_coordinator_starts_at_menu() {
        let app = AppCoordinator::new();
        assert!(!app.should_quit);
        assert!(matches!(app.screen, Screen::Menu { selected: 0, .. }));
    }

    #[test]
    fn test_menu_navigation_up_down() {
        let mut app = AppCoordinator::new();

        // Start at 0, can't go up
        app.menu_up();
        if let Screen::Menu { selected, .. } = &app.screen {
            assert_eq!(*selected, 0);
        }

        // Go down
        app.menu_down();
        if let Screen::Menu { selected, .. } = &app.screen {
            assert_eq!(*selected, 1);
        }

        // Go down again
        app.menu_down();
        if let Screen::Menu { selected, .. } = &app.screen {
            assert_eq!(*selected, 2);
        }

        // Go down to last
        app.menu_down();
        if let Screen::Menu { selected, .. } = &app.screen {
            assert_eq!(*selected, 3);
        }

        // Can't go past last
        app.menu_down();
        if let Screen::Menu { selected, .. } = &app.screen {
            assert_eq!(*selected, 3);
        }

        // Go back up
        app.menu_up();
        if let Screen::Menu { selected, .. } = &app.screen {
            assert_eq!(*selected, 2);
        }
    }

    #[test]
    fn test_handle_editing() {
        let mut app = AppCoordinator::new();

        // Enter editing mode
        app.menu_tab();
        if let Screen::Menu { editing_handle, .. } = &app.screen {
            assert!(*editing_handle);
        }

        // Type characters
        app.menu_char('A');
        app.menu_char('B');
        app.menu_char('C');

        // Exit editing mode
        app.menu_tab();
        if let Screen::Menu { editing_handle, handle, .. } = &app.screen {
            assert!(!*editing_handle);
            assert!(handle.ends_with("ABC") || handle.contains("ABC"));
        }
    }

    #[test]
    fn test_handle_backspace() {
        let mut app = AppCoordinator::new();

        // Enter editing mode
        app.menu_tab();

        // Clear and type new
        // First clear existing content
        for _ in 0..20 {
            app.menu_backspace();
        }

        app.menu_char('X');
        app.menu_char('Y');
        app.menu_backspace();

        if let Screen::Menu { handle_input, .. } = &app.screen {
            assert_eq!(handle_input, "X");
        }
    }

    #[test]
    fn test_handle_max_length() {
        let mut app = AppCoordinator::new();
        app.menu_tab(); // Enter editing mode

        // Clear existing
        for _ in 0..20 {
            app.menu_backspace();
        }

        // Type 15 characters (max is 12)
        for _ in 0..15 {
            app.menu_char('A');
        }

        if let Screen::Menu { handle_input, .. } = &app.screen {
            assert!(handle_input.len() <= 12);
        }
    }

    #[test]
    fn test_navigation_disabled_while_editing() {
        let mut app = AppCoordinator::new();

        // Enter editing mode
        app.menu_tab();

        // Try to navigate
        app.menu_down();
        if let Screen::Menu { selected, .. } = &app.screen {
            assert_eq!(*selected, 0); // Should not move
        }
    }

    #[test]
    fn test_quit() {
        let mut app = AppCoordinator::new();
        assert!(!app.should_quit);
        app.quit();
        assert!(app.should_quit);
    }

    #[test]
    fn test_go_to_menu() {
        let mut app = AppCoordinator::new();

        // Navigate away from initial state
        app.menu_down();
        app.menu_down();

        // Go to menu resets
        app.go_to_menu();
        if let Screen::Menu { selected, .. } = &app.screen {
            assert_eq!(*selected, 0);
        }
    }

    #[test]
    fn test_menu_select_quit() {
        let mut app = AppCoordinator::new();

        // Navigate to Quit (index 3)
        app.menu_down();
        app.menu_down();
        app.menu_down();
        app.menu_select();

        assert!(app.should_quit);
    }

    #[test]
    fn test_menu_select_solo_practice() {
        let mut app = AppCoordinator::new();

        // Navigate to Solo Practice (index 2)
        app.menu_down();
        app.menu_down();
        app.menu_select();

        assert!(matches!(app.screen, Screen::Playing { .. }));
    }

    #[test]
    fn test_map_reject_reasons() {
        assert_eq!(
            AppCoordinator::map_reject_reason_pub(ClaimRejectReason::TooShort),
            super::super::state::MissReason::TooShort
        );
        assert_eq!(
            AppCoordinator::map_reject_reason_pub(ClaimRejectReason::NotInDictionary),
            super::super::state::MissReason::NotInDictionary
        );
        assert!(matches!(
            AppCoordinator::map_reject_reason_pub(ClaimRejectReason::InvalidLetters { missing: vec!['X'] }),
            super::super::state::MissReason::InvalidLetters
        ));
        assert!(matches!(
            AppCoordinator::map_reject_reason_pub(ClaimRejectReason::AlreadyClaimed { by: "Bob".to_string() }),
            super::super::state::MissReason::AlreadyClaimed { by } if by == "Bob"
        ));
    }

    #[test]
    fn test_enter_editing_saves_on_exit() {
        let mut app = AppCoordinator::new();

        // Enter editing
        app.menu_tab();

        // Clear and type a new handle
        for _ in 0..20 {
            app.menu_backspace();
        }
        app.menu_char('Z');
        app.menu_char('E');
        app.menu_char('D');

        // Exit editing
        app.menu_tab();

        if let Screen::Menu { handle, .. } = &app.screen {
            assert_eq!(handle, "ZED");
        }
    }

    #[test]
    fn test_empty_handle_restores_previous() {
        let mut app = AppCoordinator::new();

        // Get current handle
        let original = match &app.screen {
            Screen::Menu { handle, .. } => handle.clone(),
            _ => panic!(),
        };

        // Enter editing
        app.menu_tab();

        // Clear everything
        for _ in 0..20 {
            app.menu_backspace();
        }

        // Exit editing with empty input - should restore
        app.menu_tab();

        if let Screen::Menu { handle, .. } = &app.screen {
            assert_eq!(*handle, original);
        }
    }
}
