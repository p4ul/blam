//! Persistent storage using SQLite (rusqlite)
//!
//! This module provides:
//! - OS-standard data directory location (via `directories` crate)
//! - SQLite database with schema versioning
//! - Append-only event log for CRDT sync
//! - Actor identity management
//! - CRDT sync logic for peer-to-peer event exchange

pub mod sync;

use directories::ProjectDirs;
use rusqlite::{params, Connection, Result as SqlResult};
use std::path::PathBuf;

/// Current schema version. Bump this when making schema changes.
const SCHEMA_VERSION: u32 = 1;

/// Errors that can occur during storage operations.
#[derive(Debug)]
pub enum StorageError {
    /// Database error from SQLite
    Database(rusqlite::Error),
    /// Could not determine data directory
    NoDataDirectory,
    /// Schema version mismatch (future version)
    FutureSchemaVersion { found: u32, supported: u32 },
    /// Failed to create data directory
    CreateDirFailed(std::io::Error),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Database(e) => write!(f, "database error: {}", e),
            StorageError::NoDataDirectory => write!(f, "could not determine data directory"),
            StorageError::FutureSchemaVersion { found, supported } => {
                write!(
                    f,
                    "database schema version {} is newer than supported version {}",
                    found, supported
                )
            }
            StorageError::CreateDirFailed(e) => write!(f, "failed to create data directory: {}", e),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        StorageError::Database(e)
    }
}

/// A stored event in the append-only log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The actor (device) that created this event
    pub actor_id: ActorId,
    /// Sequence number, unique per actor
    pub seq: i64,
    /// Type of event (e.g., "round_start", "claim", "round_end")
    pub event_type: String,
    /// JSON payload containing event data
    pub payload: String,
    /// Unix timestamp (milliseconds) when event was created
    pub created_at: i64,
}

/// A unique identifier for an actor (device/player).
/// 16-byte random ID, stored as BLOB in SQLite.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorId(pub [u8; 16]);

impl ActorId {
    /// Generate a new random actor ID.
    pub fn generate() -> Self {
        use rand::Rng;
        let mut bytes = [0u8; 16];
        rand::rng().fill(&mut bytes);
        ActorId(bytes)
    }

    /// Create from raw bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() == 16 {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(bytes);
            Some(ActorId(arr))
        } else {
            None
        }
    }

    /// Get the raw bytes.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Convert to hex string for display.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

/// The main storage handle for BLAM! data.
pub struct Storage {
    conn: Connection,
    actor_id: ActorId,
}

impl Storage {
    /// Open or create the storage database.
    ///
    /// Uses OS-standard directories:
    /// - Linux: `$XDG_DATA_HOME/blam/` or `~/.local/share/blam/`
    /// - macOS: `~/Library/Application Support/blam/`
    pub fn open() -> Result<Self, StorageError> {
        let data_dir = Self::data_dir()?;

        // Ensure directory exists
        std::fs::create_dir_all(&data_dir).map_err(StorageError::CreateDirFailed)?;

        let db_path = data_dir.join("blam.db");
        let conn = Connection::open(&db_path)?;

        let mut storage = Storage {
            conn,
            actor_id: ActorId([0; 16]), // Placeholder, will be loaded/created
        };

        storage.initialize_schema()?;
        storage.actor_id = storage.load_or_create_actor_id()?;

        Ok(storage)
    }

