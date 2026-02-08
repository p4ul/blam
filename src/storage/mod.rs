#![allow(dead_code)]
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
/// Version history:
/// - v1: Initial schema with meta and events tables
/// - v2: Added derived_stats and derived_elo cache tables
const SCHEMA_VERSION: u32 = 2;

/// Event payload version. Included in all event payloads for forward compatibility.
/// Older binaries can read newer payloads by ignoring unknown fields.
pub const PAYLOAD_VERSION: u32 = 1;

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
    /// Migration failed
    MigrationFailed { from: u32, to: u32, reason: String },
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
            StorageError::MigrationFailed { from, to, reason } => {
                write!(f, "migration from v{} to v{} failed: {}", from, to, reason)
            }
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

            -- Derived stats cache: stores computed player statistics
            -- Can be dropped and rebuilt from events table
            CREATE TABLE derived_stats (
                handle TEXT PRIMARY KEY,
                elo REAL NOT NULL DEFAULT 1200.0,
                rounds_played INTEGER NOT NULL DEFAULT 0,
                total_points INTEGER NOT NULL DEFAULT 0,
                best_score INTEGER NOT NULL DEFAULT 0,
                longest_word TEXT NOT NULL DEFAULT '',
                words_claimed INTEGER NOT NULL DEFAULT 0,
                wins INTEGER NOT NULL DEFAULT 0,
                last_updated INTEGER NOT NULL
            );

            -- Derived Elo history: stores rating snapshots after each match
            -- Can be dropped and rebuilt from events table
            CREATE TABLE derived_elo_history (
                match_id INTEGER NOT NULL,
                handle TEXT NOT NULL,
                elo_before REAL NOT NULL,
                elo_after REAL NOT NULL,
                elo_change REAL NOT NULL,
                PRIMARY KEY (match_id, handle)
            );

            -- Index for efficient player lookups in Elo history
            CREATE INDEX idx_elo_history_handle ON derived_elo_history (handle);

            -- Cache metadata: tracks when caches were last rebuilt
            CREATE TABLE derived_cache_meta (
                cache_name TEXT PRIMARY KEY,
                last_event_seq INTEGER NOT NULL DEFAULT 0,
                last_rebuilt INTEGER NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0
            );
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
        let mut current_version = from_version;

        // Apply migrations sequentially
        while current_version < SCHEMA_VERSION {
            match current_version {
                1 => {
                    // Migrate from v1 to v2: Add derived cache tables
                    self.migrate_v1_to_v2()?;
                    current_version = 2;
                }
                _ => {
                    // Unknown version, can't migrate from it
                    return Err(StorageError::MigrationFailed {
                        from: current_version,
                        to: SCHEMA_VERSION,
                        reason: format!("no migration path from version {}", current_version),
                    });
                }
            }
        }

        // Update schema version in meta
        self.conn.execute(
            "UPDATE meta SET schema_version = ?1",
            params![SCHEMA_VERSION],
        )?;

        Ok(())
    }

    /// Migrate from schema v1 to v2: Add derived cache tables
    fn migrate_v1_to_v2(&self) -> Result<(), StorageError> {
        self.conn.execute_batch(
            r#"
            -- Derived stats cache: stores computed player statistics
            -- Can be dropped and rebuilt from events table
            CREATE TABLE IF NOT EXISTS derived_stats (
                handle TEXT PRIMARY KEY,
                elo REAL NOT NULL DEFAULT 1200.0,
                rounds_played INTEGER NOT NULL DEFAULT 0,
                total_points INTEGER NOT NULL DEFAULT 0,
                best_score INTEGER NOT NULL DEFAULT 0,
                longest_word TEXT NOT NULL DEFAULT '',
                words_claimed INTEGER NOT NULL DEFAULT 0,
                wins INTEGER NOT NULL DEFAULT 0,
                last_updated INTEGER NOT NULL
            );

            -- Derived Elo history: stores rating snapshots after each match
            -- Can be dropped and rebuilt from events table
            CREATE TABLE IF NOT EXISTS derived_elo_history (
                match_id INTEGER NOT NULL,
                handle TEXT NOT NULL,
                elo_before REAL NOT NULL,
                elo_after REAL NOT NULL,
                elo_change REAL NOT NULL,
                PRIMARY KEY (match_id, handle)
            );

            -- Index for efficient player lookups in Elo history
            CREATE INDEX IF NOT EXISTS idx_elo_history_handle ON derived_elo_history (handle);

            -- Cache metadata: tracks when caches were last rebuilt
            CREATE TABLE IF NOT EXISTS derived_cache_meta (
                cache_name TEXT PRIMARY KEY,
                last_event_seq INTEGER NOT NULL DEFAULT 0,
                last_rebuilt INTEGER NOT NULL,
                event_count INTEGER NOT NULL DEFAULT 0
            );
            "#,
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

    // === Derived Cache Methods ===

    /// Drop and rebuild all derived caches from the event log.
    ///
    /// This is safe to call at any time - derived data can always be
    /// recomputed from the authoritative event log. Useful after:
    /// - Schema upgrades
    /// - CRDT sync that added many events
    /// - Suspected cache corruption
    pub fn rebuild_derived_caches(&self) -> Result<(), StorageError> {
        // Clear existing derived data
        self.conn.execute_batch(
            r#"
            DELETE FROM derived_stats;
            DELETE FROM derived_elo_history;
            DELETE FROM derived_cache_meta;
            "#,
        )?;

        // Rebuild from events
        self.rebuild_stats_cache()?;
        self.rebuild_elo_cache()?;

        Ok(())
    }

    /// Rebuild the derived_stats cache from match_end events.
    fn rebuild_stats_cache(&self) -> Result<(), StorageError> {
        use std::collections::HashMap;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // Collect all match_end events
        let mut stmt = self.conn.prepare(
            "SELECT payload FROM events WHERE event_type = 'match_end' ORDER BY created_at, actor_id, seq"
        )?;

        let payloads: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Track stats for each player
        struct Stats {
            elo: f64,
            rounds_played: u32,
            total_points: u32,
            best_score: u32,
            longest_word: String,
            words_claimed: u32,
            wins: u32,
        }

        let mut player_stats: HashMap<String, Stats> = HashMap::new();

        for payload in &payloads {
            if let Some(match_result) = parse_match_result_payload(payload) {
                // Find winner(s)
                let max_score = match_result.scores.iter().map(|(_, s)| *s).max().unwrap_or(0);
                let is_multiplayer = match_result.scores.len() >= 2;

                for (handle, score) in &match_result.scores {
                    let stats = player_stats.entry(handle.clone()).or_insert(Stats {
                        elo: 1200.0,
                        rounds_played: 0,
                        total_points: 0,
                        best_score: 0,
                        longest_word: String::new(),
                        words_claimed: 0,
                        wins: 0,
                    });

                    stats.rounds_played += 1;
                    stats.total_points += score;
                    if *score > stats.best_score {
                        stats.best_score = *score;
                    }
                    if is_multiplayer && *score == max_score {
                        stats.wins += 1;
                    }
                }
            }
        }

        // Also count word claims from word_claimed events
        let mut stmt = self.conn.prepare(
            "SELECT payload FROM events WHERE event_type = 'word_claimed' ORDER BY created_at"
        )?;

        let claim_payloads: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        for payload in &claim_payloads {
            if let (Some(handle), Some(word)) = (
                extract_json_string(payload, "player_name"),
                extract_json_string(payload, "word"),
            ) {
                let stats = player_stats.entry(handle).or_insert(Stats {
                    elo: 1200.0,
                    rounds_played: 0,
                    total_points: 0,
                    best_score: 0,
                    longest_word: String::new(),
                    words_claimed: 0,
                    wins: 0,
                });

                stats.words_claimed += 1;
                if word.len() > stats.longest_word.len() {
                    stats.longest_word = word;
                }
            }
        }

        // Insert into derived_stats
        for (handle, stats) in &player_stats {
            self.conn.execute(
                "INSERT INTO derived_stats (handle, elo, rounds_played, total_points, best_score, longest_word, words_claimed, wins, last_updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    handle,
                    stats.elo, // Will be updated by Elo rebuild
                    stats.rounds_played,
                    stats.total_points,
                    stats.best_score,
                    &stats.longest_word,
                    stats.words_claimed,
                    stats.wins,
                    now
                ],
            )?;
        }

        // Update cache metadata
        let event_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_type IN ('match_end', 'word_claimed')",
            [],
            |row| row.get(0),
        )?;

        self.conn.execute(
            "INSERT OR REPLACE INTO derived_cache_meta (cache_name, last_event_seq, last_rebuilt, event_count)
             VALUES ('stats', 0, ?1, ?2)",
            params![now, event_count],
        )?;

        Ok(())
    }

    /// Rebuild the derived_elo_history and update Elo ratings in derived_stats.
    fn rebuild_elo_cache(&self) -> Result<(), StorageError> {
        use std::collections::HashMap;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // Collect all match_end events sorted by match_id for deterministic replay
        let mut stmt = self.conn.prepare(
            "SELECT payload FROM events WHERE event_type = 'match_end' ORDER BY created_at, actor_id, seq"
        )?;

        let payloads: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Replay matches to compute Elo
        const K: f64 = 32.0;
        const DEFAULT_ELO: f64 = 1200.0;

        let mut ratings: HashMap<String, f64> = HashMap::new();

        for payload in &payloads {
            if let Some(match_result) = parse_match_result_payload(payload) {
                if !match_result.completed || match_result.scores.len() < 2 {
                    continue;
                }

                let n = match_result.scores.len();
                let k_adjusted = K / (n - 1) as f64;

                // Get current ratings
                let player_ratings: Vec<(String, u32, f64)> = match_result
                    .scores
                    .iter()
                    .map(|(name, score)| {
                        let rating = *ratings.get(name).unwrap_or(&DEFAULT_ELO);
                        (name.clone(), *score, rating)
                    })
                    .collect();

                // Calculate rating changes using pairwise comparisons
                let mut rating_changes: HashMap<String, f64> = HashMap::new();

                for (i, (player_a, score_a, rating_a)) in player_ratings.iter().enumerate() {
                    let mut total_change = 0.0;

                    for (j, (_, score_b, rating_b)) in player_ratings.iter().enumerate() {
                        if i == j {
                            continue;
                        }

                        let actual = match score_a.cmp(score_b) {
                            std::cmp::Ordering::Greater => 1.0,
                            std::cmp::Ordering::Equal => 0.5,
                            std::cmp::Ordering::Less => 0.0,
                        };

                        let expected = 1.0 / (1.0 + 10.0_f64.powf((rating_b - rating_a) / 400.0));
                        total_change += k_adjusted * (actual - expected);
                    }

                    rating_changes.insert(player_a.clone(), total_change);
                }

                // Record Elo history and apply changes
                for (player, change) in &rating_changes {
                    let elo_before = *ratings.get(player).unwrap_or(&DEFAULT_ELO);
                    let elo_after = elo_before + change;

                    self.conn.execute(
                        "INSERT OR REPLACE INTO derived_elo_history (match_id, handle, elo_before, elo_after, elo_change)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![match_result.match_id, player, elo_before, elo_after, change],
                    )?;

                    ratings.insert(player.clone(), elo_after);
                }
            }
        }

        // Update Elo ratings in derived_stats
        for (handle, elo) in &ratings {
            self.conn.execute(
                "UPDATE derived_stats SET elo = ?1, last_updated = ?2 WHERE handle = ?3",
                params![elo, now, handle],
            )?;
        }

        // Update cache metadata
        let event_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_type = 'match_end'",
            [],
            |row| row.get(0),
        )?;

        self.conn.execute(
            "INSERT OR REPLACE INTO derived_cache_meta (cache_name, last_event_seq, last_rebuilt, event_count)
             VALUES ('elo', 0, ?1, ?2)",
            params![now, event_count],
        )?;

        Ok(())
    }

    /// Get cached stats for a player from derived_stats.
    pub fn get_cached_stats(&self, handle: &str) -> Result<Option<CachedPlayerStats>, StorageError> {
        let result = self.conn.query_row(
            "SELECT elo, rounds_played, total_points, best_score, longest_word, words_claimed, wins
             FROM derived_stats WHERE handle = ?1",
            params![handle],
            |row| {
                Ok(CachedPlayerStats {
                    handle: handle.to_string(),
                    elo: row.get(0)?,
                    rounds_played: row.get(1)?,
                    total_points: row.get(2)?,
                    best_score: row.get(3)?,
                    longest_word: row.get(4)?,
                    words_claimed: row.get(5)?,
                    wins: row.get(6)?,
                })
            },
        );

        match result {
            Ok(stats) => Ok(Some(stats)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Database(e)),
        }
    }

    /// Get the Elo leaderboard from cached stats.
    pub fn get_cached_leaderboard(&self) -> Result<Vec<(String, f64)>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT handle, elo FROM derived_stats ORDER BY elo DESC"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })?;

        let mut leaderboard = Vec::new();
        for row in rows {
            leaderboard.push(row?);
        }

        Ok(leaderboard)
    }

    /// Check if caches need rebuilding (e.g., after CRDT sync added new events).
    pub fn caches_need_rebuild(&self) -> Result<bool, StorageError> {
        // Get current event counts
        let match_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE event_type = 'match_end'",
            [],
            |row| row.get(0),
        )?;

        // Get cached event count
        let cached_count: i64 = self.conn.query_row(
            "SELECT COALESCE(event_count, 0) FROM derived_cache_meta WHERE cache_name = 'elo'",
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        Ok(match_count != cached_count)
    }
}

