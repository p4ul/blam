#![allow(dead_code)]
//! Lobby management for multiplayer games
//!
//! Handles:
//! - Hosting a lobby (server + mDNS advertisement)
//! - Joining a lobby (client connection)
//! - Player list management
//! - Synchronized round start
//! - Claim arbitration during gameplay

use crate::game::arbitrator::{ClaimResult, RoundArbitrator};
use crate::network::{
    ClaimRejectReason, Client, DiscoveryEvent, Message, PeerInfo, PeerTracker, Server, ServerEvent,
    ServiceDiscovery,
};
use rand::prelude::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::Receiver;
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum number of players in a lobby
pub const MAX_PLAYERS: usize = 12;

/// Minimum number of players to start a game
pub const MIN_PLAYERS: usize = 2;

/// A player in the lobby
#[derive(Debug, Clone)]
pub struct Player {
    /// Player's display name
    pub name: String,
    /// Whether the player is ready
    pub ready: bool,
    /// Whether this is the local player
    pub is_local: bool,
    /// Whether this is the host
    pub is_host: bool,
}

/// State of the lobby
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyState {
    /// Waiting for players to join
    Waiting,
    /// Countdown to start (3, 2, 1...)
    Countdown(u32),
    /// Game is starting
    Starting,
}

/// Events from the lobby
#[derive(Debug, Clone)]
pub enum LobbyEvent {
    /// A player joined the lobby
    PlayerJoined(String),
    /// A player left the lobby
    PlayerLeft(String),
    /// Countdown to round start
    Countdown {
        letters: Vec<char>,
        duration: u32,
        countdown: u32,
    },
    /// The round is starting with these letters
    RoundStart { letters: Vec<char>, duration: u32 },
    /// A claim was accepted (broadcast to all)
    ClaimAccepted {
        word: String,
        player_name: String,
        points: u32,
    },
    /// A claim was rejected (sent to requester only)
    ClaimRejected {
        word: String,
        reason: ClaimRejectReason,
    },
    /// Word claimed event for CRDT log (broadcast to all)
    WordClaimed {
        word: String,
        player_name: String,
        points: u32,
        actor_id: String,
        timestamp_ms: u64,
        claim_sequence: u64,
    },
    /// Score update
    ScoreUpdate { scores: Vec<(String, u32)> },
    /// Round has ended
    RoundEnd,
    /// Connection was lost
    Disconnected,
}

/// A hosted lobby (server side)
pub struct HostedLobby {
    /// Our player name
    pub host_name: String,
    /// Lobby name (auto-generated)
    pub lobby_name: String,
    /// TCP server for connections
    server: Server,
    /// mDNS service discovery
    discovery: ServiceDiscovery,
    /// Players in the lobby (including host)
    players: Vec<Player>,
    /// Mapping from socket address to player index
    addr_to_player: HashMap<SocketAddr, usize>,
    /// Mapping from player name to socket address (for non-host players)
    player_to_addr: HashMap<String, SocketAddr>,
    /// Current lobby state
    pub state: LobbyState,
    /// Actor ID for this instance
    actor_id: String,
    /// Round arbitrator (active during gameplay)
    arbitrator: Option<RoundArbitrator>,
    /// Current letters for the round
    current_letters: Vec<char>,
    /// Round duration (seconds)
    round_duration: u32,
    /// Current countdown value (seconds remaining until start)
    countdown_remaining: u32,
}

impl HostedLobby {
    /// Create a new hosted lobby
    pub fn new(host_name: String) -> Result<Self, String> {
        // Generate a unique actor ID
        let actor_id = format!("blam-{:08x}", rand::rng().random::<u32>());

        // Generate a lobby name
        let lobby_name = generate_lobby_name();

        // Start the server
        let server = Server::start().map_err(|e| format!("Failed to start server: {}", e))?;
        let port = server.port();

        // Create mDNS discovery
        let mut discovery = ServiceDiscovery::new(actor_id.clone())?;

        // Advertise our lobby
        discovery.advertise(&host_name, Some(&lobby_name), port)?;

        // Add host as the first player
        let host_player = Player {
            name: host_name.clone(),
            ready: true,
            is_local: true,
            is_host: true,
        };

        Ok(Self {
            host_name,
            lobby_name,
            server,
            discovery,
            players: vec![host_player],
            addr_to_player: HashMap::new(),
            player_to_addr: HashMap::new(),
            state: LobbyState::Waiting,
            actor_id,
            arbitrator: None,
            current_letters: Vec::new(),
            round_duration: 0,
            countdown_remaining: 0,
        })
    }