    /// Open an in-memory database (for testing).
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        let mut storage = Storage {
            conn,
            actor_id: ActorId([0; 16]),
        };
        storage.initialize_schema()?;
        storage.actor_id = storage.load_or_create_actor_id()?;
        Ok(storage)
    }

    /// Get the OS-standard data directory for BLAM!
    pub fn data_dir() -> Result<PathBuf, StorageError> {
        ProjectDirs::from("", "", "blam")
            .map(|dirs| dirs.data_dir().to_path_buf())
            .ok_or(StorageError::NoDataDirectory)
    }

    /// Get this device's actor ID.
    pub fn actor_id(&self) -> &ActorId {
        &self.actor_id
    }

    /// Get the current handle (player name).
    pub fn handle(&self) -> SqlResult<Option<String>> {
        self.conn
            .query_row("SELECT handle FROM meta LIMIT 1", [], |row| row.get::<_, Option<String>>(0))
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(e),
            })
    }

    /// Set the handle (player name).
    pub fn set_handle(&self, handle: &str) -> SqlResult<()> {
        self.conn
            .execute("UPDATE meta SET handle = ?1", params![handle])?;
        Ok(())
    }

    /// Append an event to the log.
    ///
    /// The sequence number is automatically assigned as the next value for this actor.
    pub fn append_event(&self, event_type: &str, payload: &str) -> Result<Event, StorageError> {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // Get next sequence number for this actor
        let seq: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE actor_id = ?1",
                params![self.actor_id.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .unwrap_or(1);

        self.conn.execute(
            "INSERT INTO events (actor_id, seq, event_type, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                self.actor_id.as_bytes().as_slice(),
                seq,
                event_type,
                payload,
                created_at
            ],
        )?;

        Ok(Event {
            actor_id: self.actor_id.clone(),
            seq,
            event_type: event_type.to_string(),
            payload: payload.to_string(),
            created_at,
        })
    }

    /// Insert an event from another actor (for CRDT sync).
    ///
    /// Returns true if the event was inserted, false if it already existed.
    pub fn insert_remote_event(&self, event: &Event) -> Result<bool, StorageError> {
        let result = self.conn.execute(
            "INSERT OR IGNORE INTO events (actor_id, seq, event_type, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                event.actor_id.as_bytes().as_slice(),
                event.seq,
                &event.event_type,
                &event.payload,
                event.created_at
            ],
        )?;
        Ok(result > 0)
    }

    /// Get the highest sequence number seen for each actor (vector clock).
    pub fn get_vector_clock(&self) -> Result<Vec<(ActorId, i64)>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT actor_id, MAX(seq) FROM events GROUP BY actor_id")?;
        let rows = stmt.query_map([], |row| {
            let actor_bytes: Vec<u8> = row.get(0)?;
            let seq: i64 = row.get(1)?;
            Ok((actor_bytes, seq))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (actor_bytes, seq) = row?;
            if let Some(actor_id) = ActorId::from_bytes(&actor_bytes) {
                result.push((actor_id, seq));
            }
        }
        Ok(result)
    }

    /// Get events from a specific actor after a given sequence number.
    ///
    /// Used for CRDT sync: "give me all events from actor X after seq N".
    pub fn get_events_after(
        &self,
        actor_id: &ActorId,
        after_seq: i64,
    ) -> Result<Vec<Event>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT actor_id, seq, event_type, payload, created_at FROM events WHERE actor_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;

        let rows = stmt.query_map(params![actor_id.as_bytes().as_slice(), after_seq], |row| {
            let actor_bytes: Vec<u8> = row.get(0)?;
            let seq: i64 = row.get(1)?;
            let event_type: String = row.get(2)?;
            let payload: String = row.get(3)?;
            let created_at: i64 = row.get(4)?;
            Ok((actor_bytes, seq, event_type, payload, created_at))
        })?;

        let mut events = Vec::new();
        for row in rows {
            let (actor_bytes, seq, event_type, payload, created_at) = row?;
            if let Some(actor_id) = ActorId::from_bytes(&actor_bytes) {
                events.push(Event {
                    actor_id,
                    seq,
                    event_type,
                    payload,
                    created_at,
                });
            }
        }
        Ok(events)
    }

    /// Get all events in chronological order (by created_at, then actor_id, then seq).
    pub fn get_all_events(&self) -> Result<Vec<Event>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT actor_id, seq, event_type, payload, created_at FROM events ORDER BY created_at, actor_id, seq",
        )?;

        let rows = stmt.query_map([], |row| {
            let actor_bytes: Vec<u8> = row.get(0)?;
            let seq: i64 = row.get(1)?;
            let event_type: String = row.get(2)?;
            let payload: String = row.get(3)?;
            let created_at: i64 = row.get(4)?;
            Ok((actor_bytes, seq, event_type, payload, created_at))
        })?;

        let mut events = Vec::new();
        for row in rows {
            let (actor_bytes, seq, event_type, payload, created_at) = row?;
            if let Some(actor_id) = ActorId::from_bytes(&actor_bytes) {
                events.push(Event {
                    actor_id,
                    seq,
                    event_type,
                    payload,
                    created_at,
                });
            }
        }
        Ok(events)
    }

    /// Get the total number of events in the log.
    pub fn event_count(&self) -> Result<i64, StorageError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;
        Ok(count)
    }

    // Private helper methods

    fn initialize_schema(&self) -> Result<(), StorageError> {
        // Check current schema version
        let current_version = self.get_schema_version()?;

        if current_version == 0 {
            // Fresh database, create schema
            self.create_schema_v1()?;
        } else if current_version < SCHEMA_VERSION {
            // Need to migrate
            self.migrate_schema(current_version)?;
        } else if current_version > SCHEMA_VERSION {
            // Database is from a newer version of BLAM!
            return Err(StorageError::FutureSchemaVersion {
                found: current_version,
                supported: SCHEMA_VERSION,
            });
        }

        Ok(())
    }

    fn get_schema_version(&self) -> Result<u32, StorageError> {
        // Check if meta table exists
        let table_exists: bool = self.conn.query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='meta'",
            [],
            |row| row.get(0),
        )?;

        if !table_exists {
            return Ok(0);
        }

        let version: u32 = self
            .conn
            .query_row("SELECT schema_version FROM meta LIMIT 1", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);

        Ok(version)
    }

    fn create_schema_v1(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            r#"
            -- Meta table: stores device identity and schema version
            CREATE TABLE meta (
                schema_version INTEGER NOT NULL,
                actor_id BLOB NOT NULL,
                handle TEXT,
                created_at INTEGER NOT NULL
            );

            -- Events table: append-only log for CRDT sync
            -- Primary key (actor_id, seq) ensures uniqueness per actor
            CREATE TABLE events (
                actor_id BLOB NOT NULL,
                seq INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (actor_id, seq)
            );

            -- Index for efficient event retrieval by type
            CREATE INDEX idx_events_type ON events (event_type);

            -- Index for chronological ordering
            CREATE INDEX idx_events_created ON events (created_at);
            "#,
        )?;

        // Insert initial meta row
        let actor_id = ActorId::generate();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        self.conn.execute(
            "INSERT INTO meta (schema_version, actor_id, handle, created_at) VALUES (?1, ?2, NULL, ?3)",
            params![SCHEMA_VERSION, actor_id.as_bytes().as_slice(), created_at],
        )?;

        Ok(())
    }

    fn migrate_schema(&self, from_version: u32) -> Result<(), StorageError> {
        // Future migrations go here
        // For now, we only have version 1, so any older version is unsupported
        if from_version < SCHEMA_VERSION {
            // Unknown version, can't migrate from it
            return Err(StorageError::FutureSchemaVersion {
                found: from_version,
                supported: SCHEMA_VERSION,
            });
        }

        // Update schema version in meta (for future use when migrations exist)
        self.conn.execute(
            "UPDATE meta SET schema_version = ?1",
            params![SCHEMA_VERSION],
        )?;

        Ok(())
    }

    fn load_or_create_actor_id(&self) -> Result<ActorId, StorageError> {
        let actor_bytes: Vec<u8> =
            self.conn
                .query_row("SELECT actor_id FROM meta LIMIT 1", [], |row| row.get(0))?;

        ActorId::from_bytes(&actor_bytes).ok_or_else(|| {
            StorageError::Database(rusqlite::Error::InvalidParameterCount(0, 16))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actor_id_generation() {
        let id1 = ActorId::generate();
        let id2 = ActorId::generate();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_actor_id_roundtrip() {
        let id = ActorId::generate();
        let bytes = id.as_bytes();
        let recovered = ActorId::from_bytes(bytes).unwrap();
        assert_eq!(id, recovered);
    }

    #[test]
    fn test_actor_id_hex() {
        let id = ActorId([0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
                         0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10]);
        assert_eq!(id.to_hex(), "0123456789abcdeffedcba9876543210");
    }

    #[test]
    fn test_storage_creation() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(!storage.actor_id().as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn test_handle_storage() {
        let storage = Storage::open_in_memory().unwrap();

        // Initially no handle
        assert!(storage.handle().unwrap().is_none());

        // Set and retrieve
        storage.set_handle("TestPlayer").unwrap();
        assert_eq!(storage.handle().unwrap(), Some("TestPlayer".to_string()));

        // Update
        storage.set_handle("NewName").unwrap();
        assert_eq!(storage.handle().unwrap(), Some("NewName".to_string()));
    }

    #[test]
    fn test_append_event() {
        let storage = Storage::open_in_memory().unwrap();

        let event1 = storage.append_event("test", r#"{"data": 1}"#).unwrap();
        assert_eq!(event1.seq, 1);
        assert_eq!(event1.event_type, "test");

        let event2 = storage.append_event("test", r#"{"data": 2}"#).unwrap();
        assert_eq!(event2.seq, 2);

        assert_eq!(storage.event_count().unwrap(), 2);
    }

    #[test]
    fn test_vector_clock() {
        let storage = Storage::open_in_memory().unwrap();

        storage.append_event("test", "{}").unwrap();
        storage.append_event("test", "{}").unwrap();
        storage.append_event("test", "{}").unwrap();

        let vclock = storage.get_vector_clock().unwrap();
        assert_eq!(vclock.len(), 1);
        assert_eq!(vclock[0].0, *storage.actor_id());
        assert_eq!(vclock[0].1, 3);
    }

    #[test]
    fn test_insert_remote_event() {
        let storage = Storage::open_in_memory().unwrap();

        let remote_actor = ActorId::generate();
        let remote_event = Event {
            actor_id: remote_actor.clone(),
            seq: 1,
            event_type: "remote_test".to_string(),
            payload: r#"{"from": "remote"}"#.to_string(),
            created_at: 1234567890000,
        };

        // First insert should succeed
        assert!(storage.insert_remote_event(&remote_event).unwrap());

        // Duplicate insert should be ignored
        assert!(!storage.insert_remote_event(&remote_event).unwrap());

        // Vector clock should show both actors
        let vclock = storage.get_vector_clock().unwrap();
        assert_eq!(vclock.len(), 1); // Only remote actor has events
        assert_eq!(vclock[0].0, remote_actor);
        assert_eq!(vclock[0].1, 1);
    }

    #[test]
    fn test_get_events_after() {
        let storage = Storage::open_in_memory().unwrap();

        storage.append_event("test", r#"{"n": 1}"#).unwrap();
        storage.append_event("test", r#"{"n": 2}"#).unwrap();
        storage.append_event("test", r#"{"n": 3}"#).unwrap();

        let events = storage.get_events_after(storage.actor_id(), 1).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 2);
        assert_eq!(events[1].seq, 3);

        let events = storage.get_events_after(storage.actor_id(), 3).unwrap();
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_get_all_events() {
        let storage = Storage::open_in_memory().unwrap();

        storage.append_event("a", "{}").unwrap();
        storage.append_event("b", "{}").unwrap();
        storage.append_event("c", "{}").unwrap();

        let events = storage.get_all_events().unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_data_dir() {
        // Just verify it returns something on supported platforms
        let result = Storage::data_dir();
        // This might fail in weird environments, but should work on Linux/macOS
        if let Ok(path) = result {
            assert!(path.to_string_lossy().contains("blam"));
        }
    }
}
