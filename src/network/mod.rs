#![allow(dead_code)]
//! Networking: mDNS discovery, peer sync, lobby hosting, TCP connections
//!
//! This module provides:
//! - mDNS-SD discovery for finding BLAM! instances on local network
//! - TCP server for hosting games (default port 55333 with auto-increment)
//! - TCP client for joining games (manual connect via --connect IP:PORT)
//! - Length-prefixed JSON protocol for peer-to-peer messaging

pub mod client;
pub mod peer;
pub mod protocol;
pub mod server;

pub use client::Client;
pub use protocol::{ClaimRejectReason, Message};
pub use server::{Server, ServerEvent};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;

/// BLAM! service type for mDNS discovery
pub const SERVICE_TYPE: &str = "_blam._tcp.local.";

/// Current protocol version
pub const PROTOCOL_VERSION: &str = "1";

/// Information about a discovered peer
#[derive(Debug, Clone)]
pub struct PeerInfo {
    /// Unique identifier for this peer
    pub actor_id: String,
    /// Player's display handle/nickname
    pub handle: String,
    /// Name of the lobby they're hosting (if any)
    pub lobby_name: Option<String>,
    /// Protocol version they're running
    pub version: String,
    /// Hostname of the peer
    pub hostname: String,
    /// IP addresses of the peer
    pub addresses: Vec<std::net::IpAddr>,
    /// Port the peer is listening on
    pub port: u16,
}

/// Events from the service discovery system
#[derive(Debug)]
pub enum DiscoveryEvent {
    /// A new peer was discovered
    PeerDiscovered(PeerInfo),
    /// A peer went offline
    PeerLost(String), // actor_id
}

/// Service discovery manager for finding BLAM! instances on the local network
pub struct ServiceDiscovery {
    daemon: ServiceDaemon,
    our_actor_id: String,
    registered_instance: Option<String>,
}

impl ServiceDiscovery {
    /// Create a new service discovery instance
    ///
    /// # Arguments
    /// * `actor_id` - Unique identifier for this instance
    pub fn new(actor_id: String) -> Result<Self, String> {
        let daemon = ServiceDaemon::new().map_err(|e| format!("Failed to create mDNS daemon: {}", e))?;

        Ok(Self {
            daemon,
            our_actor_id: actor_id,
            registered_instance: None,
        })
    }

    /// Advertise this instance on the local network
    ///
    /// # Arguments
    /// * `handle` - Player's display name
    /// * `lobby_name` - Optional lobby name if hosting
    /// * `port` - Port to advertise
    pub fn advertise(
        &mut self,
        handle: &str,
        lobby_name: Option<&str>,
        port: u16,
    ) -> Result<(), String> {
        // Build TXT record properties
        let mut properties: Vec<(&str, &str)> = vec![
            ("version", PROTOCOL_VERSION),
            ("handle", handle),
            ("actor_id", &self.our_actor_id),
        ];

        // Add lobby_name to a temporary variable so it lives long enough
        let lobby_owned: String;
        if let Some(lobby) = lobby_name {
            lobby_owned = lobby.to_string();
            properties.push(("lobby_name", &lobby_owned));
        }

        // Instance name is the actor_id (must be unique on the network)
        let instance_name = &self.our_actor_id;

        // Build the hostname - use actor_id as subdomain
        let hostname = format!("{}.local.", self.our_actor_id);

        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            instance_name,
            &hostname,
            (),
            port,
            &properties[..],
        )
        .map_err(|e| format!("Failed to create service info: {}", e))?
        .enable_addr_auto();

        self.daemon
            .register(service_info)
            .map_err(|e| format!("Failed to register service: {}", e))?;

