//! Lobby management for BLAM! multiplayer games
//!
//! This module provides:
//! - Lobby creation with auto-generated names
//! - Lobby settings (duration, letter count, min word length)
//! - Player list tracking
//! - Host-controlled game configuration

use rand::Rng;

/// Default lobby settings
pub const DEFAULT_DURATION: u32 = 60;
pub const DEFAULT_MIN_WORD_LENGTH: u8 = 3;
pub const DEFAULT_MIN_LETTERS: u8 = 12;
pub const DEFAULT_MAX_LETTERS: u8 = 20;

/// Word lists for generating auto-lobby names
/// Format: ADJECTIVE-NOUN (like "LAN-ORBIT")
const NAME_ADJECTIVES: &[&str] = &[
    "LAN", "FAST", "WORD", "TYPE", "BLAM", "WILD", "QUICK", "SHARP",
    "BOLD", "KEEN", "MEGA", "ULTRA", "HYPER", "TURBO", "SUPER", "PRIME",
];

const NAME_NOUNS: &[&str] = &[
    "ORBIT", "ARENA", "CLASH", "STORM", "BLITZ", "SURGE", "BURST", "FLASH",
    "SPARK", "ZONE", "CORE", "NEXUS", "FORGE", "VAULT", "DOME", "BASE",
];

/// Configurable lobby settings controlled by the host
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbySettings {
    /// Round duration in seconds
    pub duration_secs: u32,
    /// Minimum word length to accept
    pub min_word_length: u8,
    /// Minimum number of letters in the rack
    pub min_letters: u8,
    /// Maximum number of letters in the rack
    pub max_letters: u8,
}

impl Default for LobbySettings {
    fn default() -> Self {
        Self {
            duration_secs: DEFAULT_DURATION,
            min_word_length: DEFAULT_MIN_WORD_LENGTH,
            min_letters: DEFAULT_MIN_LETTERS,
            max_letters: DEFAULT_MAX_LETTERS,
        }
    }
}

impl LobbySettings {
    /// Create settings with custom duration
    pub fn with_duration(mut self, secs: u32) -> Self {
        self.duration_secs = secs;
        self
    }

    /// Create settings with custom min word length
    pub fn with_min_word_length(mut self, len: u8) -> Self {
        self.min_word_length = len;
        self
    }

    /// Create settings with custom letter range
    pub fn with_letter_range(mut self, min: u8, max: u8) -> Self {
        self.min_letters = min;
        self.max_letters = max;
        self
    }
}

/// A player in the lobby
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    /// Player's display name/handle
    pub name: String,
    /// Whether this player is the host
    pub is_host: bool,
    /// Whether this player is ready (for future use)
    pub is_ready: bool,
}

impl Player {
    /// Create a new player
    pub fn new(name: String, is_host: bool) -> Self {
        Self {
            name,
            is_host,
            is_ready: false,
        }
    }
}

/// A lobby for a BLAM! game session
#[derive(Debug, Clone)]
pub struct Lobby {
    /// Auto-generated lobby name
    pub name: String,
    /// Game settings controlled by host
    pub settings: LobbySettings,
    /// List of players in the lobby
    pub players: Vec<Player>,
    /// Name of the local player (for identifying self)
    pub local_player: String,
    /// Whether we are the host
    pub is_host: bool,
}

impl Lobby {
    /// Create a new lobby as the host
    pub fn create(player_name: String) -> Self {
        let name = generate_lobby_name();
        Self {
            name,
            settings: LobbySettings::default(),
            players: vec![Player::new(player_name.clone(), true)],
            local_player: player_name,
            is_host: true,
        }
    }

    /// Join an existing lobby
    pub fn join(lobby_name: String, player_name: String) -> Self {
        Self {
            name: lobby_name,
            settings: LobbySettings::default(),
            players: vec![Player::new(player_name.clone(), false)],
            local_player: player_name,
            is_host: false,
        }
    }