/// Cached player statistics from derived_stats table.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedPlayerStats {
    pub handle: String,
    pub elo: f64,
    pub rounds_played: u32,
    pub total_points: u32,
    pub best_score: u32,
    pub longest_word: String,
    pub words_claimed: u32,
    pub wins: u32,
}

/// Parsed match result from event payload.
struct ParsedMatchResult {
    match_id: i64,
    scores: Vec<(String, u32)>,
    completed: bool,
}

/// Parse a match_end event payload to extract match result.
fn parse_match_result_payload(payload: &str) -> Option<ParsedMatchResult> {
    let match_id = extract_json_i64(payload, "match_id")?;
    let completed = extract_json_bool(payload, "completed").unwrap_or(true);
    let scores = extract_json_scores(payload)?;

    Some(ParsedMatchResult {
        match_id,
        scores,
        completed,
    })
}

// === Versioned Payload Helpers ===

/// Create a versioned event payload with the current payload version.
/// The payload_version field enables forward compatibility - older binaries
/// can read newer payloads by ignoring unknown fields.
pub fn create_versioned_payload(inner_json: &str) -> String {
    // Insert payload_version at the beginning of the JSON object
    if inner_json.starts_with('{') {
        format!(
            r#"{{"payload_version":{},"#,
            PAYLOAD_VERSION
        ) + &inner_json[1..]
    } else {
        // If not a JSON object, wrap it
        format!(
            r#"{{"payload_version":{},"data":{}}}"#,
            PAYLOAD_VERSION, inner_json
        )
    }
}

