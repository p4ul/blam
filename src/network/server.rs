#![allow(dead_code)]
//! TCP server for hosting games

use super::peer::Peer;
use super::protocol::Message;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

/// Default port for BLAM servers
pub const DEFAULT_PORT: u16 = 55333;

/// Maximum port to try when auto-incrementing
const MAX_PORT: u16 = 55433;

/// A BLAM game server that accepts peer connections
pub struct Server {
    /// Local address the server is bound to
    addr: SocketAddr,
    /// Channel to receive new peer connections
    new_peers_rx: Receiver<Peer>,
    /// Connected peers
    peers: Vec<Peer>,
    /// Running flag
    running: bool,
}

impl Server {
    /// Start a new server on the default port with auto-increment
    pub fn start() -> io::Result<Self> {
        Self::start_on_port(DEFAULT_PORT)
    }

    /// Start a new server on a specific port with auto-increment fallback
    pub fn start_on_port(start_port: u16) -> io::Result<Self> {
        let mut port = start_port;
        let listener = loop {
            match TcpListener::bind(format!("0.0.0.0:{}", port)) {
                Ok(l) => break l,
                Err(e) if e.kind() == io::ErrorKind::AddrInUse && port < MAX_PORT => {
                    port += 1;
                }
                Err(e) => return Err(e),
            }
        };

        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;

        let (new_peers_tx, new_peers_rx) = channel();

        // Spawn acceptor thread
        thread::spawn(move || {
            accept_loop(listener, new_peers_tx);
        });

        Ok(Server {
            addr,
            new_peers_rx,
            peers: Vec::new(),
            running: true,
        })
    }

    /// Get the address the server is listening on
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Get the port the server is listening on
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Poll for new connections and messages
    pub fn poll(&mut self) -> Vec<ServerEvent> {
        let mut events = Vec::new();

        // Accept new peers
        loop {
            match self.new_peers_rx.try_recv() {
                Ok(peer) => {
                    events.push(ServerEvent::PeerConnected { addr: peer.addr });
                    self.peers.push(peer);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.running = false;
                    break;
                }
            }
        }

        // Collect messages from peers and track disconnections
        let mut disconnected = Vec::new();
        for (i, peer) in self.peers.iter_mut().enumerate() {
            for msg in peer.recv_all() {
                // Handle Join messages to set player name
                if let Message::Join { ref player_name } = msg {
                    peer.set_player_name(player_name.clone());
                }
                events.push(ServerEvent::MessageReceived {
                    from: peer.addr,
                    player_name: peer.player_name.clone(),
                    message: msg,
                });
            }
            if !peer.is_alive() {
                disconnected.push(i);
            }
        }

        // Remove disconnected peers (in reverse order to preserve indices)
        for i in disconnected.into_iter().rev() {
            let peer = self.peers.remove(i);
            events.push(ServerEvent::PeerDisconnected {
                addr: peer.addr,
                player_name: peer.player_name,
            });
        }

        events
    }

    /// Broadcast a message to all connected peers (serializes once)
    pub fn broadcast(&self, msg: &Message) {
        let bytes = msg.to_bytes();
        for peer in &self.peers {
            let _ = peer.send_raw(bytes.clone());
        }
    }

    /// Send a message to a specific peer by address
    pub fn send_to(&self, addr: SocketAddr, msg: &Message) -> io::Result<()> {
        let bytes = msg.to_bytes();
        for peer in &self.peers {
            if peer.addr == addr {
                return peer.send_raw(bytes);
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, "peer not found"))
    }

    /// Get the number of connected peers
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Get addresses of all connected peers
    pub fn peer_addrs(&self) -> Vec<SocketAddr> {
        self.peers.iter().map(|p| p.addr).collect()
    }

    /// Check if the server is still running
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Stop the server
    pub fn stop(&mut self) {
        self.running = false;
        self.peers.clear();
    }
}

/// Events from the server
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// A new peer connected
    PeerConnected { addr: SocketAddr },
    /// A peer disconnected
    PeerDisconnected {
        addr: SocketAddr,
        player_name: Option<String>,
    },
    /// A message was received from a peer
    MessageReceived {
        from: SocketAddr,
        player_name: Option<String>,
        message: Message,
    },
}

fn accept_loop(listener: TcpListener, tx: Sender<Peer>) {
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => {
                if let Ok(peer) = Peer::new(stream) {
                    if tx.send(peer).is_err() {
                        break;
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_starts_on_default_port() {
        let server = Server::start();
        assert!(server.is_ok());
        let server = server.unwrap();
        // Port should be in the expected range
        assert!(server.port() >= DEFAULT_PORT);
        assert!(server.port() <= MAX_PORT);
    }

    #[test]
    fn test_server_auto_increment_port() {
        // Start first server
        let server1 = Server::start_on_port(55400).unwrap();
        let port1 = server1.port();

        // Start second server - should get different port
        let server2 = Server::start_on_port(port1).unwrap();
        let port2 = server2.port();

        assert_ne!(port1, port2);
        assert_eq!(port2, port1 + 1);
    }

    #[test]
    fn test_server_accepts_connection() {
        let mut server = Server::start_on_port(55410).unwrap();
        let addr = server.addr();

        // Connect a client
        let _client = Peer::connect(addr).unwrap();

        // Poll should show the new connection
        thread::sleep(Duration::from_millis(100));
        let events = server.poll();

        assert!(events.iter().any(|e| matches!(e, ServerEvent::PeerConnected { .. })));
        assert_eq!(server.peer_count(), 1);
    }
}
