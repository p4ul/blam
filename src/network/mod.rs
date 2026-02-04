//! Networking: mDNS discovery, peer sync, lobby hosting
//!
//! Uses mDNS-SD (multicast DNS Service Discovery) to find other BLAM! instances
//! on the local network. Each instance advertises itself with TXT records
//! containing game metadata.

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
            (), // Empty IP - will be auto-detected
            port,
            &properties[..],
        )
        .map_err(|e| format!("Failed to create service info: {}", e))?;

        self.daemon
            .register(service_info)
            .map_err(|e| format!("Failed to register service: {}", e))?;

        self.registered_instance = Some(instance_name.to_string());
        Ok(())
    }

    /// Stop advertising this instance
    pub fn stop_advertising(&mut self) -> Result<(), String> {
        if let Some(instance) = self.registered_instance.take() {
            let fullname = format!("{}.{}", instance, SERVICE_TYPE);
            self.daemon
                .unregister(&fullname)
                .map_err(|e| format!("Failed to unregister service: {}", e))?;
        }
        Ok(())
    }

    /// Start browsing for other BLAM! instances
    ///
    /// Returns a receiver that will emit discovery events.
    /// Call this once and keep the receiver to get updates.
    pub fn browse(&self) -> Result<mpsc::Receiver<DiscoveryEvent>, String> {
        let mdns_receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| format!("Failed to start browsing: {}", e))?;

        let our_actor_id = self.our_actor_id.clone();
        let (tx, rx) = mpsc::channel();

        // Spawn a thread to process mDNS events and convert them to DiscoveryEvents
        thread::spawn(move || {
            while let Ok(event) = mdns_receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        // Extract TXT properties
                        let properties = info.get_properties();

                        let actor_id = properties
                            .get("actor_id")
                            .map(|p| p.val_str().to_string())
                            .unwrap_or_default();

                        // Skip our own service
                        if actor_id == our_actor_id {
                            continue;
                        }

                        let handle = properties
                            .get("handle")
                            .map(|p| p.val_str().to_string())
                            .unwrap_or_else(|| "Unknown".to_string());

                        let lobby_name = properties.get("lobby_name").map(|p| p.val_str().to_string());

                        let version = properties
                            .get("version")
                            .map(|p| p.val_str().to_string())
                            .unwrap_or_else(|| "0".to_string());

                        let peer = PeerInfo {
                            actor_id,
                            handle,
                            lobby_name,
                            version,
                            hostname: info.get_hostname().to_string(),
                            addresses: info
                                .get_addresses()
                                .iter()
                                .filter_map(|scoped| match scoped {
                                    mdns_sd::ScopedIp::V4(v4) => Some(std::net::IpAddr::V4(*v4.addr())),
                                    mdns_sd::ScopedIp::V6(v6) => Some(std::net::IpAddr::V6(*v6.addr())),
                                    _ => None, // Handle potential future variants
                                })
                                .collect(),
                            port: info.get_port(),
                        };

                        let _ = tx.send(DiscoveryEvent::PeerDiscovered(peer));
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        // Extract actor_id from fullname (format: "actor_id._blam._tcp.local.")
                        if let Some(actor_id) = fullname.strip_suffix(&format!(".{}", SERVICE_TYPE)) {
                            let _ = tx.send(DiscoveryEvent::PeerLost(actor_id.to_string()));
                        }
                    }
                    _ => {
                        // Ignore other events (SearchStarted, SearchStopped)
                    }
                }
            }
        });

        Ok(rx)
    }

    /// Stop browsing for services
    pub fn stop_browsing(&self) -> Result<(), String> {
        self.daemon
            .stop_browse(SERVICE_TYPE)
            .map_err(|e| format!("Failed to stop browsing: {}", e))
    }

    /// Shutdown the service discovery daemon
    pub fn shutdown(self) -> Result<(), String> {
        let _ = self
            .daemon
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

    /// Process a discovery event and update peer state
    pub fn process_event(&mut self, event: DiscoveryEvent) {
        match event {
            DiscoveryEvent::PeerDiscovered(peer) => {
                self.peers.insert(peer.actor_id.clone(), peer);
            }
            DiscoveryEvent::PeerLost(actor_id) => {
                self.peers.remove(&actor_id);
            }
        }
    }

    /// Get all currently known peers
    pub fn peers(&self) -> impl Iterator<Item = &PeerInfo> {
        self.peers.values()
    }

    /// Get a specific peer by actor_id
    pub fn get_peer(&self, actor_id: &str) -> Option<&PeerInfo> {
        self.peers.get(actor_id)
    }

    /// Get the number of known peers
    pub fn peer_count(&self) -> usize {
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
        // Service type must end with .local.
        assert!(SERVICE_TYPE.ends_with(".local."));
        // Must be a valid service type format
        assert!(SERVICE_TYPE.starts_with('_'));
        assert!(SERVICE_TYPE.contains("._tcp") || SERVICE_TYPE.contains("._udp"));
    }

    #[test]
    fn test_peer_tracker_add_remove() {
        let mut tracker = PeerTracker::new();
        assert_eq!(tracker.peer_count(), 0);

        let peer = PeerInfo {
            actor_id: "test-123".to_string(),
            handle: "TestPlayer".to_string(),
            lobby_name: Some("Test Lobby".to_string()),
            version: "1".to_string(),
            hostname: "test.local.".to_string(),
            addresses: vec![],
            port: 5555,
        };

        tracker.process_event(DiscoveryEvent::PeerDiscovered(peer.clone()));
        assert_eq!(tracker.peer_count(), 1);
        assert!(tracker.get_peer("test-123").is_some());

        tracker.process_event(DiscoveryEvent::PeerLost("test-123".to_string()));
        assert_eq!(tracker.peer_count(), 0);
        assert!(tracker.get_peer("test-123").is_none());
    }

    #[test]
    fn test_peer_tracker_update() {
        let mut tracker = PeerTracker::new();

        let peer1 = PeerInfo {
            actor_id: "test-123".to_string(),
            handle: "OldHandle".to_string(),
            lobby_name: None,
            version: "1".to_string(),
            hostname: "test.local.".to_string(),
            addresses: vec![],
            port: 5555,
        };

        let peer2 = PeerInfo {
            actor_id: "test-123".to_string(),
            handle: "NewHandle".to_string(),
            lobby_name: Some("My Lobby".to_string()),
            version: "1".to_string(),
            hostname: "test.local.".to_string(),
            addresses: vec![],
            port: 5555,
        };

        tracker.process_event(DiscoveryEvent::PeerDiscovered(peer1));
        tracker.process_event(DiscoveryEvent::PeerDiscovered(peer2));

        // Should update to new info
        assert_eq!(tracker.peer_count(), 1);
        let peer = tracker.get_peer("test-123").unwrap();
        assert_eq!(peer.handle, "NewHandle");
        assert_eq!(peer.lobby_name.as_deref(), Some("My Lobby"));
    }

    #[test]
    fn test_peer_info_clone() {
        let peer = PeerInfo {
            actor_id: "abc".to_string(),
            handle: "Player1".to_string(),
            lobby_name: None,
            version: "1".to_string(),
            hostname: "test.local.".to_string(),
            addresses: vec![],
            port: 1234,
        };

        let cloned = peer.clone();
        assert_eq!(peer.actor_id, cloned.actor_id);
        assert_eq!(peer.handle, cloned.handle);
    }
}