        self.registered_instance = Some(instance_name.to_string());
        Ok(())
    }

    /// Stop advertising on the network
    pub fn stop_advertising(&mut self) -> Result<(), String> {
        if let Some(instance_name) = self.registered_instance.take() {
            let fullname = format!("{}.{}", instance_name, SERVICE_TYPE);
            self.daemon
                .unregister(&fullname)
                .map_err(|e| format!("Failed to unregister service: {}", e))?;
        }
        Ok(())
    }

    /// Start browsing for other BLAM! instances
    ///
    /// Returns a receiver that will emit DiscoveryEvents as peers are found/lost
    pub fn browse(&self) -> Result<mpsc::Receiver<DiscoveryEvent>, String> {
        let receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| format!("Failed to start browsing: {}", e))?;

        let (tx, rx) = mpsc::channel();
        let our_actor_id = self.our_actor_id.clone();

        thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        // Extract properties from TXT record
                        let properties = info.get_properties();

                        let actor_id = properties
                            .get_property_val_str("actor_id")
                            .unwrap_or_default()
                            .to_string();

                        // Skip our own instance
                        if actor_id == our_actor_id {
                            continue;
                        }

                        let handle = properties
                            .get_property_val_str("handle")
                            .unwrap_or_default()
                            .to_string();

                        let lobby_name = properties
                            .get_property_val_str("lobby_name")
                            .map(|s| s.to_string());

                        let version = properties
                            .get_property_val_str("version")
                            .unwrap_or(PROTOCOL_VERSION)
                            .to_string();

                        // Collect addresses, preferring IPv4 over IPv6
                        // IPv6 link-local addresses (fe80::) require scope_id
                        // for TCP connections, which IpAddr doesn't carry
                        let mut addresses: Vec<std::net::IpAddr> = info
                            .get_addresses()
                            .iter()
                            .map(|s| s.to_ip_addr())
                            .collect();
                        addresses.sort_by_key(|addr| match addr {
                            std::net::IpAddr::V4(_) => 0,
                            std::net::IpAddr::V6(_) => 1,
                        });

                        let peer_info = PeerInfo {
                            actor_id,
                            handle,
                            lobby_name,
                            version,
                            hostname: info.get_hostname().to_string(),
                            addresses,
                            port: info.get_port(),
                        };

                        let _ = tx.send(DiscoveryEvent::PeerDiscovered(peer_info));
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        // Extract actor_id from fullname (format: "actor_id._blam._tcp.local.")
                        if let Some(actor_id) = fullname.strip_suffix(&format!(".{}", SERVICE_TYPE)) {
                            let _ = tx.send(DiscoveryEvent::PeerLost(actor_id.to_string()));
                        }
                    }
                    _ => {}
                }
            }
        });

        Ok(rx)
    }

    /// Stop browsing for peers
    pub fn stop_browsing(&self) -> Result<(), String> {
        self.daemon
            .stop_browse(SERVICE_TYPE)
            .map_err(|e| format!("Failed to stop browsing: {}", e))
    }

    /// Shutdown the discovery service
    pub fn shutdown(self) -> Result<(), String> {
        self.daemon
            .shutdown()
            .map_err(|e| format!("Failed to shutdown daemon: {}", e))?;
        Ok(())
    }
}

/// Tracks discovered peers and their state
pub struct PeerTracker {
    peers: HashMap<String, PeerInfo>,
}

impl PeerTracker {
    /// Create a new peer tracker
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    /// Add or update a peer
    pub fn update(&mut self, peer: PeerInfo) {
        self.peers.insert(peer.actor_id.clone(), peer);
    }

    /// Remove a peer by actor_id
    pub fn remove(&mut self, actor_id: &str) -> Option<PeerInfo> {
        self.peers.remove(actor_id)
    }

    /// Get all known peers
    pub fn peers(&self) -> impl Iterator<Item = &PeerInfo> {
        self.peers.values()
    }

    /// Get a specific peer by actor_id
    pub fn get(&self, actor_id: &str) -> Option<&PeerInfo> {
        self.peers.get(actor_id)
    }

    /// Get number of tracked peers
    pub fn count(&self) -> usize {
        self.peers.len()
    }
}