/// Extract the payload version from a versioned payload.
/// Returns None if the payload doesn't have a version (pre-versioning payloads).
pub fn extract_payload_version(payload: &str) -> Option<u32> {
    extract_json_i64(payload, "payload_version").map(|v| v as u32)
}

/// Check if a payload is compatible with the current version.
/// Returns true if:
/// - The payload has no version (legacy payload, always compatible)
/// - The payload version is <= current version (backward compatible)
pub fn is_payload_compatible(payload: &str) -> bool {
    match extract_payload_version(payload) {
        None => true, // Legacy payload without version
        Some(v) => v <= PAYLOAD_VERSION,
    }
}

// === JSON Helper Functions ===

fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let pattern = format!(r#""{}":""#, key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..];
    let end = find_unescaped_quote(rest)?;
    Some(unescape_json(&rest[..end]))
}

fn extract_json_i64(json: &str, key: &str) -> Option<i64> {
    let pattern = format!(r#""{}":"#, key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_json_bool(json: &str, key: &str) -> Option<bool> {
    let pattern = format!(r#""{}":"#, key);
    let start = json.find(&pattern)? + pattern.len();
    let rest = &json[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_json_scores(json: &str) -> Option<Vec<(String, u32)>> {
    let pattern = r#""scores":["#;
    let start = json.find(pattern)? + pattern.len();
    let rest = &json[start..];

    // Find matching closing bracket
    let mut depth = 1;
    let mut end = 0;
    for (i, c) in rest.chars().enumerate() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }

    let array = &rest[..end];
    let mut scores = Vec::new();
    let mut current = array;

    while let Some(start) = current.find('[') {
        let inner = &current[start + 1..];
        if let Some(end) = inner.find(']') {
            let item = &inner[..end];
            if let Some(comma) = item.find(',') {
                let name = item[..comma].trim().trim_matches('"');
                let score_str = item[comma + 1..].trim();
                if let Ok(score) = score_str.parse() {
                    scores.push((unescape_json(name), score));
                }
            }
            current = &inner[end + 1..];
        } else {
            break;
        }
    }

    Some(scores)
}

fn find_unescaped_quote(s: &str) -> Option<usize> {
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            return Some(i);
        } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    None
}

fn unescape_json(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => result.push('"'),
                Some('\\') => result.push('\\'),
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
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

    // === Schema Migration Tests ===

    #[test]
    fn test_schema_version_is_current() {
        let storage = Storage::open_in_memory().unwrap();
        let version: u32 = storage
            .conn
            .query_row("SELECT schema_version FROM meta", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_derived_tables_exist() {
        let storage = Storage::open_in_memory().unwrap();

        // Verify derived_stats table exists
        let stats_exists: bool = storage
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='derived_stats'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(stats_exists, "derived_stats table should exist");

        // Verify derived_elo_history table exists
        let elo_exists: bool = storage
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='derived_elo_history'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(elo_exists, "derived_elo_history table should exist");

        // Verify derived_cache_meta table exists
        let meta_exists: bool = storage
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='derived_cache_meta'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(meta_exists, "derived_cache_meta table should exist");
    }

    // === Versioned Payload Tests ===

    #[test]
    fn test_create_versioned_payload() {
        let inner = r#"{"data":"test"}"#;
        let versioned = create_versioned_payload(inner);

        // Should contain payload_version
        assert!(versioned.contains("payload_version"));
        assert!(versioned.contains(&format!("\"payload_version\":{}", PAYLOAD_VERSION)));

        // Should still contain original data
        assert!(versioned.contains("\"data\":\"test\""));
    }

    #[test]
    fn test_extract_payload_version() {
        // Versioned payload
        let versioned = r#"{"payload_version":1,"data":"test"}"#;
        assert_eq!(extract_payload_version(versioned), Some(1));

        // Future version
        let future = r#"{"payload_version":99,"data":"test"}"#;
        assert_eq!(extract_payload_version(future), Some(99));

        // Legacy payload (no version)
        let legacy = r#"{"data":"test"}"#;
        assert_eq!(extract_payload_version(legacy), None);
    }

    #[test]
    fn test_payload_compatibility() {
        // Current version is compatible
        let current = format!(r#"{{"payload_version":{},"data":"test"}}"#, PAYLOAD_VERSION);
        assert!(is_payload_compatible(&current));

        // Older version is compatible (backward compatible)
        let older = r#"{"payload_version":0,"data":"test"}"#;
        assert!(is_payload_compatible(older));

        // Legacy (no version) is compatible
        let legacy = r#"{"data":"test"}"#;
        assert!(is_payload_compatible(legacy));

        // Future version is NOT compatible
        let future = r#"{"payload_version":999,"data":"test"}"#;
        assert!(!is_payload_compatible(future));
    }

    // === Derived Cache Tests ===

    #[test]
    fn test_rebuild_derived_caches_empty() {
        let storage = Storage::open_in_memory().unwrap();

        // Should not fail with empty event log
        storage.rebuild_derived_caches().unwrap();

        // Verify caches are empty
        let count: i64 = storage
            .conn
            .query_row("SELECT COUNT(*) FROM derived_stats", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_rebuild_derived_caches_with_matches() {
        let storage = Storage::open_in_memory().unwrap();

        // Add some match_end events
        let match1 = r#"{"match_id":1,"scores":[["Alice",50],["Bob",30]],"host_actor_id":"host1","completed":true}"#;
        let match2 = r#"{"match_id":2,"scores":[["Alice",40],["Bob",60]],"host_actor_id":"host1","completed":true}"#;

        storage.append_event("match_end", match1).unwrap();
        storage.append_event("match_end", match2).unwrap();

        // Rebuild caches
        storage.rebuild_derived_caches().unwrap();

        // Verify stats were computed
        let alice_stats = storage.get_cached_stats("Alice").unwrap().unwrap();
        assert_eq!(alice_stats.rounds_played, 2);
        assert_eq!(alice_stats.total_points, 90);
        assert_eq!(alice_stats.best_score, 50);
        assert_eq!(alice_stats.wins, 1); // Won match 1

        let bob_stats = storage.get_cached_stats("Bob").unwrap().unwrap();
        assert_eq!(bob_stats.rounds_played, 2);
        assert_eq!(bob_stats.total_points, 90);
        assert_eq!(bob_stats.best_score, 60);
        assert_eq!(bob_stats.wins, 1); // Won match 2
    }

    #[test]
    fn test_cached_leaderboard() {
        let storage = Storage::open_in_memory().unwrap();

        // Add match where Alice wins
        let match1 = r#"{"match_id":1,"scores":[["Alice",50],["Bob",30]],"host_actor_id":"host1","completed":true}"#;
        storage.append_event("match_end", match1).unwrap();

        storage.rebuild_derived_caches().unwrap();

        let leaderboard = storage.get_cached_leaderboard().unwrap();
        assert_eq!(leaderboard.len(), 2);

        // Alice should be first (higher Elo after win)
        assert_eq!(leaderboard[0].0, "Alice");
        assert!(leaderboard[0].1 > 1200.0);

        // Bob should be second (lower Elo after loss)
        assert_eq!(leaderboard[1].0, "Bob");
        assert!(leaderboard[1].1 < 1200.0);
    }

    #[test]
    fn test_caches_need_rebuild() {
        let storage = Storage::open_in_memory().unwrap();

        // Initially no events, caches don't need rebuild (both are 0)
        assert!(!storage.caches_need_rebuild().unwrap());

        // Add an event
        let match1 = r#"{"match_id":1,"scores":[["Alice",50],["Bob",30]],"host_actor_id":"host1","completed":true}"#;
        storage.append_event("match_end", match1).unwrap();

        // Now caches need rebuild
        assert!(storage.caches_need_rebuild().unwrap());

        // Rebuild caches
        storage.rebuild_derived_caches().unwrap();

        // Now caches don't need rebuild
        assert!(!storage.caches_need_rebuild().unwrap());

        // Add another event
        let match2 = r#"{"match_id":2,"scores":[["Alice",40],["Bob",60]],"host_actor_id":"host1","completed":true}"#;
        storage.append_event("match_end", match2).unwrap();

        // Caches need rebuild again
        assert!(storage.caches_need_rebuild().unwrap());
    }

    #[test]
    fn test_word_claim_tracking_in_cache() {
        let storage = Storage::open_in_memory().unwrap();

        // Add word_claimed events
        let claim1 = r#"{"word":"ELEPHANT","player_name":"Alice","points":8}"#;
        let claim2 = r#"{"word":"CAT","player_name":"Alice","points":3}"#;
        let claim3 = r#"{"word":"DOG","player_name":"Bob","points":3}"#;

        storage.append_event("word_claimed", claim1).unwrap();
        storage.append_event("word_claimed", claim2).unwrap();
        storage.append_event("word_claimed", claim3).unwrap();

        storage.rebuild_derived_caches().unwrap();

        let alice_stats = storage.get_cached_stats("Alice").unwrap().unwrap();
        assert_eq!(alice_stats.words_claimed, 2);
        assert_eq!(alice_stats.longest_word, "ELEPHANT");

        let bob_stats = storage.get_cached_stats("Bob").unwrap().unwrap();
        assert_eq!(bob_stats.words_claimed, 1);
        assert_eq!(bob_stats.longest_word, "DOG");
    }

    // === JSON Helper Tests ===

    #[test]
    fn test_extract_json_string() {
        let json = r#"{"name":"Alice","score":50}"#;
        assert_eq!(extract_json_string(json, "name"), Some("Alice".to_string()));
    }

    #[test]
    fn test_extract_json_i64() {
        let json = r#"{"match_id":12345,"name":"test"}"#;
        assert_eq!(extract_json_i64(json, "match_id"), Some(12345));
    }

    #[test]
    fn test_extract_json_bool() {
        let json_true = r#"{"completed":true}"#;
        let json_false = r#"{"completed":false}"#;
        assert_eq!(extract_json_bool(json_true, "completed"), Some(true));
        assert_eq!(extract_json_bool(json_false, "completed"), Some(false));
    }

    #[test]
    fn test_extract_json_scores() {
        let json = r#"{"scores":[["Alice",50],["Bob",30]]}"#;
        let scores = extract_json_scores(json).unwrap();
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0], ("Alice".to_string(), 50));
        assert_eq!(scores[1], ("Bob".to_string(), 30));
    }

    #[test]
    fn test_parse_match_result_payload() {
        let payload = r#"{"match_id":123,"scores":[["Alice",50],["Bob",30]],"host_actor_id":"host1","completed":true}"#;
        let result = parse_match_result_payload(payload).unwrap();

        assert_eq!(result.match_id, 123);
        assert_eq!(result.scores.len(), 2);
        assert_eq!(result.scores[0], ("Alice".to_string(), 50));
        assert!(result.completed);
    }
}
