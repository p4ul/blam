#![allow(dead_code)]
//! CRDT sync logic for exchanging events between peers.
//!
//! Implements a simple grow-only set merge:
//! 1. Exchange vector clocks (highest seq per actor)
//! 2. Compute missing events based on clock differences
//! 3. Transfer missing events
//! 4. Idempotent merge (INSERT OR IGNORE)

use crate::network::protocol::{Message, SyncEvent};
use crate::storage::{ActorId, Event, Storage, StorageError};

/// Convert storage events to protocol sync events.
pub fn events_to_sync(events: Vec<Event>) -> Vec<SyncEvent> {
    events
        .into_iter()
        .map(|e| SyncEvent {
            actor_id: e.actor_id.to_hex(),
            seq: e.seq,
            event_type: e.event_type,
            payload: e.payload,
            created_at: e.created_at,
        })
        .collect()
}

/// Convert protocol sync events to storage events.
pub fn sync_to_events(events: Vec<SyncEvent>) -> Vec<Event> {
    events
        .into_iter()
        .filter_map(|e| {
            let actor_bytes = hex_to_bytes(&e.actor_id)?;
            let actor_id = ActorId::from_bytes(&actor_bytes)?;
            Some(Event {
                actor_id,
                seq: e.seq,
                event_type: e.event_type,
                payload: e.payload,
                created_at: e.created_at,
            })
        })
        .collect()
}

/// Convert a hex string to bytes.
fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() != 32 {
        return None;
    }
    let mut bytes = Vec::with_capacity(16);
    for i in 0..16 {
        let byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
        bytes.push(byte);
    }
    Some(bytes)
}

/// Create a sync request message from storage.
pub fn create_sync_request(storage: &Storage) -> Result<Message, StorageError> {
    let vclock = storage.get_vector_clock()?;
    let clock_vec: Vec<(String, i64)> = vclock
        .into_iter()
        .map(|(actor_id, seq)| (actor_id.to_hex(), seq))
        .collect();
    Ok(Message::SyncRequest {
        vector_clock: clock_vec,
    })
}

/// Process a received sync request and return events the peer is missing.
///
/// For each actor in our local storage, if the peer's clock shows a lower seq
/// (or doesn't know about the actor at all), we include those missing events.
pub fn process_sync_request(
    storage: &Storage,
    peer_clock: &[(String, i64)],
) -> Result<Message, StorageError> {
    // Convert peer clock to a map for easy lookup
    let peer_map: std::collections::HashMap<String, i64> = peer_clock
        .iter()
        .cloned()
        .collect();

    // Get our local clock
    let our_clock = storage.get_vector_clock()?;

    // Collect events the peer is missing
    let mut missing_events = Vec::new();

    for (actor_id, our_seq) in our_clock {
        let actor_hex = actor_id.to_hex();
        let peer_seq = peer_map.get(&actor_hex).copied().unwrap_or(0);

        if our_seq > peer_seq {
            // Peer is missing events from this actor
            let events = storage.get_events_after(&actor_id, peer_seq)?;
            missing_events.extend(events);
        }
    }

    Ok(Message::SyncEvents {
        events: events_to_sync(missing_events),
    })
}