    /// Get the port the server is listening on
    pub fn port(&self) -> u16 {
        self.server.port()
    }

    /// Get all players in the lobby
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// Get the number of players
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Check if we can start the game
    pub fn can_start(&self) -> bool {
        self.players.len() >= MIN_PLAYERS && self.state == LobbyState::Waiting
    }

    /// Poll for lobby events
    pub fn poll(&mut self) -> Vec<LobbyEvent> {
        let mut events = Vec::new();

        // Poll server for new connections and messages
        for server_event in self.server.poll() {
            match server_event {
                ServerEvent::PeerConnected { addr } => {
                    // Don't add player yet - wait for Join message
                    // Just track that someone connected
                    let _ = addr;
                }
                ServerEvent::PeerDisconnected { addr, player_name } => {
                    if let Some(idx) = self.addr_to_player.remove(&addr) {
                        if idx < self.players.len() {
                            let player = self.players.remove(idx);
                            self.player_to_addr.remove(&player.name);
                            events.push(LobbyEvent::PlayerLeft(player.name.clone()));

                            // Update indices for remaining players
                            for (_a, i) in self.addr_to_player.iter_mut() {
                                if *i > idx {
                                    *i -= 1;
                                }
                            }
                        }
                    } else if let Some(name) = player_name {
                        self.player_to_addr.remove(&name);
                        events.push(LobbyEvent::PlayerLeft(name));
                    }
                }
                ServerEvent::MessageReceived { from, message, .. } => {
                    match message {
                        Message::Join { player_name } => {
                            // Check if we're at capacity
                            if self.players.len() >= MAX_PLAYERS {
                                // TODO: Send rejection message
                                continue;
                            }

                            // Add the player
                            let player = Player {
                                name: player_name.clone(),
                                ready: true,
                                is_local: false,
                                is_host: false,
                            };
                            let idx = self.players.len();
                            self.players.push(player);
                            self.addr_to_player.insert(from, idx);
                            self.player_to_addr.insert(player_name.clone(), from);

                            events.push(LobbyEvent::PlayerJoined(player_name));
                        }
                        Message::Leave { player_name } => {
                            if let Some(idx) = self.addr_to_player.remove(&from) {
                                if idx < self.players.len() {
                                    self.players.remove(idx);
                                    // Update indices
                                    for (_, i) in self.addr_to_player.iter_mut() {
                                        if *i > idx {
                                            *i -= 1;
                                        }
                                    }
                                }
                            }
                            self.player_to_addr.remove(&player_name);
                            events.push(LobbyEvent::PlayerLeft(player_name));
                        }
                        Message::ClaimAttempt { word } => {
                            // Handle claim attempt from a player
                            if let Some(idx) = self.addr_to_player.get(&from) {
                                if let Some(player) = self.players.get(*idx) {
                                    let player_name = player.name.clone();
                                    if let Some(claim_events) =
                                        self.handle_claim_attempt(&word, &player_name, Some(from))
                                    {
                                        events.extend(claim_events);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        events
    }

    /// Handle a claim attempt (can be called for host's own claims too)
    fn handle_claim_attempt(
        &mut self,
        word: &str,
        player_name: &str,
        requester_addr: Option<SocketAddr>,
    ) -> Option<Vec<LobbyEvent>> {
        let arbitrator = self.arbitrator.as_mut()?;

        let result = arbitrator.try_claim(word, player_name);

        match result {
            ClaimResult::Accepted { points, claim_sequence } => {
                let word_upper = word.to_uppercase();

                // Get timestamp for CRDT event
                let timestamp_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);

                // Broadcast ClaimAccepted to all clients
                let msg = Message::ClaimAccepted {
                    word: word_upper.clone(),
                    player_name: player_name.to_string(),
                    points,
                };
                self.server.broadcast(&msg);

                // Broadcast WordClaimed for CRDT log
                let crdt_msg = Message::WordClaimed {
                    word: word_upper.clone(),
                    player_name: player_name.to_string(),
                    points,
                    actor_id: self.actor_id.clone(),
                    timestamp_ms,
                    claim_sequence,
                };
                self.server.broadcast(&crdt_msg);

                // Also broadcast updated scores
                let scores = arbitrator.scores();
                let score_msg = Message::ScoreUpdate { scores: scores.clone() };
                self.server.broadcast(&score_msg);

                Some(vec![
                    LobbyEvent::ClaimAccepted {
                        word: word_upper.clone(),
                        player_name: player_name.to_string(),
                        points,
                    },
                    LobbyEvent::WordClaimed {
                        word: word_upper,
                        player_name: player_name.to_string(),
                        points,
                        actor_id: self.actor_id.clone(),
                        timestamp_ms,
                        claim_sequence,
                    },
                    LobbyEvent::ScoreUpdate { scores },
                ])
            }
            ClaimResult::AlreadyClaimed { by } => {
                let reason = ClaimRejectReason::AlreadyClaimed { by };
                self.send_rejection(word, &reason, requester_addr);
                Some(vec![LobbyEvent::ClaimRejected {
                    word: word.to_uppercase(),
                    reason,
                }])
            }
            ClaimResult::TooShort => {
                let reason = ClaimRejectReason::TooShort;
                self.send_rejection(word, &reason, requester_addr);
                Some(vec![LobbyEvent::ClaimRejected {
                    word: word.to_uppercase(),
                    reason,
                }])
            }
            ClaimResult::InvalidLetters { missing } => {
                let reason = ClaimRejectReason::InvalidLetters { missing };
                self.send_rejection(word, &reason, requester_addr);
                Some(vec![LobbyEvent::ClaimRejected {
                    word: word.to_uppercase(),
                    reason,
                }])
            }
            ClaimResult::NotInDictionary => {
                let reason = ClaimRejectReason::NotInDictionary;
                self.send_rejection(word, &reason, requester_addr);
                Some(vec![LobbyEvent::ClaimRejected {
                    word: word.to_uppercase(),
                    reason,
                }])
            }
            ClaimResult::RoundEnded => {
                let reason = ClaimRejectReason::RoundEnded;
                self.send_rejection(word, &reason, requester_addr);
                Some(vec![LobbyEvent::ClaimRejected {
                    word: word.to_uppercase(),
                    reason,
                }])
            }
        }
    }

    /// Send rejection message to a specific client
    fn send_rejection(
        &self,
        word: &str,
        reason: &ClaimRejectReason,
        requester_addr: Option<SocketAddr>,
    ) {
        if let Some(addr) = requester_addr {
            let msg = Message::ClaimRejected {
                word: word.to_uppercase(),
                reason: reason.clone(),
            };
            let _ = self.server.send_to(addr, &msg);
        }
    }

    /// Host submits a claim (called from local gameplay)
    pub fn host_claim(&mut self, word: &str) -> Option<Vec<LobbyEvent>> {
        self.handle_claim_attempt(word, &self.host_name.clone(), None)
    }

    /// End the current round
    pub fn end_round(&mut self) -> Vec<LobbyEvent> {
        if let Some(arbitrator) = &mut self.arbitrator {
            arbitrator.end_round();
        }
        self.state = LobbyState::Waiting;

        // Broadcast round end to all clients
        self.server.broadcast(&Message::RoundEnd);

        // Get final scores
        let scores = self
            .arbitrator
            .as_ref()
            .map(|a| a.scores())
            .unwrap_or_default();

        vec![
            LobbyEvent::RoundEnd,
            LobbyEvent::ScoreUpdate { scores },
        ]
    }

    /// Get current scores
    pub fn scores(&self) -> Vec<(String, u32)> {
        self.arbitrator
            .as_ref()
            .map(|a| a.scores())
            .unwrap_or_default()
    }

    /// Start the countdown sequence (3-2-1-BLAM!)
    /// Returns the initial countdown value
    pub fn start_countdown(&mut self, letters: Vec<char>, duration: u32) -> u32 {
        const COUNTDOWN_SECONDS: u32 = 3;

        self.current_letters = letters.clone();
        self.round_duration = duration;
        self.countdown_remaining = COUNTDOWN_SECONDS;
        self.state = LobbyState::Countdown(COUNTDOWN_SECONDS);

        // Broadcast countdown to all clients
        let msg = Message::Countdown {
            letters,
            duration_secs: duration,
            countdown_secs: COUNTDOWN_SECONDS,
        };
        self.server.broadcast(&msg);

        COUNTDOWN_SECONDS
    }

    /// Tick the countdown, returns true if countdown finished and round should start
    pub fn tick_countdown(&mut self) -> Option<LobbyEvent> {
        if let LobbyState::Countdown(count) = &mut self.state {
            if *count > 1 {
                *count -= 1;
                self.countdown_remaining = *count;

                // Broadcast updated countdown
                let msg = Message::Countdown {
                    letters: self.current_letters.clone(),
                    duration_secs: self.round_duration,
                    countdown_secs: *count,
                };
                self.server.broadcast(&msg);

                Some(LobbyEvent::Countdown {
                    letters: self.current_letters.clone(),
                    duration: self.round_duration,
                    countdown: *count,
                })
            } else {
                // Countdown finished - start the round
                self.begin_round();
                Some(LobbyEvent::RoundStart {
                    letters: self.current_letters.clone(),
                    duration: self.round_duration,
                })
            }
        } else {
            None
        }
    }

    /// Internal: Actually begin the round after countdown
    fn begin_round(&mut self) {
        self.state = LobbyState::Starting;

        // Create the arbitrator with all player names
        let player_names: Vec<String> = self.players.iter().map(|p| p.name.clone()).collect();
        self.arbitrator = Some(RoundArbitrator::new(
            self.current_letters.clone(),
            &player_names,
        ));

        // Broadcast round start to all connected clients
        let msg = Message::RoundStart {
            letters: self.current_letters.clone(),
            duration_secs: self.round_duration,
        };
        self.server.broadcast(&msg);
    }

    /// Get the current countdown remaining (0 if not in countdown)
    pub fn countdown_remaining(&self) -> u32 {
        self.countdown_remaining
    }

    /// Get the current letters (for display during countdown)
    pub fn current_letters(&self) -> &[char] {
        &self.current_letters
    }

    /// Get the round duration (for display during countdown)
    pub fn round_duration(&self) -> u32 {
        self.round_duration
    }

    /// Start the round - broadcast to all players
    pub fn start_round(&mut self, letters: Vec<char>, duration: u32) {
        self.state = LobbyState::Starting;
        self.current_letters = letters.clone();

        // Create the arbitrator with all player names
        let player_names: Vec<String> = self.players.iter().map(|p| p.name.clone()).collect();
        self.arbitrator = Some(RoundArbitrator::new(letters.clone(), &player_names));

        // Broadcast round start to all connected clients
        let msg = Message::RoundStart {
            letters,
            duration_secs: duration,
        };
        self.server.broadcast(&msg);
    }

    /// Clean up and stop the lobby
    pub fn shutdown(mut self) -> Result<(), String> {
        self.discovery.stop_advertising()?;
        self.server.stop();
        self.discovery.shutdown()?;
        Ok(())
    }
}

/// A joined lobby (client side)
pub struct JoinedLobby {
    /// Our player name
    pub player_name: String,
    /// The lobby we joined
    pub lobby_name: String,
    /// Host's name
    pub host_name: String,
    /// Connection to the host
    client: Client,
    /// Players in the lobby (as reported by host)
    players: Vec<Player>,
    /// Current lobby state
    pub state: LobbyState,
    /// Letters for upcoming round (set during countdown)
    pending_letters: Vec<char>,
    /// Duration for upcoming round (set during countdown)
    pending_duration: u32,
    /// Current countdown value
    countdown_remaining: u32,
}

impl JoinedLobby {
    /// Join a lobby by connecting to a peer
    pub fn join(peer: &PeerInfo, player_name: String) -> Result<Self, String> {
        // Get the first available address
        let addr = peer
            .addresses
            .first()
            .ok_or("No address available for peer")?;

        let socket_addr = std::net::SocketAddr::new(*addr, peer.port);

        // Connect to the host
        let mut client = Client::connect_addr(socket_addr, player_name.clone())
            .map_err(|e| format!("Failed to connect: {}", e))?;

        // Send join message
        client.join().map_err(|e| format!("Failed to join: {}", e))?;

        // Create initial player list (just us and the host)
        let host_player = Player {
            name: peer.handle.clone(),
            ready: true,
            is_local: false,
            is_host: true,
        };

        let our_player = Player {
            name: player_name.clone(),
            ready: true,
            is_local: true,
            is_host: false,
        };

        Ok(Self {
            player_name,
            lobby_name: peer.lobby_name.clone().unwrap_or_else(|| "Unknown".to_string()),
            host_name: peer.handle.clone(),
            client,
            players: vec![host_player, our_player],
            state: LobbyState::Waiting,
            pending_letters: Vec::new(),
            pending_duration: 0,
            countdown_remaining: 0,
        })
    }

    /// Get all players in the lobby
    pub fn players(&self) -> &[Player] {
        &self.players
    }

    /// Get the number of players
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Poll for lobby events
    pub fn poll(&mut self) -> Vec<LobbyEvent> {
        let mut events = Vec::new();

        // Check if still connected
        if !self.client.is_connected() {
            events.push(LobbyEvent::Disconnected);
            return events;
        }

        // Poll for messages from host
        for msg in self.client.poll() {
            match msg {
                Message::Countdown {
                    letters,
                    duration_secs,
                    countdown_secs,
                } => {
                    self.pending_letters = letters.clone();
                    self.pending_duration = duration_secs;
                    self.countdown_remaining = countdown_secs;
                    self.state = LobbyState::Countdown(countdown_secs);
                    events.push(LobbyEvent::Countdown {
                        letters,
                        duration: duration_secs,
                        countdown: countdown_secs,
                    });
                }
                Message::RoundStart { letters, duration_secs } => {
                    self.state = LobbyState::Starting;
                    self.countdown_remaining = 0;
                    events.push(LobbyEvent::RoundStart {
                        letters,
                        duration: duration_secs,
                    });
                }
                Message::Join { player_name } => {
                    // Another player joined
                    let player = Player {
                        name: player_name.clone(),
                        ready: true,
                        is_local: false,
                        is_host: false,
                    };
                    self.players.push(player);
                    events.push(LobbyEvent::PlayerJoined(player_name));
                }
                Message::Leave { player_name } => {
                    self.players.retain(|p| p.name != player_name);
                    events.push(LobbyEvent::PlayerLeft(player_name));
                }
                Message::ClaimAccepted {
                    word,
                    player_name,
                    points,
                } => {
                    events.push(LobbyEvent::ClaimAccepted {
                        word,
                        player_name,
                        points,
                    });
                }
                Message::ClaimRejected { word, reason } => {
                    events.push(LobbyEvent::ClaimRejected { word, reason });
                }
                Message::WordClaimed {
                    word,
                    player_name,
                    points,
                    actor_id,
                    timestamp_ms,
                    claim_sequence,
                } => {
                    events.push(LobbyEvent::WordClaimed {
                        word,
                        player_name,
                        points,
                        actor_id,
                        timestamp_ms,
                        claim_sequence,
                    });
                }
                Message::ScoreUpdate { scores } => {
                    events.push(LobbyEvent::ScoreUpdate { scores });
                }
                Message::RoundEnd => {
                    self.state = LobbyState::Waiting;
                    events.push(LobbyEvent::RoundEnd);
                }
                _ => {}
            }
        }

        events
    }

    /// Get the current countdown remaining (0 if not in countdown)
    pub fn countdown_remaining(&self) -> u32 {
        self.countdown_remaining
    }

    /// Get the pending letters (for display during countdown)
    pub fn pending_letters(&self) -> &[char] {
        &self.pending_letters
    }

    /// Get the pending round duration (for display during countdown)
    pub fn pending_duration(&self) -> u32 {
        self.pending_duration
    }

    /// Send a claim attempt to the host
    pub fn send_claim(&self, word: &str) -> Result<(), String> {
        self.client
            .send_claim_attempt(word)
            .map_err(|e| format!("Failed to send claim: {}", e))
    }

    /// Leave the lobby
    pub fn leave(self) {
        let _ = self.client.leave();
    }
}

/// Lobby browser for finding available lobbies on the network
pub struct LobbyBrowser {
    /// mDNS service discovery
    discovery: ServiceDiscovery,
    /// Receiver for discovery events
    discovery_rx: Receiver<DiscoveryEvent>,
    /// Discovered peers
    peers: PeerTracker,
    /// Actor ID for this instance
    actor_id: String,
}

impl LobbyBrowser {
    /// Create a new lobby browser
    pub fn new() -> Result<Self, String> {
        let actor_id = format!("blam-{:08x}", rand::rng().random::<u32>());
        let discovery = ServiceDiscovery::new(actor_id.clone())?;
        let discovery_rx = discovery.browse()?;

        Ok(Self {
            discovery,
            discovery_rx,
            peers: PeerTracker::new(),
            actor_id,
        })
    }

    /// Poll for discovered lobbies
    pub fn poll(&mut self) -> Vec<PeerInfo> {
        // Process discovery events
        while let Ok(event) = self.discovery_rx.try_recv() {
            match event {
                DiscoveryEvent::PeerDiscovered(peer) => {
                    // Only track peers that are hosting a lobby
                    if peer.lobby_name.is_some() {
                        self.peers.update(peer);
                    }
                }
                DiscoveryEvent::PeerLost(actor_id) => {
                    self.peers.remove(&actor_id);
                }
            }
        }

        // Return list of available lobbies
        self.peers.peers().cloned().collect()
    }

    /// Stop browsing
    pub fn stop(self) -> Result<(), String> {
        self.discovery.stop_browsing()?;
        self.discovery.shutdown()
    }
}

/// Generate a random lobby name
fn generate_lobby_name() -> String {
    const ADJECTIVES: &[&str] = &[
        "SWIFT", "BOLD", "WILD", "FAST", "KEEN", "EPIC", "NOVA", "STAR",
    ];
    const NOUNS: &[&str] = &[
        "ORBIT", "BLAZE", "STORM", "QUEST", "RUSH", "DASH", "BOLT", "ZOOM",
    ];

    let mut rng = rand::rng();
    let adj = ADJECTIVES[rng.random_range(0..ADJECTIVES.len())];
    let noun = NOUNS[rng.random_range(0..NOUNS.len())];
    format!("{}-{}", adj, noun)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lobby_name_generation() {
        let name = generate_lobby_name();
        assert!(name.contains('-'));
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn test_player_count_limits() {
        assert!(MIN_PLAYERS >= 2);
        assert!(MAX_PLAYERS <= 12);
        assert!(MIN_PLAYERS <= MAX_PLAYERS);
    }

    #[test]
    fn test_lobby_name_unique() {
        // Generate many names and verify they're non-empty
        let names: Vec<String> = (0..20).map(|_| generate_lobby_name()).collect();
        for name in &names {
            assert!(!name.is_empty());
            assert!(name.contains('-'));
        }
    }

    #[test]
    fn test_lobby_state_equality() {
        assert_eq!(LobbyState::Waiting, LobbyState::Waiting);
        assert_eq!(LobbyState::Countdown(3), LobbyState::Countdown(3));
        assert_ne!(LobbyState::Countdown(3), LobbyState::Countdown(2));
        assert_ne!(LobbyState::Waiting, LobbyState::Starting);
    }

    #[test]
    fn test_player_struct() {
        let player = Player {
            name: "Alice".to_string(),
            ready: true,
            is_local: true,
            is_host: true,
        };
        assert_eq!(player.name, "Alice");
        assert!(player.ready);
        assert!(player.is_local);
        assert!(player.is_host);
    }

    #[test]
    fn test_player_not_host_not_local() {
        let player = Player {
            name: "Bob".to_string(),
            ready: false,
            is_local: false,
            is_host: false,
        };
        assert_eq!(player.name, "Bob");
        assert!(!player.ready);
        assert!(!player.is_local);
        assert!(!player.is_host);
    }

    #[test]
    fn test_lobby_state_all_variants() {
        let waiting = LobbyState::Waiting;
        let countdown = LobbyState::Countdown(3);
        let starting = LobbyState::Starting;

        assert_eq!(waiting, LobbyState::Waiting);
        assert_eq!(countdown, LobbyState::Countdown(3));
        assert_eq!(starting, LobbyState::Starting);

        // Different variants aren't equal
        assert_ne!(waiting, starting);
        assert_ne!(countdown, waiting);
        assert_ne!(starting, countdown);
    }

    #[test]
    fn test_lobby_event_player_joined() {
        let event = LobbyEvent::PlayerJoined("Alice".to_string());
        if let LobbyEvent::PlayerJoined(name) = event {
            assert_eq!(name, "Alice");
        } else {
            panic!("Expected PlayerJoined");
        }
    }

    #[test]
    fn test_lobby_event_player_left() {
        let event = LobbyEvent::PlayerLeft("Bob".to_string());
        if let LobbyEvent::PlayerLeft(name) = event {
            assert_eq!(name, "Bob");
        } else {
            panic!("Expected PlayerLeft");
        }
    }

    #[test]
    fn test_lobby_event_claim_accepted() {
        let event = LobbyEvent::ClaimAccepted {
            word: "CAT".to_string(),
            player_name: "Alice".to_string(),
            points: 3,
        };
        if let LobbyEvent::ClaimAccepted { word, player_name, points } = event {
            assert_eq!(word, "CAT");
            assert_eq!(player_name, "Alice");
            assert_eq!(points, 3);
        } else {
            panic!("Expected ClaimAccepted");
        }
    }

    #[test]
    fn test_lobby_event_claim_rejected() {
        let event = LobbyEvent::ClaimRejected {
            word: "XYZ".to_string(),
            reason: ClaimRejectReason::NotInDictionary,
        };
        if let LobbyEvent::ClaimRejected { word, reason } = event {
            assert_eq!(word, "XYZ");
            assert_eq!(reason, ClaimRejectReason::NotInDictionary);
        } else {
            panic!("Expected ClaimRejected");
        }
    }

    #[test]
    fn test_lobby_event_round_start() {
        let event = LobbyEvent::RoundStart {
            letters: vec!['A', 'B', 'C'],
            duration: 60,
        };
        if let LobbyEvent::RoundStart { letters, duration } = event {
            assert_eq!(letters, vec!['A', 'B', 'C']);
            assert_eq!(duration, 60);
        } else {
            panic!("Expected RoundStart");
        }
    }

    #[test]
    fn test_lobby_event_round_end() {
        let event = LobbyEvent::RoundEnd;
        assert!(matches!(event, LobbyEvent::RoundEnd));
    }

    #[test]
    fn test_lobby_event_disconnected() {
        let event = LobbyEvent::Disconnected;
        assert!(matches!(event, LobbyEvent::Disconnected));
    }

    #[test]
    fn test_lobby_event_score_update() {
        let event = LobbyEvent::ScoreUpdate {
            scores: vec![("Alice".to_string(), 10), ("Bob".to_string(), 5)],
        };
        if let LobbyEvent::ScoreUpdate { scores } = event {
            assert_eq!(scores.len(), 2);
            assert_eq!(scores[0].0, "Alice");
            assert_eq!(scores[0].1, 10);
        } else {
            panic!("Expected ScoreUpdate");
        }
    }

    #[test]
    fn test_lobby_event_countdown() {
        let event = LobbyEvent::Countdown {
            letters: vec!['B', 'L', 'A', 'M'],
            duration: 60,
            countdown: 3,
        };
        if let LobbyEvent::Countdown { letters, duration, countdown } = event {
            assert_eq!(letters.len(), 4);
            assert_eq!(duration, 60);
            assert_eq!(countdown, 3);
        } else {
            panic!("Expected Countdown");
        }
    }

    #[test]
    fn test_lobby_event_word_claimed() {
        let event = LobbyEvent::WordClaimed {
            word: "BLAM".to_string(),
            player_name: "Alice".to_string(),
            points: 4,
            actor_id: "test-123".to_string(),
            timestamp_ms: 1000000,
            claim_sequence: 1,
        };
        if let LobbyEvent::WordClaimed { word, player_name, points, actor_id, timestamp_ms, claim_sequence } = event {
            assert_eq!(word, "BLAM");
            assert_eq!(player_name, "Alice");
            assert_eq!(points, 4);
            assert_eq!(actor_id, "test-123");
            assert_eq!(timestamp_ms, 1000000);
            assert_eq!(claim_sequence, 1);
        } else {
            panic!("Expected WordClaimed");
        }
    }

    #[test]
    fn test_lobby_name_format() {
        // Verify names follow ADJ-NOUN format with uppercase
        for _ in 0..20 {
            let name = generate_lobby_name();
            let parts: Vec<&str> = name.split('-').collect();
            assert_eq!(parts.len(), 2);
            assert!(parts[0].chars().all(|c| c.is_ascii_uppercase()));
            assert!(parts[1].chars().all(|c| c.is_ascii_uppercase()));
        }
    }

    #[test]
    fn test_player_clone() {
        let player = Player {
            name: "Test".to_string(),
            ready: true,
            is_local: false,
            is_host: false,
        };
        let cloned = player.clone();
        assert_eq!(player.name, cloned.name);
        assert_eq!(player.ready, cloned.ready);
    }
}
