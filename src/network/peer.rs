//! Peer connection handling

use super::protocol::Message;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Duration;

/// A connected peer
pub struct Peer {
    /// Peer's address
    pub addr: SocketAddr,
    /// Peer's player name (once they've joined)
    pub player_name: Option<String>,
    /// Channel to send messages to this peer
    tx: Sender<Message>,
    /// Channel to receive messages from this peer
    rx: Receiver<Message>,
    /// Whether the connection is still alive
    alive: bool,
}

impl Peer {
    /// Create a new peer from a TCP stream
    pub fn new(stream: TcpStream) -> io::Result<Self> {
        let addr = stream.peer_addr()?;

        // Set non-blocking for the reader thread
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        let (outgoing_tx, outgoing_rx) = channel::<Message>();
        let (incoming_tx, incoming_rx) = channel::<Message>();

        // Clone stream for writer thread
        let read_stream = stream.try_clone()?;
        let mut write_stream = stream;

        // Writer thread
        thread::spawn(move || {
            while let Ok(msg) = outgoing_rx.recv() {
                if msg.write_to(&mut write_stream).is_err() {
                    break;
                }
            }
        });

        // Reader thread
        thread::spawn(move || {
            let mut read_stream = read_stream;
            loop {
                match Message::read_from(&mut read_stream) {
                    Ok(msg) => {
                        if incoming_tx.send(msg).is_err() {
                            break;
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                        // Timeout, continue trying
                        continue;
                    }
                    Err(_) => {
                        // Connection closed or error
                        break;
                    }
                }
            }
        });

        Ok(Peer {
            addr,
            player_name: None,
            tx: outgoing_tx,
            rx: incoming_rx,
            alive: true,
        })
    }

    /// Connect to a peer at the given address
    pub fn connect(addr: SocketAddr) -> io::Result<Self> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
        Self::new(stream)
    }

    /// Send a message to this peer
    pub fn send(&self, msg: Message) -> io::Result<()> {
        self.tx
            .send(msg)
            .map_err(|_| io::Error::new(ErrorKind::BrokenPipe, "peer disconnected"))
    }

    /// Try to receive a message from this peer (non-blocking)
    pub fn try_recv(&mut self) -> Option<Message> {
        match self.rx.try_recv() {
            Ok(msg) => Some(msg),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.alive = false;
                None
            }
        }
    }

    /// Receive all pending messages from this peer
    pub fn recv_all(&mut self) -> Vec<Message> {
        let mut messages = Vec::new();
        while let Some(msg) = self.try_recv() {
            messages.push(msg);
        }
        messages
    }

    /// Check if the peer connection is still alive
    pub fn is_alive(&self) -> bool {
        self.alive
    }

    /// Set the player name for this peer
    pub fn set_player_name(&mut self, name: String) {
        self.player_name = Some(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn test_peer_connect_and_send() {
        // Start a listener
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        // Connect in a separate thread
        let handle = thread::spawn(move || {
            let mut peer = Peer::connect(addr).unwrap();
            peer.send(Message::Ping).unwrap();
            thread::sleep(Duration::from_millis(100));
            peer
        });

        // Accept connection
        let (stream, _) = listener.accept().unwrap();
        let mut server_peer = Peer::new(stream).unwrap();

        // Wait for the client to send
        thread::sleep(Duration::from_millis(200));

        // Check we received the ping
        let messages = server_peer.recv_all();
        assert!(messages.contains(&Message::Ping));

        handle.join().unwrap();
    }
}