/// Process received sync events by inserting them into storage.
///
/// Returns the number of new events inserted (duplicates are ignored).
pub fn process_sync_events(
    storage: &Storage,
    sync_events: Vec<SyncEvent>,
) -> Result<usize, StorageError> {
    let events = sync_to_events(sync_events);
    let mut inserted = 0;

    for event in events {
        if storage.insert_remote_event(&event)? {
            inserted += 1;
        }
    }

    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_roundtrip() {
        let actor_id = ActorId::generate();
        let hex = actor_id.to_hex();
        let bytes = hex_to_bytes(&hex).unwrap();
        let recovered = ActorId::from_bytes(&bytes).unwrap();
        assert_eq!(actor_id, recovered);
    }

    #[test]
    fn test_event_conversion_roundtrip() {
        let actor_id = ActorId::generate();
        let event = Event {
            actor_id: actor_id.clone(),
            seq: 42,
            event_type: "test".to_string(),
            payload: r#"{"data": 123}"#.to_string(),
            created_at: 1700000000000,
        };

        let sync_events = events_to_sync(vec![event.clone()]);
        assert_eq!(sync_events.len(), 1);
        assert_eq!(sync_events[0].actor_id, actor_id.to_hex());
        assert_eq!(sync_events[0].seq, 42);

        let events = sync_to_events(sync_events);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);
    }

    #[test]
    fn test_sync_request_creation() {
        let storage = Storage::open_in_memory().unwrap();
        storage.append_event("test", "{}").unwrap();
        storage.append_event("test", "{}").unwrap();

        let msg = create_sync_request(&storage).unwrap();
        if let Message::SyncRequest { vector_clock } = msg {
            assert_eq!(vector_clock.len(), 1);
            assert_eq!(vector_clock[0].0, storage.actor_id().to_hex());
            assert_eq!(vector_clock[0].1, 2);
        } else {
            panic!("Expected SyncRequest message");
        }
    }

    #[test]
    fn test_sync_between_peers() {
        // Simulate two peers with different events
        let storage_a = Storage::open_in_memory().unwrap();
        let storage_b = Storage::open_in_memory().unwrap();

        // Peer A creates some events
        storage_a.append_event("event1", "{}").unwrap();
        storage_a.append_event("event2", "{}").unwrap();
        storage_a.append_event("event3", "{}").unwrap();

        // Peer B creates some events
        storage_b.append_event("eventX", "{}").unwrap();
        storage_b.append_event("eventY", "{}").unwrap();

        // Peer B sends sync request (with empty knowledge of A)
        let b_request = create_sync_request(&storage_b).unwrap();
        let b_clock = match &b_request {
            Message::SyncRequest { vector_clock } => vector_clock.clone(),
            _ => panic!("Expected SyncRequest"),
        };

        // Peer A processes request and returns missing events
        let a_response = process_sync_request(&storage_a, &b_clock).unwrap();
        let a_events = match a_response {
            Message::SyncEvents { events } => events,
            _ => panic!("Expected SyncEvents"),
        };

        // All 3 events from A should be sent (B didn't know about A)
        assert_eq!(a_events.len(), 3);

        // Peer B processes the events
        let inserted = process_sync_events(&storage_b, a_events).unwrap();
        assert_eq!(inserted, 3);

        // Now B has 5 events total (2 own + 3 from A)
        assert_eq!(storage_b.event_count().unwrap(), 5);

        // Reverse: A gets B's events
        let a_request = create_sync_request(&storage_a).unwrap();
        let a_clock = match &a_request {
            Message::SyncRequest { vector_clock } => vector_clock.clone(),
            _ => panic!("Expected SyncRequest"),
        };

        let b_response = process_sync_request(&storage_b, &a_clock).unwrap();
        let b_events = match b_response {
            Message::SyncEvents { events } => events,
            _ => panic!("Expected SyncEvents"),
        };

        // B has 2 own events that A doesn't know about
        assert_eq!(b_events.len(), 2);

        let inserted = process_sync_events(&storage_a, b_events).unwrap();
        assert_eq!(inserted, 2);

        // Both have 5 events now
        assert_eq!(storage_a.event_count().unwrap(), 5);
        assert_eq!(storage_b.event_count().unwrap(), 5);
    }

    #[test]
    fn test_idempotent_sync() {
        let storage_a = Storage::open_in_memory().unwrap();
        let storage_b = Storage::open_in_memory().unwrap();

        // A creates events
        storage_a.append_event("test", "{}").unwrap();

        // B gets A's events
        let request = create_sync_request(&storage_b).unwrap();
        let clock = match &request {
            Message::SyncRequest { vector_clock } => vector_clock.clone(),
            _ => panic!(),
        };
        let response = process_sync_request(&storage_a, &clock).unwrap();
        let events = match response {
            Message::SyncEvents { events } => events,
            _ => panic!(),
        };

        // First sync
        let inserted1 = process_sync_events(&storage_b, events.clone()).unwrap();
        assert_eq!(inserted1, 1);

        // Second sync (duplicate) - should insert 0
        let inserted2 = process_sync_events(&storage_b, events).unwrap();
        assert_eq!(inserted2, 0);

        // Still only 1 event in B
        assert_eq!(storage_b.event_count().unwrap(), 1);
    }

    #[test]
    fn test_partial_sync() {
        let storage_a = Storage::open_in_memory().unwrap();
        let storage_b = Storage::open_in_memory().unwrap();

        // A creates 5 events
        for i in 1..=5 {
            storage_a
                .append_event("test", &format!(r#"{{"n":{}}}"#, i))
                .unwrap();
        }

        // B syncs and gets all 5
        let request = create_sync_request(&storage_b).unwrap();
        let clock = match &request {
            Message::SyncRequest { vector_clock } => vector_clock.clone(),
            _ => panic!(),
        };
        let response = process_sync_request(&storage_a, &clock).unwrap();
        let events = match response {
            Message::SyncEvents { events } => events,
            _ => panic!(),
        };
        assert_eq!(events.len(), 5);
        process_sync_events(&storage_b, events).unwrap();

        // A creates 2 more events
        storage_a.append_event("test", r#"{"n":6}"#).unwrap();
        storage_a.append_event("test", r#"{"n":7}"#).unwrap();

        // B syncs again - should only get 2 new events
        let request = create_sync_request(&storage_b).unwrap();
        let clock = match &request {
            Message::SyncRequest { vector_clock } => vector_clock.clone(),
            _ => panic!(),
        };
        let response = process_sync_request(&storage_a, &clock).unwrap();
        let events = match response {
            Message::SyncEvents { events } => events,
            _ => panic!(),
        };
        assert_eq!(events.len(), 2);

        let inserted = process_sync_events(&storage_b, events).unwrap();
        assert_eq!(inserted, 2);
        assert_eq!(storage_b.event_count().unwrap(), 7);
    }

    #[test]
    fn test_hex_to_bytes_invalid() {
        // Wrong length
        assert!(hex_to_bytes("0123").is_none());
        assert!(hex_to_bytes("").is_none());

        // Invalid hex chars
        assert!(hex_to_bytes("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_none());
    }

    #[test]
    fn test_sync_to_events_filters_invalid() {
        let sync_events = vec![
            SyncEvent {
                actor_id: "bad_hex".to_string(), // Invalid - not 32 chars
                seq: 1,
                event_type: "test".to_string(),
                payload: "{}".to_string(),
                created_at: 1000,
            },
            SyncEvent {
                actor_id: "0123456789abcdef0123456789abcdef".to_string(), // Valid
                seq: 2,
                event_type: "test".to_string(),
                payload: "{}".to_string(),
                created_at: 2000,
            },
        ];

        let events = sync_to_events(sync_events);
        assert_eq!(events.len(), 1); // Only valid one survives
        assert_eq!(events[0].seq, 2);
    }

    #[test]
    fn test_sync_empty_storages() {
        let storage_a = Storage::open_in_memory().unwrap();
        let storage_b = Storage::open_in_memory().unwrap();

        // Both empty - sync should produce empty results
        let request = create_sync_request(&storage_a).unwrap();
        let clock = match &request {
            Message::SyncRequest { vector_clock } => vector_clock.clone(),
            _ => panic!(),
        };

        let response = process_sync_request(&storage_b, &clock).unwrap();
        let events = match response {
            Message::SyncEvents { events } => events,
            _ => panic!(),
        };
        assert!(events.is_empty());
    }

    #[test]
    fn test_three_way_sync() {
        let storage_a = Storage::open_in_memory().unwrap();
        let storage_b = Storage::open_in_memory().unwrap();
        let storage_c = Storage::open_in_memory().unwrap();

        // Each peer creates unique events
        storage_a.append_event("a_event", "{}").unwrap();
        storage_b.append_event("b_event", "{}").unwrap();
        storage_c.append_event("c_event", "{}").unwrap();

        // A syncs with B (bidirectional)
        let req = create_sync_request(&storage_a).unwrap();
        let clock = match &req { Message::SyncRequest { vector_clock } => vector_clock.clone(), _ => panic!() };
        let resp = process_sync_request(&storage_b, &clock).unwrap();
        let evts = match resp { Message::SyncEvents { events } => events, _ => panic!() };
        process_sync_events(&storage_a, evts).unwrap();

        let req = create_sync_request(&storage_b).unwrap();
        let clock = match &req { Message::SyncRequest { vector_clock } => vector_clock.clone(), _ => panic!() };
        let resp = process_sync_request(&storage_a, &clock).unwrap();
        let evts = match resp { Message::SyncEvents { events } => events, _ => panic!() };
        process_sync_events(&storage_b, evts).unwrap();

        // A now has A + B events
        assert_eq!(storage_a.event_count().unwrap(), 2);

        // A syncs with C (bidirectional)
        let req = create_sync_request(&storage_a).unwrap();
        let clock = match &req { Message::SyncRequest { vector_clock } => vector_clock.clone(), _ => panic!() };
        let resp = process_sync_request(&storage_c, &clock).unwrap();
        let evts = match resp { Message::SyncEvents { events } => events, _ => panic!() };
        process_sync_events(&storage_a, evts).unwrap();

        let req = create_sync_request(&storage_c).unwrap();
        let clock = match &req { Message::SyncRequest { vector_clock } => vector_clock.clone(), _ => panic!() };
        let resp = process_sync_request(&storage_a, &clock).unwrap();
        let evts = match resp { Message::SyncEvents { events } => events, _ => panic!() };
        process_sync_events(&storage_c, evts).unwrap();

        // A and C should both have all 3 events
        assert_eq!(storage_a.event_count().unwrap(), 3);
        assert_eq!(storage_c.event_count().unwrap(), 3);
    }
}
