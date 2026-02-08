#![allow(dead_code)]
//! Network protocol message types
//!
//! Simple length-prefixed JSON messages over TCP.

use std::io::{self, Read, Write};
use std::net::TcpStream;

/// Reason a claim was rejected
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimRejectReason {
    /// Word was already claimed by another player
    AlreadyClaimed { by: String },
    /// Word is not in the dictionary
    NotInDictionary,
    /// Word uses letters not available in the rack
    InvalidLetters { missing: Vec<char> },
    /// Word is too short
    TooShort,
    /// Round has ended
    RoundEnded,
}

impl ClaimRejectReason {
    /// Get a user-friendly message for the rejection
    pub fn message(&self) -> String {
        match self {
            ClaimRejectReason::AlreadyClaimed { by } => {
                format!("Already claimed by {}", by)
            }
            ClaimRejectReason::NotInDictionary => "Not in dictionary".to_string(),
            ClaimRejectReason::InvalidLetters { missing } => {
                let letters: String = missing.iter().collect();
                format!("Missing letters: {}", letters)
            }
            ClaimRejectReason::TooShort => "Too short".to_string(),
            ClaimRejectReason::RoundEnded => "Round has ended".to_string(),
        }
    }
}

/// Messages sent between peers
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Announce player joining
    Join { player_name: String },
    /// Player is leaving
    Leave { player_name: String },
    /// Client requests to claim a word (client -> host)
    ClaimAttempt { word: String },
    /// Host accepts a claim and broadcasts to all (host -> all)
    ClaimAccepted {
        word: String,
        player_name: String,
        points: u32,
    },
    /// Host rejects a claim (host -> requester only)
    ClaimRejected {
        word: String,
        reason: ClaimRejectReason,
    },
    /// Word claimed event for CRDT log (host -> all)
    ///
    /// Contains additional metadata for deterministic ordering and
    /// event replay. Broadcast alongside ClaimAccepted for CRDT consistency.
    WordClaimed {
        /// The claimed word (uppercase)
        word: String,
        /// Player who claimed it
        player_name: String,
        /// Points awarded
        points: u32,
        /// Actor ID of the host that validated this claim
        actor_id: String,
        /// Timestamp when claim was validated (millis since epoch)
        timestamp_ms: u64,
        /// Monotonic sequence number for ordering within the round
        claim_sequence: u64,
    },
    /// Word claimed by a player (broadcast, legacy compatibility)
    Claim { player_name: String, word: String, points: u32 },
    /// Countdown to round start (3, 2, 1, BLAM!)
    Countdown { letters: Vec<char>, duration_secs: u32, countdown_secs: u32 },
    /// Round starting with these letters and duration
    RoundStart { letters: Vec<char>, duration_secs: u32 },
    /// Round has ended
    RoundEnd,
    /// Match completed event for CRDT log (host -> all)
    ///
    /// Contains final scores and match metadata for Elo calculations
    /// and stats tracking. Used for deterministic replay of match history.
    MatchEnded {
        /// Unique match ID (timestamp-based for deterministic ordering)
        match_id: i64,
        /// Final scores for each player: (player_handle, score)
        scores: Vec<(String, u32)>,
        /// Actor ID of the host that ran this match
        host_actor_id: String,
        /// Whether the match completed successfully
        completed: bool,
    },
    /// Scoreboard update (host -> all)
    ScoreUpdate { scores: Vec<(String, u32)> },
    /// Ping to check connection
    Ping,
    /// Response to ping
    Pong,
    /// CRDT sync: Request missing events by sending our vector clock
    /// Each entry is (actor_id_hex, highest_seq_seen)
    SyncRequest { vector_clock: Vec<(String, i64)> },
    /// CRDT sync: Send events the peer is missing
    SyncEvents { events: Vec<SyncEvent> },
}

/// An event for CRDT sync (matches storage::Event but with hex actor_id for JSON)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncEvent {
    /// Actor ID as hex string (32 chars)
    pub actor_id: String,
    /// Sequence number
    pub seq: i64,
    /// Event type
    pub event_type: String,
    /// JSON payload
    pub payload: String,
    /// Unix timestamp (ms)
    pub created_at: i64,
}

