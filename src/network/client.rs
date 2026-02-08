#![allow(dead_code)]
//! TCP client for joining games

use super::peer::Peer;
use super::protocol::Message;
use super::server::DEFAULT_PORT;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs};

/// A BLAM game client that connects to a host
pub struct Client {
    /// Connection to the host
    peer: Peer,
    /// Our player name
    player_name: String,
    /// Whether we've joined the game
    joined: bool,
}

impl Client {
    /// Connect to a host at the given address
    ///
    /// The address can be:
    /// - "IP:PORT" (e.g., "192.168.1.100:55333")
    /// - "IP" (uses default port 55333)
    /// - "hostname:PORT"
    /// - "hostname" (uses default port)
    pub fn connect(addr: &str, player_name: String) -> io::Result<Self> {
        let socket_addr = parse_address(addr)?;
        let peer = Peer::connect(socket_addr)?;

        Ok(Client {
            peer,
            player_name,
            joined: false,
        })
    }

    /// Connect to a host at the given socket address
    pub fn connect_addr(addr: SocketAddr, player_name: String) -> io::Result<Self> {
        let peer = Peer::connect(addr)?;

        Ok(Client {
            peer,
            player_name,
            joined: false,
        })
    }

    /// Send a join message to the host
    pub fn join(&mut self) -> io::Result<()> {
        if self.joined {
            return Ok(());
        }
        self.peer.send(Message::Join {
            player_name: self.player_name.clone(),
        })?;
        self.joined = true;
        Ok(())
    }

    /// Send a claim message to the host (legacy, for compatibility)
    pub fn claim(&self, word: &str, points: u32) -> io::Result<()> {
        self.peer.send(Message::Claim {
            player_name: self.player_name.clone(),
            word: word.to_string(),
            points,
        })
    }

    /// Send a claim attempt to the host (new arbitration protocol)
    pub fn send_claim_attempt(&self, word: &str) -> io::Result<()> {
        self.peer.send(Message::ClaimAttempt {
            word: word.to_string(),
        })
    }

    /// Send a leave message and disconnect
    pub fn leave(&self) -> io::Result<()> {
        self.peer.send(Message::Leave {
            player_name: self.player_name.clone(),
        })
    }

    /// Poll for incoming messages from the host
    pub fn poll(&mut self) -> Vec<Message> {
        self.peer.recv_all()
    }

    /// Check if still connected
    pub fn is_connected(&self) -> bool {
        self.peer.is_alive()
    }

    /// Get the host's address
    pub fn host_addr(&self) -> SocketAddr {
        self.peer.addr
    }

    /// Get our player name
    pub fn player_name(&self) -> &str {
        &self.player_name
    }
}

/// Parse an address string into a SocketAddr
///
/// Handles formats:
/// - "192.168.1.100:55333" -> parse directly
/// - "192.168.1.100" -> add default port
/// - "hostname:55333" -> resolve and use port
/// - "hostname" -> resolve and use default port
pub fn parse_address(addr: &str) -> io::Result<SocketAddr> {
    // Check if it already has a port
    if addr.contains(':') {
        // Try to parse directly or resolve
        addr.to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not resolve address"))
    } else {
        // Add default port
        let with_port = format!("{}:{}", addr, DEFAULT_PORT);
        with_port
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not resolve address"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::server::Server;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_parse_address_with_port() {
        let addr = parse_address("127.0.0.1:55333").unwrap();
        assert_eq!(addr.port(), 55333);
    }

    #[test]
    fn test_parse_address_without_port() {
        let addr = parse_address("127.0.0.1").unwrap();
        assert_eq!(addr.port(), DEFAULT_PORT);
    }

    #[test]
    fn test_client_connects_to_server() {
        let mut server = Server::start_on_port(55420).unwrap();
        let addr = format!("127.0.0.1:{}", server.port());

        let mut client = Client::connect(&addr, "TestPlayer".to_string()).unwrap();
        client.join().unwrap();

        // Wait for server to receive the connection and join
        thread::sleep(Duration::from_millis(200));
        let events = server.poll();

        // Should have connection and join events
        assert!(events.iter().any(|e| matches!(e, crate::network::server::ServerEvent::PeerConnected { .. })));
        assert!(events.iter().any(|e| matches!(
            e,
            crate::network::server::ServerEvent::MessageReceived {
                message: Message::Join { player_name },
                ..
            } if player_name == "TestPlayer"
        )));
    }

    #[test]
    fn test_client_receives_broadcast() {
        let mut server = Server::start_on_port(55421).unwrap();
        let addr = format!("127.0.0.1:{}", server.port());

        let mut client = Client::connect(&addr, "TestPlayer".to_string()).unwrap();
        client.join().unwrap();

        // Wait for connection
        thread::sleep(Duration::from_millis(100));
        server.poll();

        // Server broadcasts round start
        let letters = vec!['B', 'L', 'A', 'M'];
        server.broadcast(&Message::RoundStart {
            letters: letters.clone(),
            duration_secs: 60,
        });

        // Wait for message to arrive
        thread::sleep(Duration::from_millis(100));
        let messages = client.poll();

        assert!(messages.iter().any(|m| matches!(
            m,
            Message::RoundStart { letters: l, duration_secs: 60 } if *l == letters
        )));
    }
}
