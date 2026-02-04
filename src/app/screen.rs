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
use crate::lobby::{HostedLobby, JoinedLobby, LobbyBrowser, LobbyEvent, Player};
use crate::network::PeerInfo;

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
        let (selected, handle, editing_handle) = match &self.screen {
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
                };
            }
            MenuOption::Quit => {
                self.should_quit = true;
            }
        }
    }

    /// Browser navigation (up)
    pub fn browser_up(&mut self) {
        if let Screen::Browser { selected, lobbies, .. } = &mut self.screen {
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
                // Events are handled - player list is updated internally
            }
            Screen::JoinedLobby { lobby, countdown } => {
                let events = lobby.poll();
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
                            let mut app = App::new();
                            app.start_round(letters, duration);
                            self.screen = Screen::Playing {
                                app,
                                is_host: false,
                                hosted_lobby: None,
                            };
                            return;
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
            }
            _ => {}
        }
    }
}
