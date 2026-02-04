//! Network protocol message types
//!
//! Simple length-prefixed JSON messages over TCP.

use std::io::{self, Read, Write};
use std::net::TcpStream;

/// Messages sent between peers
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Announce player joining
    Join { player_name: String },
    /// Player is leaving
    Leave { player_name: String },
    /// Word claimed by a player
    Claim { player_name: String, word: String, points: u32 },
    /// Round starting with these letters and duration
    RoundStart { letters: Vec<char>, duration_secs: u32 },
    /// Round has ended
    RoundEnd,
    /// Ping to check connection
    Ping,
    /// Response to ping
    Pong,
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
            Message::Claim { player_name, word, points } => {
                format!(
                    r#"{{"type":"claim","player_name":"{}","word":"{}","points":{}}}"#,
                    escape_json(player_name),
                    escape_json(word),
                    points
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
            Message::Ping => r#"{"type":"ping"}"#.to_string(),
            Message::Pong => r#"{"type":"pong"}"#.to_string(),
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
            "claim" => {
                let player_name = get_str("player_name")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing player_name"))?;
                let word = get_str("word")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing word"))?;
                let points = get_u32("points")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing points"))?;
                Ok(Message::Claim { player_name, word, points })
            }
            "round_start" => {
                let letters = get_chars("letters")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing letters"))?;
                let duration_secs = get_u32("duration_secs")
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing duration_secs"))?;
                Ok(Message::RoundStart { letters, duration_secs })
            }
            "round_end" => Ok(Message::RoundEnd),
            "ping" => Ok(Message::Ping),
            "pong" => Ok(Message::Pong),
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
}