impl Message {
    /// Serialize message to bytes (length-prefixed JSON)
    pub fn to_bytes(&self) -> Vec<u8> {
        let json = self.to_json();
        let len = json.len() as u32;
        let mut bytes = Vec::with_capacity(4 + json.len());
        bytes.extend_from_slice(&len.to_be_bytes());
        bytes.extend_from_slice(json.as_bytes());
        bytes
    }

    /// Deserialize message from bytes (length-prefixed JSON)
    pub fn from_bytes(bytes: &[u8]) -> io::Result<(Self, usize)> {
        if bytes.len() < 4 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "need 4 bytes for length"));
        }
        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if bytes.len() < 4 + len {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "incomplete message"));
        }
        let json = std::str::from_utf8(&bytes[4..4 + len])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let msg = Self::from_json(json)?;
        Ok((msg, 4 + len))
    }

    fn to_json(&self) -> String {
        match self {
            Message::Join { player_name } => {
                format!(r#"{{"type":"join","player_name":"{}"}}"#, escape_json(player_name))
            }
            Message::Leave { player_name } => {
                format!(r#"{{"type":"leave","player_name":"{}"}}"#, escape_json(player_name))
            }
            Message::ClaimAttempt { word } => {
                format!(r#"{{"type":"claim_attempt","word":"{}"}}"#, escape_json(word))
            }
            Message::ClaimAccepted { word, player_name, points } => {
                format!(
                    r#"{{"type":"claim_accepted","word":"{}","player_name":"{}","points":{}}}"#,
                    escape_json(word),
                    escape_json(player_name),
                    points
                )
            }
            Message::ClaimRejected { word, reason } => {
                let reason_json = match reason {
                    ClaimRejectReason::AlreadyClaimed { by } => {
                        format!(r#"{{"reason":"already_claimed","by":"{}"}}"#, escape_json(by))
                    }
                    ClaimRejectReason::NotInDictionary => {
                        r#"{"reason":"not_in_dictionary"}"#.to_string()
                    }
                    ClaimRejectReason::InvalidLetters { missing } => {
                        let letters_json: String = missing.iter().map(|c| format!(r#""{}""#, c)).collect::<Vec<_>>().join(",");
                        format!(r#"{{"reason":"invalid_letters","missing":[{}]}}"#, letters_json)
                    }
                    ClaimRejectReason::TooShort => {
                        r#"{"reason":"too_short"}"#.to_string()
                    }
                    ClaimRejectReason::RoundEnded => {
                        r#"{"reason":"round_ended"}"#.to_string()
                    }
                };
                format!(
                    r#"{{"type":"claim_rejected","word":"{}","reason_data":{}}}"#,
                    escape_json(word),
                    reason_json
                )
            }
            Message::WordClaimed { word, player_name, points, actor_id, timestamp_ms, claim_sequence } => {
                format!(
                    r#"{{"type":"word_claimed","word":"{}","player_name":"{}","points":{},"actor_id":"{}","timestamp_ms":{},"claim_sequence":{}}}"#,
                    escape_json(word),
                    escape_json(player_name),
                    points,
                    escape_json(actor_id),
                    timestamp_ms,
                    claim_sequence
                )
            }
            Message::Claim { player_name, word, points } => {
                format!(
                    r#"{{"type":"claim","player_name":"{}","word":"{}","points":{}}}"#,
                    escape_json(player_name),
                    escape_json(word),
                    points
                )
            }
            Message::Countdown { letters, duration_secs, countdown_secs } => {
                let letters_json: String = letters.iter().map(|c| format!(r#""{}""#, c)).collect::<Vec<_>>().join(",");
                format!(
                    r#"{{"type":"countdown","letters":[{}],"duration_secs":{},"countdown_secs":{}}}"#,
                    letters_json,
                    duration_secs,
                    countdown_secs
                )
            }
            Message::RoundStart { letters, duration_secs } => {
                let letters_json: String = letters.iter().map(|c| format!(r#""{}""#, c)).collect::<Vec<_>>().join(",");
                format!(
                    r#"{{"type":"round_start","letters":[{}],"duration_secs":{}}}"#,
                    letters_json,
                    duration_secs
                )
            }
            Message::RoundEnd => r#"{"type":"round_end"}"#.to_string(),
            Message::MatchEnded { match_id, scores, host_actor_id, completed } => {
                let scores_json: String = scores
                    .iter()
                    .map(|(name, score)| format!(r#"["{}",{}]"#, escape_json(name), score))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    r#"{{"type":"match_ended","match_id":{},"scores":[{}],"host_actor_id":"{}","completed":{}}}"#,
                    match_id,
                    scores_json,
                    escape_json(host_actor_id),
                    completed
                )
            }
            Message::ScoreUpdate { scores } => {
                let scores_json: String = scores
                    .iter()
                    .map(|(name, score)| format!(r#"["{}",{}]"#, escape_json(name), score))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(r#"{{"type":"score_update","scores":[{}]}}"#, scores_json)
            }
            Message::Ping => r#"{"type":"ping"}"#.to_string(),
            Message::Pong => r#"{"type":"pong"}"#.to_string(),
            Message::SyncRequest { vector_clock } => {
                let clock_json: String = vector_clock
                    .iter()
                    .map(|(actor_id, seq)| format!(r#"["{}",{}]"#, escape_json(actor_id), seq))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(r#"{{"type":"sync_request","vector_clock":[{}]}}"#, clock_json)
            }
            Message::SyncEvents { events } => {
                let events_json: String = events
                    .iter()
                    .map(|e| {
                        format!(
                            r#"{{"actor_id":"{}","seq":{},"event_type":"{}","payload":"{}","created_at":{}}}"#,
                            escape_json(&e.actor_id),
                            e.seq,
                            escape_json(&e.event_type),
                            escape_json(&e.payload),
                            e.created_at
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!(r#"{{"type":"sync_events","events":[{}]}}"#, events_json)
            }
        }
    }

    fn from_json(json: &str) -> io::Result<Self> {
        // Simple JSON parsing without serde
        let json = json.trim();

        let get_str = |key: &str| -> Option<String> {
            let pattern = format!(r#""{}":""#, key);
            let start = json.find(&pattern)? + pattern.len();
            let rest = &json[start..];
            // Find the closing quote that isn't escaped
            let end = find_unescaped_quote(rest)?;
            Some(unescape_json(&rest[..end]))
        };

        let get_u32 = |key: &str| -> Option<u32> {
            let pattern = format!(r#""{}":"#, key);
            let start = json.find(&pattern)? + pattern.len();
            let rest = &json[start..];
            let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            rest[..end].parse().ok()
        };

        let get_u64 = |key: &str| -> Option<u64> {
            let pattern = format!(r#""{}":"#, key);
            let start = json.find(&pattern)? + pattern.len();
            let rest = &json[start..];
            let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
            rest[..end].parse().ok()
        };

        let get_i64 = |key: &str| -> Option<i64> {
            let pattern = format!(r#""{}":"#, key);
            let start = json.find(&pattern)? + pattern.len();
            let rest = &json[start..];
            let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(rest.len());
            rest[..end].parse().ok()
        };

        let get_bool = |key: &str| -> Option<bool> {
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
        };

        let get_chars = |key: &str| -> Option<Vec<char>> {
            let pattern = format!(r#""{}":["#, key);
            let start = json.find(&pattern)? + pattern.len();
            let rest = &json[start..];
            let end = rest.find(']')?;
            let array = &rest[..end];
            Some(
                array
                    .split(',')
                    .filter_map(|s| {
                        let s = s.trim().trim_matches('"');
                        s.chars().next()
                    })
                    .collect()
            )
        };

        // Parse scores array [[name, score], ...]
        let get_scores = || -> Option<Vec<(String, u32)>> {
            let pattern = r#""scores":[["#;
            let start = json.find(pattern)?;
            let rest = &json[start + r#""scores":["#.len()..];
            let end = rest.find("]]")?;
            let array = &rest[..end + 1]; // Include last ]

            let mut scores = Vec::new();
            let mut current = array;
            while let Some(start) = current.find('[') {
                let rest = &current[start + 1..];
                let end = rest.find(']')?;
                let item = &rest[..end];

                // Parse ["name", score]
                let comma = item.find(',')?;
                let name = item[..comma].trim().trim_matches('"');
                let score_str = item[comma + 1..].trim();
                let score: u32 = score_str.parse().ok()?;
                scores.push((unescape_json(name), score));

                if end + 1 < rest.len() {
                    current = &rest[end + 1..];
                } else {
                    break;
                }
            }
            Some(scores)
        };

        // Get type field
        let msg_type = get_str("type")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing type field"))?;

        match msg_type.as_str() {
            "join" => {
                let player_name = get_str("player_name")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing player_name"))?;
                Ok(Message::Join { player_name })
            }
            "leave" => {
                let player_name = get_str("player_name")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing player_name"))?;
                Ok(Message::Leave { player_name })
            }
            "claim_attempt" => {
                let word = get_str("word")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing word"))?;
                Ok(Message::ClaimAttempt { word })
            }
            "claim_accepted" => {
                let word = get_str("word")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing word"))?;
                let player_name = get_str("player_name")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing player_name"))?;
                let points = get_u32("points")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing points"))?;
                Ok(Message::ClaimAccepted { word, player_name, points })
            }
            "claim_rejected" => {
                let word = get_str("word")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing word"))?;

                // Parse the reason from reason_data
                let reason_str = get_str("reason")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing reason"))?;

                let reason = match reason_str.as_str() {
                    "already_claimed" => {
                        let by = get_str("by")
                            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing by"))?;
                        ClaimRejectReason::AlreadyClaimed { by }
                    }
                    "not_in_dictionary" => ClaimRejectReason::NotInDictionary,
                    "invalid_letters" => {
                        let missing = get_chars("missing").unwrap_or_default();
                        ClaimRejectReason::InvalidLetters { missing }
                    }
                    "too_short" => ClaimRejectReason::TooShort,
                    "round_ended" => ClaimRejectReason::RoundEnded,
                    _ => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown reason: {}", reason_str))),
                };

                Ok(Message::ClaimRejected { word, reason })
            }
            "word_claimed" => {
                let word = get_str("word")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing word"))?;
                let player_name = get_str("player_name")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing player_name"))?;
                let points = get_u32("points")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing points"))?;
                let actor_id = get_str("actor_id")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing actor_id"))?;
                let timestamp_ms = get_u64("timestamp_ms")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing timestamp_ms"))?;
                let claim_sequence = get_u64("claim_sequence")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing claim_sequence"))?;
                Ok(Message::WordClaimed {
                    word,
                    player_name,
                    points,
                    actor_id,
                    timestamp_ms,
                    claim_sequence,
                })
            }
            "claim" => {
                let player_name = get_str("player_name")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing player_name"))?;
                let word = get_str("word")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing word"))?;
                let points = get_u32("points")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing points"))?;
                Ok(Message::Claim { player_name, word, points })
            }
            "countdown" => {
                let letters = get_chars("letters")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing letters"))?;
                let duration_secs = get_u32("duration_secs")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing duration_secs"))?;
                let countdown_secs = get_u32("countdown_secs")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing countdown_secs"))?;
                Ok(Message::Countdown { letters, duration_secs, countdown_secs })
            }
            "round_start" => {
                let letters = get_chars("letters")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing letters"))?;
                let duration_secs = get_u32("duration_secs")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing duration_secs"))?;
                Ok(Message::RoundStart { letters, duration_secs })
            }
            "round_end" => Ok(Message::RoundEnd),
            "match_ended" => {
                let match_id = get_i64("match_id")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing match_id"))?;
                let scores = get_scores()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing or invalid scores"))?;
                let host_actor_id = get_str("host_actor_id")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing host_actor_id"))?;
                let completed = get_bool("completed").unwrap_or(true);
                Ok(Message::MatchEnded {
                    match_id,
                    scores,
                    host_actor_id,
                    completed,
                })
            }
            "score_update" => {
                let scores = get_scores()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing or invalid scores"))?;
                Ok(Message::ScoreUpdate { scores })
            }
            "ping" => Ok(Message::Ping),
            "pong" => Ok(Message::Pong),
            "sync_request" => {
                let vector_clock = parse_vector_clock(json)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid vector_clock"))?;
                Ok(Message::SyncRequest { vector_clock })
            }
            "sync_events" => {
                let events = parse_sync_events(json)
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid events"))?;
                Ok(Message::SyncEvents { events })
            }
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown message type: {}", msg_type))),
        }
    }

    /// Write message to a TCP stream
    pub fn write_to(&self, stream: &mut TcpStream) -> io::Result<()> {
        let bytes = self.to_bytes();
        stream.write_all(&bytes)?;
        stream.flush()
    }

    /// Read message from a TCP stream
    pub fn read_from(stream: &mut TcpStream) -> io::Result<Self> {
        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf)?;
        let len = u32::from_be_bytes(len_buf) as usize;

        if len > 1024 * 1024 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
        }

        let mut body = vec![0u8; len];
        stream.read_exact(&mut body)?;

        let json = std::str::from_utf8(&body)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Self::from_json(json)
    }
}

/// Find the position of the first unescaped quote in a string
fn find_unescaped_quote(s: &str) -> Option<usize> {
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'"' {
            return Some(i);
        } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
            // Skip escaped character
            i += 2;
        } else {
            i += 1;
        }
    }
    None
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Parse vector clock from JSON: [["actor_hex", seq], ...]
fn parse_vector_clock(json: &str) -> Option<Vec<(String, i64)>> {
    let pattern = r#""vector_clock":["#;
    let start = json.find(pattern)? + pattern.len();
    let rest = &json[start..];

    // Find matching close bracket
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
    if array.is_empty() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    let mut current = array;

    while let Some(start) = current.find('[') {
        let rest = &current[start + 1..];
        let end = rest.find(']')?;
        let item = &rest[..end];

        // Parse ["actor_hex", seq]
        let comma = item.find(',')?;
        let actor_id = item[..comma].trim().trim_matches('"');
        let seq_str = item[comma + 1..].trim();
        let seq: i64 = seq_str.parse().ok()?;
        result.push((unescape_json(actor_id), seq));

        if end + 1 < rest.len() {
            current = &rest[end + 1..];
        } else {
            break;
        }
    }

    Some(result)
}

/// Parse sync events from JSON: [{actor_id, seq, event_type, payload, created_at}, ...]
fn parse_sync_events(json: &str) -> Option<Vec<SyncEvent>> {
    let pattern = r#""events":["#;
    let start = json.find(pattern)? + pattern.len();
    let rest = &json[start..];

    // Find matching close bracket, respecting string boundaries
    let mut depth = 1;
    let mut end = 0;
    let mut in_string = false;
    let mut prev_char = ' ';
    for (i, c) in rest.chars().enumerate() {
        if c == '"' && prev_char != '\\' {
            in_string = !in_string;
        } else if !in_string {
            match c {
                '[' | '{' => depth += 1,
                ']' | '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        prev_char = c;
    }

    let array = &rest[..end];
    if array.is_empty() {
        return Some(Vec::new());
    }

    let mut result = Vec::new();
    let mut current = array;

    while let Some(obj_start) = current.find('{') {
        let rest = &current[obj_start + 1..];
        // Find matching close brace, respecting strings
        let mut depth = 1;
        let mut obj_end = 0;
        let mut in_string = false;
        let mut prev_char = ' ';
        for (i, c) in rest.chars().enumerate() {
            if c == '"' && prev_char != '\\' {
                in_string = !in_string;
            } else if !in_string {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            obj_end = i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            prev_char = c;
        }
        let obj = &rest[..obj_end];

        // Parse object fields
        let get_str = |key: &str| -> Option<String> {
            let pattern = format!(r#""{}":""#, key);
            let s = obj.find(&pattern)? + pattern.len();
            let r = &obj[s..];
            let e = find_unescaped_quote(r)?;
            Some(unescape_json(&r[..e]))
        };

        let get_i64 = |key: &str| -> Option<i64> {
            let pattern = format!(r#""{}":"#, key);
            let s = obj.find(&pattern)? + pattern.len();
            let r = &obj[s..];
            let e = r.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(r.len());
            r[..e].parse().ok()
        };

        let event = SyncEvent {
            actor_id: get_str("actor_id")?,
            seq: get_i64("seq")?,
            event_type: get_str("event_type")?,
            payload: get_str("payload")?,
            created_at: get_i64("created_at")?,
        };
        result.push(event);

        if obj_end + 1 < rest.len() {
            current = &rest[obj_end + 1..];
        } else {
            break;
        }
    }

    Some(result)
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
    fn test_join_roundtrip() {
        let msg = Message::Join { player_name: "Alice".to_string() };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_claim_roundtrip() {
        let msg = Message::Claim {
            player_name: "Bob".to_string(),
            word: "BLAM".to_string(),
            points: 4,
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_round_start_roundtrip() {
        let msg = Message::RoundStart {
            letters: vec!['B', 'L', 'A', 'M'],
            duration_secs: 60,
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_ping_pong() {
        let ping = Message::Ping;
        let pong = Message::Pong;

        let (parsed_ping, _) = Message::from_bytes(&ping.to_bytes()).unwrap();
        let (parsed_pong, _) = Message::from_bytes(&pong.to_bytes()).unwrap();

        assert_eq!(parsed_ping, Message::Ping);
        assert_eq!(parsed_pong, Message::Pong);
    }

    #[test]
    fn test_escape_special_chars() {
        let msg = Message::Join { player_name: "Test\"User".to_string() };
        let bytes = msg.to_bytes();
        let (parsed, _) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn test_claim_attempt_roundtrip() {
        let msg = Message::ClaimAttempt { word: "BLAM".to_string() };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_claim_accepted_roundtrip() {
        let msg = Message::ClaimAccepted {
            word: "BLAM".to_string(),
            player_name: "Alice".to_string(),
            points: 4,
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_claim_rejected_already_claimed() {
        let msg = Message::ClaimRejected {
            word: "BLAM".to_string(),
            reason: ClaimRejectReason::AlreadyClaimed { by: "Bob".to_string() },
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_word_claimed_roundtrip() {
        let msg = Message::WordClaimed {
            word: "BLAM".to_string(),
            player_name: "Alice".to_string(),
            points: 4,
            actor_id: "blam-12345678".to_string(),
            timestamp_ms: 1704067200000,
            claim_sequence: 42,
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_match_ended_roundtrip() {
        let msg = Message::MatchEnded {
            match_id: 1704067200000,
            scores: vec![
                ("Alice".to_string(), 50),
                ("Bob".to_string(), 30),
            ],
            host_actor_id: "host123".to_string(),
            completed: true,
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_match_ended_incomplete() {
        let msg = Message::MatchEnded {
            match_id: 1704067200000,
            scores: vec![("Alice".to_string(), 25)],
            host_actor_id: "host123".to_string(),
            completed: false,
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_claim_rejected_invalid_letters() {
        let msg = Message::ClaimRejected {
            word: "TEST".to_string(),
            reason: ClaimRejectReason::InvalidLetters { missing: vec!['X', 'Y'] },
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_score_update_roundtrip() {
        let msg = Message::ScoreUpdate {
            scores: vec![
                ("Alice".to_string(), 15),
                ("Bob".to_string(), 12),
            ],
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_countdown_roundtrip() {
        let msg = Message::Countdown {
            letters: vec!['B', 'L', 'A', 'M'],
            duration_secs: 60,
            countdown_secs: 3,
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_sync_request_roundtrip() {
        let msg = Message::SyncRequest {
            vector_clock: vec![
                ("0123456789abcdef0123456789abcdef".to_string(), 5),
                ("fedcba9876543210fedcba9876543210".to_string(), 10),
            ],
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_sync_request_empty() {
        let msg = Message::SyncRequest { vector_clock: vec![] };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_sync_events_roundtrip() {
        let msg = Message::SyncEvents {
            events: vec![
                SyncEvent {
                    actor_id: "0123456789abcdef0123456789abcdef".to_string(),
                    seq: 1,
                    event_type: "round_start".to_string(),
                    payload: r#"{"letters":["B","L","A","M"]}"#.to_string(),
                    created_at: 1700000000000,
                },
                SyncEvent {
                    actor_id: "0123456789abcdef0123456789abcdef".to_string(),
                    seq: 2,
                    event_type: "claim".to_string(),
                    payload: r#"{"word":"BLAM","player":"Alice"}"#.to_string(),
                    created_at: 1700000001000,
                },
            ],
        };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_sync_events_empty() {
        let msg = Message::SyncEvents { events: vec![] };
        let bytes = msg.to_bytes();
        let (parsed, len) = Message::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, msg);
        assert_eq!(len, bytes.len());
    }

    #[test]
    fn test_claim_reject_reason_messages() {
        assert_eq!(
            ClaimRejectReason::AlreadyClaimed { by: "Alice".to_string() }.message(),
            "Already claimed by Alice"
        );
        assert_eq!(
            ClaimRejectReason::NotInDictionary.message(),
            "Not in dictionary"
        );
        assert_eq!(
            ClaimRejectReason::InvalidLetters { missing: vec!['X', 'Y'] }.message(),
            "Missing letters: XY"
        );
        assert_eq!(
            ClaimRejectReason::TooShort.message(),
            "Too short"
        );
        assert_eq!(
            ClaimRejectReason::RoundEnded.message(),
            "Round has ended"
        );
    }
}