impl Default for PeerTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_type_format() {
        assert!(SERVICE_TYPE.starts_with("_"));
        assert!(SERVICE_TYPE.ends_with(".local."));
    }

    #[test]
    fn test_peer_info_clone() {
        let peer = PeerInfo {
            actor_id: "test-123".to_string(),
            handle: "TestPlayer".to_string(),
            lobby_name: Some("Test Lobby".to_string()),
            version: "1".to_string(),
            hostname: "test.local.".to_string(),
            addresses: vec![],
            port: 55333,
        };

        let cloned = peer.clone();
        assert_eq!(cloned.actor_id, peer.actor_id);
        assert_eq!(cloned.handle, peer.handle);
    }

    #[test]
    fn test_peer_tracker_add_remove() {
        let mut tracker = PeerTracker::new();

        let peer = PeerInfo {
            actor_id: "peer-1".to_string(),
            handle: "Player1".to_string(),
            lobby_name: None,
            version: "1".to_string(),
            hostname: "peer1.local.".to_string(),
            addresses: vec![],
            port: 55333,
        };

        tracker.update(peer);
        assert_eq!(tracker.count(), 1);
        assert!(tracker.get("peer-1").is_some());

        tracker.remove("peer-1");
        assert_eq!(tracker.count(), 0);
        assert!(tracker.get("peer-1").is_none());
    }

    #[test]
    fn test_peer_tracker_update() {
        let mut tracker = PeerTracker::new();

        let peer1 = PeerInfo {
            actor_id: "peer-1".to_string(),
            handle: "OldName".to_string(),
            lobby_name: None,
            version: "1".to_string(),
            hostname: "peer1.local.".to_string(),
            addresses: vec![],
            port: 55333,
        };

        tracker.update(peer1);
        assert_eq!(tracker.get("peer-1").unwrap().handle, "OldName");

        let peer1_updated = PeerInfo {
            actor_id: "peer-1".to_string(),
            handle: "NewName".to_string(),
            lobby_name: Some("My Lobby".to_string()),
            version: "1".to_string(),
            hostname: "peer1.local.".to_string(),
            addresses: vec![],
            port: 55333,
        };

        tracker.update(peer1_updated);
        assert_eq!(tracker.count(), 1); // Still one peer
        assert_eq!(tracker.get("peer-1").unwrap().handle, "NewName");
        assert!(tracker.get("peer-1").unwrap().lobby_name.is_some());
    }

    #[test]
    fn test_peer_tracker_multiple_peers() {
        let mut tracker = PeerTracker::new();

        for i in 0..5 {
            let peer = PeerInfo {
                actor_id: format!("peer-{}", i),
                handle: format!("Player{}", i),
                lobby_name: None,
                version: "1".to_string(),
                hostname: format!("peer{}.local.", i),
                addresses: vec![],
                port: 55333 + i as u16,
            };
            tracker.update(peer);
        }

        assert_eq!(tracker.count(), 5);

        // Remove one
        let removed = tracker.remove("peer-2");
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().handle, "Player2");
        assert_eq!(tracker.count(), 4);

        // Remove nonexistent
        let removed = tracker.remove("peer-99");
        assert!(removed.is_none());
        assert_eq!(tracker.count(), 4);
    }

    #[test]
    fn test_peer_tracker_default() {
        let tracker = PeerTracker::default();
        assert_eq!(tracker.count(), 0);
    }

    #[test]
    fn test_peer_tracker_get_nonexistent() {
        let tracker = PeerTracker::new();
        assert!(tracker.get("nonexistent").is_none());
    }

    #[test]
    fn test_peer_tracker_peers_iterator() {
        let mut tracker = PeerTracker::new();

        let peer = PeerInfo {
            actor_id: "peer-1".to_string(),
            handle: "Player1".to_string(),
            lobby_name: None,
            version: "1".to_string(),
            hostname: "peer1.local.".to_string(),
            addresses: vec![],
            port: 55333,
        };
        tracker.update(peer);

        let peers: Vec<&PeerInfo> = tracker.peers().collect();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].handle, "Player1");
    }

    #[test]
    fn test_peer_info_with_addresses() {
        use std::net::IpAddr;

        let peer = PeerInfo {
            actor_id: "peer-1".to_string(),
            handle: "Player1".to_string(),
            lobby_name: Some("TestLobby".to_string()),
            version: "1".to_string(),
            hostname: "peer1.local.".to_string(),
            addresses: vec![
                "127.0.0.1".parse::<IpAddr>().unwrap(),
                "192.168.1.1".parse::<IpAddr>().unwrap(),
            ],
            port: 55333,
        };

        assert_eq!(peer.addresses.len(), 2);
        assert_eq!(peer.port, 55333);
        assert_eq!(peer.lobby_name.as_deref(), Some("TestLobby"));
    }

    #[test]
    fn test_protocol_version_is_set() {
        assert!(!PROTOCOL_VERSION.is_empty());
        assert_eq!(PROTOCOL_VERSION, "1");
    }

    #[test]
    fn test_mdns_advertise_and_browse_same_machine() {
        use rand::Rng;

        // Host advertises a service
        let host_actor_id = format!("blam-test-{:08x}", rand::rng().random::<u32>());
        let mut host_discovery = ServiceDiscovery::new(host_actor_id.clone()).unwrap();
        host_discovery
            .advertise("TestHost", Some("TEST-LOBBY"), 55999)
            .unwrap();

        // Browser discovers services
        let browser_actor_id = format!("blam-test-{:08x}", rand::rng().random::<u32>());
        let browser_discovery = ServiceDiscovery::new(browser_actor_id).unwrap();
        let rx = browser_discovery.browse().unwrap();

        // Wait for discovery
        let mut found = false;
        let mut events_seen = Vec::new();
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10);

        while start.elapsed() < timeout {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(DiscoveryEvent::PeerDiscovered(peer)) => {
                    events_seen.push(format!(
                        "PeerDiscovered: actor_id={}, handle={}, addresses={:?}, port={}",
                        peer.actor_id, peer.handle, peer.addresses, peer.port
                    ));
                    if peer.actor_id == host_actor_id {
                        assert_eq!(peer.handle, "TestHost");
                        assert_eq!(peer.lobby_name.as_deref(), Some("TEST-LOBBY"));
                        assert_eq!(peer.port, 55999);
                        assert!(
                            !peer.addresses.is_empty(),
                            "Discovered peer should have at least one address, got: {:?}",
                            peer.addresses
                        );
                        found = true;
                        break;
                    }
                }
                Ok(DiscoveryEvent::PeerLost(id)) => {
                    events_seen.push(format!("PeerLost: {}", id));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    events_seen.push("Channel disconnected".to_string());
                    break;
                }
            }
        }

        assert!(
            found,
            "Browser should discover the host via mDNS within 10s. Events seen: {:?}",
            events_seen
        );

        // Cleanup
        host_discovery.stop_advertising().unwrap();
        browser_discovery.stop_browsing().unwrap();
        host_discovery.shutdown().unwrap();
        browser_discovery.shutdown().unwrap();
    }

    #[test]
    fn test_mdns_discovered_peer_has_connectable_address() {
        use rand::Rng;
        use std::net::TcpListener;

        // Start a real TCP listener
        let listener = TcpListener::bind("0.0.0.0:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Host advertises with the real port
        let host_actor_id = format!("blam-test-{:08x}", rand::rng().random::<u32>());
        let mut host_discovery = ServiceDiscovery::new(host_actor_id.clone()).unwrap();
        host_discovery
            .advertise("TestHost", Some("CONNECT-LOBBY"), port)
            .unwrap();

        // Browser discovers services
        let browser_actor_id = format!("blam-test-{:08x}", rand::rng().random::<u32>());
        let browser_discovery = ServiceDiscovery::new(browser_actor_id).unwrap();
        let rx = browser_discovery.browse().unwrap();

        // Wait for discovery
        let mut found_peer: Option<PeerInfo> = None;
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10);

        while start.elapsed() < timeout {
            match rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(DiscoveryEvent::PeerDiscovered(peer)) => {
                    if peer.actor_id == host_actor_id {
                        found_peer = Some(peer);
                        break;
                    }
                }
                _ => continue,
            }
        }

        let peer = found_peer.expect("Should discover host via mDNS");
        assert!(!peer.addresses.is_empty(), "Peer must have at least one address");

        // IPv4 addresses should be sorted first
        let first_addr = peer.addresses.first().unwrap();
        assert!(
            first_addr.is_ipv4(),
            "First address should be IPv4, got: {:?}. All addresses: {:?}",
            first_addr,
            peer.addresses
        );

        // Try connecting to addresses in order (like JoinedLobby::join does)
        let mut connected = false;
        for addr in &peer.addresses {
            let socket_addr = std::net::SocketAddr::new(*addr, peer.port);
            if std::net::TcpStream::connect_timeout(
                &socket_addr,
                std::time::Duration::from_secs(2),
            )
            .is_ok()
            {
                connected = true;
                break;
            }
        }
        assert!(
            connected,
            "Should be able to connect to at least one discovered address. Addresses: {:?}",
            peer.addresses
        );

        // Cleanup
        drop(listener);
        host_discovery.stop_advertising().unwrap();
        browser_discovery.stop_browsing().unwrap();
        host_discovery.shutdown().unwrap();
        browser_discovery.shutdown().unwrap();
    }
}