    /// Add a player to the lobby
    pub fn add_player(&mut self, name: String) {
        // Don't add duplicates
        if !self.players.iter().any(|p| p.name == name) {
            self.players.push(Player::new(name, false));
        }
    }

    /// Remove a player from the lobby
    pub fn remove_player(&mut self, name: &str) {
        self.players.retain(|p| p.name != name);
    }

    /// Update lobby settings (host only)
    pub fn update_settings(&mut self, settings: LobbySettings) {
        self.settings = settings;
    }

    /// Get the number of players
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Check if we can start the game (host only, at least 1 player)
    pub fn can_start(&self) -> bool {
        self.is_host && !self.players.is_empty()
    }

    /// Get the host's name
    pub fn host_name(&self) -> Option<&str> {
        self.players
            .iter()
            .find(|p| p.is_host)
            .map(|p| p.name.as_str())
    }
}

/// Generate an auto-lobby name like "LAN-ORBIT"
pub fn generate_lobby_name() -> String {
    let mut rng = rand::rng();
    let adj = NAME_ADJECTIVES[rng.random_range(0..NAME_ADJECTIVES.len())];
    let noun = NAME_NOUNS[rng.random_range(0..NAME_NOUNS.len())];
    format!("{}-{}", adj, noun)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = LobbySettings::default();
        assert_eq!(settings.duration_secs, 60);
        assert_eq!(settings.min_word_length, 3);
        assert_eq!(settings.min_letters, 12);
        assert_eq!(settings.max_letters, 20);
    }

    #[test]
    fn test_settings_builder() {
        let settings = LobbySettings::default()
            .with_duration(90)
            .with_min_word_length(4)
            .with_letter_range(15, 25);

        assert_eq!(settings.duration_secs, 90);
        assert_eq!(settings.min_word_length, 4);
        assert_eq!(settings.min_letters, 15);
        assert_eq!(settings.max_letters, 25);
    }

    #[test]
    fn test_create_lobby() {
        let lobby = Lobby::create("Alice".to_string());
        assert!(lobby.is_host);
        assert_eq!(lobby.player_count(), 1);
        assert_eq!(lobby.players[0].name, "Alice");
        assert!(lobby.players[0].is_host);
    }

    #[test]
    fn test_join_lobby() {
        let lobby = Lobby::join("TEST-LOBBY".to_string(), "Bob".to_string());
        assert!(!lobby.is_host);
        assert_eq!(lobby.name, "TEST-LOBBY");
        assert_eq!(lobby.player_count(), 1);
        assert!(!lobby.players[0].is_host);
    }

    #[test]
    fn test_add_remove_players() {
        let mut lobby = Lobby::create("Alice".to_string());

        lobby.add_player("Bob".to_string());
        assert_eq!(lobby.player_count(), 2);

        lobby.add_player("Charlie".to_string());
        assert_eq!(lobby.player_count(), 3);

        // No duplicates
        lobby.add_player("Bob".to_string());
        assert_eq!(lobby.player_count(), 3);

        lobby.remove_player("Bob");
        assert_eq!(lobby.player_count(), 2);
        assert!(lobby.players.iter().all(|p| p.name != "Bob"));
    }

    #[test]
    fn test_generate_lobby_name() {
        let name = generate_lobby_name();
        assert!(name.contains('-'));

        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2);
        assert!(NAME_ADJECTIVES.contains(&parts[0]));
        assert!(NAME_NOUNS.contains(&parts[1]));
    }

    #[test]
    fn test_can_start() {
        let lobby = Lobby::create("Alice".to_string());
        assert!(lobby.can_start());

        let lobby = Lobby::join("TEST".to_string(), "Bob".to_string());
        assert!(!lobby.can_start()); // Not host
    }

    #[test]
    fn test_host_name() {
        let lobby = Lobby::create("Alice".to_string());
        assert_eq!(lobby.host_name(), Some("Alice"));
    }
}
