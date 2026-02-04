//! Lobby management for multiplayer games
//!
//! Handles:
//! - Hosting a lobby (server + mDNS advertisement)
//! - Joining a lobby (client connection)
//! - Player list management
//! - Synchronized round start

use crate::network::{
    Client, DiscoveryEvent, Message, PeerInfo, PeerTracker, Server, ServerEvent, ServiceDiscovery,
};
use rand::prelude::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::mpsc::Receiver;

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
    /// The round is starting with these letters
    RoundStart { letters: Vec<char>, duration: u32 },
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
    /// Current lobby state
    pub state: LobbyState,
    /// Actor ID for this instance
    actor_id: String,
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
            state: LobbyState::Waiting,
            actor_id,
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
                            events.push(LobbyEvent::PlayerLeft(player.name.clone()));

                            // Update indices for remaining players
                            for (a, i) in self.addr_to_player.iter_mut() {
                                if *i > idx {
                                    *i -= 1;
                                }
                            }
                        }
                    } else if let Some(name) = player_name {
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
                            events.push(LobbyEvent::PlayerLeft(player_name));
                        }
                        _ => {}
                    }
                }
            }
        }

        events
    }

    /// Start the round - broadcast to all players
    pub fn start_round(&mut self, letters: Vec<char>, duration: u32) {
        self.state = LobbyState::Starting;

        // Broadcast round start to all connected clients
        let msg = Message::RoundStart {
            letters: letters.clone(),
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
                Message::RoundStart { letters, duration_secs } => {
                    self.state = LobbyState::Starting;
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
                _ => {}
            }
        }

        events
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
}
